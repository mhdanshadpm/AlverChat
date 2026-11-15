//! The desktop half of chunked file transfer: state, drivers and the paths bytes live on.
//!
//! [`alver_core::file_transfer`] holds the protocol; this file decides what it means for
//! one running app. Two rules shape everything below.
//!
//! * The receiver pulls. It opens the session, names the first chunk it still needs and
//!   only then does the sender stream. A pause, a crash or a reconnect therefore costs
//!   nothing, and a sender never queues bytes nobody is reading.
//! * The database row is the user's intent, the in-memory map is the driver's progress.
//!   A driver reads the row between batches to find out whether it should stop and never
//!   writes it mid-run: a driver that wrote `transferring` back would resurrect a pause the
//!   user clicked a moment earlier.

use alver_core::discovery::PeerBeacon;
use alver_core::file_transfer::{
    frame, push_accepted, receive_chunks, Gate, ReceiveRequest, SendRequest, TransferAddress,
    TransferUpdate, CANCEL_REASON, PAUSE_REASON,
};
use alver_core::files::{self, FileOffer, Transfer, TransferPolicy, TransferState};
use alver_core::payloads::{
    kind, FileAcceptPayload, FileCancelPayload, FileOfferPayload, MAX_BODY_BYTES,
};
use alver_core::security::validate_received_file_name;
use alver_core::store_ext::Attachment;
use alver_core::transport::{RemoteIdentity, SecureStream};
use alver_core::{Envelope, Hlc, Message, PROTOCOL_VERSION};
use base64::Engine as _;
use serde::de::DeserializeOwned;
use serde_json::Value;
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    thread,
};
use tauri::{AppHandle, Emitter, Manager};
use uuid::Uuid;

use crate::{commands, now_ms, open_session, platform, publish, OverlayState};

/// How far a driver has come, as a whole percentage, kept out of the database so a running
/// transfer does not fight the UI for the store lock on every batch.
#[derive(Clone, Copy, Debug)]
pub(crate) struct LiveTransfer {
    pub(crate) percent: u32,
    /// When this progress was last announced, for the rate limit in [`tick`].
    ticked_ms: u64,
}

/// Publishing cadence for a running transfer: enough for a moving bar, few enough that the
/// hundreds of batch boundaries in a big file cannot drown the webview in refetches.
const PROGRESS_EVENT_MS: u64 = 500;

/// True for the frames the file engine owns, so the session guard lets them through.
#[must_use]
pub(crate) fn handled_kind(name: &str) -> bool {
    matches!(
        name,
        kind::FILE_OFFER
            | kind::FILE_ACCEPT
            | kind::FILE_CHUNK
            | kind::FILE_ACK
            | kind::FILE_CANCEL
    )
}

// ------------------------------------------------------------------------------------
// Local plumbing: locks, ids, folders, policy.
// ------------------------------------------------------------------------------------

fn store<'a>(
    app: &'a AppHandle,
) -> Result<std::sync::MutexGuard<'a, alver_core::storage::Store>, String> {
    app.state::<OverlayState>()
        .inner()
        .store
        .lock()
        .map_err(|_| "message store lock poisoned".into())
}

fn my_id(app: &AppHandle) -> String {
    app.state::<OverlayState>().identity.peer_id()
}

fn endpoint(app: &AppHandle, peer_id: &str) -> Result<PeerBeacon, String> {
    let state = app.state::<OverlayState>();
    let found = state
        .endpoints
        .lock()
        .map_err(|_| "peer endpoint lock poisoned")?
        .get(peer_id)
        .cloned();
    found.ok_or_else(|| "This peer is no longer available on the local network.".to_owned())
}

/// The peer must be known right now, exactly as a composer message insists, so a file can
/// never be reported as sent when the network has nowhere to put it.
pub(crate) fn require_peer(app: &AppHandle, chat_id: &str) -> Result<PeerBeacon, String> {
    endpoint(app, chat_id)
}

fn row_opt(app: &AppHandle, id: Uuid) -> Result<Option<Attachment>, String> {
    store(app)?.attachment(id)
}

/// The attachment row behind one id, for the commands that answer with a DTO.
pub(crate) fn row_now(app: &AppHandle, id: Uuid) -> Result<Attachment, String> {
    row_opt(app, id)?.ok_or_else(|| "That file is not in this database.".to_owned())
}

