//! Desktop implementations of the typed bridge that are neither overlay windows nor
//! settings: message richness, groups, announcements, avatars and history.
//!
//! Local-first by rule. The encrypted store is the only source of truth here, and a
//! command claims delivery only when the peer answered with a receipt — what the network
//! does not carry yet is stored, listed as pending, and never reported as sent.
//!
//! Lock rule for this whole file: `deliver_envelope`, `publish` and `snapshot` take the
//! store and peer locks themselves, so no helper may hold one while calling another.

use alver_core::discovery::PeerBeacon;
use alver_core::files::TransferState;
use alver_core::groups::{GroupState, EVERYONE_GROUP_ID};
use alver_core::payloads::kind as frame;
use alver_core::payloads::{
    AvatarDataPayload, AvatarRequestPayload, MessageDeletePayload, MessageEditPayload, MessageRef,
    PinUpdatePayload, ReactionPayload, MAX_AVATAR_BYTES, MAX_BODY_BYTES, MAX_REACTION_BYTES,
};
use alver_core::rules::check_announcement_authority;
use alver_core::security::{read_image_header, validate_received_file_name};
use alver_core::storage::Store;
use alver_core::store_ext::{self, Announcement, Avatar, CallEvent, CallEventKind, PinnedMessage};
use alver_core::{Envelope, Hlc, Message, PROTOCOL_VERSION};
use base64::Engine as _;
use serde::Deserialize;
use serde::Serialize;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_opener::OpenerExt;
use uuid::Uuid;

use crate::{
    deliver_envelope, file_transfer, now_ms, platform, publish, send_frame, start_outbox_retry,
    MessageDto, OverlayState,
};

/// The sender id the desktop shell writes for its own messages, edits and deletes.
pub(crate) const ME: &str = "local";

/// Settings rows this module owns, keyed by group id.
mod key {
    pub fn description(group_id: &str) -> String {
        format!("group.description.{group_id}")
    }

    pub fn icon(group_id: &str) -> String {
        format!("group.icon.{group_id}")
    }

    pub fn left(group_id: &str) -> String {
        format!("group.left.{group_id}")
    }

    /// How long a voice recording runs; the transfer itself carries only bytes.
    pub fn attachment_duration(attachment_id: &str) -> String {
        format!("attachment.duration.{attachment_id}")
    }
}

// ------------------------------------------------------------------------------------
// DTOs mirroring the TypeScript shapes in `packages/bridge/src/index.ts`.
// ------------------------------------------------------------------------------------

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReactionTallyDto {
    emoji: String,
    peers: Vec<String>,
    mine: bool,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PinnedMessageDto {
    message_id: String,
    chat_id: String,
    sender: String,
    text: String,
    pinned_ms: u64,
    /// The device that pinned it: pins are local-only until a `pin` frame exists.
    pinned_by: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SearchHitDto {
    chat_id: String,
    chat_name: String,
    message_id: String,
    sender: String,
    text: String,
    timestamp_ms: u64,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct UnreadStateDto {
    chat_id: String,
    unread: u32,
    first_unread_id: Option<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AttachmentDto {
    id: String,
    chat_id: String,
    message_id: Option<String>,
    kind: &'static str,
    name: String,
    bytes: u64,
    mime: &'static str,
    state: &'static str,
    progress: Option<f64>,
    transfer_rate_bytes_per_second: u64,
    duration_ms: Option<u64>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AnnouncementDto {
    id: String,
    group_id: String,
    author: String,
    author_name: String,
    text: String,
    created_ms: u64,
    acknowledged_by: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CallEventDto {
    id: String,
    chat_id: String,
    kind: &'static str,
    status: &'static str,
    caller: String,
    caller_name: String,
    started_ms: u64,
    ended_ms: Option<u64>,
    participants: Vec<String>,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub(crate) struct GroupDto {
    id: String,
    name: String,
    description: String,
    icon: Option<String>,
    epoch: u64,
    admin: String,
    members: Vec<String>,
    is_everyone: bool,
    is_admin: bool,
}

#[derive(Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct GroupPatch {
    name: String,
    description: String,
    icon: Option<String>,
}

// ------------------------------------------------------------------------------------
// Shared helpers.
// ------------------------------------------------------------------------------------

fn store<'a>(app: &'a AppHandle) -> Result<std::sync::MutexGuard<'a, Store>, String> {
    // `inner()` keeps the `&'a OverlayState` alive past the temporary `State` guard,
    // which is what lets the returned lock borrow from the app rather than the call.
    app.state::<OverlayState>()
        .inner()
        .store
        .lock()
        .map_err(|_| "message store lock poisoned".into())
}

/// The signed peer id of this device, used for group and roster authority.
fn my_peer_id(app: &AppHandle) -> String {
    app.state::<OverlayState>().identity.peer_id()
}

fn identity(app: &AppHandle) -> std::sync::Arc<alver_core::identity::Identity> {
    std::sync::Arc::clone(&app.state::<OverlayState>().identity)
}

fn parse_id(value: &str, label: &str) -> Result<Uuid, String> {
    Uuid::parse_str(value.trim()).map_err(|_| format!("{label} is not a valid id"))
}

fn parse_chat(chat_id: &str) -> Result<String, String> {
    let chat_id = chat_id.trim().to_owned();
    if chat_id.is_empty() {
        return Err("A chat id is required.".into());
    }
    Ok(chat_id)
}

/// Display names of every known peer and group, plus the automatic Everyone chat.
fn names(app: &AppHandle, guard: &Store) -> HashMap<String, String> {
    let mut out = HashMap::new();
    if let Ok(known) = guard.known_peers() {
        for peer in known {
            out.insert(peer.id, peer.name);
        }
    }
    if let Ok(peers) = app.state::<OverlayState>().peers.lock() {
        for peer in peers.iter() {
            out.insert(peer.id.clone(), peer.name.clone());
        }
    }
    if let Ok(groups) = guard.all_group_states() {
        for group in groups {
            out.insert(group.id, group.name);
        }
    }
    out.insert(EVERYONE_GROUP_ID.into(), "Everyone on this network".into());
    out
}

fn name_of(all: &HashMap<String, String>, id: &str) -> String {
    all.get(id)
        .cloned()
        .unwrap_or_else(|| id.chars().take(10).collect::<String>())
}

fn message_dto(
    guard: &Store,
    message: &Message,
    replies: &BTreeMap<Uuid, Uuid>,
    unread: &HashSet<Uuid>,
    peer_reads: &HashSet<Uuid>,
) -> Result<MessageDto, String> {
    let mine = message.sender == ME;
    // The transfer id doubles as the message id on every row this app writes, so the file
    // card is found by primary key instead of scanning attachments by message. A tombstone
    // hides the card as well, since its bytes are gone from the chat.
    let file = if message.tombstone {
        None
    } else {
        guard.attachment(message.id)?
    };
    let has_file = file.is_some();
    Ok(MessageDto {
        id: message.id.to_string(),
        sender: message.sender.clone(),
        text: if message.tombstone {
            "This message was deleted".into()
        } else {
            message.body.clone()
        },
        time: String::new(),
        mine,
        // My own ticks follow the peer's read receipt; their messages use my unread
        // state, which is what draws the unread divider.
        read: if mine {
            peer_reads.contains(&message.id)
        } else {
            !unread.contains(&message.id)
        },
        kind: if message.tombstone {
            "deleted".into()
        } else if has_file {
            "file".into()
        } else {
            "text".into()
        },
        file_name: file.map(|attachment| attachment.file_name),
        timestamp_ms: message.clock.physical_ms,
        reply_to: replies.get(&message.id).map(Uuid::to_string),
        delivery_status: if mine && guard.is_message_pending(message.id)? {
            "pending".into()
        } else {
            "delivered".into()
        },
        // A revision row is the record that the body was replaced; a tombstone shows its own
        // text instead, so an edit that was later unsend never claims to be editable.
        edited: !message.tombstone && guard.revision_count(message.id)? > 0,
        forwarded: message.forwarded,
    })
}

fn replies_of(guard: &Store, chat_id: &str) -> Result<BTreeMap<Uuid, Uuid>, String> {
    Ok(guard.replies_for_chat(chat_id)?.into_iter().collect())
}

/// Messages of a chat this device has not read yet, used for the unread divider.
fn unread_of(guard: &Store, chat_id: &str) -> Result<HashSet<Uuid>, String> {
    Ok(guard
        .unread_ids_for_chat(chat_id, ME)?
        .into_iter()
        .collect())
}

/// My messages in a direct chat that the peer has marked read.
///
/// A group thread has no single reader, so its ticks stay at `delivered`.
fn peer_reads_of(guard: &Store, chat_id: &str) -> Result<HashSet<Uuid>, String> {
    if guard.get_group_state(chat_id)?.is_some() {
        return Ok(HashSet::new());
    }
    guard.read_receipt_ids_for_chat(chat_id, ME, chat_id)
}

/// The thread of one chat, as the bridge reads it.
pub(crate) fn message_dtos(app: &AppHandle, chat_id: &str) -> Result<Vec<MessageDto>, String> {
    let guard = store(app)?;
    let replies = replies_of(&guard, chat_id)?;
    let unread = unread_of(&guard, chat_id)?;
    let peer_reads = peer_reads_of(&guard, chat_id)?;
    let mut out = Vec::new();
    for message in guard.messages_for_chat(chat_id)? {
        out.push(message_dto(
            &guard,
            &message,
            &replies,
            &unread,
            &peer_reads,
        )?);
    }
    Ok(out)
}

/// Whether a message this device writes is its own words or a copy of something from elsewhere.
///
/// A forward is a new message with an old body, and the copy has to say so: the bubble's
/// "Forwarded" chip is the only thing that tells a reader the text was not written here.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Origin {
    Original,
    Forwarded,
}

/// What every `message` frame carries, whichever audience it fans out to.
///
/// `forwarded` is the flag the bubble reads to say "Forwarded"; a copy keeps no link back
/// to its source, so the only thing the wire needs to say is that it is a copy.
fn message_payload(
    body: &str,
    reply_to: Option<Uuid>,
    chat_id: &str,
    forwarded: bool,
) -> serde_json::Value {
    serde_json::json!({
        "text": body,
        "reply_to": reply_to.map(|id| id.to_string()),
        "chat_id": chat_id,
        "forwarded": forwarded,
    })
}

/// Writes one outgoing message and queues it for its recipients.
///
/// A direct send fails loudly when the peer vanished, because the composer must not
/// imply delivery. A group post is stored once under the group id and queued as one
/// outbox row per other member, so offline members receive it when discovery returns.
fn outgoing(
    app: &AppHandle,
    chat_id: &str,
    body: &str,
    reply_to: Option<Uuid>,
    origin: Origin,
    strict: bool,
) -> Result<MessageDto, String> {
    let body = body.trim();
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(format!("Message must contain 1–{MAX_BODY_BYTES} bytes."));
    }
    let my_id = my_peer_id(app);
    let (group, roster, epoch) = {
        let guard = store(app)?;
        let group = is_group(&guard, chat_id);
        let roster = if group {
            roster_of(&guard, chat_id, &my_id)
        } else {
            Vec::new()
        };
        let epoch = guard
            .get_group_state(chat_id)
            .ok()
            .flatten()
            .map(|state| state.epoch)
            .unwrap_or(1);
        (group, roster, epoch)
    };
    if !group {
        let beacon = app
            .state::<OverlayState>()
            .endpoints
            .lock()
            .map_err(|_| "peer endpoint lock poisoned")?
            .get(chat_id)
            .cloned();
        if strict && beacon.is_none() {
            return Err("This peer is no longer available on the local network.".into());
        }
    }
    let forwarded = origin == Origin::Forwarded;
    let message = Message {
        id: Uuid::now_v7(),
        chat_id: chat_id.to_owned(),
        sender: ME.into(),
        body: body.to_owned(),
        clock: Hlc {
            physical_ms: now_ms(),
            logical: 0,
        },
        tombstone: false,
        forwarded,
    };
    let frames = if group {
        alver_core::groups::delivery_plan_for(&roster, &my_id, chat_id, epoch, message.id)
            .into_iter()
            .map(|target| {
                (
                    target.recipient.clone(),
                    Envelope {
                        v: PROTOCOL_VERSION,
                        id: message.id,
                        kind: frame::MESSAGE.into(),
                        from: my_id.clone(),
                        to: target.recipient,
                        hlc: message.clock,
                        payload: message_payload(&message.body, reply_to, chat_id, forwarded),
                    },
                )
            })
            .collect::<Vec<_>>()
    } else {
        vec![(
            chat_id.to_owned(),
            Envelope {
                v: PROTOCOL_VERSION,
                id: message.id,
                kind: frame::MESSAGE.into(),
                from: my_id.clone(),
                to: chat_id.to_owned(),
                hlc: message.clock,
                payload: message_payload(&message.body, reply_to, chat_id, forwarded),
            },
        )]
    };
    {
        let mut guard = store(app)?;
        if group {
            guard.enqueue_group_message(&message, &frames, reply_to)?;
        } else {
            let (peer, envelope) = &frames[0];
            guard.enqueue_quoted_message(&message, peer, envelope, reply_to)?;
        }
    }
    // Every recipient with a live endpoint gets its own delivery thread; the outbox and
    // its retry loop own everyone else, so the composer never waits for a handshake.
    let endpoints = app
        .state::<OverlayState>()
        .endpoints
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();
    for (peer, _) in &frames {
        let Some(beacon) = endpoints.get(peer).cloned() else {
            if group {
                start_outbox_retry(app.clone(), peer.clone());
            }
            continue;
        };
        let delivery_app = app.clone();
        let delivery_peer = peer.clone();
        let _ = std::thread::Builder::new()
            .name("alverchat-delivery".into())
            .spawn(move || flush_outbox(&delivery_app, &delivery_peer, &beacon));
    }
    if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == chat_id) {
            peer.preview = message.body.clone();
            peer.last_message_ms = message.clock.physical_ms;
        }
    }
    let _ = app.emit("messages-changed", chat_id);
    publish(app);
    let guard = store(app)?;
    let replies = replies_of(&guard, chat_id)?;
    let unread = unread_of(&guard, chat_id)?;
    message_dto(&guard, &message, &replies, &unread, &HashSet::new())
}

/// Sends one composer message, quoting `reply_to_id` when the UI supplied one.
///
/// Runs on a blocking thread: the caller is the async `send_message` command, and the
/// store write is what has to finish before the bubble can be listed.
pub(crate) fn send_message(
    app: AppHandle,
    chat_id: String,
    text: String,
    reply_to_id: Option<String>,
) -> Result<MessageDto, String> {
    if chat_id.trim().is_empty() {
        return Err("A chat id is required.".into());
    }
    let quoted = match reply_to_id.as_deref() {
        None => None,
        Some(raw) if raw.trim().is_empty() => None,
        Some(raw) => Some(parse_id(raw, "Reply target")?),
    };
    outgoing(&app, chat_id.trim(), &text, quoted, Origin::Original, true)
}

/// Drains the outbox of one reachable peer; a failure hands the peer to the retry loop.
pub(crate) fn flush_outbox(app: &AppHandle, chat_id: &str, beacon: &PeerBeacon) {
    let Ok(pending) = store(app).and_then(|guard| guard.pending_outbox(chat_id)) else {
        return;
    };
    for queued in pending {
        if deliver_envelope(app, beacon, &queued).is_err() {
            start_outbox_retry(app.clone(), beacon.peer_id.clone());
            return;
        }
    }
}

fn reaction_tally(guard: &Store, message_id: Uuid) -> Result<Vec<ReactionTallyDto>, String> {
    let mut order: Vec<String> = Vec::new();
    let mut grouped: BTreeMap<String, Vec<String>> = BTreeMap::new();
    for reaction in guard.reactions_for_message(message_id)? {
        let entry = grouped.entry(reaction.emoji.clone()).or_default();
        if entry.is_empty() {
            order.push(reaction.emoji.clone());
        }
        entry.push(reaction.peer_id);
    }
    Ok(order
        .into_iter()
        .map(|emoji| {
            let peers = grouped.remove(&emoji).unwrap_or_default();
            ReactionTallyDto {
                mine: peers.iter().any(|peer| peer == ME),
                peers,
                emoji,
            }
        })
        .collect())
}

fn pinned_dtos(
    app: &AppHandle,
    guard: &Store,
    chat_id: &str,
) -> Result<Vec<PinnedMessageDto>, String> {
    let all = names(app, guard);
    guard
        .pinned_for_chat(chat_id)?
        .into_iter()
        .map(
            |PinnedMessage {
                 message,
                 pinned_ms,
                 pinned_by,
                 ..
             }| {
                Ok(PinnedMessageDto {
                    // Whichever device asked for the pin, recorded when it landed — this
                    // device's own command or a peer's `pin_update` frame. A row written
                    // before migration v7 has no author, and stays an empty string rather
                    // than borrowing this device's name.
                    pinned_by: if pinned_by.is_empty() {
                        String::new()
                    } else {
                        name_of(&all, &pinned_by)
                    },
                    pinned_ms,
                    message_id: message.id.to_string(),
                    chat_id: message.chat_id.clone(),
                    sender: message.sender,
                    text: if message.tombstone {
                        "This message was deleted".into()
                    } else {
                        message.body
                    },
                })
            },
        )
        .collect()
}

/// The `kind`/`mime` buckets the bridge renders, read off the file extension.
fn media_kind(file_name: &str) -> (&'static str, &'static str) {
    let extension = file_name
        .rsplit('.')
        .next()
        .unwrap_or_default()
        .to_lowercase();
    match extension.as_str() {
        "png" => ("image", "image/png"),
        "jpg" | "jpeg" => ("image", "image/jpeg"),
        "gif" => ("image", "image/gif"),
        "webp" => ("image", "image/webp"),
        "heic" => ("image", "image/heic"),
        "mp4" | "m4v" => ("video", "video/mp4"),
        "mov" => ("video", "video/quicktime"),
        "webm" => ("video", "video/webm"),
        "mkv" => ("video", "video/x-matroska"),
        "mp3" => ("audio", "audio/mpeg"),
        "m4a" => ("audio", "audio/mp4"),
        "wav" => ("audio", "audio/wav"),
        "ogg" => ("audio", "audio/ogg"),
        "txt" | "md" => ("file", "text/plain"),
        "pdf" => ("file", "application/pdf"),
        "zip" => ("file", "application/zip"),
        "json" => ("file", "application/json"),
        _ => ("file", "application/octet-stream"),
    }
}

/// The bridge state word for one stored transfer state.
///
/// Direction is not in the state, only in whose message it is, so the same
/// `transferring` reads as an upload on my bubble and a download on the peer's.
fn transfer_state(state: TransferState, mine: bool) -> &'static str {
    match state {
        TransferState::Idle => "idle",
        TransferState::Offered | TransferState::Accepted => "queued",
        TransferState::Transferring => {
            if mine {
                "uploading"
            } else {
                "downloading"
            }
        }
        TransferState::Paused => "paused",
        TransferState::Completed => "done",
        TransferState::Failed => "failed",
        TransferState::Cancelled => "cancelled",
    }
}

fn attachment_dto(
    app: &AppHandle,
    attachment: &store_ext::Attachment,
    mine: bool,
) -> AttachmentDto {
    let (kind, mime) = media_kind(&attachment.file_name);
    let duration_ms = if kind == "audio" {
        store(app).ok().and_then(|guard| {
            guard
                .get_setting::<u64>(&key::attachment_duration(&attachment.id.to_string()))
                .ok()
                .flatten()
        })
    } else {
        None
    };
    AttachmentDto {
        id: attachment.id.to_string(),
        chat_id: attachment.chat_id.clone(),
        message_id: attachment.message_id.map(|id| id.to_string()),
        kind,
        name: attachment.file_name.clone(),
        bytes: attachment.size_bytes,
        mime,
        state: transfer_state(attachment.state, mine),
        // A rate needs a sampled window; the drivers report bytes, not speed.
        progress: file_transfer::progress(app, attachment, mine),
        transfer_rate_bytes_per_second: 0,
        duration_ms,
    }
}

fn announcement_dto(
    guard: &Store,
    announcement: &Announcement,
    all: &HashMap<String, String>,
) -> Result<AnnouncementDto, String> {
    let acknowledged_by = guard
        .acknowledgements_for_announcement(announcement.id)?
        .map(|summary| summary.acks.into_iter().map(|ack| ack.peer_id).collect())
        .unwrap_or_default();
    Ok(AnnouncementDto {
        id: announcement.id.to_string(),
        group_id: announcement.group_id.clone(),
        author: announcement.sender.clone(),
        author_name: name_of(all, &announcement.sender),
        text: announcement.body.clone(),
        created_ms: announcement.created_ms,
        acknowledged_by,
    })
}

fn group_dto(guard: &Store, state: &GroupState, my_id: &str) -> Result<GroupDto, String> {
    Ok(GroupDto {
        id: state.id.clone(),
        name: state.name.clone(),
        description: guard
            .get_setting::<String>(&key::description(&state.id))?
            .unwrap_or_default(),
        icon: guard.get_setting::<String>(&key::icon(&state.id))?,
        epoch: state.epoch,
        admin: state.admin.clone(),
        members: state.members.clone(),
        is_everyone: false,
        is_admin: state.admin == my_id,
    })
}

/// Who belongs to a chat: the signed roster, the discovered peers for Everyone, or the
/// two ends of a direct conversation.
fn roster_of(guard: &Store, chat_id: &str, my_id: &str) -> Vec<String> {
    if chat_id == EVERYONE_GROUP_ID {
        let mut members = guard
            .known_peers()
            .map(|peers| peers.into_iter().map(|peer| peer.id).collect::<Vec<_>>())
            .unwrap_or_default();
        members.push(my_id.to_owned());
        members.sort();
        members.dedup();
        return members;
    }
    if let Some(state) = guard.get_group_state(chat_id).ok().flatten() {
        return state.members;
    }
    vec![my_id.to_owned(), chat_id.to_owned()]
}

fn is_group(guard: &Store, chat_id: &str) -> bool {
    chat_id == EVERYONE_GROUP_ID || guard.get_group_state(chat_id).ok().flatten().is_some()
}

/// The status word one call's signalling history settled on.
///
/// Pure so the reduce step is testable without a database or a network.
fn call_status(events: &[&CallEvent]) -> (&'static str, Option<u64>) {
    let has = |kind: CallEventKind| events.iter().any(|event| event.kind == kind);
    let ended = events.iter().filter_map(|event| event.ended_ms).max();
    if has(CallEventKind::Reject) {
        (
            "declined",
            ended.or(events.last().and_then(|event| event.ended_ms)),
        )
    } else if has(CallEventKind::Busy) {
        ("failed", ended)
    } else if has(CallEventKind::Answer) {
        if has(CallEventKind::End) {
            ("ended", ended)
        } else {
            ("ongoing", None)
        }
    } else if has(CallEventKind::End) {
        ("missed", ended)
    } else {
        ("ringing", None)
    }
}

fn call_events(app: &AppHandle, chat_id: &str) -> Result<Vec<CallEventDto>, String> {
    let my_id = my_peer_id(app);
    let guard = store(app)?;
    let all = names(app, &guard);
    let grouped = is_group(&guard, chat_id);
    let roster = roster_of(&guard, chat_id, &my_id);
    let events = guard.calls_for_chat(chat_id)?;
    let mut by_call: BTreeMap<Uuid, Vec<&CallEvent>> = BTreeMap::new();
    for event in &events {
        by_call.entry(event.call_id).or_default().push(event);
    }
    let mut out = Vec::new();
    for (call_id, rows) in by_call {
        let (status, ended_ms) = call_status(&rows);
        let caller = rows
            .iter()
            .find(|event| event.kind == CallEventKind::Offer)
            .or_else(|| rows.first())
            .map(|event| event.peer_id.clone())
            .unwrap_or_default();
        let mut participants = roster.clone();
        if !participants.contains(&caller) {
            participants.push(caller.clone());
        }
        out.push(CallEventDto {
            id: call_id.to_string(),
            chat_id: chat_id.to_owned(),
            kind: if grouped { "group" } else { "1to1" },
            status,
            caller_name: name_of(&all, &caller),
            caller,
            started_ms: rows.iter().map(|event| event.started_ms).min().unwrap_or(0),
            ended_ms,
            participants,
        });
    }
    out.sort_by_key(|row| std::cmp::Reverse(row.started_ms));
    Ok(out)
}