/// The size limits the user actually edits, which live in `settings.json` rather than in
/// the store's settings rows.
pub(crate) fn policy(app: &AppHandle) -> TransferPolicy {
    let settings = app.state::<platform::SettingsState>().snapshot();
    // A zero cap would refuse every file in silence, so an unset limit means the default.
    let max = if settings.max_file_size_mb == 0 {
        files::DEFAULT_MAX_FILE_BYTES
    } else {
        megabytes(settings.max_file_size_mb)
    };
    TransferPolicy::new(max, megabytes(settings.auto_download_limit_mb))
}

fn megabytes(value: u32) -> u64 {
    u64::from(value) * 1024 * 1024
}

/// Finished downloads: the folder the user chose, else a subfolder of the app data.
pub(crate) fn download_root(app: &AppHandle) -> Result<PathBuf, String> {
    let configured = app
        .state::<platform::SettingsState>()
        .snapshot()
        .downloads_dir;
    let root = if configured.trim().is_empty() {
        crate::instance_data_dir(app)?.join("downloads")
    } else {
        PathBuf::from(configured)
    };
    prepare(&root)
}

/// The copies of the files this device offers. They stay here because the peer may pull at
/// any time, and a sender that dropped its copy could never answer again.
fn outgoing_root(app: &AppHandle) -> Result<PathBuf, String> {
    prepare(&crate::instance_data_dir(app)?.join("outgoing"))
}

/// Creates `directory` when needed and returns the canonical path the engines insist on.
fn prepare(directory: &Path) -> Result<PathBuf, String> {
    fs::create_dir_all(directory)
        .map_err(|error| format!("cannot create {}: {error}", directory.display()))?;
    let root = fs::canonicalize(directory).map_err(|error| error.to_string())?;
    if !root.is_dir() {
        return Err(format!("{} is not a directory", root.display()));
    }
    Ok(root)
}

/// Where one transfer's bytes live on this device, named by its id so a re-offer under a
/// different file name can never clobber them.
fn local_path(app: &AppHandle, attachment: &Attachment) -> Result<PathBuf, String> {
    let root = if is_outgoing(app, attachment)? {
        outgoing_root(app)?
    } else {
        download_root(app)?
    };
    Ok(root.join(files::stored_name(attachment.id, &attachment.file_name)))
}

/// The file itself, checked against the disk rather than the stored state: "completed"
/// only means it landed, and the user is free to move it afterwards.
pub(crate) fn file_path(app: &AppHandle, attachment: &Attachment) -> Result<PathBuf, String> {
    let path = local_path(app, attachment)?;
    if path.is_file() {
        Ok(path)
    } else {
        Err("The file is not on this device any more.".into())
    }
}

/// A file attached to a message we wrote is ours; only the message's sender settles that,
/// and an attachment without a message cannot have been sent by us.
pub(crate) fn is_outgoing(app: &AppHandle, attachment: &Attachment) -> Result<bool, String> {
    let Some(message_id) = attachment.message_id else {
        return Ok(false);
    };
    Ok(store(app)?
        .get_message(message_id)?
        .is_some_and(|message| message.sender == commands::ME))
}

/// The bridge's 0-1 progress: live numbers while a driver runs, the resume index once it
/// is not running, nothing once the file is complete.
pub(crate) fn progress(app: &AppHandle, attachment: &Attachment, outgoing: bool) -> Option<f64> {
    if attachment.state == TransferState::Completed {
        return None;
    }
    let live = app
        .state::<OverlayState>()
        .inner()
        .transfers
        .lock()
        .ok()
        .and_then(|live| live.get(&attachment.id).copied());
    if let Some(live) = live {
        return Some(f64::from(live.percent) / 100.0);
    }
    // An upload has no local evidence of what the peer kept, but a download's verified
    // chunks are indexed beside the part file, so a stopped or restarted one still shows
    // how far it came.
    if !outgoing {
        if let Ok(root) = download_root(app) {
            let (_, bytes) = files::resume_progress(&root, attachment.id, attachment.size_bytes);
            if attachment.size_bytes > 0 {
                return Some((bytes as f64 / attachment.size_bytes as f64).min(1.0));
            }
        }
    }
    Some(0.0)
}