/// Splits a base64 data URL into an accepted image plus its header geometry.
///
/// The renderer resizes to 256 px and re-encodes without EXIF before calling this; the
/// store then re-checks the byte, edge and pixel-count budgets, so a hand-crafted URL
/// still cannot smuggle a decompression bomb.
fn decode_avatar_url(url: &str) -> Result<(&'static str, u32, u32, Vec<u8>), String> {
    let (meta, payload) = url
        .trim()
        .split_once(',')
        .ok_or("Avatar data is incomplete.")?;
    if !meta.starts_with("data:") || !meta.contains(";base64") {
        return Err("Avatar must be a base64 data URL.".into());
    }
    if payload.len() > alver_core::payloads::MAX_AVATAR_BYTES * 2 {
        return Err("Avatar is too large.".into());
    }
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(payload.trim())
        .map_err(|_| "Avatar data is not valid base64.".to_owned())?;
    let (media_type, width, height) =
        read_image_header(&bytes).ok_or("Avatar must be a PNG or JPEG image.")?;
    if !alver_core::security::ACCEPTED_AVATAR_MEDIA_TYPES.contains(&media_type) {
        return Err("Avatar must be a PNG or JPEG image.".into());
    }
    Ok((media_type, width, height, bytes))
}

fn encode_avatar_url(media_type: &str, bytes: &[u8]) -> String {
    format!(
        "data:{media_type};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )
}

// ------------------------------------------------------------------------------------
// Message richness.
// ------------------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn pin_message(
    app: AppHandle,
    chat_id: String,
    message_id: String,
    pinned: bool,
) -> Result<Vec<PinnedMessageDto>, String> {
    let chat_id = parse_chat(&chat_id)?;
    let id = parse_id(&message_id, "Message id")?;
    let my_id = my_peer_id(&app);
    let position = {
        let mut guard = store(&app)?;
        let message = guard
            .get_message(id)?
            .ok_or("That message is not in this database.")?;
        if message.chat_id != chat_id {
            return Err("A message can only be pinned in its own chat.".into());
        }
        if pinned {
            let position = guard
                .pinned_for_chat(&chat_id)?
                .iter()
                .filter(|pin| pin.message.id != id)
                .count() as u32;
            guard.pin(id, position, now_ms(), &my_id)?;
            position
        } else {
            guard.unpin(id)?;
            0
        }
    };
    // A pin is a claim about everyone's copy of the chat, which is why it travels: the list
    // used to diverge quietly, one Pinned tab per device. `pinned_by` records who asked.
    let payload = serde_json::to_value(PinUpdatePayload {
        chat_id: chat_id.clone(),
        message_id: id,
        pinned,
        position,
    });
    if let Ok(payload) = payload {
        propagate_change(&app, &chat_id, frame::PIN_UPDATE, payload);
    }
    let guard = store(&app)?;
    pinned_dtos(&app, &guard, &chat_id)
}

#[tauri::command]
pub(crate) fn pinned_messages(
    app: AppHandle,
    chat_id: String,
) -> Result<Vec<PinnedMessageDto>, String> {
    let chat_id = parse_chat(&chat_id)?;
    let guard = store(&app)?;
    pinned_dtos(&app, &guard, &chat_id)
}

#[tauri::command]
pub(crate) fn star_message(
    app: AppHandle,
    message_id: String,
    starred: bool,
) -> Result<bool, String> {
    let id = parse_id(&message_id, "Message id")?;
    let mut guard = store(&app)?;
    if starred {
        guard.star(id, now_ms())
    } else {
        guard.unstar(id)
    }
}

#[tauri::command]
pub(crate) fn starred_messages(app: AppHandle, chat_id: String) -> Result<Vec<MessageDto>, String> {
    let chat_id = parse_chat(&chat_id)?;
    let guard = store(&app)?;
    let replies = replies_of(&guard, &chat_id)?;
    let unread = unread_of(&guard, &chat_id)?;
    let peer_reads = peer_reads_of(&guard, &chat_id)?;
    let mut out = Vec::new();
    for starred in guard.starred_for_chat(&chat_id)? {
        out.push(message_dto(
            &guard,
            &starred.message,
            &replies,
            &unread,
            &peer_reads,
        )?);
    }
    Ok(out)
}

#[tauri::command]
pub(crate) fn reactions_for(
    app: AppHandle,
    message_id: String,
) -> Result<Vec<ReactionTallyDto>, String> {
    let id = parse_id(&message_id, "Message id")?;
    let guard = store(&app)?;
    reaction_tally(&guard, id)
}

#[tauri::command]
pub(crate) fn react(
    app: AppHandle,
    message_id: String,
    emoji: String,
) -> Result<Vec<ReactionTallyDto>, String> {
    let id = parse_id(&message_id, "Message id")?;
    let emoji = emoji.trim().to_owned();
    if emoji.is_empty() {
        return Err("A reaction needs an emoji.".into());
    }
    if emoji.len() > MAX_REACTION_BYTES {
        return Err(format!("A reaction is at most {MAX_REACTION_BYTES} bytes."));
    }
    let (chat_id, changed, removed) = {
        let mut guard = store(&app)?;
        let chat_id = guard.get_message(id)?.map(|message| message.chat_id);
        let already = guard
            .reactions_for_message(id)?
            .iter()
            .any(|reaction| reaction.peer_id == ME && reaction.emoji == emoji);
        let changed = if already {
            guard.remove_reaction(id, ME, &emoji)?
        } else {
            guard.add_reaction(id, ME, &emoji, now_ms())?
        };
        (chat_id, changed, already)
    };
    // A chip that did not move on our own row has nothing to tell anyone; a real one has to
    // reach every reader of the chat, or the two devices show different threads.
    if changed {
        if let Some(chat_id) = chat_id {
            let payload = serde_json::to_value(ReactionPayload {
                message: MessageRef {
                    chat_id: chat_id.clone(),
                    message_id: id,
                },
                emoji: emoji.clone(),
                remove: removed,
            });
            if let Ok(payload) = payload {
                propagate_change(&app, &chat_id, frame::REACTION, payload);
            }
        }
    }
    let guard = store(&app)?;
    reaction_tally(&guard, id)
}

#[tauri::command]
pub(crate) fn forward_messages(
    app: AppHandle,
    message_ids: Vec<String>,
    chat_ids: Vec<String>,
) -> Result<usize, String> {
    if message_ids.is_empty() || chat_ids.is_empty() {
        return Err("Pick at least one message and one chat to forward into.".into());
    }
    let mut bodies = Vec::new();
    {
        let guard = store(&app)?;
        for raw in &message_ids {
            let id = parse_id(raw, "Message id")?;
            let message = guard
                .get_message(id)?
                .ok_or("That message is not in this database.")?;
            if message.tombstone {
                return Err("A deleted message cannot be forwarded.".into());
            }
            bodies.push(message.body);
        }
    }
    let mut landed = 0;
    for raw in &chat_ids {
        let chat_id = parse_chat(raw)?;
        for body in &bodies {
            // A copy loses its quote: the quoted id belongs to the source chat.
            if outgoing(&app, &chat_id, body, None, Origin::Forwarded, false).is_ok() {
                landed += 1;
            }
        }
    }
    if landed == 0 {
        return Err("Nothing could be forwarded.".into());
    }
    Ok(landed)
}

#[tauri::command]
pub(crate) fn search_messages(
    app: AppHandle,
    query: String,
    chat_id: Option<String>,
) -> Result<Vec<SearchHitDto>, String> {
    let scoped = match chat_id.as_deref() {
        Some(raw) => Some(parse_chat(raw)?),
        None => None,
    };
    let guard = store(&app)?;
    let all = names(&app, &guard);
    let mut out = Vec::new();
    for (id, _) in guard.search_messages(&query, 60)? {
        let Some(message) = guard.get_message(id)? else {
            continue;
        };
        if message.tombstone {
            continue;
        }
        if let Some(chat) = &scoped {
            if &message.chat_id != chat {
                continue;
            }
        }
        out.push(SearchHitDto {
            chat_name: name_of(&all, &message.chat_id),
            sender: message.sender,
            message_id: message.id.to_string(),
            chat_id: message.chat_id,
            text: message.body,
            timestamp_ms: message.clock.physical_ms,
        });
    }
    out.sort_by_key(|hit| std::cmp::Reverse(hit.timestamp_ms));
    Ok(out)
}

#[tauri::command]
pub(crate) async fn set_typing(
    app: AppHandle,
    chat_id: String,
    typing: bool,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || set_typing_blocking(app, chat_id, typing))
        .await
        .map_err(|error| error.to_string())?
}

/// Typing is advisory: a peer that is not reachable simply does not see the indicator,
/// so the frame is best-effort and its failure never reaches the composer.
///
/// The audience is the same one every other change to the chat is sent to, which is what
/// makes a group indicator work: a group id is never a key in the endpoint map, because
/// endpoints are per device, and looking a group up there used to answer "nobody to tell".
fn set_typing_blocking(app: AppHandle, chat_id: String, typing: bool) -> Result<(), String> {
    let chat_id = parse_chat(&chat_id)?;
    let my_id = my_peer_id(&app);
    let recipients = {
        let guard = store(&app)?;
        chat_audience(&guard, &chat_id, &my_id)
    };
    let endpoints = app
        .state::<OverlayState>()
        .endpoints
        .lock()
        .map_err(|_| "peer endpoint lock poisoned")?
        .clone();
    for peer in recipients {
        let Some(beacon) = endpoints.get(&peer).cloned() else {
            continue;
        };
        let envelope = Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: frame::TYPING.into(),
            from: my_id.clone(),
            to: peer,
            hlc: Hlc {
                physical_ms: now_ms(),
                logical: 0,
            },
            payload: serde_json::json!({"typing": typing}),
        };
        let _ = send_frame(&app, &beacon, &envelope);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn mark_unread(app: AppHandle, chat_id: String) -> Result<Vec<UnreadStateDto>, String> {
    let chat_id = parse_chat(&chat_id)?;
    let mut guard = store(&app)?;
    let unread = guard.mark_chat_unread(&chat_id, ME)?;
    let first_unread_id = guard
        .first_unread_message_for_chat(&chat_id, ME)?
        .map(|id| id.to_string());
    drop(guard);
    if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == chat_id) {
            peer.unread = if unread > 0 {
                Some(unread as u32)
            } else {
                None
            };
        }
    }
    publish(&app);
    Ok(vec![UnreadStateDto {
        unread: unread as u32,
        first_unread_id,
        chat_id,
    }])
}

#[tauri::command]
pub(crate) fn clear_chat(app: AppHandle, chat_id: String) -> Result<(), String> {
    let chat_id = parse_chat(&chat_id)?;
    store(&app)?.clear_chat_history(&chat_id)?;
    if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == chat_id) {
            peer.preview = String::new();
            peer.last_message_ms = 0;
            peer.unread = None;
        }
    }
    let _ = app.emit("messages-changed", chat_id);
    publish(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn delete_chat(app: AppHandle, chat_id: String) -> Result<(), String> {
    let chat_id = parse_chat(&chat_id)?;
    store(&app)?.clear_chat_history(&chat_id)?;
    {
        let state = app.state::<OverlayState>();
        if let Ok(mut peers) = state.peers.lock() {
            peers.retain(|peer| peer.id != chat_id);
        }
        if let Ok(mut endpoints) = state.endpoints.lock() {
            endpoints.remove(&chat_id);
        };
    }
    let _ = app.emit("messages-changed", chat_id);
    // The peer row survives: discovery owns it, and the chat returns if they reappear.
    publish(&app);
    Ok(())
}

#[tauri::command]
pub(crate) fn delete_for_everyone(app: AppHandle, message_id: String) -> Result<bool, String> {
    let id = parse_id(&message_id, "Message id")?;
    let mut guard = store(&app)?;
    let message = guard
        .get_message(id)?
        .ok_or("That message is not in this database.")?;
    let chat_id = message.chat_id.clone();
    let removed = if message.sender == ME {
        guard.tombstone_message(id, ME)?
    } else {
        // A group admin may unsend anyone's message inside their group; nobody else can.
        let my_id = my_peer_id(&app);
        guard
            .admin_tombstone_in_group(id, &my_id, &chat_id)
            .unwrap_or(false)
    };
    drop(guard);
    if removed {
        // The tombstone is only real once every device in the chat agrees to it, so the
        // unsend travels before this device reports success.
        let payload = serde_json::to_value(MessageDeletePayload {
            message_id: id,
            for_everyone: true,
            reason: None,
        });
        if let Ok(payload) = payload {
            propagate_change(&app, &chat_id, frame::MESSAGE_DELETE, payload);
        }
        let _ = app.emit("messages-changed", chat_id);
        publish(&app);
    }
    Ok(removed)
}

// ------------------------------------------------------------------------------------
// Files.
// ------------------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn attachments_for(
    app: AppHandle,
    chat_id: String,
) -> Result<Vec<AttachmentDto>, String> {
    let chat_id = parse_chat(&chat_id)?;
    let attachments = store(&app)?.attachments_for_chat(&chat_id)?;
    let mut out = Vec::with_capacity(attachments.len());
    for attachment in &attachments {
        let mine = file_transfer::is_outgoing(&app, attachment)?;
        out.push(attachment_dto(&app, attachment, mine));
    }
    Ok(out)
}