// ------------------------------------------------------------------------------------
// State, progress and the two drivers.
// ------------------------------------------------------------------------------------

/// The one signal the shell already refetches on: a transfer is a message with a card.
fn announce(app: &AppHandle, chat_id: &str) {
    let _ = app.emit("messages-changed", chat_id.to_owned());
}

/// Publishing a running transfer's percentage, at most twice a second.
///
/// A chunk is 40 KiB, so a big file crosses hundreds of batch boundaries; emitting on every
/// one would drown our own webview in refetches of the whole thread. The stored number is
/// always current, so whatever the UI does read is the truth.
fn tick(app: &AppHandle, attachment: &Attachment, percent: u32) {
    let state = app.state::<OverlayState>();
    let Ok(mut live) = state.transfers.lock() else {
        return;
    };
    let stamp = now_ms();
    let held = live.get(&attachment.id).copied();
    let changed = held.is_none_or(|past| past.percent != percent);
    // A transfer that has never announced itself has no clock, so its first tick passes.
    let quiet = held.is_some_and(|past| stamp.saturating_sub(past.ticked_ms) < PROGRESS_EVENT_MS);
    let chat_id = attachment.chat_id.clone();
    live.insert(
        attachment.id,
        LiveTransfer {
            percent,
            ticked_ms: held.map_or(stamp, |past| if quiet { past.ticked_ms } else { stamp }),
        },
    );
    drop(live);
    if changed && !quiet {
        let _ = app.emit("messages-changed", chat_id);
    }
}

/// Moves one transfer to `state`, dropping its live progress once it stops running.
pub(crate) fn set_state(app: &AppHandle, id: Uuid, state: TransferState) -> Result<(), String> {
    let mut guard = store(app)?;
    let Some(mut attachment) = guard.attachment(id)? else {
        return Err("That file is not in this database.".into());
    };
    if attachment.state == state {
        return Ok(());
    }
    attachment.state = state;
    attachment.updated_ms = now_ms();
    let chat_id = attachment.chat_id.clone();
    guard.upsert_attachment(&attachment)?;
    drop(guard);
    // Only a running transfer has a live number. `Failed` is not terminal, so a paused,
    // failed or waiting download reports from its resume index instead of a stale percent.
    if state != TransferState::Transferring {
        if let Ok(mut live) = app.state::<OverlayState>().inner().transfers.lock() {
            live.remove(&id);
        }
    }
    announce(app, &chat_id);
    Ok(())
}

/// Records what a driver just did: the state the row has to carry.
fn persist(app: &AppHandle, id: Uuid, update: &TransferUpdate) -> Result<(), String> {
    if let Some(reason) = update.reason.as_deref() {
        eprintln!("File transfer {id} stopped: {reason}");
    }
    set_state(app, id, update.state)
}

/// What happens when a driver gives up: the row has to say so, or the bubble keeps
/// spinning on a thread that no longer exists.
fn fail(app: &AppHandle, id: Uuid, transfer: &Transfer, error: String) -> Result<(), String> {
    let progress = transfer.progress();
    let update = TransferUpdate {
        state: if transfer.state() == TransferState::Cancelled {
            TransferState::Cancelled
        } else {
            TransferState::Failed
        },
        next_index: transfer.next_index(),
        transferred_bytes: progress.received_bytes,
        total_bytes: progress.total_bytes,
        percent: progress.percent,
        path: None,
        reason: Some(error.clone()),
    };
    let _ = persist(app, id, &update);
    Err(error)
}

/// The driver's checkpoint between batches: publish progress, then obey whatever the user
/// clicked. It deliberately never writes state.
fn gate(app: AppHandle, id: Uuid) -> impl FnMut(&TransferUpdate) -> Gate {
    move |update| {
        let Ok(attachment) = row_now(&app, id) else {
            return Gate::Proceed;
        };
        tick(&app, &attachment, update.percent);
        match attachment.state {
            TransferState::Paused => Gate::Pause,
            TransferState::Cancelled => Gate::Cancel,
            _ => Gate::Proceed,
        }
    }
}

fn address(app: &AppHandle, peer_id: &str) -> TransferAddress {
    TransferAddress {
        from: my_id(app),
        to: peer_id.to_owned(),
    }
}

/// Requests and stores every chunk the peer still owes for one incoming transfer.
fn pull(app: AppHandle, id: Uuid) -> Result<(), String> {
    let attachment = row_now(&app, id)?;
    let Some(blake3_hash) = attachment.blake3_hash else {
        return Err("The offer for this file carries no hash.".into());
    };
    let offer = FileOffer {
        id,
        file_name: attachment.file_name.clone(),
        size: attachment.size_bytes,
        blake3_hash,
    };
    let mut transfer = Transfer::incoming(&download_root(&app)?, offer, policy(&app))
        .map_err(|error| format!("Cannot prepare the download folder: {error}"))?;
    if transfer.receive_offer().is_err() {
        return fail(
            &app,
            id,
            &transfer,
            "The file is over the size limit set on this device.".into(),
        );
    }
    if transfer.state() != TransferState::Accepted {
        // Clicking download outranks the auto-download limit; that is all an accept means.
        transfer
            .accept()
            .map_err(|error| format!("Cannot accept this file: {error}"))?;
    }
    transfer
        .begin()
        .map_err(|error| format!("Cannot open the download folder: {error}"))?;
    set_state(&app, id, TransferState::Transferring)?;
    let beacon = match endpoint(&app, &attachment.chat_id) {
        Ok(beacon) => beacon,
        Err(error) => return fail(&app, id, &transfer, error),
    };
    let mut session = match open_session(&app, &beacon) {
        Ok(session) => session,
        Err(error) => return fail(&app, id, &transfer, error),
    };
    let target = address(&app, &attachment.chat_id);
    let request = ReceiveRequest {
        address: &target,
        clock_ms: now_ms(),
    };
    match receive_chunks(&mut session, &mut transfer, &request, gate(app.clone(), id)) {
        Ok(update) => persist(&app, id, &update),
        Err(error) => fail(&app, id, &transfer, error),
    }
}

/// Starts one download thread; a second click while a driver runs is a no-op, exactly the
/// way `retrying_peers` keeps a single outbox retry loop per peer.
pub(crate) fn start_pull(app: AppHandle, id: Uuid) {
    let claimed = app
        .state::<OverlayState>()
        .inner()
        .pulling
        .lock()
        .map(|mut pulling| pulling.insert(id))
        .unwrap_or(false);
    if !claimed {
        return;
    }
    let worker = app.clone();
    let releasing = app.clone();
    let spawned = thread::Builder::new()
        .name("alverchat-file-pull".into())
        .spawn(move || {
            if let Err(error) = pull(worker, id) {
                eprintln!("File download ended: {error}");
            }
            release(&releasing, id);
        });
    if spawned.is_err() {
        release(&app, id);
    }
}

/// Hands a transfer id back so the next click can start a driver again.
fn release(app: &AppHandle, id: Uuid) {
    if let Ok(mut pulling) = app.state::<OverlayState>().inner().pulling.lock() {
        pulling.remove(&id);
    }
}

// ------------------------------------------------------------------------------------
// Inbound frames.
// ------------------------------------------------------------------------------------

/// The typed body of one inbound frame.
///
/// A file frame names its transfer inside the payload, never as the envelope id: the core
/// protocol gives every accept, chunk, ack and cancel a fresh id of its own, so only an
/// offer, which this app builds itself, can tie the two together.
fn payload<T: DeserializeOwned>(envelope: &Envelope<Value>) -> Result<T, String> {
    serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("{} frame is malformed: {error}", envelope.kind))
}

/// The `file_cancel` sent when a transfer is not going to happen, or is over.
fn cancel_frame(
    app: &AppHandle,
    peer_id: &str,
    transfer_id: Uuid,
    reason: &str,
) -> Result<Envelope<Value>, String> {
    let target = address(app, peer_id);
    frame(
        &target,
        kind::FILE_CANCEL,
        &FileCancelPayload {
            transfer_id,
            reason: Some(reason.to_owned()),
        },
        Hlc {
            physical_ms: now_ms(),
            logical: 0,
        },
    )
}