/// Keeps a copy of the file, offers it, and hands the offer to the peer.
///
/// The bytes are not sent here: the peer pulls them on its own session, so this returns as
/// soon as the offer is delivered, with the transfer queued rather than "sent".
#[tauri::command]
pub(crate) async fn send_file(
    app: AppHandle,
    chat_id: String,
    file_name: String,
    data_url: String,
    duration_ms: Option<u64>,
) -> Result<AttachmentDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        send_file_blocking(app, chat_id, file_name, data_url, duration_ms)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn send_file_blocking(
    app: AppHandle,
    chat_id: String,
    file_name: String,
    data_url: String,
    duration_ms: Option<u64>,
) -> Result<AttachmentDto, String> {
    let chat_id = parse_chat(&chat_id)?;
    validate_received_file_name(&file_name).map_err(|error| format!("{error:?}"))?;
    let max_bytes = file_transfer::policy(&app).max_file_bytes;
    let bytes = file_transfer::decode_data_url(&data_url, max_bytes)?;
    let beacon = file_transfer::require_peer(&app, &chat_id)?;
    let attachment = file_transfer::stage_outgoing(&app, &chat_id, &file_name, &bytes)?;
    if let Some(duration_ms) = duration_ms {
        store(&app)?.set_setting(
            &key::attachment_duration(&attachment.id.to_string()),
            &duration_ms,
            now_ms(),
        )?;
    }
    flush_outbox(&app, &chat_id, &beacon);
    publish(&app);
    Ok(attachment_dto(&app, &attachment, true))
}

#[tauri::command]
pub(crate) async fn control_transfer(
    app: AppHandle,
    attachment_id: String,
    action: String,
) -> Result<Option<AttachmentDto>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        control_transfer_blocking(app, attachment_id, action)
    })
    .await
    .map_err(|error| error.to_string())?
}

fn control_transfer_blocking(
    app: AppHandle,
    attachment_id: String,
    action: String,
) -> Result<Option<AttachmentDto>, String> {
    let id = parse_id(&attachment_id, "Attachment id")?;
    file_transfer::control(&app, id, action.trim())?;
    let attachment = file_transfer::row_now(&app, id)?;
    let mine = file_transfer::is_outgoing(&app, &attachment)?;
    Ok(Some(attachment_dto(&app, &attachment, mine)))
}

/// Opens the file with the system's default handler once its bytes are here.
#[tauri::command]
pub(crate) fn open_attachment(
    app: AppHandle,
    attachment_id: String,
) -> Result<Option<String>, String> {
    let id = parse_id(&attachment_id, "Attachment id")?;
    let attachment = file_transfer::row_now(&app, id)?;
    let path = file_transfer::file_path(&app, &attachment)?;
    let opened = path.to_string_lossy().into_owned();
    app.opener()
        .open_path(&opened, None::<String>)
        .map_err(|error| format!("Cannot open the file: {error}"))?;
    Ok(Some(opened))
}

/// The bytes of a downloaded attachment as a data URL, so a voice note can play and a picture can
/// show in the thread without a second protocol for serving files into the webview.
#[tauri::command]
pub(crate) fn attachment_data(
    app: AppHandle,
    attachment_id: String,
) -> Result<Option<String>, String> {
    /// Anything this small crosses the IPC bridge whole; larger files are opened, not inlined.
    const MAX_INLINE_BYTES: u64 = 16 * 1024 * 1024;
    let id = parse_id(&attachment_id, "Attachment id")?;
    let attachment = file_transfer::row_now(&app, id)?;
    if attachment.size_bytes > MAX_INLINE_BYTES {
        return Err("That file is too large to open inside the app.".into());
    }
    let path = file_transfer::file_path(&app, &attachment)?;
    let bytes = std::fs::read(&path).map_err(|error| format!("Cannot read the file: {error}"))?;
    let (_, mime) = media_kind(&attachment.file_name);
    Ok(Some(format!(
        "data:{mime};base64,{}",
        base64::engine::general_purpose::STANDARD.encode(bytes)
    )))
}

// ------------------------------------------------------------------------------------
// Groups and announcements.
// ------------------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn list_groups(app: AppHandle) -> Result<Vec<GroupDto>, String> {
    let my_id = my_peer_id(&app);
    let guard = store(&app)?;
    let mut out = vec![GroupDto {
        id: EVERYONE_GROUP_ID.into(),
        name: "Everyone on this network".into(),
        description: "Automatic conversation with every peer AlverChat has discovered.".to_owned(),
        icon: None,
        epoch: 1,
        admin: my_id.clone(),
        members: roster_of(&guard, EVERYONE_GROUP_ID, &my_id),
        is_everyone: true,
        is_admin: true,
    }];
    for state in guard.all_group_states()? {
        if guard
            .get_setting::<bool>(&key::left(&state.id))?
            .unwrap_or(false)
        {
            continue;
        }
        out.push(group_dto(&guard, &state, &my_id)?);
    }
    Ok(out)
}

#[tauri::command]
pub(crate) fn create_group(
    app: AppHandle,
    name: String,
    member_ids: Vec<String>,
    icon: Option<String>,
) -> Result<GroupDto, String> {
    let name = name.trim().to_owned();
    if name.is_empty() || name.len() > 120 {
        return Err("Group name must contain 1–120 bytes.".into());
    }
    if member_ids.is_empty() {
        return Err("Pick at least one peer for the group.".into());
    }
    if name == EVERYONE_GROUP_ID {
        return Err("That name is reserved by the automatic group.".into());
    }
    let my_id = my_peer_id(&app);
    let mut members: Vec<String> = Vec::new();
    let state = {
        let guard = store(&app)?;
        let known: HashSet<String> = guard
            .known_peers()?
            .into_iter()
            .map(|peer| peer.id)
            .collect();
        for member in member_ids {
            let member = member.trim();
            if member.is_empty() {
                continue;
            }
            if !known.contains(member) {
                return Err("A group member must be a peer you have connected to.".into());
            }
            members.push(member.to_owned());
        }
        members.push(my_id.clone());
        GroupState::create(&identity(&app), Uuid::now_v7().to_string(), name, members)
            .map_err(|error| format!("Group could not be created: {error:?}"))?
    };
    let mut guard = store(&app)?;
    guard.save_group_state(&state)?;
    if let Some(icon) = icon.filter(|value| !value.trim().is_empty()) {
        guard.set_setting(&key::icon(&state.id), &icon, now_ms())?;
    }
    guard.set_setting(&key::left(&state.id), &false, now_ms())?;
    let dto = group_dto(&guard, &state, &my_id)?;
    drop(guard);
    deliver_group_state(&app, &state, &[]);
    Ok(dto)
}

#[tauri::command]
pub(crate) fn update_group(
    app: AppHandle,
    id: String,
    patch: GroupPatch,
) -> Result<GroupDto, String> {
    let id = parse_chat(&id)?;
    let my_id = my_peer_id(&app);
    let mut guard = store(&app)?;
    let mut state = guard
        .get_group_state(&id)?
        .ok_or("That group does not exist on this device.")?;
    let previous = state.members.clone();
    let name = patch.name.trim();
    let mut roster_changed = false;
    if !name.is_empty() && name != state.name {
        state
            .rename(&identity(&app), name.to_owned())
            .map_err(|error| format!("Group rename refused: {error:?}"))?;
        guard.save_group_state(&state)?;
        roster_changed = true;
    }
    guard.set_setting(&key::description(&id), &patch.description, now_ms())?;
    if let Some(icon) = patch.icon.filter(|value| !value.trim().is_empty()) {
        guard.set_setting(&key::icon(&id), &icon, now_ms())?;
    }
    let dto = group_dto(&guard, &state, &my_id)?;
    drop(guard);
    if roster_changed {
        deliver_group_state(&app, &state, &previous);
    }
    Ok(dto)
}

#[tauri::command]
pub(crate) fn add_group_members(
    app: AppHandle,
    id: String,
    member_ids: Vec<String>,
) -> Result<GroupDto, String> {
    roster_change(app, id, member_ids, true)
}

#[tauri::command]
pub(crate) fn remove_group_member(
    app: AppHandle,
    id: String,
    peer_id: String,
) -> Result<GroupDto, String> {
    roster_change(app, id, vec![peer_id], false)
}

fn roster_change(
    app: AppHandle,
    id: String,
    peer_ids: Vec<String>,
    add: bool,
) -> Result<GroupDto, String> {
    let id = parse_chat(&id)?;
    let mut changed: Vec<String> = Vec::new();
    for peer in peer_ids {
        let peer = peer.trim().to_owned();
        if !peer.is_empty() {
            changed.push(peer);
        }
    }
    if changed.is_empty() {
        return Err("Pick the peers to change.".into());
    }
    let my_id = my_peer_id(&app);
    let mut guard = store(&app)?;
    let mut state = guard
        .get_group_state(&id)?
        .ok_or("That group does not exist on this device.")?;
    if add {
        let known: HashSet<String> = guard
            .known_peers()?
            .into_iter()
            .map(|peer| peer.id)
            .collect();
        for member in &changed {
            if !known.contains(member) {
                return Err("A group member must be a peer you have connected to.".into());
            }
        }
    }
    let mut members = state.members.clone();
    for peer in &changed {
        if add {
            if !members.iter().any(|member| member == peer) {
                members.push(peer.clone());
            }
        } else {
            members.retain(|member| member != peer);
        }
    }
    if members == state.members {
        return group_dto(&guard, &state, &my_id);
    }
    let previous = state.members.clone();
    state
        .update_members(&identity(&app), members)
        .map_err(|error| format!("Roster change refused: {error:?}"))?;
    guard.save_group_state(&state)?;
    let dto = group_dto(&guard, &state, &my_id)?;
    drop(guard);
    deliver_group_state(&app, &state, &previous);
    Ok(dto)
}

#[tauri::command]
pub(crate) fn leave_group(app: AppHandle, id: String) -> Result<(), String> {
    let id = parse_chat(&id)?;
    if id == EVERYONE_GROUP_ID {
        return Err("The automatic Everyone group cannot be left.".into());
    }
    let mut guard = store(&app)?;
    let state = guard
        .get_group_state(&id)?
        .ok_or("That group does not exist on this device.")?;
    // A roster change must be signed by its admin, so a member can only hide the group
    // locally; an admin hands it on to nobody and drops it the same way.
    guard.set_setting(&key::left(&id), &true, now_ms())?;
    drop(state);
    Ok(())
}

#[tauri::command]
pub(crate) fn list_announcements(
    app: AppHandle,
    group_id: String,
) -> Result<Vec<AnnouncementDto>, String> {
    let group_id = parse_chat(&group_id)?;
    let guard = store(&app)?;
    let all = names(&app, &guard);
    let mut out = Vec::new();
    for announcement in guard.announcements_for_group(&group_id)? {
        out.push(announcement_dto(&guard, &announcement, &all)?);
    }
    Ok(out)
}