/// Stores a `file_offer` and returns the frames to send back after its receipt.
///
/// An offer carries no bytes, so this is bookkeeping: the message and attachment rows
/// appear, small files start downloading, and anything the policy forbids is answered with
/// a `file_cancel` so the sender stops holding a file that will never arrive.
///
/// A repeat offer for an id already stored is not ignored: it is the receiver's cue that
/// the sender is ready again, so a download the user has asked for starts right here. What
/// makes an offer safe to redo is that it names a transfer, not new bytes.
pub(crate) fn handle_offer(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<Value>,
) -> Result<Vec<Envelope<Value>>, String> {
    let offer: FileOfferPayload = payload(envelope)?;
    // `chat_id` is the receiver the sender addressed, so it must name this device.
    if offer.chat_id != my_id(app) {
        return Err("peer offered a file that is not addressed to this device".into());
    }
    let Ok(blake3_hash) = <[u8; 32]>::try_from(offer.blake3_hash.as_slice()) else {
        return Err("peer offered a file without a blake3 hash".into());
    };
    let id = offer.transfer_id;
    if offer.size == 0 || validate_received_file_name(&offer.file_name).is_err() {
        // Nothing can be stored for a name this device cannot use, so this one is refused
        // by frame alone.
        return Ok(vec![cancel_frame(
            app,
            remote,
            id,
            "The offer names an empty file or an unsafe file name.",
        )?]);
    }
    if let Some(existing) = row_opt(app, id)? {
        if existing.file_name != offer.file_name || existing.size_bytes != offer.size {
            return Err("peer re-offered the same transfer with different details".into());
        }
        // A re-offer is the sender saying it is ready to stream again, so a download the
        // user still wants starts here. A paused one stays paused, an offered one still
        // waits for a tap, and `start_pull` ignores a transfer with a live driver.
        if !is_outgoing(app, &existing)?
            && matches!(
                existing.state,
                TransferState::Accepted | TransferState::Failed | TransferState::Transferring
            )
        {
            start_pull(app.clone(), id);
        }
        return Ok(Vec::new());
    }
    let policy = policy(app);
    let state = if offer.size > policy.max_file_bytes {
        TransferState::Failed
    } else if policy.auto_downloads(offer.size) {
        TransferState::Accepted
    } else {
        TransferState::Offered
    };
    let body = if offer.note.trim().is_empty() || offer.note.len() > MAX_BODY_BYTES {
        offer.file_name.clone()
    } else {
        offer.note.clone()
    };
    let chat_id = remote.to_owned();
    let mut guard = store(app)?;
    guard.save_message(&Message {
        id,
        chat_id: chat_id.clone(),
        sender: chat_id.clone(),
        body,
        clock: envelope.hlc,
        tombstone: false,
        forwarded: false,
    })?;
    guard.upsert_attachment(&Attachment {
        id,
        chat_id: chat_id.clone(),
        message_id: Some(id),
        file_name: offer.file_name.clone(),
        size_bytes: offer.size,
        blake3_hash: Some(blake3_hash),
        state,
        updated_ms: now_ms(),
    })?;
    drop(guard);
    publish(app);
    announce(app, &chat_id);

    let mut replies = Vec::new();
    if state == TransferState::Failed {
        replies.push(cancel_frame(
            app,
            remote,
            id,
            "The file is over the receiving limit on this device.",
        )?);
    }
    if state == TransferState::Accepted {
        start_pull(app.clone(), id);
    }
    Ok(replies)
}

/// Streams an outgoing file over the session the receiver opened.
///
/// This blocks the peer-session thread until the transfer ends, and that is the point: the
/// receiver is reading, so there is always someone on the other end of the socket, while a
/// background sender would fill the socket buffer instead.
pub(crate) fn push_on_session<S: Read + Write>(
    app: &AppHandle,
    session: &mut SecureStream<S>,
    remote: &RemoteIdentity,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    let accept: FileAcceptPayload = payload(envelope)?;
    let id = accept.transfer_id;
    let Some(attachment) = row_opt(app, id)? else {
        return reject(
            app,
            session,
            &remote.peer_id,
            id,
            "No such file on this device.",
        );
    };
    if attachment.chat_id != remote.peer_id || !is_outgoing(app, &attachment)? {
        return reject(
            app,
            session,
            &remote.peer_id,
            id,
            "That file does not belong to this peer.",
        );
    }
    if attachment.state == TransferState::Cancelled {
        return reject(app, session, &remote.peer_id, id, CANCEL_REASON);
    }
    let source = local_path(app, &attachment)?;
    let reason = match fs::metadata(&source) {
        Ok(metadata) if metadata.len() == attachment.size_bytes => None,
        Ok(_) => Some("The copy of this file changed size after it was offered."),
        Err(_) => Some("The copy of this file is no longer on this device."),
    };
    if let Some(reason) = reason {
        return reject(app, session, &remote.peer_id, id, reason);
    }
    // A pause clicked before this frame must not be undone by claiming we are transferring.
    if attachment.state != TransferState::Paused {
        set_state(app, id, TransferState::Transferring)?;
    }
    let target = address(app, &remote.peer_id);
    let request = SendRequest {
        address: &target,
        transfer_id: id,
        source: &source,
        clock_ms: now_ms(),
    };
    match push_accepted(session, &request, &accept, gate(app.clone(), id)) {
        Ok(update) => persist(app, id, &update),
        Err(error) => {
            let _ = set_state(app, id, TransferState::Failed);
            Err(error)
        }
    }
}

fn reject<S: Read + Write>(
    app: &AppHandle,
    session: &mut SecureStream<S>,
    peer_id: &str,
    transfer_id: Uuid,
    reason: &str,
) -> Result<(), String> {
    let frame = cancel_frame(app, peer_id, transfer_id, reason)?;
    session.send(&frame)
}

/// Tells a peer we have stopped wanting a download it is not currently feeding us.
///
/// A running driver sends this itself at its next batch boundary; this is the other case -
/// a queued or broken download the user paused or cancelled - where the sender's own row
/// would otherwise keep saying "waiting for the other side" forever. One connection, one
/// frame, no receipt: the session loop deliberately does not acknowledge a cancel, and a
/// button press must not fail because a peer walked away.
fn notify(app: &AppHandle, peer_id: &str, transfer_id: Uuid, reason: &str) -> Result<(), String> {
    let beacon = endpoint(app, peer_id)?;
    let frame = cancel_frame(app, peer_id, transfer_id, reason)?;
    open_session(app, &beacon)?.send(&frame)
}

/// A `file_cancel` from the peer, for the transfers no driver of ours is watching.
///
/// While a driver runs it reads this frame itself; what reaches the session loop is a
/// cancel for a transfer that is merely queued, and the row has to stop pretending.
pub(crate) fn handle_cancel(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    let cancel: FileCancelPayload = payload(envelope)?;
    let id = cancel.transfer_id;
    let Some(attachment) = row_opt(app, id)? else {
        return Ok(());
    };
    if attachment.chat_id != remote {
        return Err("peer cancelled a file that is not theirs".into());
    }
    let reason = cancel
        .reason
        .unwrap_or_else(|| "peer cancelled the transfer".into());
    let state = match reason.as_str() {
        CANCEL_REASON => TransferState::Cancelled,
        PAUSE_REASON => TransferState::Paused,
        _ => TransferState::Failed,
    };
    if state == TransferState::Failed {
        eprintln!("File transfer {id} refused by the peer: {reason}");
    }
    if attachment.state.is_terminal() {
        return Ok(());
    }
    set_state(app, id, state)
}

// ------------------------------------------------------------------------------------
// What the commands call.
// ------------------------------------------------------------------------------------

/// Acts on one transfer as the UI's pause, resume, retry or cancel button.
pub(crate) fn control(app: &AppHandle, id: Uuid, action: &str) -> Result<(), String> {
    let attachment = row_now(app, id)?;
    let outgoing = is_outgoing(app, &attachment)?;
    match action {
        // Both of these only record intent: a running driver sees it at its next batch and
        // a stopped one stays exactly where it is, ready for a resume.
        "pause" | "cancel" => {
            let (state, reason) = if action == "pause" {
                (TransferState::Paused, PAUSE_REASON)
            } else {
                (TransferState::Cancelled, CANCEL_REASON)
            };
            set_state(app, id, state)?;
            if !outgoing {
                let _ = notify(app, &attachment.chat_id, id, reason);
            }
            Ok(())
        }
        "resume" | "retry" if !outgoing => {
            set_state(app, id, TransferState::Accepted)?;
            start_pull(app.clone(), id);
            Ok(())
        }
        "resume" | "retry" => reoffer(app, &attachment),
        other => Err(format!("Unknown transfer action '{other}'.")),
    }
}