#[tauri::command]
pub(crate) fn send_announcement(
    app: AppHandle,
    group_id: String,
    text: String,
) -> Result<AnnouncementDto, String> {
    let group_id = parse_chat(&group_id)?;
    let body = text.trim().to_owned();
    if body.is_empty() || body.len() > MAX_BODY_BYTES {
        return Err(format!(
            "Announcement must contain 1–{MAX_BODY_BYTES} bytes."
        ));
    }
    let my_id = my_peer_id(&app);
    let mut guard = store(&app)?;
    let (admin, members) = if group_id == EVERYONE_GROUP_ID {
        // Everyone has no elected admin, so the local device posts to its own list.
        (my_id.clone(), roster_of(&guard, &group_id, &my_id))
    } else {
        let state = guard
            .get_group_state(&group_id)?
            .ok_or("That group does not exist on this device.")?;
        (state.admin.clone(), state.members.clone())
    };
    // The live settings are what the user toggles; the stored key is only the
    // snapshot carried into each announcement row.
    let admins_only = app
        .state::<platform::SettingsState>()
        .snapshot()
        .announcements_from_admins_only;
    check_announcement_authority(&my_id, &admin, &members, admins_only)
        .map_err(|violation| format!("Announcement refused: {violation:?}"))?;
    let now = now_ms();
    let announcement = Announcement {
        id: Uuid::now_v7(),
        group_id,
        sender: my_id,
        body,
        clock: Hlc {
            physical_ms: now,
            logical: 0,
        },
        admins_only,
        created_ms: now,
    };
    guard.save_announcement(&announcement)?;
    let all = names(&app, &guard);
    let dto = announcement_dto(&guard, &announcement, &all)?;
    drop(guard);
    deliver_announcement(&app, &announcement, &members);
    Ok(dto)
}

#[tauri::command]
pub(crate) fn acknowledge_announcement(
    app: AppHandle,
    announcement_id: String,
) -> Result<Option<AnnouncementDto>, String> {
    let id = parse_id(&announcement_id, "Announcement id")?;
    let my_id = my_peer_id(&app);
    let mut guard = store(&app)?;
    let announcement = guard
        .acknowledgements_for_announcement(id)?
        .map(|summary| summary.announcement)
        .ok_or("That announcement is not in this database.")?;
    let already = guard
        .acknowledgements_for_announcement(id)?
        .map(|summary| summary.acks.iter().any(|ack| ack.peer_id == my_id))
        .unwrap_or(false);
    if !already {
        guard.acknowledge_announcement(id, &my_id, now_ms())?;
    }
    let all = names(&app, &guard);
    let dto = announcement_dto(&guard, &announcement, &all).map(Some)?;
    drop(guard);
    crate::refresh_announcement_bubbles(&app);
    if !already && announcement.sender != my_id {
        let envelope = Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: frame::ANNOUNCEMENT_ACK.into(),
            from: my_id.clone(),
            to: announcement.sender.clone(),
            hlc: Hlc {
                physical_ms: now_ms(),
                logical: 0,
            },
            payload: serde_json::json!({"announcement_id": id.to_string()}),
        };
        if let Ok(mut queued) = store(&app) {
            let _ = queued.enqueue(&announcement.sender, &envelope);
        }
        deliver_to_peers(&app, std::slice::from_ref(&announcement.sender));
    }
    Ok(dto)
}

// ------------------------------------------------------------------------------------
// Group and announcement transport.
// ------------------------------------------------------------------------------------

/// The kinds this build takes ownership of inside the session reader.
///
/// Anything outside this list and outside the core, file and call sets ends the session,
/// so every name here has to be a type the protocol declares. `frame_table_tests` in
/// `main.rs` checks both directions against the table rather than against a second copy.
pub(crate) const OWNED_KINDS: &[&str] = &[
    frame::GROUP_STATE,
    frame::ANNOUNCEMENT,
    frame::ANNOUNCEMENT_ACK,
    frame::AVATAR_REQUEST,
    frame::AVATAR_DATA,
    frame::MESSAGE_EDIT,
    frame::MESSAGE_DELETE,
    frame::REACTION,
    frame::PIN_UPDATE,
];

/// Roster, announcement, avatar and thread frames the peer session must accept.
pub(crate) fn handled_kind(kind: &str) -> bool {
    OWNED_KINDS.contains(&kind)
}

/// Fans a signed roster out to every member (and to peers the change removed).
///
/// Only the admin signs roster changes, so the frame's sender is always the admin and a
/// receiver can bind the state's public key to the already-pinned session identity.
fn deliver_group_state(app: &AppHandle, state: &GroupState, previous: &[String]) {
    let my_id = my_peer_id(app);
    let mut recipients: BTreeSet<String> = state.members.iter().cloned().collect();
    recipients.extend(previous.iter().cloned());
    recipients.remove(&my_id);
    let Ok(payload) = serde_json::to_value(state) else {
        return;
    };
    let stamp = now_ms();
    let frames: Vec<(String, Envelope<serde_json::Value>)> = recipients
        .into_iter()
        .map(|to| {
            (
                to.clone(),
                Envelope {
                    v: PROTOCOL_VERSION,
                    id: Uuid::now_v7(),
                    kind: frame::GROUP_STATE.into(),
                    from: my_id.clone(),
                    to,
                    hlc: Hlc {
                        physical_ms: stamp,
                        logical: 0,
                    },
                    payload: payload.clone(),
                },
            )
        })
        .collect();
    queue_frames(app, frames);
}

/// What an `announcement` frame carries, under the keys `handle_announcement_frame` reads.
///
/// Split out of `deliver_announcement` so a test can hand the declared protocol table the bytes
/// this writer actually produces: the table and the wire used to disagree about `text` here,
/// exactly as they did for a `message` frame, and a round-trip fixture could not see it.
fn announcement_payload(announcement: &Announcement) -> serde_json::Value {
    serde_json::json!({
        "announcement_id": announcement.id.to_string(),
        "group_id": announcement.group_id.clone(),
        "text": announcement.body.clone(),
        "created_ms": announcement.created_ms,
        "admins_only": announcement.admins_only,
    })
}

/// Queues one announcement frame per other member of its group roster.
fn deliver_announcement(app: &AppHandle, announcement: &Announcement, members: &[String]) {
    let my_id = my_peer_id(app);
    let recipients: BTreeSet<String> = members
        .iter()
        .filter(|member| member.as_str() != my_id)
        .cloned()
        .collect();
    let payload = announcement_payload(announcement);
    let frames: Vec<(String, Envelope<serde_json::Value>)> = recipients
        .into_iter()
        .map(|to| {
            (
                to.clone(),
                Envelope {
                    v: PROTOCOL_VERSION,
                    id: Uuid::now_v7(),
                    kind: frame::ANNOUNCEMENT.into(),
                    from: my_id.clone(),
                    to,
                    hlc: announcement.clock,
                    payload: payload.clone(),
                },
            )
        })
        .collect();
    queue_frames(app, frames);
}

/// Persists frames first, then hands every recipient to a delivery thread.
fn queue_frames(app: &AppHandle, frames: Vec<(String, Envelope<serde_json::Value>)>) {
    if frames.is_empty() {
        return;
    }
    if let Ok(mut guard) = store(app) {
        for (recipient, envelope) in &frames {
            if let Err(error) = guard.enqueue(recipient, envelope) {
                eprintln!(
                    "Queueing a {kind} frame failed: {error}",
                    kind = envelope.kind
                );
            }
        }
    }
    let recipients: Vec<String> = frames.into_iter().map(|(peer, _)| peer).collect();
    deliver_to_peers(app, &recipients);
}

/// Starts one delivery attempt per peer; peers without an endpoint keep their retry loop.
fn deliver_to_peers(app: &AppHandle, peers: &[String]) {
    let endpoints = app
        .state::<OverlayState>()
        .endpoints
        .lock()
        .map(|map| map.clone())
        .unwrap_or_default();
    for peer in peers {
        match endpoints.get(peer).cloned() {
            Some(beacon) => {
                let app = app.clone();
                let peer = peer.clone();
                let _ = std::thread::Builder::new()
                    .name("alverchat-delivery".into())
                    .spawn(move || flush_outbox(&app, &peer, &beacon));
            }
            None => start_outbox_retry(app.clone(), peer.clone()),
        }
    }
}

/// Accepts a roster delivered by its own admin.
pub(crate) fn handle_group_frame(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let state: GroupState = serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("group frame is malformed: {error}"))?;
    if !state.verify() {
        return Err("group frame carries an invalid signature".into());
    }
    if state.admin != remote {
        return Err("group frames must come from the group admin".into());
    }
    if state.id == EVERYONE_GROUP_ID {
        return Err("the automatic Everyone roster cannot be replaced".into());
    }
    let my_id = my_peer_id(app);
    let mut guard = store(app)?;
    let accepted = match guard.get_group_state(&state.id)? {
        Some(previous) if previous == state => false,
        Some(previous) => {
            if !state.verify_successor_of(&previous) {
                return Err("group update is stale or skips an epoch".into());
            }
            guard.save_group_state(&state)?;
            true
        }
        None => {
            guard.save_group_state(&state)?;
            true
        }
    };
    if accepted {
        // A frame that drops this device from the roster removes the group from the rail;
        // an epoch that adds us back clears the flag again.
        guard.set_setting(&key::left(&state.id), &!state.is_member(&my_id), now_ms())?;
        drop(guard);
        let _ = app.emit("messages-changed", state.id.clone());
    }
    Ok(())
}

/// Accepts an announcement from a member authorised under the group's roster.
pub(crate) fn handle_announcement_frame(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let announcement_id = envelope
        .payload
        .get("announcement_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("announcement frame has no id")?;
    let group_id = envelope
        .payload
        .get("group_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("announcement frame has no group")?;
    let body = envelope
        .payload
        .get("text")
        .and_then(serde_json::Value::as_str)
        .ok_or("announcement frame has no text")?;
    let admins_only = envelope
        .payload
        .get("admins_only")
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false);
    let created_ms = envelope
        .payload
        .get("created_ms")
        .and_then(serde_json::Value::as_u64)
        .unwrap_or_else(now_ms);
    let id = parse_id(announcement_id, "Announcement id")?;
    let my_id = my_peer_id(app);
    let mut guard = store(app)?;
    let (admin, members) = if group_id == EVERYONE_GROUP_ID {
        (String::new(), roster_of(&guard, group_id, &my_id))
    } else {
        let state = guard
            .get_group_state(group_id)?
            .ok_or("announcement names a group this device does not know")?;
        (state.admin.clone(), state.members.clone())
    };
    check_announcement_authority(remote, &admin, &members, admins_only)
        .map_err(|violation| format!("Announcement refused: {violation:?}"))?;
    let saved = guard.save_announcement(&Announcement {
        id,
        group_id: group_id.to_owned(),
        sender: remote.to_owned(),
        body: body.to_owned(),
        clock: Hlc {
            physical_ms: envelope.hlc.physical_ms,
            logical: envelope.hlc.logical,
        },
        admins_only,
        created_ms,
    })?;
    drop(guard);
    if saved {
        crate::refresh_announcement_bubbles(app);
        let _ = app.emit("messages-changed", group_id.to_owned());
    }
    Ok(())
}

/// Records a peer's acknowledgement of an announcement this device can see.
pub(crate) fn handle_announcement_ack(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let raw = envelope
        .payload
        .get("announcement_id")
        .and_then(serde_json::Value::as_str)
        .ok_or("announcement ack has no id")?;
    let id = parse_id(raw, "Announcement id")?;
    let mut guard = store(app)?;
    let Some(summary) = guard.acknowledgements_for_announcement(id)? else {
        return Err("announcement ack names an unknown announcement".into());
    };
    let already = summary.acks.iter().any(|ack| ack.peer_id == remote);
    if !already {
        guard.acknowledge_announcement(id, remote, now_ms())?;
    }
    let group_id = summary.announcement.group_id;
    drop(guard);
    if !already {
        let _ = app.emit("messages-changed", group_id);
    }
    Ok(())
}

// ------------------------------------------------------------------------------------
// Calls and profile.
// ------------------------------------------------------------------------------------

#[tauri::command]
pub(crate) fn list_calls(
    app: AppHandle,
    chat_id: Option<String>,
) -> Result<Vec<CallEventDto>, String> {
    if let Some(raw) = chat_id {
        return call_events(&app, &parse_chat(&raw)?);
    }
    let mut chats: Vec<String> = Vec::new();
    {
        let guard = store(&app)?;
        chats.extend(guard.known_peers()?.into_iter().map(|peer| peer.id));
        chats.extend(guard.all_group_states()?.into_iter().map(|group| group.id));
        chats.push(EVERYONE_GROUP_ID.into());
    }
    let mut out = Vec::new();
    for chat in chats {
        out.extend(call_events(&app, &chat)?);
    }
    out.sort_by_key(|row| std::cmp::Reverse(row.started_ms));
    Ok(out)
}

#[tauri::command]
pub(crate) fn set_avatar(
    app: AppHandle,
    avatar_data_url: Option<String>,
) -> Result<Option<String>, String> {
    let my_id = my_peer_id(&app);
    let mut guard = store(&app)?;
    let Some(url) = avatar_data_url.filter(|value| !value.trim().is_empty()) else {
        guard.forget_avatar(&my_id)?;
        return Ok(None);
    };
    let (media_type, width, height, bytes) = decode_avatar_url(&url)?;
    let stored = guard.cache_avatar(&my_id, &bytes, width, height, media_type, now_ms())?;
    Ok(Some(encode_avatar_url(&stored.media_type, &stored.bytes)))
}

#[tauri::command]
pub(crate) fn get_avatar(app: AppHandle) -> Result<Option<String>, String> {
    let my_id = my_peer_id(&app);
    let guard = store(&app)?;
    Ok(guard
        .avatar_for_peer(&my_id)?
        .map(|avatar| encode_avatar_url(&avatar.media_type, &avatar.bytes)))
}

/// The photo a peer has published, as this device last verified it.
#[tauri::command]
pub(crate) fn peer_avatar(app: AppHandle, peer_id: String) -> Result<Option<String>, String> {
    let peer_id = parse_chat(&peer_id)?;
    let guard = store(&app)?;
    Ok(guard
        .avatar_for_peer(&peer_id)?
        .map(|avatar| encode_avatar_url(&avatar.media_type, &avatar.bytes)))
}

/// A blake3 digest the way the wire carries it: sixty-four hex characters.
fn is_avatar_hash(value: &str) -> bool {
    value.len() == 64 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

/// The hash of the photo this profile publishes, which is what a heartbeat pong offers to a
/// peer. `None` when the profile has no photo, which the pong says as an empty string.
pub(crate) fn my_avatar_hash(app: &AppHandle) -> Option<String> {
    let my_id = my_peer_id(app);
    store(app)
        .ok()?
        .avatar_for_peer(&my_id)
        .ok()
        .flatten()
        .map(|avatar| avatar.hash)
}

/// What a peer's declared avatar hash means for the copy this device holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum AvatarAction {
    /// The peer declared nothing — an older build, or one whose profile is private. The
    /// cached copy is left alone rather than emptied on a guess.
    Ignore,
    /// Nothing to do: we already hold what the peer declared, or it has none and we do not
    /// either.
    Keep,
    /// The peer publishes something we do not have. Ask for it.
    Pull,
    /// The peer says it publishes nothing, so stop drawing the photo it used to share.
    Forget,
}

/// The decision, split out so a test can drive every combination of a declared hash against
/// the states the cache can be in.
pub(crate) fn avatar_action(declared: Option<&str>, cached: Option<&str>) -> AvatarAction {
    let Some(declared) = declared.map(str::trim) else {
        return AvatarAction::Ignore;
    };
    if declared.is_empty() {
        return if cached.is_some() {
            AvatarAction::Forget
        } else {
            AvatarAction::Keep
        };
    }
    if !is_avatar_hash(declared) {
        return AvatarAction::Ignore;
    }
    if cached == Some(declared) {
        AvatarAction::Keep
    } else {
        AvatarAction::Pull
    }
}

/// Folds one heartbeat reply's avatar declaration into this device's cache: pull what the
/// peer offers, drop what it withdraws.
///
/// Only ever a response to a frame that peer sent us on a session it holds, so a peer that
/// stops beating keeps the copy we have until it says otherwise.
pub(crate) fn sync_peer_avatar(app: &AppHandle, peer_id: &str, payload: &serde_json::Value) {
    let declared = payload
        .get("avatar")
        .and_then(serde_json::Value::as_str)
        .map(|value| value.trim().to_owned());
    let cached = {
        let Ok(guard) = store(app) else {
            return;
        };
        guard
            .avatar_for_peer(peer_id)
            .ok()
            .flatten()
            .map(|avatar| avatar.hash)
    };
    match avatar_action(declared.as_deref(), cached.as_deref()) {
        AvatarAction::Ignore | AvatarAction::Keep => {}
        AvatarAction::Forget => {
            let cleared = {
                let Ok(mut guard) = store(app) else {
                    return;
                };
                guard.forget_avatar(peer_id).unwrap_or(false)
            };
            if cleared {
                let _ = app.emit("avatars-changed", peer_id.to_owned());
            }
        }
        AvatarAction::Pull => {
            let my_id = my_peer_id(app);
            let Ok(payload) = serde_json::to_value(AvatarRequestPayload {
                peer_id: my_id.clone(),
                have_hash: cached,
            }) else {
                return;
            };
            queue_frames(
                app,
                vec![(
                    peer_id.to_owned(),
                    Envelope {
                        v: PROTOCOL_VERSION,
                        id: Uuid::now_v7(),
                        kind: frame::AVATAR_REQUEST.into(),
                        from: my_id,
                        to: peer_id.to_owned(),
                        hlc: Hlc {
                            physical_ms: now_ms(),
                            logical: 0,
                        },
                        payload,
                    },
                )],
            );
        }
    }
}

/// Answers a peer's request for this profile's photo.
///
/// The reply rides the durable outbox like every other unsolicited frame, so a peer that is
/// briefly unreachable still gets it. An answer this device cannot carry is simply withheld:
/// the requester asks again on the next heartbeat rather than receiving half a file.
pub(crate) fn handle_avatar_request(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let request: AvatarRequestPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|_| "avatar request is malformed".to_owned())?;
    if request.peer_id != remote {
        return Err("avatar request names a peer other than the one that sent it".into());
    }
    // A profile that hides its status from the network hides its face too: the photo is part
    // of the same declaration a pong carries, so the same switch must decide both, or a
    // direct request would be a way to read what "Nobody" was meant to keep private.
    if platform::status_is_private(&platform::load_settings(app).who_sees_status) {
        return Ok(());
    }
    let my_id = my_peer_id(app);
    let mine = {
        let guard = store(app)?;
        guard.avatar_for_peer(&my_id)?
    };
    let Some(avatar) = mine else {
        return Ok(());
    };
    let have = request
        .have_hash
        .map(|value| value.trim().to_owned())
        .filter(|value| !value.is_empty());
    if have.as_deref() == Some(avatar.hash.as_str()) || avatar.bytes.len() > MAX_AVATAR_BYTES {
        return Ok(());
    }
    let Ok(payload) = serde_json::to_value(AvatarDataPayload {
        peer_id: my_id.clone(),
        hash: avatar.hash,
        media_type: avatar.media_type.to_owned(),
        width: avatar.width,
        height: avatar.height,
        bytes: avatar.bytes.clone(),
    }) else {
        return Ok(());
    };
    queue_frames(
        app,
        vec![(
            remote.to_owned(),
            Envelope {
                v: PROTOCOL_VERSION,
                id: Uuid::now_v7(),
                kind: frame::AVATAR_DATA.into(),
                from: my_id,
                to: remote.to_owned(),
                hlc: Hlc {
                    physical_ms: now_ms(),
                    logical: 0,
                },
                payload,
            },
        )],
    );
    Ok(())
}

/// Caches a photo a peer sent, but only from the peer that declared it and only after the
/// bytes hash to the digest the offer carries.
pub(crate) fn handle_avatar_data(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let data: AvatarDataPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|_| "avatar data is malformed".to_owned())?;
    if data.peer_id != remote {
        return Err("avatar data came from a peer other than the one it names".into());
    }
    // An empty body is the "nothing changed" answer to a request the peer no longer needs to
    // satisfy; there is nothing to verify and nothing to store.
    if data.bytes.is_empty() {
        return Ok(());
    }
    if data.bytes.len() > MAX_AVATAR_BYTES {
        return Err("avatar payload is larger than a control frame carries".into());
    }
    if !is_avatar_hash(data.hash.trim()) {
        return Err("avatar offer carries no usable hash".into());
    }
    let stored = {
        let mut guard = store(app)?;
        guard.cache_offered_avatar(&Avatar {
            peer_id: remote.to_owned(),
            media_type: data.media_type,
            width: data.width,
            height: data.height,
            hash: data.hash,
            bytes: data.bytes,
            updated_ms: now_ms(),
        })?
    };
    let _ = app.emit("avatars-changed", stored.peer_id);
    Ok(())
}

// ------------------------------------------------------------------------------------
// Thread convergence: edits, unsends and reactions.
// ------------------------------------------------------------------------------------

/// How far ahead of this device's clock a peer's frame may run before it is refused.
const MAX_CLOCK_SKEW_MS: u64 = 5 * 60 * 1000;

/// The readers whose copy of a chat has to hear that something inside it changed.
///
/// Pure so a test can prove a group change reaches the whole roster, a direct one names
/// exactly the peer at the other end, and no device is ever queued a frame to itself.
pub(crate) fn thread_audience(candidates: &[String], my_id: &str) -> Vec<String> {
    candidates
        .iter()
        .filter(|peer| peer.as_str() != my_id)
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// Every other reader of a chat: its roster when it is a group, its one peer when it is not.
fn chat_audience(guard: &Store, chat_id: &str, my_id: &str) -> Vec<String> {
    let candidates = if is_group(guard, chat_id) {
        roster_of(guard, chat_id, my_id)
    } else {
        vec![chat_id.to_owned()]
    };
    thread_audience(&candidates, my_id)
}

/// True when `peer` is one of the readers whose copy of a chat this device keeps in step.
///
/// This is `chat_audience` read in the other direction, and that symmetry is the point: the
/// set a change is sent to is exactly the set allowed to send it. A frame naming a chat its
/// sender is not part of is not a claim about a shared thread, and a thread frame is dropped
/// rather than refused outright because a stranger naming one of our chats is a bug to be
/// survived, not a reason to redial forever.
pub(crate) fn shares_chat(audience: &[String], peer: &str) -> bool {
    audience.iter().any(|member| member == peer)
}

/// Which of this device's chats a frame's `chat_id` names.
///
/// A direct thread is stored under the id of the peer at the other end, so the same
/// conversation has two names: the sender calls it ours, and we call it theirs. A thread frame
/// about a 1:1 chat therefore always arrives naming our own id, and comparing that name as it
/// stands matches nothing — which is how a reaction or a pin could cross the wire, be acked,
/// and still never appear. A frame that names nothing, or names us, is about the thread with
/// the peer we heard it from; a group id is the same on every device and passes through.
pub(crate) fn thread_chat_id(named: &str, my_id: &str, remote: &str) -> String {
    if named.is_empty() || named == my_id {
        remote.to_owned()
    } else {
        named.to_owned()
    }
}

/// Which of this device's chats a frame leaving for a peer belongs to.
///
/// `messages-changed` carries a thread id, and the shell clears unread on it, so it must not
/// carry a device id. Every fan-out row is addressed to one *member* in `to` — a group post, an
/// announcement, a reaction, a pin, an edit, a delete — while the thread they belong to is the
/// group. Handing out `to` therefore named an unrelated 1:1 chat whenever a group frame flushed
/// to that peer, and the compact rail's group row never ticked for the post it was about. The
/// writers already name the thread in the payload; an edit or a delete is about a message this
/// device has, so its own row says where it lives. `to` stays the last resort, and for a direct
/// frame `to` and the thread are the same id by definition.
///
/// Takes the store lock, so callers must not hold it.
pub(crate) fn frame_thread(app: &AppHandle, envelope: &Envelope<serde_json::Value>) -> String {
    let stored = referenced_message(envelope).and_then(|id| message_chat(app, id));
    thread_from_names(payload_thread(envelope), stored, &envelope.to)
}

/// The precedence the answer is built from: what the payload names, then what the store says
/// about the message the frame is about, then the recipient — which for a direct row is the
/// thread, and for anything else is only the best guess left.
fn thread_from_names(named: Option<String>, stored: Option<String>, recipient: &str) -> String {
    named.or(stored).unwrap_or_else(|| recipient.to_owned())
}

/// The thread a frame's own payload names, when it names one.
///
/// Split out from `frame_thread` so the shapes can be pinned against the writers that produce
/// them: every key here is what `serde_json::to_value` of the corresponding payload struct, or
/// `message_payload`, actually puts on the wire.
fn payload_thread(envelope: &Envelope<serde_json::Value>) -> Option<String> {
    let payload = &envelope.payload;
    let named = |key: &str| {
        payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    };
    let in_message = |key: &str| {
        payload
            .get("message")
            .and_then(|message| message.get(key))
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.is_empty())
            .map(str::to_owned)
    };
    named("chat_id")
        .or_else(|| named("group_id"))
        .or_else(|| in_message("chat_id"))
}