/// Offers an outgoing file again, so the receiver pulls from the chunks it still owes.
fn reoffer(app: &AppHandle, attachment: &Attachment) -> Result<(), String> {
    if attachment.state == TransferState::Completed {
        return Err("That file is already delivered.".into());
    }
    let envelope = offer_envelope(app, attachment, "")?;
    let chat_id = attachment.chat_id.clone();
    store(app)?.enqueue(&chat_id, &envelope)?;
    set_state(app, attachment.id, TransferState::Accepted)?;
    if let Ok(beacon) = endpoint(app, &chat_id) {
        commands::flush_outbox(app, &chat_id, &beacon);
    } else {
        // The peer may still come back, so keep the offer queued and let the retry loop
        // deliver it as soon as discovery finds an address.
        crate::start_outbox_retry(app.clone(), chat_id);
    }
    Ok(())
}

fn offer_envelope(
    app: &AppHandle,
    attachment: &Attachment,
    note: &str,
) -> Result<Envelope<Value>, String> {
    let Some(blake3_hash) = attachment.blake3_hash else {
        return Err("This file has no hash to offer.".into());
    };
    Ok(Envelope {
        v: PROTOCOL_VERSION,
        id: attachment.id,
        kind: kind::FILE_OFFER.into(),
        from: my_id(app),
        to: attachment.chat_id.clone(),
        hlc: Hlc {
            physical_ms: now_ms(),
            logical: 0,
        },
        payload: serde_json::to_value(FileOfferPayload {
            transfer_id: attachment.id,
            chat_id: attachment.chat_id.clone(),
            file_name: attachment.file_name.clone(),
            size: attachment.size_bytes,
            blake3_hash: blake3_hash.to_vec(),
            note: note.to_owned(),
        })
        .map_err(|error| error.to_string())?,
    })
}

/// Keeps the copy the peer will pull and stores the message and attachment rows that
/// describe it. The bytes never travel until the receiver asks for them.
pub(crate) fn stage_outgoing(
    app: &AppHandle,
    chat_id: &str,
    file_name: &str,
    bytes: &[u8],
) -> Result<Attachment, String> {
    let id = Uuid::now_v7();
    let path = outgoing_root(app)?.join(files::stored_name(id, file_name));
    fs::write(&path, bytes).map_err(|error| format!("cannot keep a copy of the file: {error}"))?;
    let attachment = Attachment {
        id,
        chat_id: chat_id.to_owned(),
        message_id: Some(id),
        file_name: file_name.to_owned(),
        size_bytes: bytes.len() as u64,
        blake3_hash: Some(files::hash_bytes(bytes)),
        state: TransferState::Accepted,
        updated_ms: now_ms(),
    };
    let envelope = offer_envelope(app, &attachment, "")?;
    let mut guard = store(app)?;
    guard.save_message(&Message {
        id,
        chat_id: chat_id.to_owned(),
        sender: commands::ME.to_owned(),
        body: file_name.to_owned(),
        clock: Hlc {
            physical_ms: now_ms(),
            logical: 0,
        },
        tombstone: false,
        forwarded: false,
    })?;
    guard.upsert_attachment(&attachment)?;
    guard.enqueue(chat_id, &envelope)?;
    drop(guard);
    announce(app, chat_id);
    Ok(attachment)
}

/// Decodes the data URL the composer read from disk, refusing anything the limit could not
/// carry before allocating for it.
pub(crate) fn decode_data_url(data_url: &str, max_bytes: u64) -> Result<Vec<u8>, String> {
    let Some((_, encoded)) = data_url.split_once(";base64,") else {
        return Err("Only base64 data URLs can be sent as files.".into());
    };
    if encoded.trim().is_empty() {
        return Err("Empty files cannot be sent.".into());
    }
    // Four characters carry three bytes, so this bounds the size without decoding first.
    if (encoded.len() / 4 + 1) * 3 > max_bytes as usize + 3 {
        return Err("That file is larger than the transfer limit.".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|error| format!("The file data is not valid base64: {error}"))?;
    if bytes.is_empty() {
        return Err("Empty files cannot be sent.".into());
    }
    if bytes.len() as u64 > max_bytes {
        return Err("That file is larger than the transfer limit.".into());
    }
    Ok(bytes)
}