/// The stored message a thread frame is about, when it names one.
fn referenced_message(envelope: &Envelope<serde_json::Value>) -> Option<Uuid> {
    let raw = |key: &str| {
        envelope
            .payload
            .get(key)
            .and_then(serde_json::Value::as_str)
            .or_else(|| {
                envelope
                    .payload
                    .get("message")
                    .and_then(|message| message.get(key))
                    .and_then(serde_json::Value::as_str)
            })
    };
    raw("message_id").and_then(|id| Uuid::parse_str(id).ok())
}

/// Queues one thread frame per reader of its chat so their copies converge on ours.
///
/// The durable outbox owns every reader that is not reachable right now, so a reaction made
/// while a peer is away still arrives when it reappears — and the UI is never made to wait on
/// a handshake.
fn propagate_change(app: &AppHandle, chat_id: &str, kind: &str, payload: serde_json::Value) {
    let my_id = my_peer_id(app);
    let recipients = {
        let Ok(guard) = store(app) else {
            return;
        };
        chat_audience(&guard, chat_id, &my_id)
    };
    let stamp = now_ms();
    let frames = recipients
        .into_iter()
        .map(|to| {
            (
                to.clone(),
                Envelope {
                    v: PROTOCOL_VERSION,
                    id: Uuid::now_v7(),
                    kind: kind.to_owned(),
                    from: my_id.clone(),
                    to,
                    hlc: Hlc {
                        physical_ms: stamp,
                        logical: 0,
                    },
                    payload: payload.clone(),
                },
            )
        })
        .collect();
    queue_frames(app, frames);
}

/// The chat one stored message lives in, or `None` when this device does not have it.
pub(crate) fn message_chat(app: &AppHandle, message_id: Uuid) -> Option<String> {
    store(app)
        .ok()?
        .get_message(message_id)
        .ok()
        .flatten()
        .map(|message| message.chat_id)
}

/// Carries an edit this device made to the thread's other readers.
///
/// The composer's command lives in the shell, so this is the seam it calls once its own store
/// write has landed. The frame carries the same clock the local revision was written under,
/// which is what lets both copies agree on what replaced what.
pub(crate) fn propagate_edit(app: &AppHandle, message_id: Uuid, body: &str, clock: Hlc) {
    let Some(chat_id) = message_chat(app, message_id) else {
        return;
    };
    let Ok(payload) = serde_json::to_value(MessageEditPayload {
        message_id,
        body: body.to_owned(),
        clock,
    }) else {
        return;
    };
    propagate_change(app, &chat_id, frame::MESSAGE_EDIT, payload);
}

/// How far a peer's clock may be ahead and still be believed.
///
/// The bound is the one every incoming message is held to: past it, an "edit" would be a way
/// to move a message to a time that has not happened, so it is refused instead of stored.
pub(crate) fn clock_is_plausible(physical_ms: u64, now_ms: u64) -> bool {
    physical_ms <= now_ms.saturating_add(MAX_CLOCK_SKEW_MS)
}

/// What an inbound `message_delete` asks this device to do.
///
/// Delete-for-me is a claim about the sender's *own* history; honouring it here would let any
/// peer erase a message from a device that never agreed to it. Only the durable
/// delete-for-everyone path converges, and this device never sends the other one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UnsendAction {
    /// Retire the message on every device in the chat.
    Tombstone,
    /// A delete-for-me request, which changes nothing here.
    Ignore,
}

pub(crate) fn unsend_action(for_everyone: bool) -> UnsendAction {
    if for_everyone {
        UnsendAction::Tombstone
    } else {
        UnsendAction::Ignore
    }
}

/// Applies a peer's edit of a message that peer sent.
///
/// The store owns the authority and the permanence rule and is handed the session peer as the
/// actor, so a frame rewriting someone else's words — or a message already tombstoned or
/// purged — is refused there and lands here as a no-op. A refusal is not a session fault: two
/// devices whose histories diverged should keep talking rather than drop each other.
pub(crate) fn handle_message_edit(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let edit: MessageEditPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("message_edit frame is malformed: {error}"))?;
    if edit.body.is_empty() || edit.body.len() > MAX_BODY_BYTES {
        return Err("edited message text is outside the supported size".into());
    }
    if !clock_is_plausible(edit.clock.physical_ms, now_ms()) {
        return Err("edit timestamp is too far in the future".into());
    }
    let (applied, chat_id) = {
        let mut guard = store(app)?;
        let chat_id = guard
            .get_message(edit.message_id)?
            .map(|message| message.chat_id);
        let applied = guard.edit_message(edit.message_id, remote, &edit.body, edit.clock)?;
        (applied, chat_id)
    };
    if applied {
        if let Some(chat_id) = chat_id {
            refresh_thread(app, &chat_id);
        }
    }
    Ok(())
}

/// Retires one message on this device because the chat agreed it should be gone.
pub(crate) fn handle_message_delete(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let delete: MessageDeletePayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("message_delete frame is malformed: {error}"))?;
    if unsend_action(delete.for_everyone) == UnsendAction::Ignore {
        return Ok(());
    }
    let (removed, chat_id) = {
        let mut guard = store(app)?;
        let chat_id = guard
            .get_message(delete.message_id)?
            .map(|message| message.chat_id);
        // A sender unsending their own message is the ordinary case. The second call is the
        // same rule this device applies to itself, so a group admin's moderation converges on
        // the members they moderate and nowhere else.
        let tombstoned = guard.tombstone_message(delete.message_id, remote)?;
        let removed = match (tombstoned, &chat_id) {
            (true, _) => true,
            (false, Some(chat_id)) => guard
                .admin_tombstone_in_group(delete.message_id, remote, chat_id)
                .unwrap_or(false),
            (false, None) => false,
        };
        (removed, chat_id)
    };
    if removed {
        if let Some(chat_id) = chat_id {
            refresh_thread(app, &chat_id);
        }
    }
    Ok(())
}

/// Draws or clears one peer's reaction chip.
pub(crate) fn handle_reaction(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let reaction: ReactionPayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("reaction frame is malformed: {error}"))?;
    let emoji = reaction.emoji.trim().to_owned();
    if emoji.is_empty() || emoji.len() > MAX_REACTION_BYTES {
        return Err("reaction is outside the supported size".into());
    }
    let (changed, chat_id) = {
        let mut guard = store(app)?;
        let my_id = my_peer_id(app);
        // The frame never names who reacted — the authenticated session does — so a peer can
        // only ever write its own name onto a chip. The chat it does name has to be resolved
        // into ours first, because a direct thread is named by whoever is not reading the
        // frame; then it has to match the one this device stores the message under, or an id
        // from one thread could paint a chip in another. The peer also has to be one of that
        // chat's readers, or a stranger could decorate a thread it was never in.
        let named = thread_chat_id(&reaction.message.chat_id, &my_id, remote);
        let known = guard
            .get_message(reaction.message.message_id)?
            .filter(|message| message.chat_id == named)
            .map(|message| message.chat_id);
        let allowed = known
            .as_ref()
            .is_some_and(|chat| shares_chat(&chat_audience(&guard, chat, &my_id), remote));
        let changed = if !allowed {
            false
        } else if reaction.remove {
            guard.remove_reaction(reaction.message.message_id, remote, &emoji)?
        } else {
            guard.add_reaction(reaction.message.message_id, remote, &emoji, now_ms())?
        };
        (changed, known)
    };
    if changed {
        if let Some(chat_id) = chat_id {
            refresh_thread(app, &chat_id);
        }
    }
    Ok(())
}

/// Pins or unpins one message because a reader of its chat asked for it.
///
/// The sender's `position` is taken as given: two devices that pin into the same chat at the
/// same moment can end up agreeing on the list and disagreeing on its order, and the next pin
/// from either side settles it. That is a cosmetic difference in a shared list, which is a
/// better outcome than this device keeping a private copy it never tells anyone about.
pub(crate) fn handle_pin_update(
    app: &AppHandle,
    remote: &str,
    envelope: &Envelope<serde_json::Value>,
) -> Result<(), String> {
    let pin: PinUpdatePayload = serde_json::from_value(envelope.payload.clone())
        .map_err(|error| format!("pin_update frame is malformed: {error}"))?;
    if envelope.payload.get("pinned").is_none() {
        // `pinned` has a serde default, so a frame that leaves it out would read as an unpin
        // and quietly take a pin away. The field is the whole point of the frame.
        return Err("pin_update frame has no flag".into());
    }
    let my_id = my_peer_id(app);
    let (changed, chat_id) = {
        let mut guard = store(app)?;
        let named = thread_chat_id(&pin.chat_id, &my_id, remote);
        let known = guard
            .get_message(pin.message_id)?
            .filter(|message| message.chat_id == named)
            .map(|message| message.chat_id);
        let allowed = known
            .as_ref()
            .is_some_and(|chat| shares_chat(&chat_audience(&guard, chat, &my_id), remote));
        let changed = if !allowed {
            false
        } else if pin.pinned {
            guard.pin(pin.message_id, pin.position, now_ms(), remote)?
        } else {
            guard.unpin(pin.message_id)?
        };
        (changed, known)
    };
    if changed {
        if let Some(chat_id) = chat_id {
            refresh_thread(app, &chat_id);
        }
    }
    Ok(())
}

/// Repaints a thread after a peer changed it, and refreshes the rail it is listed in.
fn refresh_thread(app: &AppHandle, chat_id: &str) {
    let _ = app.emit("messages-changed", chat_id.to_owned());
    publish(app);
}

#[cfg(test)]
mod thread_chat_id_tests {
    use super::thread_chat_id;

    const ANU: &str = "anu";
    const BALA: &str = "bala";
    const CY: &str = "cy";
    const DESIGN_TEAM: &str = "group-design";

    /// The rule both devices have to agree on: each stores a direct thread under the other
    /// end's id, so each receives the same thread named with its own.
    #[test]
    fn a_direct_thread_arrives_named_with_the_reader_s_own_id() {
        // Anu stores this thread under Bala's id and sends that id; Bala reads it back as its
        // thread with Anu, which is where Bala actually keeps the message.
        assert_eq!(thread_chat_id(BALA, BALA, ANU), ANU);
        assert_eq!(thread_chat_id(ANU, ANU, BALA), BALA);
    }

    /// A group id is the same string on every device, so it is the thread's name as it stands.
    #[test]
    fn a_group_thread_keeps_its_name() {
        assert_eq!(thread_chat_id(DESIGN_TEAM, ANU, BALA), DESIGN_TEAM);
    }

    /// A frame that names nothing is about the session it came in on, and a frame that names a
    /// third peer is about a thread this device will not find — passing it through unchanged is
    /// what keeps the lookup honest instead of quietly redirecting a stranger's id.
    #[test]
    fn an_unnamed_frame_falls_back_to_the_sender_and_a_foreign_name_survives() {
        assert_eq!(thread_chat_id("", ANU, BALA), BALA);
        assert_eq!(thread_chat_id(CY, ANU, BALA), CY);
    }
}

#[cfg(test)]
mod message_payload_tests {
    use super::{frame, message_payload};
    use crate::forwarded_of;
    use alver_core::payloads::{classify, Payload};
    use alver_core::{Envelope, Hlc, PROTOCOL_VERSION};
    use serde_json::{json, Value};
    use uuid::Uuid;

    fn envelope(payload: Value) -> Envelope<Value> {
        Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: frame::MESSAGE.into(),
            from: "peer-a".into(),
            to: "peer-b".into(),
            hlc: Hlc {
                physical_ms: 1,
                logical: 0,
            },
            payload,
        }
    }

    /// The two halves of one agreement: the key `outgoing` writes into a `message` frame is
    /// the key the session reader reads off it. Testing a hand-written payload on both sides
    /// would pass even if the writer had renamed the key.
    #[test]
    fn the_copy_flag_arrives_as_the_flag_the_reader_reads() {
        assert!(forwarded_of(&envelope(message_payload(
            "copy of a message",
            None,
            "chat-1",
            true,
        ))));
        assert!(!forwarded_of(&envelope(message_payload(
            "written here",
            None,
            "chat-1",
            false,
        ))));
    }

    /// What an older build looks like from this side: no key at all, which means an original
    /// message rather than a frame to refuse.
    #[test]
    fn a_frame_without_the_key_is_an_original() {
        assert!(!forwarded_of(&envelope(
            json!({"text": "hi", "chat_id": "chat-1"})
        )));
        // A key of the wrong type is read the same way, and never fails the message.
        assert!(!forwarded_of(&envelope(json!({"forwarded": "yes"}))));
    }

    #[test]
    fn a_forwarded_quote_keeps_both_pointers() {
        let quoted = Uuid::now_v7();
        let payload = message_payload("answering that", Some(quoted), "group-design", true);
        assert_eq!(payload["reply_to"], quoted.to_string());
        assert_eq!(payload["chat_id"], "group-design");
        assert!(forwarded_of(&envelope(payload)));
    }

    /// The declared protocol table parses the same keys this writer produces. A test holding
    /// its own hand-written payload would keep passing while the table refused the frames the
    /// app actually sends, which is exactly how `text` and `body` drifted apart here.
    #[test]
    fn the_frame_this_writer_produces_is_a_message_to_the_protocol_table() {
        let quoted = Uuid::now_v7();
        let payload = message_payload("a copy of something", Some(quoted), "group-design", true);
        match classify(&envelope(payload)) {
            Payload::Message(message) => {
                assert_eq!(message.text, "a copy of something");
                assert_eq!(message.chat_id, "group-design");
                assert_eq!(message.reply_to, Some(quoted));
                assert!(message.forwarded);
            }
            other => {
                panic!("the desktop's own message frame must classify as a message, got {other:?}")
            }
        }
    }
}

#[cfg(test)]
mod chat_audience_tests {
    use super::{chat_audience, shares_chat, EVERYONE_GROUP_ID};
    use alver_core::groups::GroupState;
    use alver_core::identity::Identity;
    use alver_core::storage::Store;

    /// Typing, reactions and pins all ask this one question, so it is the whole difference
    /// between a frame that reaches a group and a frame addressed to the group id itself —
    /// which is no session anybody is holding.
    #[test]
    fn a_group_is_its_roster_and_never_its_own_id() {
        let me = Identity::from_secret([1; 32]);
        let my_id = me.peer_id();
        let state = GroupState::create(
            &me,
            "group-design".into(),
            "Design".into(),
            [my_id.clone(), "peer-b".into(), "peer-c".into()],
        )
        .expect("a roster holding its admin");
        let mut store = Store::open(":memory:", &[7; 32]).unwrap();
        store.save_group_state(&state).unwrap();

        let audience = chat_audience(&store, "group-design", &my_id);
        assert_eq!(audience, vec!["peer-b".to_owned(), "peer-c".to_owned()]);
        assert!(
            !shares_chat(&audience, "group-design"),
            "nobody can be sent a frame addressed to a group"
        );
        assert!(shares_chat(&audience, "peer-b"));
    }

    /// The automatic all-peers group has no stored roster, so its audience is whoever this
    /// device has discovered — the common case for a LAN chat, and the same fan-out typing uses.
    #[test]
    fn the_everyone_group_is_every_discovered_peer_but_oneself() {
        let me = Identity::from_secret([2; 32]);
        let my_id = me.peer_id();
        let other = Identity::from_secret([3; 32]);
        let mut store = Store::open(":memory:", &[8; 32]).unwrap();
        store
            .save_peer(
                &other.peer_id(),
                "Robin",
                &other.verifying_key().to_bytes(),
                10,
            )
            .unwrap();
        assert_eq!(
            chat_audience(&store, EVERYONE_GROUP_ID, &my_id),
            vec![other.peer_id()]
        );
    }

    /// A direct thread is the peer at the other end, and a device never queues itself a frame.
    #[test]
    fn a_direct_thread_is_the_peer_at_the_other_end() {
        let store = Store::open(":memory:", &[9; 32]).unwrap();
        assert_eq!(
            chat_audience(&store, "peer-b", "me"),
            vec!["peer-b".to_owned()]
        );
        assert!(
            chat_audience(&store, "me", "me").is_empty(),
            "no device is sent a frame to itself"
        );
    }
}

#[cfg(test)]
mod frame_thread_tests {
    use super::{
        announcement_payload, frame, message_payload, payload_thread, referenced_message,
        thread_from_names, Announcement, MessageDeletePayload, MessageEditPayload, MessageRef,
        PinUpdatePayload, ReactionPayload, Store,
    };
    use alver_core::payloads::{classify, Payload};
    use alver_core::{Envelope, Hlc, Message, PROTOCOL_VERSION};
    use serde_json::Value;
    use uuid::Uuid;

    /// A device id, which is what every fan-out row puts in `to` — and what must never reach a
    /// listener that reads `messages-changed` as a chat.
    const BALA: &str = "bala-device-id";
    const DESIGN_TEAM: &str = "group-design";

    fn addressed_to(to: &str, kind: &str, payload: Value) -> Envelope<Value> {
        Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: kind.into(),
            from: "me".into(),
            to: to.into(),
            hlc: Hlc {
                physical_ms: 1,
                logical: 0,
            },
            payload,
        }
    }

    fn clock() -> Hlc {
        Hlc {
            physical_ms: 1,
            logical: 0,
        }
    }

    /// The bug this module exists for: a group post is delivered to one member, so `to` names
    /// that member, while the thread it changed is the group. Flushing it while Bala's 1:1
    /// thread was open used to clear *that* chat's unread and leave the group row still.
    #[test]
    fn a_group_post_flushing_to_a_member_names_the_group() {
        let envelope = addressed_to(
            BALA,
            frame::MESSAGE,
            message_payload("shipped tonight", None, DESIGN_TEAM, false),
        );
        assert_eq!(payload_thread(&envelope).as_deref(), Some(DESIGN_TEAM));
        assert_ne!(
            envelope.to, DESIGN_TEAM,
            "the row must be addressed to a member, or this test proves nothing"
        );
    }

    /// A direct thread is stored under the peer's id, so for the one case where `to` and the
    /// thread agree, the payload says the same thing and the rewrite changes nothing.
    #[test]
    fn a_direct_post_names_the_thread_its_recipient_is() {
        let envelope = addressed_to(
            BALA,
            frame::MESSAGE,
            message_payload("see you there", None, BALA, false),
        );
        assert_eq!(payload_thread(&envelope).as_deref(), Some(BALA));
    }

    /// A pin names its chat at the top level and a reaction nests it, because the two structs
    /// grew that way — the reader has to follow both, or one of the two frames silently names
    /// nothing and falls back to the member.
    #[test]
    fn a_pin_and_a_reaction_name_their_chat() {
        let id = Uuid::now_v7();
        let pin = serde_json::to_value(PinUpdatePayload {
            chat_id: DESIGN_TEAM.into(),
            message_id: id,
            pinned: true,
            position: 2,
        })
        .expect("a pin encodes");
        assert_eq!(
            payload_thread(&addressed_to(BALA, frame::PIN_UPDATE, pin)).as_deref(),
            Some(DESIGN_TEAM)
        );

        let reaction = serde_json::to_value(ReactionPayload {
            message: MessageRef {
                chat_id: DESIGN_TEAM.into(),
                message_id: id,
            },
            emoji: "👍".into(),
            remove: false,
        })
        .expect("a reaction encodes");
        let envelope = addressed_to(BALA, frame::REACTION, reaction);
        assert_eq!(
            payload_thread(&envelope).as_deref(),
            Some(DESIGN_TEAM),
            "the chat nested under `message` is still the chat"
        );
        assert_eq!(referenced_message(&envelope), Some(id));
    }

    /// An announcement is addressed to a member and belongs to its group, and its frame is the
    /// one the declared table used to describe with a key this app never sent. Both halves are
    /// asserted from the production writer, because a round-trip fixture cannot see that drift.
    #[test]
    fn an_announcement_names_its_group_and_is_the_frame_the_table_parses() {
        let announcement = Announcement {
            id: Uuid::now_v7(),
            group_id: DESIGN_TEAM.into(),
            sender: "me".into(),
            body: "freeze on monday".into(),
            clock: clock(),
            admins_only: true,
            created_ms: 9,
        };
        let envelope = addressed_to(
            BALA,
            frame::ANNOUNCEMENT,
            announcement_payload(&announcement),
        );
        assert_eq!(payload_thread(&envelope).as_deref(), Some(DESIGN_TEAM));
        match classify(&envelope) {
            Payload::Announcement(parsed) => {
                assert_eq!(parsed.text, "freeze on monday");
                assert_eq!(parsed.group_id, DESIGN_TEAM);
                assert_eq!(parsed.announcement_id, announcement.id);
            }
            other => {
                panic!("this build's own announcement frame must parse, got {other:?}")
            }
        }
    }

    /// An edit and a delete carry no chat at all, so `frame_thread` asks the store which thread
    /// the message they name lives in. These two assertions pin what it hands the store lookup.
    #[test]
    fn an_edit_and_a_delete_are_resolved_through_the_message_they_name() {
        let id = Uuid::now_v7();
        let edit = serde_json::to_value(MessageEditPayload {
            message_id: id,
            body: "corrected".into(),
            clock: clock(),
        })
        .expect("an edit encodes");
        let envelope = addressed_to(BALA, frame::MESSAGE_EDIT, edit);
        assert_eq!(payload_thread(&envelope), None);
        assert_eq!(referenced_message(&envelope), Some(id));

        let delete = serde_json::to_value(MessageDeletePayload {
            message_id: id,
            for_everyone: true,
            reason: None,
        })
        .expect("a delete encodes");
        let envelope = addressed_to(BALA, frame::MESSAGE_DELETE, delete);
        assert_eq!(payload_thread(&envelope), None);
        assert_eq!(referenced_message(&envelope), Some(id));
    }

    /// The precedence itself, so the step that turns "the store knows this message is in the
    /// group" into an answer is a decision with a test rather than a line nobody checked.
    #[test]
    fn the_payload_wins_then_the_store_then_the_recipient() {
        assert_eq!(
            thread_from_names(Some(DESIGN_TEAM.into()), None, BALA),
            DESIGN_TEAM
        );
        assert_eq!(
            thread_from_names(None, Some(DESIGN_TEAM.into()), BALA),
            DESIGN_TEAM
        );
        assert_eq!(thread_from_names(None, None, BALA), BALA);
    }

    /// An edit of a group message, resolved the way `frame_thread` resolves it: the frame names
    /// no chat, so the answer comes out of the row this store actually holds.
    #[test]
    fn an_edit_of_a_group_message_is_answered_by_the_store() {
        let mut store = Store::open(":memory:", &[7; 32]).expect("an in-memory store");
        let message = Message {
            id: Uuid::now_v7(),
            chat_id: DESIGN_TEAM.into(),
            sender: "me".into(),
            body: "original".into(),
            clock: clock(),
            tombstone: false,
            forwarded: false,
        };
        store.save_message(&message).expect("stored");
        let edit = serde_json::to_value(MessageEditPayload {
            message_id: message.id,
            body: "corrected".into(),
            clock: clock(),
        })
        .expect("an edit encodes");
        let envelope = addressed_to(BALA, frame::MESSAGE_EDIT, edit);
        let stored = referenced_message(&envelope)
            .and_then(|id| store.get_message(id).ok().flatten())
            .map(|row| row.chat_id);
        assert_eq!(
            thread_from_names(payload_thread(&envelope), stored, &envelope.to),
            DESIGN_TEAM,
            "a group edit flushing to one member must not name that member's thread"
        );
    }

    /// The last resort inside `frame_thread` is `to`, and it is reached only when neither the
    /// payload names a chat nor a message id is present for the store to resolve. A direct
    /// row's recipient *is* its thread, so that answer is right for exactly the case that
    /// needs it — and this is the shape that gets there: no keys at all.
    #[test]
    fn a_frame_that_names_nothing_leaves_the_recipient_as_the_only_answer() {
        let envelope = addressed_to(
            BALA,
            frame::MESSAGE,
            serde_json::json!({"text": "no chat here"}),
        );
        assert_eq!(payload_thread(&envelope), None);
        assert_eq!(referenced_message(&envelope), None);
    }
}
