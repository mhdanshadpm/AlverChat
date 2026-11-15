#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod calls;
mod commands;
mod drag;
mod file_transfer;
mod platform;
mod secrets;
mod websocket;

use alver_core::{
    discovery::{MdnsDiscovery, PeerBeacon, UdpDiscovery},
    groups::EVERYONE_GROUP_ID,
    identity::Identity,
    payloads::{is_known_type, kind},
    storage::Store,
    transport::{initiator_handshake, responder_handshake, RemoteIdentity, SecureStream},
    Envelope, Hlc, Message, ReceiptKind, PROTOCOL_VERSION,
};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine as _};
use drag::{Edge, Rect};
use mdns_sd::{RecvTimeoutError, ServiceEvent};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{hash_map::Entry, HashMap, HashSet},
    fs, io,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::atomic::{AtomicBool, AtomicU16, AtomicU64, AtomicUsize, Ordering},
    sync::{mpsc, Arc, Mutex},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

/// Port of the TLS peer listener, published for the diagnostics screen.
static LISTENER_PORT: AtomicU16 = AtomicU16::new(0);

pub fn listener_port() -> u16 {
    LISTENER_PORT.load(Ordering::Acquire)
}

/// Identity summary used by exports and the About screen.
pub fn identity_summary(app: &AppHandle) -> IdentityDto {
    let identity = &app.state::<OverlayState>().identity;
    IdentityDto {
        peer_id: identity.peer_id(),
        fingerprint: identity.fingerprint(),
    }
}

/// Tray icon id, so `refresh_tray` can find the icon again after a settings write.
const TRAY_ID: &str = "alverchat-tray";

pub(crate) fn refresh_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = rebuild_tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn rebuild_tray_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, String> {
    let settings = app.state::<platform::SettingsState>().snapshot();
    let (mute_title, dnd_title) = platform::tray_toggle_titles(&settings);
    let item = |id: &str, title: String| {
        MenuItem::with_id(app, id, title, true, None::<&str>).map_err(|error| error.to_string())
    };
    let show = item("show", "Open AlverChat".to_string())?;
    let mute = item("mute", mute_title)?;
    let dnd = item("dnd", dnd_title)?;
    let reset = item("reset-position", "Reset icon position".to_string())?;
    let quit = item("quit", "Quit".to_string())?;
    Menu::with_items(app, &[&show, &mute, &dnd, &reset, &quit]).map_err(|error| error.to_string())
}

/// Who this device is, in the two forms the shell shows.
///
/// Both are spellings of one blake3 digest of the identity key: `peer_id` is the 64 lowercase hex
/// characters the network addresses a device by, `fingerprint` the same characters uppercased and
/// broken into sixteen blocks of four so a user can read them out over another channel. One label
/// means one value — a caller that wants the digest asks for `peerId`, a caller that wants
/// something a human can say aloud asks for `fingerprint`.
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct IdentityDto {
    pub peer_id: String,
    pub fingerprint: String,
}

#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct PeerDto {
    id: String,
    name: String,
    initials: String,
    color: String,
    online: bool,
    status: String,
    /// The presence token the peer declared in its heartbeat reply ("online" / "away" /
    /// "busy"), "" when it is offline or hides its status. `status` stays our own liveness
    /// copy; these two are what the peer says about itself.
    presence: String,
    status_line: String,
    preview: String,
    time: String,
    /// Wall-clock ms of the newest message in this chat, 0 when it has none. The UI
    /// formats the row and bubble timestamps from this; `time` stays a display string
    /// for the browser preview, which has no clock to stamp.
    last_message_ms: u64,
    unread: Option<u32>,
    group: bool,
    typing: bool,
    muted: bool,
    pinned: bool,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct MessageDto {
    id: String,
    sender: String,
    text: String,
    time: String,
    mine: bool,
    read: bool,
    kind: String,
    file_name: Option<String>,
    timestamp_ms: u64,
    /// Id of the message this one quotes, when it is a reply.
    reply_to: Option<String>,
    delivery_status: String,
    /// True once the message has been edited, which is what draws the "(edited)" tag.
    edited: bool,
    /// True when this row is a copy of a message forwarded from somewhere else, which is
    /// what draws the "Forwarded" chip above the bubble.
    forwarded: bool,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct AnnouncementBubbleDto {
    id: String,
    group_id: String,
    group_name: String,
    author_name: String,
    text: String,
    created_ms: u64,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct LiveCallDto {
    chat_id: String,
    /// Display name of the chat, resolved from the live peer list; empty when unknown.
    peer_name: String,
    ongoing: bool,
    /// Wall-clock ms the call connected; 0 while it is still ringing.
    started_ms: u64,
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OverlaySnapshot {
    peers: Vec<PeerDto>,
    unread_total: u32,
    open: bool,
    dock_side: Edge,
    snap_to_edge: bool,
    bubbles_enabled: bool,
    bubble_auto_hide_seconds: u8,
    /// When false the bubble hides the message text, the same rule the toast follows.
    show_message_preview: bool,
    /// Settings → Opacity, as a percentage. The dock fades by this much.
    opacity_percent: u8,
    /// Settings → Idle icon: `logo` or `photo`, i.e. what the icon shows with nothing unread.
    idle_icon: String,
    /// Settings → Icon morph: whether the icon may take the face of the active chat at all.
    icon_morph_enabled: bool,
    /// Settings → Revert delay: how long that face stays after the newest unread.
    icon_morph_delay_seconds: u8,
    /// True while the device is quiet — hand-muted or inside the scheduled window.
    /// The same rule that suppresses the alert, so the icon cannot lie about it.
    notifications_quiet: bool,
    /// The call the floating icon should show, or none.
    live_call: Option<LiveCallDto>,
    /// Unacknowledged announcements, newest first; the dock shows the newest.
    announcements: Vec<AnnouncementBubbleDto>,
}
#[derive(Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct SavedDockPosition {
    monitor_id: String,
    edge: Edge,
    vertical_percent: f64,
}
#[derive(Serialize, serde::Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OverlayPreferences {
    dock_position: Option<SavedDockPosition>,
}
/// A dialed, authenticated session kept open between frames.
struct OutboundSession {
    stream: SecureStream<websocket::BlockingWebSocket>,
    used: Instant,
}

/// How long a warm session may sit idle. The peer closes a silent session after 60 s,
/// so this stays well inside that window.
const OUTBOUND_IDLE: Duration = Duration::from_secs(30);

/// How often an idle warm session is probed. mDNS removal is the authority for peers
/// we are not talking to, but a vanished machine that never announces its departure
/// would otherwise look online until the next beacon refresh.
const HEARTBEAT_INTERVAL: Duration = Duration::from_secs(20);

/// Read receipts travel in batches; this cap keeps each frame far below the control cap.
const MAX_READ_RECEIPT_IDS: usize = 256;

struct OverlayState {
    compact_open: Mutex<bool>,
    peers: Mutex<Vec<PeerDto>>,
    endpoints: Mutex<HashMap<String, PeerBeacon>>,
    /// Warm outbound sessions, so a follow-up frame skips the resolve and handshake.
    outbound: Mutex<HashMap<String, Arc<Mutex<OutboundSession>>>>,
    retrying_peers: Mutex<HashSet<String>>,
    /// When each peer's "typing…" flag lapses, so a dropped frame cannot strand it.
    typing_until: Mutex<HashMap<String, u64>>,
    /// Live progress of the file transfers this process is driving, by transfer id.
    transfers: Mutex<HashMap<Uuid, file_transfer::LiveTransfer>>,
    /// Transfers with a receive driver already running, so two clicks cannot pull twice.
    pulling: Mutex<HashSet<Uuid>>,
    /// Unacknowledged announcements, cached so `snapshot` never takes the store lock.
    announcements: Mutex<Vec<AnnouncementBubbleDto>>,
    /// Calls the shell has seen signalled but not seen end, by call id.
    live_calls: Mutex<HashMap<Uuid, calls::LiveCall>>,
    /// Cached `platform::icon_is_quiet`: the snapshot runs on every pointer move, and the
    /// quiet window is a date calculation, so the tick refreshes it instead.
    quiet: Mutex<bool>,
    identity: Arc<Identity>,
    peer_sessions: Arc<AtomicUsize>,
    /// Cached LAN passphrase: `None` = not read yet, `Some(None)` = no passphrase set.
    lan_passphrase: Mutex<Option<Option<Vec<u8>>>>,
    /// Bumped whenever the passphrase changes, so sessions authenticated under the
    /// previous policy stop carrying traffic until the peer proves the new one.
    auth_epoch: AtomicU64,
    store: Mutex<Store>,
    preferences: Mutex<OverlayPreferences>,
    preferences_path: PathBuf,
    active_drag: Mutex<Option<String>>,
    pointer_capture: Mutex<HashMap<String, bool>>,
    compact_detached: Mutex<bool>,
    cursor_capture: Mutex<HashMap<String, bool>>,
}

struct PeerSessionGuard(Arc<AtomicUsize>);

impl Drop for PeerSessionGuard {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

/// Wall-clock milliseconds since the epoch, clamped for 32-bit casts.
pub(crate) fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

/// How long a received "typing…" flag stays up without another frame.
const TYPING_WINDOW_MS: u64 = 6_000;

/// Applies a peer typing flag and its expiry, then republishes the overlays.
fn apply_typing(app: &AppHandle, peer_id: &str, typing: bool) {
    let state = app.state::<OverlayState>();
    if let Ok(mut until) = state.typing_until.lock() {
        if typing {
            until.insert(peer_id.to_owned(), now_ms() + TYPING_WINDOW_MS);
        } else {
            until.remove(peer_id);
        }
    }
    if let Ok(mut peers) = state.peers.lock() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == peer_id) {
            peer.typing = typing;
        }
    }
    publish(app);
}

/// Drops typing flags whose window passed; returns whether anything changed.
fn expire_typing(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    let now = now_ms();
    let mut stale = Vec::new();
    if let Ok(mut until) = state.typing_until.lock() {
        until.retain(|peer_id, until_ms| {
            let live = *until_ms > now;
            if !live {
                stale.push(peer_id.clone());
            }
            live
        });
    }
    if stale.is_empty() {
        return;
    }
    if let Ok(mut peers) = state.peers.lock() {
        for peer in peers.iter_mut() {
            if stale.contains(&peer.id) {
                peer.typing = false;
            }
        }
    }
    publish(app);
}

fn known_peers(store: &Store) -> Result<Vec<PeerDto>, String> {
    store
        .known_peers()?
        .into_iter()
        .map(|stored| {
            let last = store.messages_for_chat(&stored.id)?.pop();
            let preview = last
                .as_ref()
                .map(|message| {
                    if message.tombstone {
                        "This message was deleted".into()
                    } else {
                        message.body.clone()
                    }
                })
                .unwrap_or_default();
            let last_message_ms = last
                .as_ref()
                .map(|message| message.clock.physical_ms)
                .unwrap_or_default();
            // Read receipts are the durable source: a restart must not re-flag
            // messages the user has already seen.
            let unread = store.unread_messages_for_chat(&stored.id, "local")?;
            Ok(PeerDto {
                id: stored.id,
                name: stored.name.clone(),
                initials: stored
                    .name
                    .split_whitespace()
                    .filter_map(|part| part.chars().next())
                    .take(2)
                    .collect::<String>()
                    .to_uppercase(),
                color: "violet".into(),
                online: false,
                status: "Offline · last seen on LAN".into(),
                presence: String::new(),
                status_line: String::new(),
                preview,
                time: String::new(),
                last_message_ms,
                unread: (unread > 0).then_some(unread as u32),
                group: false,
                typing: false,
                muted: false,
                pinned: false,
            })
        })
        .collect()
}
/// The call the floating icon should wear, with the chat's display name resolved from
/// the peer list the snapshot is already carrying.
fn live_call_dto(state: &OverlayState, peers: &[PeerDto]) -> Option<LiveCallDto> {
    let calls = state.live_calls.lock().ok()?;
    let call = crate::calls::pick_live_call(&calls)?;
    Some(LiveCallDto {
        chat_id: call.chat_id.clone(),
        peer_name: peers
            .iter()
            .find(|peer| peer.id == call.chat_id)
            .map(|peer| peer.name.clone())
            .unwrap_or_default(),
        ongoing: call.ongoing,
        started_ms: call.started_ms,
    })
}
fn snapshot(app: &AppHandle) -> OverlaySnapshot {
    let state = app.state::<OverlayState>();
    let open = state.compact_open.lock().map(|v| *v).unwrap_or(false);
    let peers = state.peers.lock().map(|v| v.clone()).unwrap_or_default();
    let live_call = live_call_dto(&state, &peers);
    let unread_total = peers.iter().map(|peer| peer.unread.unwrap_or(0)).sum();
    let settings = app
        .try_state::<platform::SettingsState>()
        .map(|state| state.snapshot())
        .unwrap_or_default();
    OverlaySnapshot {
        peers,
        unread_total,
        open,
        dock_side: state
            .preferences
            .lock()
            .map(|p| {
                p.dock_position
                    .as_ref()
                    .map(|pos| pos.edge)
                    .unwrap_or(Edge::Right)
            })
            .unwrap_or(Edge::Right),
        snap_to_edge: settings.snap_to_edge,
        bubbles_enabled: settings.bubbles_enabled,
        bubble_auto_hide_seconds: settings.bubble_auto_hide_seconds,
        show_message_preview: settings.show_message_preview,
        opacity_percent: settings.opacity_percent,
        idle_icon: settings.idle_icon.clone(),
        icon_morph_enabled: settings.icon_morph_enabled,
        icon_morph_delay_seconds: settings.icon_morph_delay_seconds,
        notifications_quiet: state.quiet.lock().map(|quiet| *quiet).unwrap_or(false),
        live_call,
        announcements: state
            .announcements
            .lock()
            .map(|list| list.clone())
            .unwrap_or_default(),
    }
}
fn publish(app: &AppHandle) {
    let _ = app.emit("overlay-state", snapshot(app));
}

/// Re-reads the quiet window into the overlay cache, repainting only when it flips.
///
/// The window opens on the clock, not on an event: without this tick a Do-Not-Disturb
/// icon could keep promising alerts for hours after the window began.
pub(crate) fn refresh_overlay_quiet(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    let settings = app
        .try_state::<platform::SettingsState>()
        .map(|settings| settings.snapshot())
        .unwrap_or_default();
    let quiet = platform::icon_is_quiet(&settings, now_ms());
    let changed = state
        .quiet
        .lock()
        .map(|mut cached| {
            let changed = *cached != quiet;
            *cached = quiet;
            changed
        })
        .unwrap_or(false);
    if changed {
        publish(app);
    }
}

/// Rebuilds the dock's unacknowledged-announcement list from the store.
///
/// Cached instead of queried inside `snapshot`: the snapshot runs on every pointer
/// move, and taking the store lock there would serialize the overlay against writes.
pub(crate) fn refresh_announcement_bubbles(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    let my_id = state.identity.peer_id();
    let mut bubbles = Vec::new();
    if let Ok(store) = state.store.lock() {
        let names: HashMap<String, String> = store
            .known_peers()
            .unwrap_or_default()
            .into_iter()
            .map(|peer| (peer.id, peer.name))
            .collect();
        for announcement in store.pending_announcements(&my_id).unwrap_or_default() {
            let group_name = if announcement.group_id == alver_core::groups::EVERYONE_GROUP_ID {
                "Everyone on this network".to_string()
            } else {
                store
                    .get_group_state(&announcement.group_id)
                    .ok()
                    .flatten()
                    .map(|group| group.name)
                    .unwrap_or_else(|| announcement.group_id.clone())
            };
            bubbles.push(AnnouncementBubbleDto {
                id: announcement.id.to_string(),
                group_id: announcement.group_id,
                group_name,
                author_name: names
                    .get(&announcement.sender)
                    .cloned()
                    .unwrap_or_else(|| announcement.sender.clone()),
                text: announcement.body,
                created_ms: announcement.created_ms,
            });
        }
    }
    if let Ok(mut current) = state.announcements.lock() {
        *current = bubbles;
    }
    publish(app);
}

fn upsert_discovered_peer(app: &AppHandle, beacon: PeerBeacon) {
    let state = app.state::<OverlayState>();
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    if let Ok(mut store) = state.store.lock() {
        if let Err(error) =
            store.save_peer(&beacon.peer_id, &beacon.name, &beacon.public_key, now_ms)
        {
            eprintln!("Could not persist discovered peer: {error}");
        }
    }
    let peer_id = beacon.peer_id.clone();
    if let Ok(mut endpoints) = state.endpoints.lock() {
        endpoints.insert(peer_id.clone(), beacon.clone());
    }
    let Ok(mut peers) = state.peers.lock() else {
        return;
    };
    let mut peer = PeerDto {
        id: peer_id.clone(),
        name: beacon.name.clone(),
        initials: beacon
            .name
            .split_whitespace()
            .filter_map(|part| part.chars().next())
            .take(2)
            .collect::<String>()
            .to_uppercase(),
        color: "violet".into(),
        online: true,
        status: "Available on LAN · end-to-end encrypted".into(),
        presence: String::new(),
        status_line: String::new(),
        preview: String::new(),
        time: String::new(),
        last_message_ms: 0,
        unread: None,
        group: false,
        typing: false,
        muted: false,
        pinned: false,
    };
    if let Some(existing) = peers.iter_mut().find(|peer| peer.id == peer_id) {
        peer.preview.clone_from(&existing.preview);
        peer.time.clone_from(&existing.time);
        peer.presence.clone_from(&existing.presence);
        peer.status_line.clone_from(&existing.status_line);
        peer.last_message_ms = existing.last_message_ms;
        peer.unread = existing.unread;
        peer.muted = existing.muted;
        peer.pinned = existing.pinned;
        *existing = peer;
    } else {
        peers.push(peer);
    }
    drop(peers);
    publish(app);
}

fn start_secure_listener(app: AppHandle, identity: Arc<Identity>) -> Result<u16, String> {
    let tls_config = websocket::server_config(&identity)?;
    let listener = TcpListener::bind(("0.0.0.0", 0)).map_err(|error| error.to_string())?;
    listener
        .set_nonblocking(true)
        .map_err(|error| error.to_string())?;
    let port = listener
        .local_addr()
        .map_err(|error| error.to_string())?
        .port();
    LISTENER_PORT.store(port, Ordering::Release);
    thread::Builder::new()
        .name("alverchat-secure-listener".into())
        .spawn(move || loop {
            match listener.accept() {
                Ok((stream, _)) => {
                    let app = app.clone();
                    let identity = identity.clone();
                    let tls_config = tls_config.clone();
                    let state = app.state::<OverlayState>();
                    let sessions = state.peer_sessions.clone();
                    if sessions
                        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |active| {
                            (active < 32).then_some(active + 1)
                        })
                        .is_err()
                    {
                        drop(stream);
                        continue;
                    }
                    let guard_sessions = sessions.clone();
                    let spawned = thread::Builder::new()
                        .name("alverchat-peer-session".into())
                        .spawn(move || {
                            let _guard = PeerSessionGuard(guard_sessions);
                            if let Err(error) =
                                handle_peer_session(app, identity, stream, tls_config)
                            {
                                eprintln!("Secure peer session ended: {error}");
                            }
                        });
                    if spawned.is_err() {
                        sessions.fetch_sub(1, Ordering::AcqRel);
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50));
                }
                Err(error) => {
                    eprintln!("Secure peer listener stopped: {error}");
                    break;
                }
            }
        })
        .map_err(|error| error.to_string())?;
    Ok(port)
}

/// What the session reader does with a frame it has been handed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FrameAction {
    /// This build has a handler for it.
    Handle,
    /// The peer is closing on purpose, so the session ends here rather than timing out.
    Close,
    /// Declared by the protocol, and nothing in this build acts on it.
    Ignore,
    /// Not a frame of this protocol at all.
    Reject,
}

/// The reader's own kind table, so that "which frames end a session" is one decision with
/// one list instead of a condition spread across four sets.
///
/// `Reject` is deliberately reachable only by a kind `payloads::ALL_TYPES` does not
/// declare. A peer built to `docs/PROTOCOL.md` sends frames this build has no handler for
/// — `hello`, `pin_update`, the four per-change roster kinds — and the protocol's own rule
/// for those is that a receiver ignores them. Treating one as fatal would let a single
/// advisory frame take a peer's messages, files and calls down with the session.
fn frame_action(name: &str) -> FrameAction {
    if file_transfer::handled_kind(name)
        || calls::handled_kind(name)
        || commands::handled_kind(name)
        || matches!(
            name,
            kind::MESSAGE | kind::TYPING | kind::PING | kind::PONG | kind::RECEIPT
        )
    {
        FrameAction::Handle
    } else if name == kind::BYE {
        FrameAction::Close
    } else if is_known_type(name) {
        FrameAction::Ignore
    } else {
        FrameAction::Reject
    }
}

/// Reads the copy flag off a `message` frame.
///
/// A build that predates the flag sends no key, and "nobody forwarded this" is the right
/// reading of a missing key — not a reason to drop the message.
fn forwarded_of(envelope: &Envelope<Value>) -> bool {
    envelope
        .payload
        .get("forwarded")
        .and_then(Value::as_bool)
        .unwrap_or(false)
}

fn handle_peer_session(
    app: AppHandle,
    identity: Arc<Identity>,
    stream: TcpStream,
    tls_config: Arc<rustls::ServerConfig>,
) -> Result<(), String> {
    let stream = websocket::accept(stream, tls_config)?;
    let mut session = responder_handshake(stream, &identity)?;
    let remote = session.remote_identity().clone();
    if remote.peer_id == identity.peer_id() {
        return Err("self-connections are not accepted".into());
    }
    authenticate_session(&app, &mut session, &remote.peer_id)?;
    let epoch = app
        .state::<OverlayState>()
        .auth_epoch
        .load(Ordering::Acquire);
    app.state::<OverlayState>()
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?
        .trust_peer(&remote.peer_id, &remote.public_key)?;

    loop {
        // A passphrase change must end sessions authenticated under the old policy,
        // or enabling encryption would leave an open channel behind.
        if app
            .state::<OverlayState>()
            .auth_epoch
            .load(Ordering::Acquire)
            != epoch
        {
            return Err("LAN passphrase changed; this session must reconnect".into());
        }
        let envelope: Envelope<Value> = session.receive()?;
        let action = frame_action(&envelope.kind);
        if envelope.v != PROTOCOL_VERSION
            || envelope.from != remote.peer_id
            || envelope.to != identity.peer_id()
            || action == FrameAction::Reject
        {
            return Err("incoming message envelope is invalid".into());
        }
        if action == FrameAction::Close {
            // A goodbye is the fastest honest presence update there is: the peer said it
            // is leaving, so nothing here has to wait for a probe and a redial to agree.
            mark_peer_presence(&app, &remote.peer_id, false);
            forget_session(&app, &remote.peer_id);
            return Ok(());
        }
        // Any validated frame proves the peer is alive right now, which repairs
        // presence after a heartbeat was lost without waiting for a new beacon.
        mark_peer_presence(&app, &remote.peer_id, true);
        if action == FrameAction::Ignore {
            // Acknowledged rather than dropped silently: the sender's outbox retires the
            // row on a receipt, so an unanswered frame would be re-sent on every retry
            // for a feature this build simply does not have.
            let acked = envelope.id;
            let stamp = now_ms();
            session.send(&receipt_for(&identity, &remote.peer_id, acked, stamp))?;
            continue;
        }
        let owned_frame = commands::handled_kind(&envelope.kind);
        let call_frame = calls::handled_kind(&envelope.kind);
        if owned_frame {
            match envelope.kind.as_str() {
                kind::GROUP_STATE => {
                    commands::handle_group_frame(&app, &remote.peer_id, &envelope)?
                }
                kind::ANNOUNCEMENT => {
                    commands::handle_announcement_frame(&app, &remote.peer_id, &envelope)?
                }
                kind::ANNOUNCEMENT_ACK => {
                    commands::handle_announcement_ack(&app, &remote.peer_id, &envelope)?
                }
                kind::AVATAR_REQUEST => {
                    commands::handle_avatar_request(&app, &remote.peer_id, &envelope)?
                }
                kind::AVATAR_DATA => {
                    commands::handle_avatar_data(&app, &remote.peer_id, &envelope)?
                }
                kind::MESSAGE_EDIT => {
                    commands::handle_message_edit(&app, &remote.peer_id, &envelope)?
                }
                kind::MESSAGE_DELETE => {
                    commands::handle_message_delete(&app, &remote.peer_id, &envelope)?
                }
                kind::REACTION => commands::handle_reaction(&app, &remote.peer_id, &envelope)?,
                kind::PIN_UPDATE => commands::handle_pin_update(&app, &remote.peer_id, &envelope)?,
                _ => return Err("unknown frame kind".into()),
            }
            let acked = envelope.id;
            let stamp = now_ms();
            session.send(&receipt_for(&identity, &remote.peer_id, acked, stamp))?;
            continue;
        }
        if call_frame {
            calls::handle_signal(&app, &remote.peer_id, &envelope)?;
            let acked = envelope.id;
            let stamp = now_ms();
            session.send(&receipt_for(&identity, &remote.peer_id, acked, stamp))?;
            continue;
        }
        if envelope.kind == kind::FILE_OFFER {
            // The offer carries no bytes; it is stored, receipted, and the pull starts
            // either now (auto-download) or when the user accepts it.
            let replies = file_transfer::handle_offer(&app, &remote.peer_id, &envelope)?;
            let stamp = now_ms();
            session.send(&receipt_for(&identity, &remote.peer_id, envelope.id, stamp))?;
            for reply in replies {
                session.send(&reply)?;
            }
            continue;
        }
        if envelope.kind == kind::FILE_ACCEPT {
            // The receiver owns the data connection, so the chunks stream over this very
            // session until it is done; the loop resumes reading afterwards.
            file_transfer::push_on_session(&app, &mut session, &remote, &envelope)?;
            continue;
        }
        if envelope.kind == kind::FILE_CANCEL {
            file_transfer::handle_cancel(&app, &remote.peer_id, &envelope)?;
            continue;
        }
        if envelope.kind == kind::PING {
            // Liveness only: the pong is the reply, so this frame never reaches the
            // store, the outbox or the unread counters.
            let settings = app
                .try_state::<platform::SettingsState>()
                .map(|state| state.snapshot())
                .unwrap_or_default();
            let pong = Envelope {
                v: PROTOCOL_VERSION,
                id: Uuid::now_v7(),
                kind: kind::PONG.into(),
                from: identity.peer_id(),
                to: remote.peer_id.clone(),
                hlc: Hlc {
                    physical_ms: now_ms(),
                    logical: 0,
                },
                payload: pong_payload(
                    &settings,
                    envelope.id,
                    commands::my_avatar_hash(&app).as_deref(),
                ),
            };
            session.send(&pong)?;
            continue;
        }
        if envelope.kind == kind::PONG {
            continue;
        }
        if envelope.kind == kind::RECEIPT {
            match envelope.payload.get("status").and_then(Value::as_str) {
                // Delivery confirmations belong to the frame's own send session; an
                // inbound one is stale noise, and acknowledging it would loop.
                Some("delivered") => continue,
                Some("read") => {
                    let ids = read_receipt_ids(&envelope)?;
                    let mut changed = false;
                    {
                        let state = app.state::<OverlayState>();
                        let mut store = state
                            .store
                            .lock()
                            .map_err(|_| "message store lock poisoned")?;
                        for id in ids {
                            // A peer may only raise receipts on my own messages in its
                            // direct chat; `get_message` also filters unknown ids.
                            let mine = store.get_message(id)?.is_some_and(|message| {
                                message.chat_id == remote.peer_id && message.sender == commands::ME
                            });
                            if mine
                                && store.record_receipt(
                                    id,
                                    &remote.peer_id,
                                    ReceiptKind::Read,
                                    now_ms(),
                                )?
                            {
                                changed = true;
                            }
                        }
                    }
                    let acked = envelope.id;
                    let stamp = now_ms();
                    session.send(&receipt_for(&identity, &remote.peer_id, acked, stamp))?;
                    if changed {
                        let _ = app.emit("messages-changed", remote.peer_id.clone());
                    }
                    continue;
                }
                _ => return Err("receipt has an unknown status".into()),
            }
        }
        if envelope.kind == kind::TYPING {
            let typing = envelope
                .payload
                .get("typing")
                .and_then(Value::as_bool)
                .ok_or("typing frame has no flag")?;
            apply_typing(&app, &remote.peer_id, typing);
            let acked = envelope.id;
            let stamp = now_ms();
            session.send(&receipt_for(&identity, &remote.peer_id, acked, stamp))?;
            continue;
        }
        let body = envelope
            .payload
            .get("text")
            .and_then(Value::as_str)
            .ok_or("incoming message has no text")?;
        if body.trim().is_empty() || body.len() > 16_384 {
            return Err("incoming message text is outside the supported size".into());
        }
        // A direct message names the receiver as its chat; a group post names the group.
        // A group is only trusted when this device knows it and the sender is on the roster
        // (Everyone is automatic, so any discovered peer belongs to it).
        let chat_id = commands::thread_chat_id(
            envelope
                .payload
                .get("chat_id")
                .and_then(Value::as_str)
                .unwrap_or_default(),
            &identity.peer_id(),
            &remote.peer_id,
        );
        if chat_id != remote.peer_id && chat_id != EVERYONE_GROUP_ID {
            let state = app.state::<OverlayState>();
            let guard = state
                .store
                .lock()
                .map_err(|_| "message store lock poisoned")?;
            let group = guard
                .get_group_state(&chat_id)?
                .ok_or("incoming message names a group this device does not know")?;
            if !group.is_member(&remote.peer_id) {
                return Err("incoming group message came from a non-member".into());
            }
        }
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if !commands::clock_is_plausible(envelope.hlc.physical_ms, now_ms) {
            return Err("incoming message timestamp is too far in the future".into());
        }
        let message = Message {
            id: envelope.id,
            chat_id,
            sender: remote.peer_id.clone(),
            body: body.to_owned(),
            clock: envelope.hlc,
            tombstone: false,
            forwarded: forwarded_of(&envelope),
        };
        let state = app.state::<OverlayState>();
        let mut store = state
            .store
            .lock()
            .map_err(|_| "message store lock poisoned")?;
        let is_new = store.save_message(&message)?;
        if is_new {
            // A peer may quote one of our messages; the pointer is kept only when it
            // resolves to a real message in this same chat, which `set_reply_to` checks.
            if let Some(quoted) = envelope
                .payload
                .get("reply_to")
                .and_then(Value::as_str)
                .and_then(|raw| Uuid::parse_str(raw).ok())
            {
                let _ = store.set_reply_to(message.id, Some(quoted));
            }
        }
        drop(store);

        if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
            if let Some(peer) = peers.iter_mut().find(|peer| peer.id == remote.peer_id) {
                peer.preview = body.to_owned();
                peer.last_message_ms = message.clock.physical_ms;
                if is_new && message.chat_id == remote.peer_id {
                    peer.unread = Some(peer.unread.unwrap_or(0).saturating_add(1));
                }
            }
        }
        publish(&app);
        if is_new {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64;
            let settings = app.state::<platform::SettingsState>().snapshot();
            let focused = app
                .get_webview_window("main")
                .and_then(|main| main.is_focused().ok())
                .unwrap_or(false);
            if !focused && platform::should_notify(&settings, &message.chat_id, now_ms) {
                let sender = app
                    .state::<OverlayState>()
                    .peers
                    .lock()
                    .ok()
                    .and_then(|peers| {
                        peers
                            .iter()
                            .find(|peer| peer.id == remote.peer_id)
                            .map(|peer| peer.name.clone())
                    })
                    .unwrap_or_else(|| "New message".into());
                platform::notify_message(&app, &sender, body);
            }
        }
        session.send(&receipt_for(&identity, &remote.peer_id, message.id, now_ms))?;
        let _ = app.emit("messages-changed", message.chat_id.clone());
    }
}

/// The receipt every accepted frame gets back, so the sender can mark its outbox row.
fn receipt_for(identity: &Identity, peer_id: &str, frame_id: Uuid, at_ms: u64) -> Envelope<Value> {
    Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: kind::RECEIPT.into(),
        from: identity.peer_id(),
        to: peer_id.to_owned(),
        hlc: Hlc {
            physical_ms: at_ms,
            logical: 0,
        },
        payload: serde_json::json!({"message_id": frame_id.to_string(), "status": "delivered"}),
    }
}

/// A batch of read receipts, acknowledged by the peer like any other frame.
fn read_receipt_envelope(
    from: &str,
    peer_id: &str,
    message_ids: &[Uuid],
    at_ms: u64,
) -> Envelope<Value> {
    Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: kind::RECEIPT.into(),
        from: from.to_owned(),
        to: peer_id.to_owned(),
        hlc: Hlc {
            physical_ms: at_ms,
            logical: 0,
        },
        payload: serde_json::json!({
            "status": "read",
            "message_ids": message_ids.iter().map(Uuid::to_string).collect::<Vec<_>>(),
        }),
    }
}

fn read_receipt_ids(envelope: &Envelope<Value>) -> Result<Vec<Uuid>, String> {
    let raw = envelope
        .payload
        .get("message_ids")
        .and_then(Value::as_array)
        .ok_or("read receipt has no message ids")?;
    if raw.is_empty() || raw.len() > MAX_READ_RECEIPT_IDS {
        return Err("read receipt ids are outside the supported size".into());
    }
    raw.iter()
        .map(|value| {
            value
                .as_str()
                .and_then(|id| Uuid::parse_str(id).ok())
                .ok_or_else(|| "read receipt has an invalid message id".into())
        })
        .collect()
}

/// The LAN passphrase, read from the secret store once per process.
pub(crate) fn lan_passphrase(app: &AppHandle) -> Option<Vec<u8>> {
    let state = app.state::<OverlayState>();
    let Ok(mut cache) = state.lan_passphrase.lock() else {
        return None;
    };
    if cache.is_none() {
        let secret = crate::secrets::Secrets::open(app)
            .ok()
            .and_then(|secrets| platform::stored_passphrase(&secrets));
        *cache = Some(secret);
    }
    cache.clone().flatten()
}

/// Drops the cached passphrase after the settings screen changes it.
pub(crate) fn forget_lan_passphrase(app: &AppHandle) {
    if let Ok(mut cache) = app.state::<OverlayState>().lan_passphrase.lock() {
        *cache = None;
    }
}

/// Challenge MAC over both identities and the sender's nonce.
fn auth_mac(passphrase: &[u8], from: &str, to: &str, nonce: &str) -> [u8; 32] {
    let key = blake3::hash(passphrase);
    let message = format!("alverchat-lan-v1\0{from}\0{to}\0{nonce}");
    *blake3::keyed_hash(key.as_bytes(), message.as_bytes()).as_bytes()
}

fn constant_time_eq(left: &[u8], right: &[u8]) -> bool {
    left.len() == right.len()
        && left
            .iter()
            .zip(right)
            .fold(0_u8, |acc, (a, b)| acc | (a ^ b))
            == 0
}

/// Proves both ends know the same LAN passphrase before any traffic flows.
///
/// A device with no passphrase accepts only a peer that also has none, so a half-set
/// pair fails loudly on both sides instead of silently downgrading to open traffic.
fn authenticate_session<S>(
    app: &AppHandle,
    session: &mut SecureStream<S>,
    remote_id: &str,
) -> Result<(), String>
where
    S: std::io::Read + std::io::Write,
{
    let my_id = app.state::<OverlayState>().identity.peer_id();
    let passphrase = lan_passphrase(app);
    let mut nonce_bytes = [0_u8; 32];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = BASE64.encode(nonce_bytes);
    let mac = passphrase
        .as_deref()
        .map(|secret| BASE64.encode(auth_mac(secret, &my_id, remote_id, &nonce)));
    let challenge = Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: kind::AUTH.into(),
        from: my_id.clone(),
        to: remote_id.to_owned(),
        hlc: Hlc {
            physical_ms: now_ms(),
            logical: 0,
        },
        payload: serde_json::json!({"nonce": nonce, "mac": mac}),
    };
    session.send(&challenge)?;
    let reply: Envelope<Value> = session.receive()?;
    if reply.v != PROTOCOL_VERSION
        || reply.kind != kind::AUTH
        || reply.from != remote_id
        || reply.to != my_id
    {
        return Err("peer did not answer the LAN passphrase challenge".into());
    }
    let peer_nonce = reply
        .payload
        .get("nonce")
        .and_then(Value::as_str)
        .ok_or("peer challenge has no nonce")?;
    let peer_mac = reply
        .payload
        .get("mac")
        .and_then(Value::as_str)
        .filter(|mac| !mac.is_empty());
    match (passphrase.as_deref(), peer_mac) {
        (Some(secret), Some(mac)) => {
            let expected = auth_mac(secret, remote_id, &my_id, peer_nonce);
            let received = BASE64
                .decode(mac)
                .map_err(|_| "peer challenge MAC is not base64")?;
            if !constant_time_eq(&expected, &received) {
                return Err("LAN passphrase does not match this peer".into());
            }
        }
        (Some(_), None) => {
            return Err("peer has no LAN passphrase but this device requires one".into())
        }
        (None, Some(_)) => {
            return Err("this device has no LAN passphrase but the peer requires one".into())
        }
        (None, None) => {}
    }
    Ok(())
}

/// Dials one beacon, racing every address its mDNS name resolved to.
///
/// macOS lists the peer's link-local IPv6 address first, and a connect to it stalls for
/// the whole timeout before IPv4 gets a turn. Racing the candidates lets the reachable
/// address win, so a send is bounded by the good address instead of the bad one.
fn connect_to_peer(beacon: &PeerBeacon) -> Result<TcpStream, String> {
    const TIMEOUT: Duration = Duration::from_secs(5);
    let addresses = (beacon.host.as_str(), beacon.port)
        .to_socket_addrs()
        .map_err(|error| format!("Cannot resolve peer address: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("Peer address did not resolve on the local network".into());
    }
    let (sender, receiver) = mpsc::channel();
    for address in &addresses {
        let address = *address;
        let sender = sender.clone();
        let _ = thread::Builder::new()
            .name("alverchat-connect".into())
            .spawn(move || {
                let _ = sender.send((address, TcpStream::connect_timeout(&address, TIMEOUT)));
            });
    }
    drop(sender);
    let deadline = Instant::now() + TIMEOUT;
    let mut last_error = None;
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            break;
        }
        match receiver.recv_timeout(remaining) {
            Ok((_, Ok(stream))) => {
                stream
                    .set_nodelay(true)
                    .map_err(|error| error.to_string())?;
                return Ok(stream);
            }
            Ok((address, Err(error))) => last_error = Some(format!("{address}: {error}")),
            Err(_) => break,
        }
    }
    Err(format!(
        "Cannot connect to peer on the local network: {}",
        last_error.unwrap_or_else(|| "no usable address".into())
    ))
}

fn deliver_envelope(
    app: &AppHandle,
    beacon: &PeerBeacon,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    send_frame(app, beacon, envelope)?;
    let state = app.state::<OverlayState>();
    let mut store = state
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?;
    // A group fan-out row is keyed by (message id, recipient); a direct row uses the
    // message id itself. Try the derived id first and fall back to the plain one.
    if !store.acknowledge_outbox(alver_core::groups::derive_outbox_id(
        envelope.id,
        &beacon.peer_id,
    ))? {
        store.acknowledge_outbox(envelope.id)?;
    }
    drop(store);
    let thread_id = commands::frame_thread(app, envelope);
    let _ = app.emit("messages-changed", thread_id);
    Ok(())
}

/// Handshakes with one beacon and hands the caller an authenticated session.
///
/// The beacon's published key is the pin: the handshake fails if the peer cannot prove it
/// owns that identity, so an address spoof on the LAN buys nothing. `send_frame` uses this
/// for its single frame; the file drivers keep the session and stream over it.
pub(crate) fn open_session(
    app: &AppHandle,
    beacon: &PeerBeacon,
) -> Result<SecureStream<websocket::BlockingWebSocket>, String> {
    let state = app.state::<OverlayState>();
    let expected = RemoteIdentity {
        peer_id: beacon.peer_id.clone(),
        public_key: beacon.public_key,
    };
    let stream = connect_to_peer(beacon)?;
    let stream = websocket::connect(stream, &expected.peer_id, expected.public_key)?;
    let mut session = initiator_handshake(stream, &state.identity, &expected)?;
    authenticate_session(app, &mut session, &expected.peer_id)?;
    state
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?
        .trust_peer(&expected.peer_id, &expected.public_key)?;
    Ok(session)
}

/// Handshakes with one beacon, sends a single frame and waits for its receipt.
///
/// The dialed session is kept warm between frames: a chat message, a typing flag and a
/// reaction would otherwise each pay a fresh resolve, TCP connect and TLS handshake.
pub(crate) fn send_frame(
    app: &AppHandle,
    beacon: &PeerBeacon,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    match send_once(app, beacon, envelope) {
        Ok(()) => Ok(()),
        Err(first) => {
            // A warm session can die with the peer or the network; redial exactly once.
            forget_session(app, &beacon.peer_id);
            send_once(app, beacon, envelope)
                .map_err(|retry| format!("{first}; retry failed: {retry}"))
        }
    }
}

fn send_once(
    app: &AppHandle,
    beacon: &PeerBeacon,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    let session = session_for(app, beacon)?;
    let mut guard = session.lock().map_err(|_| "peer session lock poisoned")?;
    guard.stream.send(envelope)?;
    let receipt: Envelope<Value> = guard.stream.receive()?;
    validate_receipt(app, beacon, envelope, &receipt)?;
    guard.used = Instant::now();
    Ok(())
}

fn validate_receipt(
    app: &AppHandle,
    beacon: &PeerBeacon,
    envelope: &Envelope<Value>,
    receipt: &Envelope<Value>,
) -> Result<(), String> {
    let my_id = app.state::<OverlayState>().identity.peer_id();
    let frame_id = envelope.id.to_string();
    if receipt.v != PROTOCOL_VERSION
        || receipt.kind != kind::RECEIPT
        || receipt.from != beacon.peer_id
        || receipt.to != my_id
        || receipt.payload.get("status").and_then(Value::as_str) != Some("delivered")
        || receipt.payload.get("message_id").and_then(Value::as_str) != Some(frame_id.as_str())
    {
        return Err("peer returned an invalid delivery receipt".into());
    }
    Ok(())
}

/// The warm session for `beacon`, dialing a new one when none is cached or the cached
/// one has been idle long enough that the responder may already have closed its side.
fn session_for(
    app: &AppHandle,
    beacon: &PeerBeacon,
) -> Result<Arc<Mutex<OutboundSession>>, String> {
    let state = app.state::<OverlayState>();
    let cached = state
        .outbound
        .lock()
        .map_err(|_| "peer session lock poisoned")?
        .get(&beacon.peer_id)
        .cloned();
    if let Some(session) = cached {
        let warm = session
            .lock()
            .map_err(|_| "peer session lock poisoned")?
            .used
            .elapsed()
            < OUTBOUND_IDLE;
        if warm {
            return Ok(session);
        }
        forget_session(app, &beacon.peer_id);
    }
    let stream = open_session(app, beacon)?;
    let session = Arc::new(Mutex::new(OutboundSession {
        stream,
        used: Instant::now(),
    }));
    let mut outbound = state
        .outbound
        .lock()
        .map_err(|_| "peer session lock poisoned")?;
    match outbound.entry(beacon.peer_id.clone()) {
        Entry::Occupied(existing) => Ok(existing.get().clone()),
        Entry::Vacant(slot) => {
            slot.insert(session.clone());
            Ok(session)
        }
    }
}

fn forget_session(app: &AppHandle, peer_id: &str) {
    if let Ok(mut outbound) = app.state::<OverlayState>().outbound.lock() {
        outbound.remove(peer_id);
    }
}

/// Folds one liveness result into the peer list, repainting only on a change so a
/// quiet LAN does not re-emit the overlay snapshot every heartbeat.
fn mark_peer_presence(app: &AppHandle, peer_id: &str, online: bool) {
    let state = app.state::<OverlayState>();
    let Ok(mut peers) = state.peers.lock() else {
        return;
    };
    let Some(peer) = peers.iter_mut().find(|peer| peer.id == peer_id) else {
        return;
    };
    if peer.online == online {
        return;
    }
    peer.online = online;
    peer.status = if online {
        "Available on LAN · end-to-end encrypted".into()
    } else {
        "Offline · messages will retry when the peer returns".into()
    };
    if !online {
        // A declared status belongs to a live peer; keeping it would draw an "away" dot
        // on a machine that is simply gone, and the next pong re-declares it anyway.
        peer.presence.clear();
        peer.status_line.clear();
    }
    drop(peers);
    if !online {
        // A peer that leaves the network never sends a hang-up, so this is the only
        // signal that its call ended — without it the icon would stay on "in call".
        if let Ok(mut calls) = state.live_calls.lock() {
            crate::calls::drop_calls_for_chat(&mut calls, peer_id);
        }
    }
    publish(app);
}

/// The heartbeat reply. Liveness always travels; what the peer is told about my profile
/// is the "Who sees my status" choice, so hiding a status cannot hide that I am here.
///
/// The avatar half of that profile is a hash, not the photo: a peer that is allowed to see
/// the status may ask for the picture behind it, and one that is not cannot learn that a
/// photo exists to ask for.
fn pong_payload(
    settings: &platform::AppSettings,
    ping_id: Uuid,
    avatar_hash: Option<&str>,
) -> Value {
    let mut payload = serde_json::Map::new();
    payload.insert("ping_id".into(), ping_id.to_string().into());
    if !platform::status_is_private(&settings.who_sees_status) {
        payload.insert(
            "presence".into(),
            platform::presence_token(&settings.presence).into(),
        );
        payload.insert("status".into(), settings.custom_status.trim().into());
        payload.insert("avatar".into(), avatar_hash.unwrap_or_default().into());
    }
    Value::Object(payload)
}

/// Folds the two declared fields of a heartbeat reply into one peer row and reports whether
/// anything moved. Split out of `record_peer_status` so a test can push what `pong_payload`
/// writes through the half that reads it: the pair is joined by key names, not by types.
fn apply_peer_status(peers: &mut [PeerDto], peer_id: &str, payload: &Value) -> bool {
    let read = |key: &str| {
        payload
            .get(key)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .trim()
            .to_string()
    };
    let (presence, status_line) = (read("presence"), read("status"));
    let Some(peer) = peers.iter_mut().find(|peer| peer.id == peer_id) else {
        return false;
    };
    if peer.presence == presence && peer.status_line == status_line {
        return false;
    }
    peer.presence = presence;
    peer.status_line = status_line;
    true
}

/// Folds the status a peer declared in its heartbeat reply into the peer list, so the
/// presence dot and the subtitle show what that peer chose to share rather than what a
/// string match over our own liveness copy happens to guess.
fn record_peer_status(app: &AppHandle, peer_id: &str, payload: &Value) {
    let state = app.state::<OverlayState>();
    let changed = {
        let Ok(mut peers) = state.peers.lock() else {
            return;
        };
        apply_peer_status(&mut peers, peer_id, payload)
    };
    if changed {
        publish(app);
    }
    // The same reply declares which photo that peer publishes; that is the only nudge a
    // peer gets to pull or drop it.
    commands::sync_peer_avatar(app, peer_id, payload);
}

fn heartbeat_envelope(from: &str, to: &str) -> Envelope<Value> {
    Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: kind::PING.into(),
        from: from.into(),
        to: to.into(),
        hlc: Hlc {
            physical_ms: now_ms(),
            logical: 0,
        },
        payload: serde_json::json!({}),
    }
}

fn is_matching_pong(ping: &Envelope<Value>, reply: &Envelope<Value>) -> bool {
    reply.v == PROTOCOL_VERSION
        && reply.kind == kind::PONG
        && reply.from == ping.to
        && reply.to == ping.from
        && reply.payload.get("ping_id").and_then(Value::as_str)
            == Some(ping.id.to_string().as_str())
}

/// Probes one warm session. The session is reused, so this costs one round trip and
/// no handshake; a success refreshes its idle clock too.
fn ping_peer(app: &AppHandle, beacon: &PeerBeacon) -> Result<(), String> {
    let state = app.state::<OverlayState>();
    let session = session_for(app, beacon)?;
    let mut guard = session.lock().map_err(|_| "peer session lock poisoned")?;
    let ping = heartbeat_envelope(&state.identity.peer_id(), &beacon.peer_id);
    guard.stream.send(&ping)?;
    let reply: Envelope<Value> = guard.stream.receive()?;
    if !is_matching_pong(&ping, &reply) {
        return Err("peer returned an invalid heartbeat reply".into());
    }
    guard.used = Instant::now();
    drop(guard);
    // The reply is also the only place a peer volunteers its own status, so presence is
    // folded in from the same round trip that proved the peer is alive.
    record_peer_status(app, &beacon.peer_id, &reply.payload);
    Ok(())
}

/// Keeps presence honest for peers this device is actively talking to. A peer with no
/// warm session is left to mDNS; dialing every discovered peer on a timer would turn
/// heartbeat into a handshake storm.
fn start_heartbeats(app: AppHandle) {
    let _ = thread::Builder::new()
        .name("alverchat-heartbeat".into())
        .spawn(move || loop {
            thread::sleep(HEARTBEAT_INTERVAL);
            let peers: Vec<String> = app
                .state::<OverlayState>()
                .outbound
                .lock()
                .map(|sessions| sessions.keys().cloned().collect())
                .unwrap_or_default();
            for peer_id in peers {
                let beacon = app
                    .state::<OverlayState>()
                    .endpoints
                    .lock()
                    .ok()
                    .and_then(|endpoints| endpoints.get(&peer_id).cloned());
                let Some(beacon) = beacon else {
                    continue;
                };
                if let Err(first) = ping_peer(&app, &beacon) {
                    // A dead socket and a dead peer look alike, so redial once before
                    // calling the peer offline: a transient drop keeps its presence.
                    forget_session(&app, &peer_id);
                    match ping_peer(&app, &beacon) {
                        Ok(()) => {}
                        Err(retry) => {
                            eprintln!("heartbeat to {peer_id} failed: {first}; retry: {retry}");
                            mark_peer_presence(&app, &peer_id, false);
                            forget_session(&app, &peer_id);
                        }
                    }
                }
            }
        });
}

/// The LAN passphrase changed, so no warm session may keep carrying traffic:
/// dropping the cache forces a fresh handshake, and the epoch bump ends the
/// inbound sessions that were already open.
pub(crate) fn reset_authenticated_sessions(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    state.auth_epoch.fetch_add(1, Ordering::AcqRel);
    if let Ok(mut outbound) = state.outbound.lock() {
        outbound.clear();
    };
}

/// The protocol's goodbye, written to every warm session before the process exits.
///
/// Deliberately not queued in the outbox: a retry loop would deliver a goodbye to a peer
/// that is still here and end a session that should have stayed up. This is a farewell on
/// a socket we are about to close, so a failure is not worth reporting to anyone.
///
/// The peer this reaches depends on what it was built from. A new build drops the
/// presence immediately; the 0.1.2 build has no handler and ends the session on the
/// unowned kind, which is the same outcome by a rougher route — and either way the
/// process is exiting, so there is nothing left to lose.
fn broadcast_goodbye(app: &AppHandle) {
    let state = app.state::<OverlayState>();
    let my_id = state.identity.peer_id();
    let sessions: Vec<(String, Arc<Mutex<OutboundSession>>)> = state
        .outbound
        .lock()
        .map(|sessions| {
            sessions
                .iter()
                .map(|(peer_id, session)| (peer_id.clone(), session.clone()))
                .collect()
        })
        .unwrap_or_default();
    for (peer_id, session) in sessions {
        let frame = Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: kind::BYE.into(),
            from: my_id.clone(),
            to: peer_id,
            hlc: Hlc {
                physical_ms: now_ms(),
                logical: 0,
            },
            payload: serde_json::json!({"reason": "closed"}),
        };
        if let Ok(mut guard) = session.lock() {
            let _ = guard.stream.send(&frame);
        }
    }
}

pub(crate) fn start_outbox_retry(app: AppHandle, peer_id: String) {
    let state = app.state::<OverlayState>();
    let Ok(mut retrying) = state.retrying_peers.lock() else {
        return;
    };
    if !retrying.insert(peer_id.clone()) {
        return;
    }
    drop(retrying);
    let cleanup_app = app.clone();
    let cleanup_peer_id = peer_id.clone();
    let started = thread::Builder::new()
        .name("alverchat-outbox-retry".into())
        .spawn(move || {
            let mut delay = 1_u64;
            loop {
                let pending = app
                    .state::<OverlayState>()
                    .store
                    .lock()
                    .ok()
                    .and_then(|store| store.pending_outbox(&peer_id).ok())
                    .unwrap_or_default();
                if pending.is_empty() {
                    break;
                }
                let Some(beacon) = app
                    .state::<OverlayState>()
                    .endpoints
                    .lock()
                    .ok()
                    .and_then(|endpoints| endpoints.get(&peer_id).cloned())
                else {
                    // Discovery can lag behind a send (the peer was cached, or its
                    // service is being re-resolved), so wait rather than giving up.
                    thread::sleep(Duration::from_secs(delay));
                    delay = (delay * 2).min(60);
                    continue;
                };
                let mut failed = false;
                for envelope in pending {
                    if let Err(error) = deliver_envelope(&app, &beacon, &envelope) {
                        // A retry loop that fails in silence leaves a bubble spinning with
                        // nothing on disk to explain it, so every failed round says why.
                        eprintln!(
                            "Outbox delivery of {} to {} failed: {error}",
                            envelope.kind, peer_id
                        );
                        failed = true;
                        break;
                    }
                }
                if failed {
                    thread::sleep(Duration::from_secs(delay));
                    delay = (delay * 2).min(60);
                } else {
                    break;
                }
            }
            if let Ok(mut retrying) = app.state::<OverlayState>().retrying_peers.lock() {
                retrying.remove(&peer_id);
            }
        });
    if started.is_err() {
        if let Ok(mut retrying) = cleanup_app.state::<OverlayState>().retrying_peers.lock() {
            retrying.remove(&cleanup_peer_id);
        }
    }
}

/// Discovery handle kept in managed state so a settings change (display name)
/// can re-sign and re-announce the beacon without restarting the app.
struct DiscoveryState {
    /// `None` when the host refused mDNS: the UDP fallback is then the only path, so
    /// losing the multicast daemon must not take discovery down with it.
    mdns: Option<MdnsDiscovery>,
    identity: Arc<Identity>,
    host: String,
    port: u16,
    /// Whether the discovery port is open, for Settings → Connection diagnostics.
    udp_open: AtomicBool,
    /// The mDNS fullnames this browse currently lists, by peer id. While one of them is
    /// present, mDNS is the authority for whether that peer has left the network.
    mdns_services: Mutex<HashMap<String, String>>,
}

/// How often the fallback repeats this machine's beacon, and how long it waits for an answer
/// before calling a peer it alone vouched for offline. The expiry is a count of missed
/// announcements rather than a second figure, because that is what the two documents count.
const UDP_ANNOUNCE_INTERVAL_MS: u64 = 3_000;
const UDP_PEER_MISSED_ANNOUNCEMENTS: u64 = 5;
const UDP_ANNOUNCE_INTERVAL: Duration = Duration::from_millis(UDP_ANNOUNCE_INTERVAL_MS);
const UDP_PEER_TTL_MS: u64 = UDP_ANNOUNCE_INTERVAL_MS * UDP_PEER_MISSED_ANNOUNCEMENTS;

/// One read per pass, so a burst of beacons drains without pushing the next tick out.
const UDP_READ_SLICE: Duration = Duration::from_millis(200);

/// The name advertised to peers: the configured display name when set,
/// otherwise a stable fallback derived from the peer id.
fn advertised_display_name(app: &AppHandle, identity: &Identity) -> String {
    let custom = app
        .try_state::<platform::SettingsState>()
        .map(|state| state.snapshot().display_name)
        .unwrap_or_default();
    let trimmed = custom.trim();
    if trimmed.is_empty() {
        format!("AlverChat {}", &identity.peer_id()[..8])
    } else {
        // The beacon signature caps names at 128 bytes; keep a safe margin.
        trimmed.chars().take(120).collect()
    }
}

pub(crate) fn republish_lan_beacon(app: &AppHandle) {
    let Some(discovery) = app.try_state::<DiscoveryState>() else {
        return;
    };
    let name = advertised_display_name(app, &discovery.identity);
    let beacon = PeerBeacon::signed(
        &discovery.identity,
        name,
        discovery.host.clone(),
        discovery.port,
    );
    let settings = app.state::<platform::SettingsState>().snapshot();
    // Settings → "Visible on network": the beacon is how a peer finds this machine at all,
    // so hiding means taking it down rather than renaming it.
    if !settings.visible_on_network {
        if let Some(mdns) = &discovery.mdns {
            if let Err(error) = mdns.unpublish(&beacon) {
                eprintln!("Could not hide the LAN announcement: {error}");
            }
        }
        return;
    }
    let Some(mdns) = &discovery.mdns else {
        return;
    };
    let advertised: Vec<std::net::IpAddr> =
        platform::adapter_choice(&settings.network_adapter, &platform::local_ipv4s())
            .into_iter()
            .collect();
    if let Err(error) = mdns.publish(&beacon, &advertised) {
        eprintln!("Could not republish LAN announcement: {error}");
    }
}

/// What the fallback should do with a beacon it just heard.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UdpSighting {
    /// This machine's own announcement, returned by the broadcast address.
    Mine,
    /// A peer nobody has asked for its queued frames since it was last seen.
    New,
    /// A peer already known and already retried.
    Seen,
}

/// The one place that decides whether a broadcast sighting is news.
///
/// A datagram sent to `255.255.255.255` comes back to the sender, so the first job is
/// refusing this machine's own beacon: it verifies, and acting on it would list yourself as
/// a peer. Freshness is the second job — a beacon repeats every few seconds, and the retry
/// loop it triggers must not be handed the same peer each time.
pub(crate) fn classify_udp_sighting(
    peer_id: &str,
    my_id: &str,
    last_seen_ms: Option<u64>,
    now_ms: u64,
    ttl_ms: u64,
) -> UdpSighting {
    if peer_id == my_id {
        return UdpSighting::Mine;
    }
    let fresh = last_seen_ms.is_none_or(|seen| now_ms.saturating_sub(seen) >= ttl_ms);
    if fresh {
        UdpSighting::New
    } else {
        UdpSighting::Seen
    }
}

/// Whether the fallback is the one allowed to call this peer offline.
///
/// Silence proves nothing while somebody else is watching: mDNS removal owns a peer the
/// browse still lists, and heartbeat owns a peer this device holds a warm session to. The
/// fallback only acts on peers it is the sole path to.
pub(crate) fn udp_may_expire_peer(
    last_seen_ms: u64,
    now_ms: u64,
    ttl_ms: u64,
    listed_elsewhere: bool,
) -> bool {
    !listed_elsewhere && now_ms.saturating_sub(last_seen_ms) >= ttl_ms
}

/// Fold one broadcast sighting into the peer list, retrying queued frames only on a
/// fresh one.
fn apply_udp_beacon(app: &AppHandle, beacon: &PeerBeacon, fresh: bool) {
    upsert_discovered_peer(app, beacon.clone());
    if fresh {
        start_outbox_retry(app.clone(), beacon.peer_id.clone());
    }
}

/// Repeat this machine's beacon on every usable subnet, each copy signed with the address
/// a peer should dial.
///
/// The host inside a fallback beacon is a literal address rather than the `.local.` name
/// mDNS hands out, because the machine that needs this path is the one that cannot resolve
/// that name. The Settings adapter choice narrows the list the same way it narrows the
/// announcement, and "Visible on network" is honoured here too: hiding has to be silent on
/// every path, not just the documented one.
fn announce_over_udp(app: &AppHandle, udp: &UdpDiscovery) {
    let Some(discovery) = app.try_state::<DiscoveryState>() else {
        return;
    };
    let settings = app.state::<platform::SettingsState>().snapshot();
    if !settings.visible_on_network {
        return;
    }
    let name = advertised_display_name(app, &discovery.identity);
    for (source, destination) in platform::discovery_targets(
        &platform::local_ipv4_beacons(),
        &settings.network_adapter,
        discovery.port,
    ) {
        let beacon = PeerBeacon::signed(
            &discovery.identity,
            name.clone(),
            source.to_string(),
            discovery.port,
        );
        if let Err(error) = udp.announce(&beacon, destination) {
            eprintln!("Could not broadcast the LAN announcement: {error}");
        }
    }
}

/// Drops presence for peers the fallback stopped hearing.
///
/// The endpoint goes with them, exactly as an mDNS removal takes it: a queued frame to a
/// machine nobody can route to would otherwise be dialled until it timed out.
fn sweep_udp_peers(app: &AppHandle, seen: &mut HashMap<String, u64>) {
    let now_ms = now_ms();
    let listed = app
        .try_state::<DiscoveryState>()
        .map(|discovery| {
            discovery
                .mdns_services
                .lock()
                .map(|services| services.values().cloned().collect::<HashSet<String>>())
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let warm = app
        .state::<OverlayState>()
        .outbound
        .lock()
        .map(|sessions| sessions.keys().cloned().collect::<HashSet<String>>())
        .unwrap_or_default();
    let stale: Vec<String> = seen
        .iter()
        .filter(|(peer_id, last_seen)| {
            udp_may_expire_peer(
                **last_seen,
                now_ms,
                UDP_PEER_TTL_MS,
                listed.contains(*peer_id) || warm.contains(*peer_id),
            )
        })
        .map(|(peer_id, _)| peer_id.clone())
        .collect();
    for peer_id in stale {
        seen.remove(&peer_id);
        if let Ok(mut endpoints) = app.state::<OverlayState>().endpoints.lock() {
            endpoints.remove(&peer_id);
        }
        mark_peer_presence(app, &peer_id, false);
    }
}

/// The signed-broadcast half of discovery: a beacon repeated on the LAN and every answer
/// heard, with no session and no multicast in sight.
///
/// This is the path that keeps a machine findable when mDNS is blocked — guest Wi-Fi with
/// client isolation, a firewall that drops 5353 — which is why a failure here is logged
/// once and the app carries on with whatever discovery it does have.
fn start_udp_fallback(app: AppHandle) {
    let _ = thread::Builder::new()
        .name("alverchat-udp-discovery".into())
        .spawn(move || {
            let udp = UdpDiscovery::bind(
                (
                    std::net::Ipv4Addr::UNSPECIFIED,
                    alver_core::discovery::DISCOVERY_PORT,
                )
                    .into(),
            )
            .map_err(|error| {
                eprintln!("UDP discovery fallback is unavailable: {error}");
                error
            })
            .ok();
            if let Some(state) = app.try_state::<DiscoveryState>() {
                state.udp_open.store(udp.is_some(), Ordering::Release);
            }
            let Some(udp) = udp else { return };
            let mut seen = HashMap::<String, u64>::new();
            let mut next_announce = Instant::now() - UDP_ANNOUNCE_INTERVAL;
            let mut next_sweep = Instant::now();
            loop {
                if next_announce.elapsed() >= UDP_ANNOUNCE_INTERVAL {
                    announce_over_udp(&app, &udp);
                    next_announce = Instant::now();
                }
                match udp.receive_once(UDP_READ_SLICE) {
                    Ok(Some(beacon)) => {
                        let my_id = app.state::<OverlayState>().identity.peer_id();
                        let now_ms = now_ms();
                        let last_seen_ms = seen.get(&beacon.peer_id).copied();
                        let fresh = match classify_udp_sighting(
                            &beacon.peer_id,
                            &my_id,
                            last_seen_ms,
                            now_ms,
                            UDP_PEER_TTL_MS,
                        ) {
                            UdpSighting::Mine => None,
                            UdpSighting::New => Some(true),
                            UdpSighting::Seen => Some(false),
                        };
                        if let Some(fresh) = fresh {
                            seen.insert(beacon.peer_id.clone(), now_ms);
                            apply_udp_beacon(&app, &beacon, fresh);
                        }
                    }
                    Ok(None) => {}
                    Err(error) => eprintln!("Discovery broadcast could not be read: {error}"),
                }
                if next_sweep.elapsed() >= UDP_ANNOUNCE_INTERVAL {
                    sweep_udp_peers(&app, &mut seen);
                    next_sweep = Instant::now();
                }
            }
        });
}

fn start_mdns_browser(mdns: MdnsDiscovery, app: &AppHandle, self_id: String) -> Result<(), String> {
    let events = mdns.browse()?;
    let app = app.clone();
    std::thread::spawn(move || {
        let _daemon = mdns;
        loop {
            match events.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(ServiceEvent::ServiceResolved(service)) => {
                    let Some(beacon) = PeerBeacon::from_mdns(&service) else {
                        continue;
                    };
                    if beacon.peer_id == self_id {
                        continue;
                    }
                    if let Ok(mut services) = app.state::<DiscoveryState>().mdns_services.lock() {
                        services.insert(service.get_fullname().to_owned(), beacon.peer_id.clone());
                    }
                    upsert_discovered_peer(&app, beacon.clone());
                    start_outbox_retry(app.clone(), beacon.peer_id);
                }
                Ok(ServiceEvent::ServiceRemoved(fullname, _)) => {
                    let gone = app
                        .state::<DiscoveryState>()
                        .mdns_services
                        .lock()
                        .ok()
                        .and_then(|mut services| services.remove(&fullname));
                    if let Some(peer_id) = gone {
                        if let Ok(mut endpoints) = app.state::<OverlayState>().endpoints.lock() {
                            endpoints.remove(&peer_id);
                        }
                        mark_peer_presence(&app, &peer_id, false);
                    }
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => {
                    expire_typing(&app);
                    refresh_overlay_quiet(&app);
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(())
}

fn start_peer_discovery(
    app: &mut tauri::App,
    identity: Arc<Identity>,
    port: u16,
) -> Result<(), String> {
    let host = format!("alverchat-{}.local.", &identity.peer_id()[..12]);
    let display_name = advertised_display_name(app.handle(), &identity);
    let beacon = PeerBeacon::signed(&identity, display_name, host.clone(), port);
    let self_id = beacon.peer_id.clone();
    let mdns = match MdnsDiscovery::new() {
        Ok(mdns) => Some(mdns),
        Err(error) => {
            eprintln!("mDNS discovery is unavailable: {error}");
            None
        }
    };
    app.manage(DiscoveryState {
        mdns: mdns.clone(),
        identity: identity.clone(),
        host,
        port,
        udp_open: AtomicBool::new(false),
        mdns_services: Mutex::new(HashMap::new()),
    });
    // Startup publishes through the same path a settings change takes, so a machine that
    // was told not to be visible does not announce itself on the way in.
    republish_lan_beacon(app.handle());
    // The two paths are independent: a host that blocks multicast still has the broadcast,
    // and a host that cannot open the discovery port still has mDNS.
    if let Some(mdns) = &mdns {
        if let Err(error) = start_mdns_browser(mdns.clone(), app.handle(), self_id) {
            eprintln!("Could not browse the LAN: {error}");
        }
    }
    start_udp_fallback(app.handle().clone());
    Ok(())
}

#[tauri::command]
fn list_peers(app: AppHandle) -> Vec<PeerDto> {
    let mut peers = app
        .state::<OverlayState>()
        .peers
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default();
    // Pin and mute live in the settings file, so the list is only correct once
    // they are folded back onto the discovered peers.
    if let Ok(settings) = app.state::<platform::SettingsState>().0.lock() {
        for peer in &mut peers {
            peer.muted = settings.muted_chats.iter().any(|id| id == &peer.id);
            peer.pinned = settings.pinned_chats.iter().any(|id| id == &peer.id);
        }
        peers.sort_by_key(|peer| std::cmp::Reverse(peer.pinned));
    }
    peers
}
#[tauri::command]
fn list_messages(app: AppHandle, chat_id: String) -> Result<Vec<MessageDto>, String> {
    commands::message_dtos(&app, &chat_id)
}
#[tauri::command]
async fn send_message(
    app: AppHandle,
    chat_id: String,
    text: String,
    reply_to_id: Option<String>,
) -> Result<MessageDto, String> {
    tauri::async_runtime::spawn_blocking(move || {
        commands::send_message(app, chat_id, text, reply_to_id)
    })
    .await
    .map_err(|error| error.to_string())?
}
#[tauri::command]
fn mark_as_read(app: AppHandle, chat_id: String) {
    // Read receipts are what make an unread badge durable across restarts, so
    // clearing the in-memory counter alone would let it come back.
    // `ME` names the local reader in storage; the envelope route needs the signed peer id.
    let my_id = app.state::<OverlayState>().identity.peer_id();
    let receipted = {
        let state = app.state::<OverlayState>();
        let Ok(mut store) = state.store.lock() else {
            return;
        };
        let direct = matches!(store.get_group_state(&chat_id), Ok(None));
        let unread = if direct {
            store
                .unread_ids_for_chat(&chat_id, commands::ME)
                .unwrap_or_default()
        } else {
            // A group has one read state per member, so there is nobody to receipt.
            Vec::new()
        };
        if store
            .mark_chat_read(&chat_id, commands::ME, now_ms())
            .is_err()
        {
            return;
        }
        // Batching keeps every frame well below the control cap; the peer acks each
        // batch, so the outbox retries whatever was not delivered.
        let mut queued = false;
        for batch in unread.chunks(MAX_READ_RECEIPT_IDS) {
            let envelope = read_receipt_envelope(&my_id, &chat_id, batch, now_ms());
            if store.enqueue(&chat_id, &envelope).is_err() {
                break;
            }
            queued = true;
        }
        queued
    };
    if receipted {
        // A background drain keeps the command instant; an offline peer is left to
        // the discovery-triggered retry loop, exactly like a queued message.
        let beacon = app
            .state::<OverlayState>()
            .endpoints
            .lock()
            .ok()
            .and_then(|endpoints| endpoints.get(&chat_id).cloned());
        if let Some(beacon) = beacon {
            let delivery_app = app.clone();
            let delivery_chat = chat_id.clone();
            let _ = thread::Builder::new()
                .name("alverchat-receipt".into())
                .spawn(move || commands::flush_outbox(&delivery_app, &delivery_chat, &beacon));
        }
    }
    if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == chat_id) {
            peer.unread = None;
        }
    }
    // Deliberately no `messages-changed`: the shell answers that event with another
    // `markAsRead`, so emitting here would spin the open thread in a reload loop.
    publish(&app);
}

#[tauri::command]
fn delete_message_local(app: AppHandle, message_id: String) -> Result<bool, String> {
    let state = app.state::<OverlayState>();
    let id = Uuid::parse_str(&message_id).map_err(|e| e.to_string())?;
    let mut store = state
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?;
    store.tombstone_message(id, "local")
}

/// The minutes the Settings screen stores, as the rule the store applies.
///
/// The composer hides Edit on the same arithmetic (`withinEditWindow` in the thread view);
/// this is the authority behind it, so a stale webview, a scripted invoke or a clock that
/// moved while the composer was open cannot write a revision the setting forbids. `0` closes
/// the window entirely, which is what the board's "no editing" state means.
fn edit_window(minutes: u32) -> alver_core::rules::EditWindow {
    alver_core::rules::EditWindow::new(u64::from(minutes).saturating_mul(60_000))
}

/// A gated edit, translated back into what this command has always promised its caller:
/// `false` for a message this device may not rewrite — the composer reads that as "only your
/// own messages" — and the expired window as the sentence the UI uses, not the debug form of
/// the rule. Anything else is a real fault and travels unchanged.
fn edit_outcome(result: Result<bool, String>, minutes: u32) -> Result<bool, String> {
    result.or_else(|error| {
        if error.starts_with("EditWindowExpired") {
            Err(format!("The edit window of {minutes} minutes has passed."))
        } else if error.starts_with("NotSender") || error.starts_with("Tombstoned") {
            Ok(false)
        } else {
            Err(error)
        }
    })
}

/// The write itself, over a store the caller already holds.
///
/// Split out of the command because the command needs an `AppHandle` to reach, and a gate only
/// the running app can exercise is a gate no test can prove is applied: this is the function a
/// unit test puts a real database behind.
fn apply_local_edit(
    store: &mut Store,
    id: Uuid,
    text: &str,
    clock: Hlc,
    minutes: u32,
) -> Result<bool, String> {
    let now_ms = clock.physical_ms;
    let gated = store.edit_message_within(id, "local", text, clock, now_ms, edit_window(minutes));
    edit_outcome(gated, minutes)
}

#[tauri::command]
fn edit_message_local(app: AppHandle, message_id: String, text: String) -> Result<bool, String> {
    let state = app.state::<OverlayState>();
    let id = Uuid::parse_str(&message_id).map_err(|e| e.to_string())?;
    let text = text.trim();
    if text.is_empty() || text.len() > 16_384 {
        return Err("Message must contain 1-16384 bytes".into());
    }
    let minutes = app
        .state::<platform::SettingsState>()
        .snapshot()
        .edit_window_minutes;
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let clock = Hlc {
        physical_ms: now_ms,
        logical: 0,
    };
    let edited = {
        let mut store = state
            .store
            .lock()
            .map_err(|_| "message store lock poisoned")?;
        apply_local_edit(&mut store, id, text, clock, minutes)?
    };
    if edited {
        // Every other device in the thread keeps its own revision history, so an edit that
        // landed here has to travel; "delete for me" deliberately does not.
        commands::propagate_edit(&app, id, text, clock);
        // The rail quotes the thread's latest body, and this is the only place that revision
        // is written on our side — the peer's copy is refreshed by the frame it hears, ours by
        // this.
        publish(&app);
    }
    Ok(edited)
}

#[tauri::command]
fn get_my_identity(app: AppHandle) -> IdentityDto {
    identity_summary(&app)
}

#[tauri::command]
fn overlay_snapshot(app: AppHandle) -> OverlaySnapshot {
    snapshot(&app)
}
#[tauri::command]
fn toggle_compact(app: AppHandle) {
    let state = app.state::<OverlayState>();
    let mut open = state.compact_open.lock().expect("overlay state poisoned");
    *open = !*open;
    let compact = app
        .get_webview_window("compact")
        .expect("compact window exists");
    if *open {
        if let Ok(mut detached) = state.compact_detached.lock() {
            *detached = false;
        }
        position_compact(&app, &compact);
        let _ = compact.show();
    } else {
        let _ = compact.hide();
    }
    drop(open);
    publish(&app);
}
#[tauri::command]
fn hide_compact(app: AppHandle) {
    if let Some(compact) = app.get_webview_window("compact") {
        let _ = compact.hide();
    }
    if let Ok(mut open) = app.state::<OverlayState>().compact_open.lock() {
        *open = false;
    }
    publish(&app);
}
#[tauri::command]
fn open_full_app(app: AppHandle, chat_id: Option<String>) {
    if let Some(compact) = app.get_webview_window("compact") {
        let _ = compact.hide();
    }
    if let Some(menu) = app.get_webview_window("menu") {
        let _ = menu.hide();
    }
    if let Ok(mut open) = app.state::<OverlayState>().compact_open.lock() {
        *open = false;
    }
    if let Some(main) = app.get_webview_window("main") {
        let _ = main.show();
        let _ = main.unminimize();
        let _ = main.set_focus();
        let _ = main.emit("navigate-chat", chat_id);
    }
    publish(&app);
}
#[tauri::command]
fn set_overlay_hit_regions(window: WebviewWindow, has_hit: bool) -> Result<(), String> {
    set_window_capture(window.app_handle(), &window, has_hit)
}
#[tauri::command]
fn set_overlay_pointer_capture(
    window: WebviewWindow,
    active: bool,
    has_hit: bool,
) -> Result<(), String> {
    let app = window.app_handle();
    if let Ok(mut pointers) = app.state::<OverlayState>().pointer_capture.lock() {
        pointers.insert(window.label().to_string(), active);
    }
    set_window_capture(app, &window, active || has_hit)
}
#[tauri::command]
fn start_overlay_drag(window: WebviewWindow) -> Result<(), String> {
    let app = window.app_handle();
    set_window_capture(app, &window, true)?;
    if let Ok(mut active) = app.state::<OverlayState>().active_drag.lock() {
        *active = Some(window.label().to_string());
    }
    if window.label() == "compact" {
        window.set_focusable(false).map_err(|e| e.to_string())?;
        if let Ok(mut detached) = app.state::<OverlayState>().compact_detached.lock() {
            *detached = true;
        }
    }
    Ok(())
}
#[tauri::command]
fn move_overlay_by(window: WebviewWindow, delta_x: f64, delta_y: f64) -> Result<(), String> {
    let app = window.app_handle();
    let scale = window.scale_factor().map_err(|e| e.to_string())?;
    let position = window.outer_position().map_err(|e| e.to_string())?;
    let dx = (delta_x * scale).round() as i32;
    let dy = (delta_y * scale).round() as i32;
    window
        .set_position(tauri::PhysicalPosition::new(
            position.x + dx,
            position.y + dy,
        ))
        .map_err(|e| e.to_string())?;
    if window.label() == "dock" {
        clamp_dock_to_visible_monitor(app, &window)?;
        update_bubble_side(app, &window)?;
    } else if window.label() == "compact" {
        if let Ok(mut detached) = app.state::<OverlayState>().compact_detached.lock() {
            *detached = true;
        }
    }
    Ok(())
}
#[tauri::command]
fn end_overlay_drag(
    window: WebviewWindow,
    has_hit: bool,
    reduce_motion: bool,
) -> Result<(), String> {
    let app = window.app_handle();
    if let Ok(mut active) = app.state::<OverlayState>().active_drag.lock() {
        *active = None;
    }
    if let Ok(mut pointers) = app.state::<OverlayState>().pointer_capture.lock() {
        pointers.insert(window.label().to_string(), false);
    }
    if window.label() == "dock" {
        save_and_snap_dock(app, &window, reduce_motion)?;
    } else if window.label() == "compact" {
        clamp_panel(&window)?;
        window.set_focusable(true).map_err(|e| e.to_string())?;
    }
    set_window_capture(app, &window, has_hit)?;
    publish(app);
    Ok(())
}
#[tauri::command]
fn reset_overlay_position(app: AppHandle) -> Result<(), String> {
    let dock = app
        .get_webview_window("dock")
        .ok_or("dock window missing")?;
    if let Some(monitor) = app.primary_monitor().map_err(|e| e.to_string())? {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let size = dock.outer_size().map_err(|e| e.to_string())?;
        let x = area.position.x + area.size.width as i32
            - size.width as i32
            - (12.0 * scale).round() as i32;
        let y = area.position.y + area.size.height as i32 / 2 - size.height as i32 / 2;
        dock.set_position(tauri::PhysicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
        let position = SavedDockPosition {
            monitor_id: monitor.name().cloned().unwrap_or_else(|| "primary".into()),
            edge: Edge::Right,
            vertical_percent: 0.5,
        };
        if let Ok(mut prefs) = app.state::<OverlayState>().preferences.lock() {
            prefs.dock_position = Some(position);
            let _ = persist_preferences(&app, &prefs);
        }
    }
    publish(&app);
    Ok(())
}
#[tauri::command]
fn set_snap_to_edge(app: AppHandle, enabled: bool) -> Result<(), String> {
    update_app_settings(&app, |settings| settings.snap_to_edge = enabled)?;
    publish(&app);
    Ok(())
}

#[tauri::command]
fn set_bubble_settings(app: AppHandle, enabled: bool, auto_hide_seconds: u8) -> Result<(), String> {
    if !(1..=60).contains(&auto_hide_seconds) {
        return Err("Bubble auto-hide delay must be between 1 and 60 seconds.".into());
    }
    update_app_settings(&app, |settings| {
        settings.bubbles_enabled = enabled;
        settings.bubble_auto_hide_seconds = auto_hide_seconds;
    })?;
    publish(&app);
    Ok(())
}

/// Mutate the shared app settings, persist them, and broadcast the change.
fn update_app_settings(
    app: &AppHandle,
    mutate: impl FnOnce(&mut platform::AppSettings),
) -> Result<platform::AppSettings, String> {
    let state = app
        .try_state::<platform::SettingsState>()
        .ok_or("settings state is unavailable")?;
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    mutate(&mut guard);
    platform::persist(app, &guard)?;
    Ok(guard.clone())
}

fn persist_preferences(app: &AppHandle, prefs: &OverlayPreferences) -> Result<(), String> {
    let state = app
        .try_state::<OverlayState>()
        .ok_or("overlay state is unavailable")?;
    let bytes = serde_json::to_vec_pretty(prefs).map_err(|error| error.to_string())?;
    fs::write(&state.preferences_path, bytes)
        .map_err(|error| format!("could not save preferences: {error}"))
}

fn set_window_capture(
    app: &AppHandle,
    window: &WebviewWindow,
    capture: bool,
) -> Result<(), String> {
    let state = app.state::<OverlayState>();
    let mut last = state
        .cursor_capture
        .lock()
        .map_err(|_| "cursor capture lock poisoned")?;
    if last.get(window.label()) == Some(&capture) {
        return Ok(());
    }
    window
        .set_ignore_cursor_events(!capture)
        .map_err(|e| e.to_string())?;
    last.insert(window.label().to_string(), capture);
    Ok(())
}

fn point_in_rect(cursor: (f64, f64), x: i32, y: i32, width: i32, height: i32) -> bool {
    cursor.0 >= x as f64
        && cursor.0 < (x + width) as f64
        && cursor.1 >= y as f64
        && cursor.1 < (y + height) as f64
}

fn overlay_contains_cursor(app: &AppHandle, window: &WebviewWindow, cursor: (f64, f64)) -> bool {
    let Ok(position) = window.outer_position() else {
        return false;
    };
    let Ok(size) = window.outer_size() else {
        return false;
    };
    let Ok(scale) = window.scale_factor() else {
        return false;
    };
    match window.label() {
        "dock" => {
            let side = app
                .state::<OverlayState>()
                .preferences
                .lock()
                .ok()
                .and_then(|p| p.dock_position.as_ref().map(|p| p.edge))
                .unwrap_or(Edge::Right);
            let Ok((offset_x, offset_y, icon_size)) = icon_offset(window, side, scale) else {
                return false;
            };
            let left = position.x + offset_x;
            let top = position.y + offset_y;
            let cx = left as f64 + icon_size as f64 / 2.0;
            let cy = top as f64 + icon_size as f64 / 2.0;
            let radius = icon_size as f64 / 2.0;
            let in_icon = (cursor.0 - cx).powi(2) + (cursor.1 - cy).powi(2) <= radius * radius;
            if in_icon {
                return true;
            }
            let unread = app
                .state::<OverlayState>()
                .peers
                .lock()
                .map(|peers| {
                    peers
                        .iter()
                        .filter(|peer| peer.unread.unwrap_or(0) > 0)
                        .count()
                        .min(3) as i32
                })
                .unwrap_or(0);
            if unread == 0 {
                return false;
            }
            let bubble_w = (305.0 * scale).round() as i32;
            let row_h = (65.0 * scale).round() as i32;
            let bubble_x = match side {
                Edge::Right => position.x + (12.0 * scale).round() as i32,
                Edge::Left => position.x + (80.0 * scale).round() as i32,
            };
            let bubble_y =
                position.y + size.height as i32 - (12.0 * scale).round() as i32 - unread * row_h;
            point_in_rect(cursor, bubble_x, bubble_y, bubble_w, unread * row_h)
        }
        "compact" => point_in_rect(
            cursor,
            position.x + (12.0 * scale).round() as i32,
            position.y + (12.0 * scale).round() as i32,
            size.width as i32 - (24.0 * scale).round() as i32,
            size.height as i32 - (24.0 * scale).round() as i32,
        ),
        "menu" => point_in_rect(
            cursor,
            position.x + (8.0 * scale).round() as i32,
            position.y + (8.0 * scale).round() as i32,
            size.width as i32 - (16.0 * scale).round() as i32,
            size.height as i32 - (16.0 * scale).round() as i32,
        ),
        _ => false,
    }
}

fn start_cursor_hit_test(app: AppHandle) {
    let mut last_monitor_check = std::time::Instant::now();
    std::thread::spawn(move || loop {
        let cursor = app
            .cursor_position()
            .ok()
            .map(|position| (position.x, position.y));
        if let Some(cursor) = cursor {
            for label in ["dock", "compact", "menu"] {
                if let Some(window) = app.get_webview_window(label) {
                    let visible = window.is_visible().unwrap_or(false);
                    if !visible {
                        continue;
                    }
                    let dragging = app
                        .state::<OverlayState>()
                        .active_drag
                        .lock()
                        .map(|active| active.as_deref() == Some(label))
                        .unwrap_or(false);
                    let pointer_active = app
                        .state::<OverlayState>()
                        .pointer_capture
                        .lock()
                        .map(|pointers| pointers.get(label).copied().unwrap_or(false))
                        .unwrap_or(false);
                    let capture = dragging
                        || pointer_active
                        || overlay_contains_cursor(&app, &window, cursor);
                    let _ = set_window_capture(&app, &window, capture);
                }
            }
        }
        if last_monitor_check.elapsed() >= std::time::Duration::from_secs(1) {
            if let Some(dock) = app.get_webview_window("dock") {
                let dragging = app
                    .state::<OverlayState>()
                    .active_drag
                    .lock()
                    .map(|active| active.as_deref() == Some("dock"))
                    .unwrap_or(false);
                if !dragging {
                    let _ = clamp_dock_to_visible_monitor(&app, &dock);
                    let _ = update_bubble_side(&app, &dock);
                }
            }
            last_monitor_check = std::time::Instant::now();
        }
        std::thread::sleep(std::time::Duration::from_millis(12));
    });
}

fn rect(monitor: &tauri::Monitor) -> Rect {
    let work = monitor.work_area();
    Rect {
        x: work.position.x,
        y: work.position.y,
        width: work.size.width as i32,
        height: work.size.height as i32,
    }
}

fn icon_offset(window: &WebviewWindow, edge: Edge, scale: f64) -> Result<(i32, i32, i32), String> {
    let size = window.outer_size().map_err(|e| e.to_string())?;
    let offset_x = match edge {
        Edge::Left => (12.0 * scale).round() as i32,
        Edge::Right => size.width as i32 - (68.0 * scale).round() as i32,
    };
    Ok((
        offset_x,
        (12.0 * scale).round() as i32,
        (56.0 * scale).round() as i32,
    ))
}

fn save_and_snap_dock(
    app: &AppHandle,
    dock: &WebviewWindow,
    reduce_motion: bool,
) -> Result<(), String> {
    let monitor = dock
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("no monitor available")?;
    let work = rect(&monitor);
    let scale = monitor.scale_factor();
    let pos = dock.outer_position().map_err(|e| e.to_string())?;
    let prefs_snapshot = app
        .state::<OverlayState>()
        .preferences
        .lock()
        .map_err(|_| "preferences lock poisoned")?
        .clone();
    let old_edge = prefs_snapshot
        .dock_position
        .as_ref()
        .map(|p| p.edge)
        .unwrap_or(Edge::Right);
    let (old_offset_x, offset_y, icon_size) = icon_offset(dock, old_edge, scale)?;
    let clamped = drag::clamp_icon_into_work_area(
        (pos.x, pos.y),
        work,
        dock.outer_size().map_err(|e| e.to_string())?.into(),
        (old_offset_x, offset_y),
        icon_size,
    );
    let (x, y, edge) = drag::snap_icon_to_edge(
        clamped,
        work,
        (old_offset_x, offset_y),
        icon_size,
        app.state::<platform::SettingsState>()
            .snapshot()
            .snap_to_edge,
        (24.0 * scale).round() as i32,
    );
    let (new_offset_x, _, _) = icon_offset(dock, edge, scale)?;
    let from = dock.outer_position().map_err(|e| e.to_string())?;
    let to = (x + old_offset_x - new_offset_x, y);
    if from.x != to.0 || from.y != to.1 {
        let moving = dock.clone();
        std::thread::spawn(move || {
            if reduce_motion {
                let _ = moving.set_position(tauri::PhysicalPosition::new(to.0, to.1));
                return;
            }
            let steps = 12;
            for step in 1..=steps {
                let t = step as f64 / steps as f64;
                let eased = 1.0 - (1.0 - t) * (1.0 - t);
                let px = from.x + ((to.0 - from.x) as f64 * eased).round() as i32;
                let py = from.y + ((to.1 - from.y) as f64 * eased).round() as i32;
                let _ = moving.set_position(tauri::PhysicalPosition::new(px, py));
                std::thread::sleep(std::time::Duration::from_millis(16));
            }
        });
    }
    let icon_top = y + offset_y;
    let saved = SavedDockPosition {
        monitor_id: monitor.name().cloned().unwrap_or_else(|| "primary".into()),
        edge,
        vertical_percent: drag::vertical_percent(icon_top, work, icon_size),
    };
    if let Ok(mut prefs) = app.state::<OverlayState>().preferences.lock() {
        prefs.dock_position = Some(saved);
        let _ = persist_preferences(app, &prefs);
    }
    if let Some(compact) = app.get_webview_window("compact") {
        if app
            .state::<OverlayState>()
            .compact_open
            .lock()
            .map(|open| *open)
            .unwrap_or(false)
        {
            position_compact(app, &compact);
        }
    }
    Ok(())
}

fn update_bubble_side(app: &AppHandle, dock: &WebviewWindow) -> Result<(), String> {
    let monitor = dock
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("no monitor available")?;
    let scale = monitor.scale_factor();
    let work = rect(&monitor);
    let pos = dock.outer_position().map_err(|e| e.to_string())?;
    let prefs_snapshot = app
        .state::<OverlayState>()
        .preferences
        .lock()
        .map_err(|_| "preferences lock poisoned")?
        .clone();
    let old_edge = prefs_snapshot
        .dock_position
        .as_ref()
        .map(|p| p.edge)
        .unwrap_or(Edge::Right);
    let (old_offset, offset_y, icon_size) = icon_offset(dock, old_edge, scale)?;
    let icon_left = pos.x + old_offset;
    let new_edge = drag::edge_for_icon(icon_left, icon_size, work);
    if new_edge != old_edge {
        let (new_offset, _, _) = icon_offset(dock, new_edge, scale)?;
        dock.set_position(tauri::PhysicalPosition::new(
            pos.x + old_offset - new_offset,
            pos.y,
        ))
        .map_err(|e| e.to_string())?;
        let saved = SavedDockPosition {
            monitor_id: monitor.name().cloned().unwrap_or_else(|| "primary".into()),
            edge: new_edge,
            vertical_percent: drag::vertical_percent(pos.y + offset_y, work, icon_size),
        };
        if let Ok(mut prefs) = app.state::<OverlayState>().preferences.lock() {
            prefs.dock_position = Some(saved);
            let _ = persist_preferences(app, &prefs);
        }
        publish(app);
    }
    Ok(())
}

fn restore_dock_position(app: &AppHandle, dock: &WebviewWindow) -> Result<(), String> {
    let prefs = app
        .state::<OverlayState>()
        .preferences
        .lock()
        .map_err(|_| "preferences lock poisoned")?
        .clone();
    let Some(saved) = prefs.dock_position else {
        return Ok(());
    };
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    let monitor = monitors
        .iter()
        .find(|m| m.name().map(|name| name.as_str()) == Some(saved.monitor_id.as_str()))
        .cloned()
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("no monitor available")?;
    let area = rect(&monitor);
    let scale = monitor.scale_factor();
    let (offset_x, offset_y, icon_size) = icon_offset(dock, saved.edge, scale)?;
    let icon_top = drag::restore_icon_top(saved.vertical_percent, area, icon_size);
    let x = match saved.edge {
        Edge::Left => area.x - offset_x,
        Edge::Right => area.x + area.width - icon_size - offset_x,
    };
    dock.set_position(tauri::PhysicalPosition::new(x, icon_top - offset_y))
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Moves the dock to the corner the Settings "Screen corner / side" picker chose and
/// stores it as the saved position, so a relaunch keeps the same placement and the
/// picker matches where the icon actually is.
pub(crate) fn apply_screen_corner(app: &AppHandle, corner: &str) -> Result<(), String> {
    let Some((edge, vertical_percent)) = drag::corner_position(corner) else {
        return Ok(());
    };
    let dock = app
        .get_webview_window("dock")
        .ok_or("dock window missing")?;
    let monitor = dock
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
        .ok_or("no monitor available")?;
    let area = rect(&monitor);
    let scale = dock.scale_factor().map_err(|e| e.to_string())?;
    let (offset_x, offset_y, icon_size) = icon_offset(&dock, edge, scale)?;
    let icon_top = drag::restore_icon_top(vertical_percent, area, icon_size);
    let x = match edge {
        Edge::Left => area.x - offset_x,
        Edge::Right => area.x + area.width - icon_size - offset_x,
    };
    dock.set_position(tauri::PhysicalPosition::new(x, icon_top - offset_y))
        .map_err(|e| e.to_string())?;
    if let Ok(mut prefs) = app.state::<OverlayState>().preferences.lock() {
        prefs.dock_position = Some(SavedDockPosition {
            monitor_id: monitor.name().cloned().unwrap_or_else(|| "primary".into()),
            edge,
            vertical_percent,
        });
        let _ = persist_preferences(app, &prefs);
    }
    publish(app);
    Ok(())
}

fn clamp_panel(panel: &WebviewWindow) -> Result<(), String> {
    let monitor = panel
        .current_monitor()
        .map_err(|e| e.to_string())?
        .ok_or("no monitor available")?;
    let area = rect(&monitor);
    let pos = panel.outer_position().map_err(|e| e.to_string())?;
    let size = panel.outer_size().map_err(|e| e.to_string())?;
    let x = pos.x.clamp(area.x, area.x + area.width - size.width as i32);
    let y = pos
        .y
        .clamp(area.y, area.y + area.height - size.height as i32);
    panel
        .set_position(tauri::PhysicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

fn clamp_dock_to_visible_monitor(app: &AppHandle, dock: &WebviewWindow) -> Result<(), String> {
    let monitors = app.available_monitors().map_err(|e| e.to_string())?;
    if monitors.is_empty() {
        return Ok(());
    }
    let position = dock.outer_position().map_err(|e| e.to_string())?;
    let prefs = app
        .state::<OverlayState>()
        .preferences
        .lock()
        .map_err(|_| "preferences lock poisoned")?
        .clone();
    let edge = prefs
        .dock_position
        .as_ref()
        .map(|p| p.edge)
        .unwrap_or(Edge::Right);
    let scale = dock.scale_factor().map_err(|e| e.to_string())?;
    let (offset_x, offset_y, icon_size) = icon_offset(dock, edge, scale)?;
    let center = (
        position.x + offset_x + icon_size / 2,
        position.y + offset_y + icon_size / 2,
    );
    let monitor = monitors
        .iter()
        .find(|m| {
            let a = rect(m);
            center.0 >= a.x
                && center.0 < a.x + a.width
                && center.1 >= a.y
                && center.1 < a.y + a.height
        })
        .or_else(|| {
            monitors.iter().min_by_key(|m| {
                let a = rect(m);
                let dx = a.x + a.width / 2 - center.0;
                let dy = a.y + a.height / 2 - center.1;
                dx * dx + dy * dy
            })
        })
        .ok_or("no monitor available")?;
    let area = rect(monitor);
    let size = dock.outer_size().map_err(|e| e.to_string())?;
    let clamped = drag::clamp_icon_into_work_area(
        (position.x, position.y),
        area,
        (size.width as i32, size.height as i32),
        (offset_x, offset_y),
        icon_size,
    );
    if clamped != (position.x, position.y) {
        dock.set_position(tauri::PhysicalPosition::new(clamped.0, clamped.1))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
fn resize_dock(app: AppHandle, has_bubbles: bool) -> Result<(), String> {
    let Some(dock) = app.get_webview_window("dock") else {
        return Ok(());
    };
    let old_pos = dock.outer_position().map_err(|e| e.to_string())?;
    let old_size = dock.outer_size().map_err(|e| e.to_string())?;
    let scale = dock.scale_factor().map_err(|e| e.to_string())?;
    let side = app
        .state::<OverlayState>()
        .preferences
        .lock()
        .ok()
        .and_then(|p| p.dock_position.as_ref().map(|p| p.edge))
        .unwrap_or(Edge::Right);
    let old_offset = match side {
        Edge::Left => (12.0 * scale).round() as i32,
        Edge::Right => old_size.width as i32 - (68.0 * scale).round() as i32,
    };
    let icon_left = old_pos.x + old_offset;
    let size = if has_bubbles {
        tauri::LogicalSize::new(397.0, 220.0)
    } else {
        tauri::LogicalSize::new(84.0, 84.0)
    };
    dock.set_size(size).map_err(|e| e.to_string())?;
    let new_size = dock.outer_size().map_err(|e| e.to_string())?;
    let new_offset = match side {
        Edge::Left => (12.0 * scale).round() as i32,
        Edge::Right => new_size.width as i32 - (68.0 * scale).round() as i32,
    };
    dock.set_position(tauri::PhysicalPosition::new(
        icon_left - new_offset,
        old_pos.y,
    ))
    .map_err(|e| e.to_string())?;
    Ok(())
}
#[tauri::command]
fn open_dock_menu(app: AppHandle) {
    if let (Some(dock), Some(menu)) = (
        app.get_webview_window("dock"),
        app.get_webview_window("menu"),
    ) {
        if let (Ok(position), Ok(size)) = (dock.outer_position(), dock.outer_size()) {
            let _ = menu.set_position(tauri::PhysicalPosition::new(
                position.x + size.width as i32 - 270,
                position.y + size.height as i32 / 2,
            ));
        }
        let _ = menu.show();
    }
}
#[tauri::command]
fn hide_menu(app: AppHandle) {
    if let Some(menu) = app.get_webview_window("menu") {
        let _ = menu.hide();
    }
}
#[tauri::command]
fn quit_app(app: AppHandle) {
    broadcast_goodbye(&app);
    app.exit(0);
}

fn position_compact(app: &AppHandle, compact: &WebviewWindow) {
    if app
        .state::<OverlayState>()
        .compact_detached
        .lock()
        .map(|v| *v)
        .unwrap_or(false)
    {
        return;
    }
    let Some(dock) = app.get_webview_window("dock") else {
        return;
    };
    let (Ok(dock_pos), Ok(monitor)) = (dock.outer_position(), dock.current_monitor()) else {
        return;
    };
    let Some(monitor) = monitor else {
        return;
    };
    let area = monitor.work_area();
    let panel_w = (380.0 * monitor.scale_factor()) as i32;
    let panel_h = (640.0 * monitor.scale_factor()) as i32;
    let gap = (12.0 * monitor.scale_factor()) as i32;
    let dock_w = (56.0 * monitor.scale_factor()) as i32;
    let left = area.position.x;
    let right = left + area.size.width as i32;
    let scale = monitor.scale_factor();
    let edge = app
        .state::<OverlayState>()
        .preferences
        .lock()
        .ok()
        .and_then(|p| p.dock_position.as_ref().map(|p| p.edge))
        .unwrap_or(Edge::Right);
    let (icon_offset, _, _) =
        icon_offset(&dock, edge, scale).unwrap_or(((12.0 * scale) as i32, 0, dock_w));
    let icon_left = dock_pos.x + icon_offset;
    let left_space = icon_left - left;
    let right_space = right - (icon_left + dock_w);
    let x = if right_space >= left_space {
        icon_left + dock_w + gap
    } else {
        icon_left - panel_w - gap
    };
    let y = dock_pos.y + dock_w / 2 - panel_h / 2;
    let x = x.clamp(left, right - panel_w);
    let y = y.clamp(
        area.position.y,
        area.position.y + area.size.height as i32 - panel_h,
    );
    let _ = compact.set_position(tauri::PhysicalPosition::new(x, y));
}

fn build_overlay(
    app: &tauri::App,
    label: &'static str,
    route: &'static str,
    width: f64,
    height: f64,
    visible: bool,
) -> tauri::Result<WebviewWindow> {
    let mut builder = WebviewWindowBuilder::new(
        app,
        label,
        WebviewUrl::App(format!("index.html#/{route}").into()),
    )
    .title("AlverChat")
    .inner_size(width, height)
    .resizable(false)
    .decorations(false)
    .transparent(true)
    .shadow(false)
    .always_on_top(true)
    .skip_taskbar(true)
    .visible_on_all_workspaces(true)
    .focused(false)
    .focusable(label != "dock")
    .visible(visible);
    if label == "dock" {
        if let Some(monitor) = app.primary_monitor()? {
            let area = monitor.work_area();
            let scale = monitor.scale_factor();
            builder = builder.position(
                area.position.x as f64 / scale + area.size.width as f64 / scale - 84.0 - 12.0,
                area.position.y as f64 / scale + area.size.height as f64 / scale / 2.0 - 42.0,
            );
        }
    }
    let window = builder.build()?;
    set_native_overlay_level(&window);
    Ok(window)
}

/// Set the Dock tile from the embedded icon. Dev runs the bare binary without
/// an .app bundle (no CFBundleIconFile), so the bundle icon never applies there.
#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn set_dock_icon() {
    use objc::{class, msg_send, runtime::Object, sel, sel_impl};
    const ICON_PNG: &[u8] = include_bytes!("../icons/icon.png");
    unsafe {
        let data: *mut Object =
            msg_send![class!(NSData), dataWithBytes: ICON_PNG.as_ptr() length: ICON_PNG.len()];
        let image: *mut Object = msg_send![class!(NSImage), alloc];
        let image: *mut Object = msg_send![image, initWithData: data];
        let app: *mut Object = msg_send![class!(NSApplication), sharedApplication];
        let _: () = msg_send![app, setApplicationIconImage: image];
    }
}

#[cfg(target_os = "macos")]
#[allow(unexpected_cfgs)]
fn set_native_overlay_level(window: &WebviewWindow) {
    use objc::{msg_send, runtime::Object, sel, sel_impl};
    unsafe {
        let ns_window = window.ns_window().expect("overlay NSWindow") as *mut Object;
        let style: u64 = msg_send![ns_window, styleMask];
        let _: () = msg_send![ns_window, setStyleMask: style | (1 << 7)];
        let _: () = msg_send![ns_window, setLevel: 25isize];
        let _: () =
            msg_send![ns_window, setCollectionBehavior: (1usize | (1usize << 4) | (1usize << 8))];
    }
}
#[cfg(target_os = "windows")]
fn set_native_overlay_level(window: &WebviewWindow) {
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        GetWindowLongPtrW, SetWindowLongPtrW, SetWindowPos, GWL_EXSTYLE, HWND_TOPMOST,
        SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    };
    unsafe {
        let hwnd = window.hwnd().expect("overlay HWND").0 as windows_sys::Win32::Foundation::HWND;
        let style = GetWindowLongPtrW(hwnd, GWL_EXSTYLE);
        let mut extended_style = style | WS_EX_TOOLWINDOW as isize;
        if window.label() == "dock" {
            extended_style |= WS_EX_NOACTIVATE as isize;
        }
        SetWindowLongPtrW(hwnd, GWL_EXSTYLE, extended_style);
        SetWindowPos(
            hwnd,
            HWND_TOPMOST,
            0,
            0,
            0,
            0,
            SWP_NOMOVE
                | SWP_NOSIZE
                | if window.label() == "dock" {
                    SWP_NOACTIVATE
                } else {
                    0
                },
        );
    }
}
#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn set_native_overlay_level(_window: &WebviewWindow) {}

/// The encrypted message store inside the instance data directory.
pub(crate) const MESSAGE_DB_FILE: &str = "messages.db";

fn database_key(app: &AppHandle) -> Result<[u8; 32], String> {
    let secrets = secrets::Secrets::open(app)?;
    let secret = match secrets.get(secrets::DATABASE_KEY)? {
        Some(secret) => secret,
        None => {
            let mut secret = [0u8; 32];
            OsRng.fill_bytes(&mut secret);
            secrets.set(secrets::DATABASE_KEY, &secret)?;
            secret.to_vec()
        }
    };
    secret
        .try_into()
        .map_err(|_| "The saved database key has an invalid length.".into())
}

fn identity_key(app: &AppHandle) -> Result<Identity, String> {
    let secrets = secrets::Secrets::open(app)?;
    let secret = match secrets.get(secrets::IDENTITY_KEY)? {
        Some(secret) => secret,
        None => {
            let identity = Identity::generate();
            let secret = identity.secret_bytes();
            secrets.set(secrets::IDENTITY_KEY, &secret)?;
            secret.to_vec()
        }
    };
    let secret: [u8; 32] = secret
        .try_into()
        .map_err(|_| "The saved peer identity has an invalid length.".to_string())?;
    Ok(Identity::from_secret(secret))
}

fn open_message_store(app: &AppHandle) -> Result<Store, String> {
    let directory = instance_data_dir(app)?;
    fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let path = directory.join(MESSAGE_DB_FILE);
    let path = path
        .to_str()
        .ok_or("The app data path is not valid UTF-8")?;
    Store::open(path, &database_key(app)?)
}

/// Label of the running instance. `pnpm dev:peers` gives each copy its own
/// label and points its data and config directories somewhere private, so
/// several installs can share one machine without merging their identity,
/// stored secrets or databases. The label namespaces keychain entries; file
/// backed secrets are separated by the data directory.
fn instance_label() -> Option<String> {
    std::env::var("ALVERCHAT_MOCK_PEER")
        .ok()
        .filter(|value| !value.is_empty())
}

fn env_directory(variable: &str) -> Option<PathBuf> {
    std::env::var(variable)
        .ok()
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn instance_data_dir(app: &AppHandle) -> Result<PathBuf, String> {
    env_directory("ALVERCHAT_DATA_DIR").map_or_else(
        || app.path().app_data_dir().map_err(|error| error.to_string()),
        Ok,
    )
}

pub fn instance_config_dir(app: &AppHandle) -> Result<PathBuf, String> {
    env_directory("ALVERCHAT_CONFIG_DIR").map_or_else(
        || {
            app.path()
                .app_config_dir()
                .map_err(|error| error.to_string())
        },
        Ok,
    )
}

/// Keychain service for this instance; dev peers get an isolated namespace.
pub fn keychain_service() -> String {
    match instance_label() {
        Some(label) => format!("{} dev peer {}", alver_core::APP_NAME, label),
        None => alver_core::APP_NAME.to_string(),
    }
}

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_notification::init())
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_autostart::init(
            tauri_plugin_autostart::MacosLauncher::LaunchAgent,
            None,
        ))
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .setup(|app| {
            #[cfg(target_os = "macos")]
            set_dock_icon();
            let mut settings = platform::load_settings(app.handle());
            // `lanEncryption` is a report the rail's badge reads, not a preference: the only
            // thing that decides whether a session is passphrase-challenged is the stored
            // secret. A settings.json written by the build that had a toggle for this can say
            // anything, so the fact is recomputed from the secret instead of trusted.
            settings.lan_encryption = platform::has_lan_passphrase(app.handle().clone());
            app.manage(platform::SettingsState(Arc::new(Mutex::new(
                settings.clone(),
            ))));
            if settings.launch_at_login {
                let _ = platform::apply_autostart(app.handle(), true);
            }
            if let Err(error) = platform::register_hotkey(app.handle(), &settings.hotkey) {
                eprintln!("Global shortcut is unavailable: {error}");
            }
            let preferences_path = instance_config_dir(app.handle())?.join("overlay-settings.json");
            if let Some(parent) = preferences_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let preferences = fs::read(&preferences_path)
                .ok()
                .and_then(|data| serde_json::from_slice(&data).ok())
                .unwrap_or(OverlayPreferences {
                    dock_position: None,
                });
            let mut store = open_message_store(app.handle()).map_err(std::io::Error::other)?;
            let initial_peers = known_peers(&store).map_err(std::io::Error::other)?;
            // A transfer driver does not survive a crash, so a row still claiming to be
            // moving at startup is a row nothing is moving. Retire the claim before the
            // first snapshot is published, or every window opens promising progress.
            match store.pause_stalled_transfers() {
                Ok(0) => {}
                Ok(count) => eprintln!("Paused {count} transfer(s) the last run left mid-file"),
                Err(error) => eprintln!("Could not review unfinished transfers: {error}"),
            }
            let identity = Arc::new(identity_key(app.handle()).map_err(std::io::Error::other)?);
            app.manage(OverlayState {
                compact_open: Mutex::new(false),
                peers: Mutex::new(initial_peers),
                endpoints: Mutex::new(HashMap::new()),
                outbound: Mutex::new(HashMap::new()),
                retrying_peers: Mutex::new(HashSet::new()),
                typing_until: Mutex::new(HashMap::new()),
                transfers: Mutex::new(HashMap::new()),
                pulling: Mutex::new(HashSet::new()),
                announcements: Mutex::new(Vec::new()),
                live_calls: Mutex::new(HashMap::new()),
                quiet: Mutex::new(false),
                identity: identity.clone(),
                peer_sessions: Arc::new(AtomicUsize::new(0)),
                lan_passphrase: Mutex::new(None),
                auth_epoch: AtomicU64::new(0),
                store: Mutex::new(store),
                preferences: Mutex::new(preferences),
                preferences_path,
                active_drag: Mutex::new(None),
                pointer_capture: Mutex::new(HashMap::new()),
                compact_detached: Mutex::new(false),
                cursor_capture: Mutex::new(HashMap::new()),
            });
            refresh_announcement_bubbles(app.handle());
            refresh_overlay_quiet(app.handle());
            let transport = start_secure_listener(app.handle().clone(), identity.clone())
                .and_then(|port| start_peer_discovery(app, identity, port));
            match transport {
                Ok(()) => start_heartbeats(app.handle().clone()),
                Err(error) => eprintln!("LAN discovery is unavailable: {error}"),
            }
            let tray_menu = rebuild_tray_menu(app.handle())?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            TrayIconBuilder::with_id(TRAY_ID)
                .icon(icon)
                .menu(&tray_menu)
                .tooltip(alver_core::APP_NAME)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(main) = app.get_webview_window("main") {
                            let _ = main.show();
                            let _ = main.set_focus();
                        }
                    }
                    "mute" => {
                        let next = !app
                            .state::<platform::SettingsState>()
                            .snapshot()
                            .notifications_muted;
                        if let Ok(settings) =
                            update_app_settings(app, |prefs| prefs.notifications_muted = next)
                        {
                            let _ = app.emit("settings-changed", settings);
                            refresh_tray(app);
                        }
                    }
                    "dnd" => {
                        let next = !app
                            .state::<platform::SettingsState>()
                            .snapshot()
                            .dnd_enabled;
                        if let Ok(settings) =
                            update_app_settings(app, |prefs| prefs.dnd_enabled = next)
                        {
                            let _ = app.emit("settings-changed", settings);
                            refresh_tray(app);
                        }
                    }
                    "reset-position" => {
                        let _ = reset_overlay_position(app.clone());
                    }
                    "quit" => {
                        broadcast_goodbye(app);
                        app.exit(0)
                    }
                    _ => {}
                })
                .build(app)?;
            let dock = build_overlay(app, "dock", "dock", 84.0, 84.0, true)?;
            restore_dock_position(app.handle(), &dock)?;
            build_overlay(app, "compact", "compact", 404.0, 664.0, false)?;
            build_overlay(app, "menu", "menu", 264.0, 176.0, false)?;
            start_cursor_hit_test(app.handle().clone());
            if let Some(compact) = app.get_webview_window("compact") {
                let app_handle = app.handle().clone();
                compact.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Focused(false)) {
                        let dragging = app_handle
                            .state::<OverlayState>()
                            .active_drag
                            .lock()
                            .map(|active| active.as_deref() == Some("compact"))
                            .unwrap_or(false);
                        if !dragging {
                            hide_compact(app_handle.clone());
                        }
                    }
                });
            }
            if let Some(dock) = app.get_webview_window("dock") {
                let app_handle = app.handle().clone();
                dock.on_window_event(move |event| {
                    if matches!(event, tauri::WindowEvent::Moved(_))
                        && app_handle
                            .state::<OverlayState>()
                            .compact_open
                            .lock()
                            .map(|open| *open)
                            .unwrap_or(false)
                    {
                        if let Some(compact) = app_handle.get_webview_window("compact") {
                            position_compact(&app_handle, &compact);
                        }
                    }
                });
            }
            if let Some(main) = app.get_webview_window("main") {
                let main_for_event = main.clone();
                main.on_window_event(move |event| {
                    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                        api.prevent_close();
                        let _ = main_for_event.hide();
                    }
                });
            }
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_peers,
            list_messages,
            send_message,
            overlay_snapshot,
            toggle_compact,
            hide_compact,
            open_full_app,
            set_overlay_hit_regions,
            set_overlay_pointer_capture,
            start_overlay_drag,
            move_overlay_by,
            end_overlay_drag,
            reset_overlay_position,
            set_snap_to_edge,
            set_bubble_settings,
            mark_as_read,
            delete_message_local,
            edit_message_local,
            get_my_identity,
            resize_dock,
            open_dock_menu,
            hide_menu,
            quit_app,
            platform::get_settings,
            platform::update_settings,
            platform::set_hotkey,
            platform::set_launch_at_login,
            platform::is_dnd_active,
            platform::set_lan_passphrase,
            platform::has_lan_passphrase,
            platform::permission_status,
            platform::open_permission_settings,
            platform::open_firewall_guide,
            platform::run_connection_test,
            platform::create_backup,
            platform::list_backups,
            platform::restore_backup,
            platform::export_data,
            platform::reveal_path,
            platform::set_muted_chat,
            platform::set_pinned_chat,
            commands::pin_message,
            commands::pinned_messages,
            commands::star_message,
            commands::starred_messages,
            commands::reactions_for,
            commands::react,
            commands::forward_messages,
            commands::search_messages,
            commands::set_typing,
            commands::mark_unread,
            commands::clear_chat,
            commands::delete_chat,
            commands::delete_for_everyone,
            commands::attachments_for,
            commands::send_file,
            commands::control_transfer,
            commands::open_attachment,
            commands::attachment_data,
            commands::list_groups,
            commands::create_group,
            commands::update_group,
            commands::add_group_members,
            commands::remove_group_member,
            commands::leave_group,
            commands::list_announcements,
            commands::send_announcement,
            commands::acknowledge_announcement,
            commands::list_calls,
            calls::send_call_signal,
            commands::set_avatar,
            commands::get_avatar,
            commands::peer_avatar
        ])
        .build(tauri::generate_context!())
        .expect("failed to build AlverChat")
        .run(|app_handle, event| {
            // Cmd+Q and the Dock's Quit leave through here rather than through either `quit`
            // handler above, so without this a peer kept showing us online until the heartbeat
            // gave up. A duplicate on the paths that already said goodbye is not visible to
            // anyone: the session it would travel on is the one being closed.
            if let tauri::RunEvent::ExitRequested { .. } = event {
                broadcast_goodbye(app_handle);
            }
        });
}

#[cfg(test)]
mod heartbeat_tests {
    use super::*;

    /// A digest in the shape the wire carries: sixty-four hex characters.
    const AVATAR_HASH: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn pong_for(ping: &Envelope<Value>) -> Envelope<Value> {
        Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: "pong".into(),
            from: ping.to.clone(),
            to: ping.from.clone(),
            hlc: Hlc::default(),
            payload: pong_payload(
                &platform::AppSettings::default(),
                ping.id,
                Some(AVATAR_HASH),
            ),
        }
    }

    /// A row as the announcement path builds it for a peer that is here, before any of
    /// that peer's own profile has been folded in.
    fn row(id: &str) -> PeerDto {
        PeerDto {
            id: id.into(),
            name: "Sarath Shailan".into(),
            initials: "SS".into(),
            color: "violet".into(),
            online: true,
            status: "Available on LAN · end-to-end encrypted".into(),
            presence: String::new(),
            status_line: String::new(),
            preview: String::new(),
            time: String::new(),
            last_message_ms: 0,
            unread: None,
            group: false,
            typing: false,
            muted: false,
            pinned: false,
        }
    }

    fn shared_profile() -> platform::AppSettings {
        platform::AppSettings {
            presence: "In a meeting".into(),
            custom_status: "  Reviewing designs  ".into(),
            who_sees_status: "everyone".into(),
            ..Default::default()
        }
    }

    #[test]
    fn heartbeat_envelope_round_trips_through_the_reply_check() {
        let ping = heartbeat_envelope("peer-a", "peer-b");
        assert_eq!(ping.kind, "ping");
        assert_eq!(ping.from, "peer-a");
        assert_eq!(ping.to, "peer-b");
        assert!(is_matching_pong(&ping, &pong_for(&ping)));
    }

    #[test]
    fn a_pong_must_reverse_the_route_and_echo_the_ping_id() {
        let ping = heartbeat_envelope("peer-a", "peer-b");
        let mut wrong_route = pong_for(&ping);
        wrong_route.from = "peer-c".into();
        assert!(!is_matching_pong(&ping, &wrong_route));

        let mut wrong_id = pong_for(&ping);
        wrong_id.payload = serde_json::json!({ "ping_id": Uuid::now_v7().to_string() });
        assert!(!is_matching_pong(&ping, &wrong_id));

        let mut wrong_kind = pong_for(&ping);
        wrong_kind.kind = "receipt".into();
        assert!(!is_matching_pong(&ping, &wrong_kind));
    }

    #[test]
    fn a_pong_carries_the_profile_peers_are_allowed_to_see() {
        let settings = platform::AppSettings {
            presence: "In a meeting".into(),
            custom_status: "  Reviewing designs  ".into(),
            who_sees_status: "everyone".into(),
            ..Default::default()
        };
        let payload = pong_payload(&settings, Uuid::now_v7(), Some(AVATAR_HASH));
        assert_eq!(
            payload.get("presence").and_then(Value::as_str),
            Some("busy")
        );
        assert_eq!(
            payload.get("status").and_then(Value::as_str),
            Some("Reviewing designs")
        );
    }

    #[test]
    fn hiding_my_status_never_hides_that_i_am_here() {
        let settings = platform::AppSettings {
            presence: "Away".into(),
            custom_status: "Out for lunch".into(),
            who_sees_status: "nobody".into(),
            ..Default::default()
        };
        let ping_id = Uuid::now_v7();
        let payload = pong_payload(&settings, ping_id, Some(AVATAR_HASH));
        assert_eq!(
            payload.get("ping_id").and_then(Value::as_str),
            Some(ping_id.to_string().as_str())
        );
        assert!(payload.get("presence").is_none());
        assert!(payload.get("status").is_none());
        // A private profile says nothing about a photo either: there is no hash to ask about.
        assert!(payload.get("avatar").is_none());
    }

    #[test]
    fn a_pong_lands_the_profile_the_peer_chose_to_share() {
        let mut peers = vec![row("peer-a"), row("peer-b")];
        let payload = pong_payload(&shared_profile(), Uuid::now_v7(), Some(AVATAR_HASH));

        assert!(apply_peer_status(&mut peers, "peer-b", &payload));
        assert_eq!(peers[1].presence, "busy");
        assert_eq!(peers[1].status_line, "Reviewing designs");
        // The declared fields are the peer's words; the shell's own liveness copy is untouched.
        assert_eq!(peers[1].status, "Available on LAN · end-to-end encrypted");
        assert_eq!(peers[0].presence, "");
        assert_eq!(peers[0].status_line, "");
    }

    #[test]
    fn turning_a_shared_status_off_empties_the_row_but_not_its_liveness() {
        let hidden = platform::AppSettings {
            who_sees_status: "nobody".into(),
            ..Default::default()
        };
        let mut peers = vec![row("peer-a")];
        assert!(apply_peer_status(
            &mut peers,
            "peer-a",
            &pong_payload(&shared_profile(), Uuid::now_v7(), Some(AVATAR_HASH))
        ));

        // The next heartbeat answers with a ping id and nothing else.
        assert!(apply_peer_status(
            &mut peers,
            "peer-a",
            &pong_payload(&hidden, Uuid::now_v7(), None)
        ));
        assert_eq!(peers[0].presence, "");
        assert_eq!(peers[0].status_line, "");
        assert!(peers[0].online);
        assert_eq!(peers[0].status, "Available on LAN · end-to-end encrypted");
    }

    #[test]
    fn an_unchanged_pong_does_not_repaint_the_overlays() {
        let settings = shared_profile();
        let mut peers = vec![row("peer-a")];
        assert!(apply_peer_status(
            &mut peers,
            "peer-a",
            &pong_payload(&settings, Uuid::now_v7(), Some(AVATAR_HASH))
        ));

        // The heartbeat runs on a timer, so the same profile answers it again and again; the
        // ping id changes every time and must not count as a status change.
        assert!(!apply_peer_status(
            &mut peers,
            "peer-a",
            &pong_payload(&settings, Uuid::now_v7(), Some(AVATAR_HASH))
        ));
    }

    #[test]
    fn a_pong_for_a_peer_we_do_not_have_folds_in_nothing() {
        let mut peers = vec![row("peer-a")];
        assert!(!apply_peer_status(
            &mut peers,
            "peer-gone",
            &pong_payload(&shared_profile(), Uuid::now_v7(), Some(AVATAR_HASH))
        ));
        assert_eq!(peers[0].presence, "");
        assert_eq!(peers[0].status_line, "");
    }

    #[test]
    fn a_declared_hash_is_checked_against_the_copy_we_hold() {
        // The pong writer and the cache reader are two halves of one rule, so this pushes a
        // real reply through the decision that follows it.
        let payload = pong_payload(&shared_profile(), Uuid::now_v7(), Some(AVATAR_HASH));
        let declared = payload.get("avatar").and_then(Value::as_str);
        assert_eq!(declared, Some(AVATAR_HASH));
        assert_eq!(
            commands::avatar_action(declared, None),
            commands::AvatarAction::Pull
        );
        assert_eq!(
            commands::avatar_action(declared, Some(AVATAR_HASH)),
            commands::AvatarAction::Keep
        );
        let other = "f".repeat(64);
        assert_eq!(
            commands::avatar_action(declared, Some(other.as_str())),
            commands::AvatarAction::Pull,
            "a second photo from the same peer is a different offer"
        );
    }

    #[test]
    fn an_undeclared_avatar_leaves_the_cache_alone_and_a_withdrawn_one_clears_it() {
        // A peer that declares nothing — an older build, or a profile hidden behind
        // "Nobody" — must not be read as a peer without a photo.
        assert_eq!(
            commands::avatar_action(None, Some(AVATAR_HASH)),
            commands::AvatarAction::Ignore
        );
        assert_eq!(
            commands::avatar_action(None, None),
            commands::AvatarAction::Ignore
        );
        assert_eq!(
            commands::avatar_action(Some("  "), Some(AVATAR_HASH)),
            commands::AvatarAction::Forget
        );
        assert_eq!(
            commands::avatar_action(Some(""), None),
            commands::AvatarAction::Keep
        );
        // A hash that is not a digest would only fail again once the bytes arrived.
        assert_eq!(
            commands::avatar_action(Some("nope"), None),
            commands::AvatarAction::Ignore
        );
    }
}

#[cfg(test)]
mod read_receipt_tests {
    use super::*;

    #[test]
    fn a_batch_round_trips_its_ids() {
        let ids = vec![Uuid::now_v7(), Uuid::now_v7()];
        let envelope = read_receipt_envelope("local", "peer-a", &ids, 42);
        assert_eq!(envelope.kind, "receipt");
        assert_eq!(envelope.from, "local");
        assert_eq!(envelope.to, "peer-a");
        assert_eq!(envelope.hlc.physical_ms, 42);
        assert_eq!(
            envelope.payload.get("status").and_then(Value::as_str),
            Some("read")
        );
        assert_eq!(read_receipt_ids(&envelope).unwrap(), ids);
    }

    #[test]
    fn a_batch_must_carry_a_supported_number_of_ids() {
        let empty = read_receipt_envelope("local", "peer-a", &[], 1);
        assert!(read_receipt_ids(&empty).is_err());

        let too_many = vec![Uuid::now_v7(); MAX_READ_RECEIPT_IDS + 1];
        let oversized = read_receipt_envelope("local", "peer-a", &too_many, 1);
        assert!(read_receipt_ids(&oversized).is_err());

        let full = vec![Uuid::now_v7(); MAX_READ_RECEIPT_IDS];
        let at_cap = read_receipt_envelope("local", "peer-a", &full, 1);
        assert_eq!(
            read_receipt_ids(&at_cap).unwrap().len(),
            MAX_READ_RECEIPT_IDS
        );
    }

    #[test]
    fn a_batch_rejects_malformed_ids() {
        let mut missing = read_receipt_envelope("local", "peer-a", &[Uuid::now_v7()], 1);
        missing.payload = serde_json::json!({ "status": "read" });
        assert!(read_receipt_ids(&missing).is_err());

        let mut malformed = read_receipt_envelope("local", "peer-a", &[Uuid::now_v7()], 1);
        malformed.payload = serde_json::json!({ "status": "read", "message_ids": ["not-a-uuid"] });
        assert!(read_receipt_ids(&malformed).is_err());
    }
}

#[cfg(test)]
mod thread_sync_tests {
    use super::*;
    use alver_core::payloads::{
        MessageDeletePayload, MessageEditPayload, PinUpdatePayload, ReactionPayload,
    };

    fn peer(id: &str) -> String {
        id.to_owned()
    }

    #[test]
    fn a_direct_change_reaches_the_peer_and_nobody_else() {
        assert_eq!(
            commands::thread_audience(&[peer("peer-a")], "local"),
            vec![peer("peer-a")]
        );
    }

    #[test]
    fn a_group_change_reaches_every_other_member_once() {
        let roster = ["peer-b", "local", "peer-a", "peer-b"]
            .iter()
            .map(|id| peer(id))
            .collect::<Vec<_>>();
        // Ordered and deduplicated: one frame per reader, in the same order every time, so a
        // roster that repeats a peer cannot make the outbox repeat the work.
        assert_eq!(
            commands::thread_audience(&roster, "local"),
            vec![peer("peer-a"), peer("peer-b")]
        );
    }

    #[test]
    fn this_device_is_never_in_its_own_audience() {
        assert!(commands::thread_audience(&[peer("local")], "local").is_empty());
        assert!(commands::thread_audience(&[], "local").is_empty());
    }

    #[test]
    fn a_peer_clock_is_believed_up_to_the_skew_bound() {
        let now = 1_700_000_000_000;
        assert!(commands::clock_is_plausible(now - 1, now));
        assert!(commands::clock_is_plausible(now + 5 * 60 * 1_000, now));
        assert!(!commands::clock_is_plausible(now + 5 * 60 * 1_000 + 1, now));
    }

    #[test]
    fn the_edit_window_setting_reaches_the_store_rule() {
        // The composer multiplies the same minutes by 60 000 in `withinEditWindow`; both sides
        // have to land on the same instants, or the setting means one thing to the menu and
        // another to the write.
        assert_eq!(edit_window(15), alver_core::rules::EditWindow::new(900_000));
        assert_eq!(edit_window(0), alver_core::rules::EditWindow::new(0));
        assert_eq!(
            edit_window(u32::MAX),
            alver_core::rules::EditWindow::new(u64::from(u32::MAX) * 60_000),
            "a huge setting multiplies out wide rather than wrapping into a small window"
        );
        assert!(edit_window(15)
            .allows_edit("local", "local", false, 1_000, 901_000)
            .is_ok());
        assert!(edit_window(15)
            .allows_edit("local", "local", false, 1_000, 901_001)
            .is_err());
        assert!(edit_window(0)
            .allows_edit("local", "local", false, 1_000, 1_000)
            .is_ok());
        assert!(edit_window(0)
            .allows_edit("local", "local", false, 1_000, 1_001)
            .is_err());

        // The three refusals the rule can raise, as this command has always answered them.
        assert_eq!(edit_outcome(Ok(true), 15), Ok(true));
        assert_eq!(
            edit_outcome(
                Err("EditWindowExpired { sent_ms: 1, now_ms: 2, window_ms: 3 }".into()),
                15
            ),
            Err("The edit window of 15 minutes has passed.".to_owned()),
            "the expired window reaches the user as the sentence the UI uses for it"
        );
        for refusal in ["NotSender", "Tombstoned"] {
            assert_eq!(
                edit_outcome(Err(refusal.into()), 15),
                Ok(false),
                "{refusal} still means this device may not rewrite the message"
            );
        }
        assert_eq!(
            edit_outcome(Err("no such row".into()), 15),
            Err("no such row".into()),
            "a real fault is not swallowed as a polite false"
        );
    }

    /// The same rule one level up: the command's write half, over a real database.
    #[test]
    fn a_local_edit_outside_the_window_is_refused_before_it_writes() {
        let mut store = Store::open(":memory:", &[43; 32]).expect("an in-memory store");
        let row = |sender: &str, body: &str, at: u64| Message {
            id: Uuid::now_v7(),
            chat_id: "chat-1".to_owned(),
            sender: sender.to_owned(),
            body: body.to_owned(),
            clock: Hlc {
                physical_ms: at,
                logical: 0,
            },
            tombstone: false,
            forwarded: false,
        };
        let stale = row("local", "written an hour ago", 1_000);
        let fresh = row("local", "written now", 3_600_000);
        let theirs = row("peer-a", "not this device's", 3_600_000);
        for message in [&stale, &fresh, &theirs] {
            store.save_message(message).unwrap();
        }
        let now = Hlc {
            physical_ms: 3_601_000,
            logical: 0,
        };

        assert_eq!(
            apply_local_edit(&mut store, stale.id, "edited late", now, 15),
            Err("The edit window of 15 minutes has passed.".to_owned())
        );
        assert_eq!(
            store.get_message(stale.id).unwrap().unwrap().body,
            "written an hour ago",
            "a refused edit leaves the stored body alone"
        );
        assert!(apply_local_edit(&mut store, fresh.id, "in time", now, 15).unwrap());
        assert_eq!(
            store.get_message(fresh.id).unwrap().unwrap().body,
            "in time"
        );
        assert_eq!(
            apply_local_edit(&mut store, theirs.id, "forged", now, 15),
            Ok(false),
            "somebody else's message stays the plain false the composer words as its own"
        );
        assert_eq!(
            store.get_message(theirs.id).unwrap().unwrap().body,
            "not this device's"
        );
        // The setting gates the write, not a constant baked into the command.
        assert!(apply_local_edit(&mut store, stale.id, "an hour is fine", now, 60).unwrap());
        assert_eq!(
            store.get_message(stale.id).unwrap().unwrap().body,
            "an hour is fine"
        );
    }

    #[test]
    fn only_a_delete_for_everyone_converges() {
        assert_eq!(
            commands::unsend_action(true),
            commands::UnsendAction::Tombstone
        );
        // A peer's delete-for-me is a claim about its own history, never about ours.
        assert_eq!(
            commands::unsend_action(false),
            commands::UnsendAction::Ignore
        );
    }

    #[test]
    fn a_thread_frame_missing_its_scope_still_asks_for_everyone() {
        // The wire default matters for safety: a frame that forgets the field must not become
        // a silent delete-for-me that this device drops.
        let lean: MessageDeletePayload = serde_json::from_value(serde_json::json!({
            "message_id": Uuid::now_v7().to_string(),
        }))
        .unwrap();
        assert!(lean.for_everyone);
        assert_eq!(
            commands::unsend_action(lean.for_everyone),
            commands::UnsendAction::Tombstone
        );
    }

    #[test]
    fn every_thread_frame_round_trips_through_its_handler_shape() {
        let id = Uuid::now_v7();
        let chat = "peer-a".to_owned();

        let edit = MessageEditPayload {
            message_id: id,
            body: "corrected".into(),
            clock: Hlc {
                physical_ms: 1_700_000_000_000,
                logical: 3,
            },
        };
        let restored: MessageEditPayload =
            serde_json::from_value(serde_json::to_value(&edit).unwrap()).unwrap();
        assert_eq!(restored, edit);

        let delete = MessageDeletePayload {
            message_id: id,
            for_everyone: true,
            reason: None,
        };
        let restored: MessageDeletePayload =
            serde_json::from_value(serde_json::to_value(&delete).unwrap()).unwrap();
        assert_eq!(restored, delete);

        let reaction = ReactionPayload {
            message: alver_core::payloads::MessageRef {
                chat_id: chat.clone(),
                message_id: id,
            },
            emoji: "👍".into(),
            remove: false,
        };
        let restored: ReactionPayload =
            serde_json::from_value(serde_json::to_value(&reaction).unwrap()).unwrap();
        assert_eq!(restored, reaction);
        // The handler re-checks the chat the frame names, so it has to survive the wire.
        assert_eq!(restored.message.chat_id, chat);

        let pin = PinUpdatePayload {
            chat_id: chat.clone(),
            message_id: id,
            pinned: true,
            position: 2,
        };
        let value = serde_json::to_value(&pin).unwrap();
        let restored: PinUpdatePayload = serde_json::from_value(value.clone()).unwrap();
        assert_eq!(restored, pin);
        // The handler refuses a frame without it, and the writer must never produce one.
        assert!(value.get("pinned").is_some());
        assert_eq!(value.get("position").and_then(Value::as_u64), Some(2));
    }

    #[test]
    fn only_a_reader_of_a_chat_can_change_its_shared_list() {
        let direct = vec![peer("peer-a")];
        assert!(commands::shares_chat(&direct, "peer-a"));
        // A stranger naming one of our 1:1 threads is dropped, not refused outright: the
        // frame is a bug to survive, and ending the session would take the peer's real
        // traffic down with it.
        assert!(!commands::shares_chat(&direct, "peer-b"));
        let roster = vec![peer("peer-a"), peer("peer-b")];
        assert!(commands::shares_chat(&roster, "peer-b"));
        assert!(!commands::shares_chat(&roster, "local"));
        assert!(!commands::shares_chat(&[], "peer-a"));
    }
}

#[cfg(test)]
mod discovery_tests {
    use super::*;
    use alver_core::discovery::MAX_BEACON_BYTES;

    #[test]
    fn our_own_broadcast_comes_back_and_is_not_a_peer() {
        // A datagram sent to the limited address returns to the sender, and this machine's
        // own beacon verifies, so the only thing that separates it from a peer is the id.
        let mine = PeerBeacon::signed(
            &Identity::from_secret([7; 32]),
            "Sarath Shailan".into(),
            "192.168.1.10".into(),
            45_123,
        );
        let my_id = mine.peer_id.clone();
        assert_eq!(
            classify_udp_sighting(&my_id, &my_id, None, 1_000, UDP_PEER_TTL_MS),
            UdpSighting::Mine
        );
        assert_eq!(
            classify_udp_sighting("peer-b", &my_id, None, 1_000, UDP_PEER_TTL_MS),
            UdpSighting::New
        );
    }

    #[test]
    fn a_beacon_that_keeps_arriving_is_news_once() {
        // The retry loop it starts costs a session, so a repeat sighting must not restart it.
        let ttl = UDP_PEER_TTL_MS;
        assert_eq!(
            classify_udp_sighting("peer-b", "peer-a", Some(100), 100 + ttl - 1, ttl),
            UdpSighting::Seen
        );
        assert_eq!(
            classify_udp_sighting("peer-b", "peer-a", Some(100), 100 + ttl, ttl),
            UdpSighting::New
        );
    }

    #[test]
    fn silence_expires_only_peers_the_fallback_alone_vouches_for() {
        let ttl = UDP_PEER_TTL_MS;
        assert!(udp_may_expire_peer(0, ttl, ttl, false));
        assert!(!udp_may_expire_peer(0, ttl - 1, ttl, false));
        // mDNS still lists this peer, or a warm session is already answering pings:
        // whichever owns it says whether it has left.
        assert!(!udp_may_expire_peer(0, ttl * 10, ttl, true));
    }

    #[test]
    fn an_announced_beacon_fits_the_datagram_that_carries_it() {
        // The fallback has no framing of its own beyond a bounded datagram, so the widest
        // beacon the signature allows still has to travel in one piece.
        let widest = PeerBeacon::signed(
            &Identity::from_secret([3; 32]),
            "S".repeat(120),
            "2001:0db8:85a3:0000:0000:8a2e:0370:7334".into(),
            45_123,
        );
        let bytes = serde_json::to_vec(&widest).expect("a beacon serialises");
        assert!(
            bytes.len() <= MAX_BEACON_BYTES,
            "a {} byte beacon does not fit the discovery datagram",
            bytes.len()
        );
        let restored: PeerBeacon = serde_json::from_slice(&bytes).expect("the same shape");
        assert_eq!(restored, widest);
        // The host a fallback peer dials is the literal address the announcer signed.
        assert!(restored.host.parse::<std::net::IpAddr>().is_ok());
        assert!(restored.verify());
    }

    /// The English the two documents use for these durations. A constant that moves into a gap
    /// in this table fails the test below by name, which is the point: the sentence is then
    /// describing a duration nothing runs.
    fn spelled(seconds: u64) -> &'static str {
        match seconds {
            1 => "one",
            2 => "two",
            3 => "three",
            4 => "four",
            5 => "five",
            10 => "ten",
            15 => "fifteen",
            20 => "twenty",
            30 => "thirty",
            60 => "sixty",
            other => panic!("no spelling for {other} in this table, so no sentence can name it"),
        }
    }

    /// Two documents state the fallback's port, its repeat interval, its expiry and the heartbeat
    /// period. None of the four is compiled, and three of them are prose rather than a config a
    /// builder reads, so the only defence is to read the sentences back against the constants.
    #[test]
    fn the_documents_name_the_discovery_numbers_this_build_uses() {
        let root = super::seam_scan::root();
        let announce = spelled(UDP_ANNOUNCE_INTERVAL.as_secs());
        let port = alver_core::discovery::DISCOVERY_PORT.to_string();
        for document in ["README.md", "docs/PROTOCOL.md"] {
            let path = root.join(document);
            let text = fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display()));
            assert!(
                text.contains(&port),
                "{document} no longer names discovery port {port}, which is the number a firewall \
                 rule is built from"
            );
            assert!(
                text.contains(&format!("every {announce} seconds")),
                "{document} no longer says the beacon repeats every {announce} seconds"
            );
        }
        let protocol = fs::read_to_string(root.join("docs/PROTOCOL.md")).expect("docs/PROTOCOL.md");
        assert!(
            protocol.contains(&format!("{} seconds", spelled(UDP_PEER_TTL_MS / 1_000))),
            "docs/PROTOCOL.md no longer states the expiry as {} seconds",
            spelled(UDP_PEER_TTL_MS / 1_000)
        );
        assert!(
            protocol.contains(&format!(
                "{} missed announcements",
                spelled(UDP_PEER_MISSED_ANNOUNCEMENTS)
            )),
            "docs/PROTOCOL.md no longer states the expiry as {} missed announcements",
            spelled(UDP_PEER_MISSED_ANNOUNCEMENTS)
        );
        assert!(
            protocol.contains(&format!("every {} s", HEARTBEAT_INTERVAL.as_secs())),
            "docs/PROTOCOL.md no longer probes a warm session every {} s",
            HEARTBEAT_INTERVAL.as_secs()
        );
    }
}

/// The kind table is what two builds agree on, so the frames this one writes have to appear
/// in it. A kind the session reader hands to a handler but core never declared is a peer that
/// cannot stay connected, and the reverse is just as true: a kind the table declares must not
/// end a session because this build happens not to act on it. `frame_action` is where both
/// halves are decided, once.
#[cfg(test)]
mod frame_table_tests {
    use super::*;
    use alver_core::groups::GroupState;
    use alver_core::payloads::{
        classify, is_known_type, ByePayload, GroupStatePayload, Payload, ALL_TYPES,
    };

    #[test]
    fn every_frame_this_build_owns_is_a_declared_protocol_type() {
        for frame_kind in commands::OWNED_KINDS {
            assert!(
                is_known_type(frame_kind),
                "{frame_kind} reaches a handler but is not in the declared table"
            );
            assert!(
                commands::handled_kind(frame_kind),
                "{frame_kind} is declared but the session reader refuses it"
            );
        }
        // The handshake that opens the session and the kinds the reader answers itself.
        for frame_kind in [
            kind::AUTH,
            kind::MESSAGE,
            kind::TYPING,
            kind::PING,
            kind::PONG,
            kind::RECEIPT,
        ] {
            assert!(
                is_known_type(frame_kind),
                "{frame_kind} is answered but is not in the declared table"
            );
        }
    }

    #[test]
    fn a_challenge_from_a_device_with_no_passphrase_still_parses() {
        // `mac: null` is the half of the pair that has no secret; it has to arrive as a
        // well-formed challenge, or a passphrase-less LAN cannot tell it from a hostile one.
        let challenge = Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: kind::AUTH.into(),
            from: "peer-a".into(),
            to: "peer-b".into(),
            hlc: Hlc {
                physical_ms: 1,
                logical: 0,
            },
            payload: serde_json::json!({"nonce": "t9G7Q0Z0m1Vg2w8oQq4hKdLs7nYp3RfBcXaJ0kUeTiM=", "mac": null}),
        };
        assert_eq!(
            classify(&challenge),
            Payload::Auth(alver_core::payloads::AuthPayload {
                nonce: "t9G7Q0Z0m1Vg2w8oQq4hKdLs7nYp3RfBcXaJ0kUeTiM=".into(),
                mac: None,
            })
        );
    }

    #[test]
    fn a_signed_roster_classifies_as_the_frame_that_carries_it() {
        let admin = Identity::from_secret([7; 32]);
        let admin_id = admin.peer_id();
        let state = GroupState::create(
            &admin,
            "group-a".into(),
            "Design".into(),
            [admin_id.clone(), "peer-b".into()],
        )
        .expect("a roster that contains its admin");
        // Byte-for-byte what `deliver_group_state` queues: the signed state, bare.
        let envelope = Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: kind::GROUP_STATE.into(),
            from: admin_id,
            to: "peer-b".into(),
            hlc: Hlc {
                physical_ms: 1,
                logical: 0,
            },
            payload: serde_json::to_value(&state).expect("a roster state serialises"),
        };
        assert_eq!(
            classify(&envelope),
            Payload::GroupState(GroupStatePayload(state)),
            "the roster frame the writer produces must be the one the table describes"
        );
    }

    /// The other direction of the same agreement: a kind the table declares must never be
    /// the reason a peer loses its session, even when this build has no handler for it.
    #[test]
    fn a_declared_kind_is_never_a_reason_to_drop_the_session() {
        for frame_kind in ALL_TYPES {
            assert_ne!(
                frame_action(frame_kind),
                FrameAction::Reject,
                "{frame_kind} is declared by the protocol but ends the session that receives it"
            );
        }
        // The frames this build reads but does not act on, named rather than inferred from
        // the table: a peer built to the documented protocol sends these, and the contract
        // for an unreadable-by-me frame is that it is ignored. `pin_update` and `bye` are
        // absent because they gained handlers after this list was first written.
        for frame_kind in [
            kind::HELLO,
            kind::HELLO_ACK,
            kind::PRESENCE,
            kind::PROFILE_UPDATE,
            kind::SYNC_REQUEST,
            kind::GROUP_CREATE,
            kind::GROUP_UPDATE,
            kind::GROUP_MEMBER_CHANGE,
            kind::GROUP_ICON_UPDATE,
        ] {
            assert_eq!(
                frame_action(frame_kind),
                FrameAction::Ignore,
                "{frame_kind} must be acknowledged and dropped, not fatal"
            );
        }
        // Only something outside the table is fatal — that is what the guard is for.
        assert_eq!(frame_action("not_a_frame"), FrameAction::Reject);
    }

    #[test]
    fn the_goodbye_is_a_close_and_not_an_error() {
        assert_eq!(frame_action(kind::BYE), FrameAction::Close);
        // What `broadcast_goodbye` writes has to be the frame the table describes.
        let frame = Envelope {
            v: PROTOCOL_VERSION,
            id: Uuid::now_v7(),
            kind: kind::BYE.into(),
            from: "peer-a".into(),
            to: "peer-b".into(),
            hlc: Hlc {
                physical_ms: 1,
                logical: 0,
            },
            payload: serde_json::json!({"reason": "closed"}),
        };
        assert_eq!(
            classify(&frame),
            Payload::Bye(ByePayload {
                reason: Some("closed".into())
            })
        );
    }
}

#[cfg(test)]
mod env_docs_tests {
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::{Path, PathBuf};

    // Spliced so this module does not contain the literal it scans for; a test that
    // finds itself in the sources it audits would audit nothing.
    const PREFIX: &str = concat!("ALVERCHAT", "_");

    /// Every uppercase-snake-case name starting with `PREFIX`.
    fn names_in(text: &str) -> BTreeSet<String> {
        let mut found = BTreeSet::new();
        let bytes = text.as_bytes();
        let mut at = 0;
        while let Some(offset) = text[at..].find(PREFIX) {
            let start = at + offset;
            let mut end = start + PREFIX.len();
            while end < bytes.len()
                && (bytes[end].is_ascii_uppercase()
                    || bytes[end].is_ascii_digit()
                    || bytes[end] == b'_')
            {
                end += 1;
            }
            let name = &text[start..end];
            if name.len() > PREFIX.len() && !name.ends_with('_') {
                found.insert(name.to_string());
            }
            at = end;
        }
        found
    }

    fn rust_files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in dir.read_dir().expect("workspace source directory") {
            let path = entry.expect("readable directory entry").path();
            let skipped = ["target", "node_modules", "gen"];
            if path.is_dir() {
                if skipped.contains(&path.file_name().unwrap_or_default().to_str().unwrap_or("")) {
                    continue;
                }
                rust_files(&path, into);
            } else if path.extension().is_some_and(|name| name == "rs") {
                into.push(path);
            }
        }
    }

    /// What README.md promises, and what the Rust app actually reads.
    fn pair() -> (BTreeSet<String>, BTreeSet<String>) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("this crate sits three directories below the repository root")
            .to_path_buf();
        let readme =
            fs::read_to_string(root.join("README.md")).expect("README.md at the repository root");
        let mut sources = Vec::new();
        for dir in ["apps", "crates"] {
            rust_files(&root.join(dir), &mut sources);
        }
        assert!(
            sources.len() > 20,
            "only {} Rust files under the workspace roots",
            sources.len()
        );
        let mut read = BTreeSet::new();
        for file in sources {
            let text = fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            read.extend(names_in(&text));
        }
        (names_in(&readme), read)
    }

    #[test]
    fn the_readme_documents_only_variables_the_app_reads() {
        let (documented, read) = pair();
        let stale: Vec<&String> = documented.difference(&read).collect();
        assert!(
            stale.is_empty(),
            "README.md documents {stale:?}, which no Rust source reads any more"
        );
        assert!(
            read.contains("ALVERCHAT_KEYCHAIN"),
            "the keychain opt-in has left the code, so the README paragraph about it is a lie"
        );
    }

    #[test]
    fn every_variable_the_app_reads_is_documented() {
        let (documented, read) = pair();
        let hidden: Vec<&String> = read.difference(&documented).collect();
        assert!(
            hidden.is_empty(),
            "the app reads {hidden:?} from the environment and README.md never mentions them"
        );
    }
}

/// The byte-level readers shared by the three source-seam guards.
///
/// They live in one place so the guard over the events, the guard over the commands and the
/// guard over the shapes cannot grow three different ideas about what one argument, one name
/// or one field looks like in a language they only read as text.
#[cfg(test)]
mod seam_scan {
    use std::path::{Path, PathBuf};

    /// The key Tauri's generated argument struct asks the front end for, given the parameter
    /// name Rust wrote — which is also what the camelCase rename puts on the wire for a field:
    /// underscores gone, the word after each one starting uppercase.
    pub(super) fn camel(name: &str) -> String {
        let mut out = String::new();
        let mut upper = false;
        for ch in name.chars() {
            if ch == '_' {
                upper = true;
            } else if upper {
                upper = false;
                out.extend(ch.to_uppercase());
            } else {
                out.push(ch);
            }
        }
        out
    }

    /// A parameter Tauri hands over itself, which is the only reason a command can be called
    /// with fewer arguments than it declares parameters, and the only reason a type in a
    /// signature is not a shape crossing the seam.
    pub(super) fn is_injected(kind: &str) -> bool {
        ["AppHandle", "State<", "Manager<", "WebviewWindow"]
            .iter()
            .any(|marker| kind.contains(marker))
    }

    /// Line and block comments gone, so a comment can neither invent a field nor hide one.
    /// Quoted text is kept, because `": "` inside a string is data and not a field separator.
    pub(super) fn strip_comments(text: &str) -> String {
        let mut out = String::with_capacity(text.len());
        let mut chars = text.chars().peekable();
        let mut quoted: Option<char> = None;
        while let Some(ch) = chars.next() {
            match quoted {
                Some(quote) => {
                    out.push(ch);
                    if ch == '\\' {
                        if let Some(escaped) = chars.next() {
                            out.push(escaped);
                        }
                    } else if ch == quote {
                        quoted = None;
                    }
                }
                None => match ch {
                    '"' | '\'' => {
                        quoted = Some(ch);
                        out.push(ch);
                    }
                    '/' if chars.peek() == Some(&'/') => loop {
                        match chars.next() {
                            None => break,
                            Some('\n') => {
                                out.push('\n');
                                break;
                            }
                            Some(_) => {}
                        }
                    },
                    '/' if chars.peek() == Some(&'*') => {
                        chars.next();
                        while let Some(next) = chars.next() {
                            if next == '\n' {
                                out.push('\n');
                            }
                            if next == '*' && chars.peek() == Some(&'/') {
                                chars.next();
                                break;
                            }
                        }
                    }
                    _ => out.push(ch),
                },
            }
        }
        out
    }

    fn skip_spaces(bytes: &[u8], at: &mut usize) {
        while bytes
            .get(*at)
            .is_some_and(|byte| matches!(byte, b' ' | b'\t' | b'\n' | b'\r'))
        {
            *at += 1;
        }
    }

    /// Steps over one balanced `<…>` type argument, which is how the front end says what shape
    /// it expects a payload to carry.
    fn skip_generics(bytes: &[u8], at: &mut usize) {
        if bytes.get(*at) != Some(&b'<') {
            return;
        }
        let mut depth = 0usize;
        while let Some(byte) = bytes.get(*at) {
            match byte {
                b'<' => depth += 1,
                b'>' => {
                    depth -= 1;
                    if depth == 0 {
                        *at += 1;
                        return;
                    }
                }
                _ => {}
            }
            *at += 1;
        }
    }

    /// The index that closes the bracket at `open`, counting every kind of bracket so that a
    /// generic, an array and an object literal all balance.
    pub(super) fn closed(bytes: &[u8], open: usize) -> usize {
        let mut depth = 0usize;
        let mut at = open;
        while at < bytes.len() {
            match bytes[at] {
                b'(' | b'<' | b'[' | b'{' => depth += 1,
                b')' | b'>' | b']' | b'}' if depth > 0 => {
                    depth -= 1;
                    if depth == 0 {
                        return at;
                    }
                }
                _ => {}
            }
            at += 1;
        }
        panic!("a bracket opened at {open} and never closed");
    }

    /// Split on commas that sit outside every bracket, which is how one argument list becomes
    /// its arguments.
    pub(super) fn top_level(list: &str) -> Vec<String> {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut start = 0;
        for (at, byte) in list.bytes().enumerate() {
            match byte {
                b'(' | b'<' | b'[' | b'{' => depth += 1,
                b')' | b'>' | b']' | b'}' if depth > 0 => depth -= 1,
                b',' if depth == 0 => {
                    parts.push(list[start..at].to_owned());
                    start = at + 1;
                }
                _ => {}
            }
        }
        parts.push(list[start..].to_owned());
        parts
    }

    /// The parentheses of the call whose name ends at `at`, stepping over a type argument when
    /// the caller says the language allows one. Nothing when this is not a call.
    pub(super) fn call_span(
        bytes: &[u8],
        mut at: usize,
        with_generics: bool,
    ) -> Option<(usize, usize)> {
        skip_spaces(bytes, &mut at);
        if with_generics {
            skip_generics(bytes, &mut at);
            skip_spaces(bytes, &mut at);
        }
        if bytes.get(at) != Some(&b'(') {
            return None;
        }
        Some((at, closed(bytes, at)))
    }

    /// The quoted literal in the first argument of the call whose name ends at `at`. Nothing
    /// when the argument is a variable, because a name built at runtime is not greppable —
    /// which is precisely the kind of wiring this guard cannot see, and so cannot claim.
    pub(super) fn first_string_argument(
        bytes: &[u8],
        at: usize,
        with_generics: bool,
    ) -> Option<String> {
        let (open, close) = call_span(bytes, at, with_generics)?;
        let mut at = open + 1;
        skip_spaces(bytes, &mut at);
        if at >= close {
            return None;
        }
        let quote = bytes[at];
        if quote != b'"' && quote != b'\'' {
            return None;
        }
        at += 1;
        let mut name = Vec::new();
        while let Some(byte) = bytes.get(at) {
            at += 1;
            if *byte == quote {
                return String::from_utf8(name).ok();
            }
            if *byte == b'\\' {
                at += 1;
                continue;
            }
            if name.len() > 64 {
                return None;
            }
            name.push(*byte);
        }
        None
    }

    pub(super) fn sources(dir: &Path, extension: &str, into: &mut Vec<PathBuf>) {
        for entry in dir.read_dir().expect("workspace source directory") {
            let path = entry.expect("readable directory entry").path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if ["node_modules", "target", "gen", "dist"].contains(&name.as_str()) {
                continue;
            }
            if path.is_dir() {
                sources(&path, extension, into);
            } else if path.extension().is_some_and(|tail| tail == extension)
                && !name.contains(".test.")
            {
                into.push(path);
            }
        }
    }

    pub(super) fn root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("this crate sits three directories below the repository root")
            .to_path_buf()
    }
}

#[cfg(test)]
mod event_wiring_tests {
    use super::seam_scan::{first_string_argument, root, sources};
    use std::collections::BTreeSet;
    use std::fs;

    // Spliced, because a scanner that matched its own module would audit nothing. The Rust
    // walk reads this file, so neither mark may appear here as a literal.
    const EMIT: &str = concat!("em", "it", "(");
    const REGISTRATION_TOKENS: [&str; 2] = [concat!("lis", "ten"), "on"];

    /// An event name as both halves of this repo write it: lowercase words, single hyphens.
    fn is_event_name(name: &str) -> bool {
        !name.is_empty()
            && name.split('-').all(|part| {
                !part.is_empty()
                    && part
                        .bytes()
                        .next()
                        .is_some_and(|first| first.is_ascii_lowercase())
                    && part
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit())
            })
    }

    /// Every event name the Rust side puts on the wire.
    fn emitted() -> BTreeSet<String> {
        let root = root();
        let mut files = Vec::new();
        for dir in ["apps", "crates"] {
            sources(&root.join(dir), "rs", &mut files);
        }
        assert!(
            files.len() > 20,
            "only {} Rust files under the workspace roots",
            files.len()
        );
        let mut names = BTreeSet::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let bytes = text.as_bytes();
            let mut at = 0;
            while let Some(offset) = text[at..].find(EMIT) {
                let start = at + offset + EMIT.len();
                // `start - 1` is the `(` the mark ends with.
                if let Some(name) = first_string_argument(bytes, start - 1, false) {
                    if is_event_name(&name) {
                        names.insert(name);
                    }
                }
                at = start;
            }
        }
        names
    }

    /// Every event name the front end registers a listener for, through `listen` or through
    /// the shell's own `on` wrapper around it.
    fn registered() -> BTreeSet<String> {
        let root = root();
        let mut files = Vec::new();
        for dir in ["ui", "bridge"] {
            for extension in ["ts", "tsx"] {
                sources(&root.join("packages").join(dir), extension, &mut files);
            }
        }
        assert!(
            files.len() > 20,
            "only {} TypeScript files under the packages",
            files.len()
        );
        let mut names = BTreeSet::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let bytes = text.as_bytes();
            for token in REGISTRATION_TOKENS {
                let mut at = 0;
                while let Some(offset) = text[at..].find(token) {
                    let start = at + offset + token.len();
                    let word_start = at + offset == 0
                        || !matches!(
                            bytes[at + offset - 1],
                            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'
                        );
                    if word_start {
                        if let Some(name) = first_string_argument(bytes, start, true) {
                            if is_event_name(&name) {
                                names.insert(name);
                            }
                        }
                    }
                    at = start;
                }
            }
        }
        names
    }

    #[test]
    fn every_event_rust_emits_is_heard_by_the_front_end() {
        let (emitted, registered) = (emitted(), registered());
        assert!(
            emitted.contains("overlay-state"),
            "the walk found {emitted:?}, which cannot be every event the shell reads"
        );
        let dead: Vec<&String> = emitted.difference(&registered).collect();
        assert!(
            dead.is_empty(),
            "Rust emits {dead:?} and nothing in the front end ever listens for it"
        );
    }

    #[test]
    fn every_event_the_front_end_listens_for_is_emitted() {
        let (emitted, registered) = (emitted(), registered());
        assert!(
            registered.contains("overlay-state"),
            "the walk found {registered:?}, which cannot be every listener the shell registers"
        );
        let waiting: Vec<&String> = registered.difference(&emitted).collect();
        assert!(
            waiting.is_empty(),
            "the front end waits for {waiting:?}, which Rust never emits"
        );
    }
}

#[cfg(test)]
mod command_wiring_tests {
    use super::seam_scan::{
        call_span, camel, closed, first_string_argument, is_injected, root, sources, top_level,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    // Spliced for the reason the event guard splices: the handler list lives in this file, so a
    // literal mark would be the first thing the scan reports, and the scan would report it.
    const HANDLER: &str = concat!("gene", "rate_handler![");
    const INVOKE: &str = concat!("in", "voke");
    const ATTRIBUTE: &str = concat!("#[tauri::", "command]");

    /// A command name as both halves of this repo write it: one lowercase word or several
    /// joined by underscores, which is also exactly what the command attribute turns into a
    /// callable string. Anything else is a call site this scan cannot read, and says so.
    fn is_command_name(name: &str) -> bool {
        !name.is_empty()
            && name
                .bytes()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase())
            && name
                .bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
    }

    /// Every name this build hands to Tauri. A `platform::get_settings` entry counts under
    /// `get_settings`, because the path is how Rust finds the function and the tail is how the
    /// front end names the call.
    fn registered() -> BTreeSet<String> {
        let path = root().join("apps/desktop/src-tauri/src/main.rs");
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let at = text
            .find(HANDLER)
            .expect("the app tells Tauri which commands exist in one handler list");
        let body = at + HANDLER.len();
        let mut depth = 1usize;
        let mut close = None;
        for (offset, byte) in text.as_bytes()[body..].iter().enumerate() {
            match byte {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        close = Some(body + offset);
                        break;
                    }
                }
                _ => {}
            }
        }
        let end = close.expect("the handler list never closes");
        let mut names = BTreeSet::new();
        for entry in text[body..end].split(',') {
            let name = entry.trim().split("::").last().unwrap_or_default().trim();
            if name.is_empty() {
                continue;
            }
            assert!(
                is_command_name(name),
                "{name:?} is in the handler list and is not shaped like a command"
            );
            names.insert(name.to_owned());
        }
        names
    }

    /// Every command the front end asks for by a literal string. An `invoke` whose first
    /// argument is built at runtime is not greppable and is not counted — the same blind spot
    /// the event guard declares about a name assembled before it is emitted.
    fn invoked() -> BTreeSet<String> {
        let root = root();
        let mut files = Vec::new();
        for dir in ["ui", "bridge"] {
            for extension in ["ts", "tsx"] {
                sources(&root.join("packages").join(dir), extension, &mut files);
            }
        }
        assert!(
            files.len() > 20,
            "only {} TypeScript files under the packages",
            files.len()
        );
        let mut names = BTreeSet::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let bytes = text.as_bytes();
            let mut at = 0;
            while let Some(offset) = text[at..].find(INVOKE) {
                let start = at + offset + INVOKE.len();
                let word_start = at + offset == 0
                    || !matches!(
                        bytes[at + offset - 1],
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'
                    );
                if word_start {
                    if let Some(name) = first_string_argument(bytes, start, true) {
                        assert!(
                            is_command_name(&name),
                            "{} calls invoke({name:?}), which this scan cannot match to a \
                             command the app owns",
                            file.display()
                        );
                        names.insert(name);
                    }
                }
                at = start;
            }
        }
        names
    }

    /// Every command behind the attribute, with the parameters each one declares. A parameter
    /// the reader cannot split into a name and a type stops the scan, because a signature half
    /// read is a seam half audited.
    fn signatures() -> BTreeMap<String, Vec<(String, String)>> {
        let root = root();
        let mut files = Vec::new();
        sources(&root.join("apps/desktop/src-tauri/src"), "rs", &mut files);
        assert!(
            files.len() > 5,
            "only {} files in the desktop crate",
            files.len()
        );
        let mut out = BTreeMap::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let bytes = text.as_bytes();
            let mut at = 0;
            while let Some(offset) = text[at..].find(ATTRIBUTE) {
                let attribute = at + offset + ATTRIBUTE.len();
                let function = attribute
                    + text[attribute..].find("fn ").unwrap_or_else(|| {
                        panic!("{}: an attributed command with no function", file.display())
                    })
                    + 3;
                let open = function
                    + text[function..]
                        .find('(')
                        .unwrap_or_else(|| panic!("{}: no parameter list", file.display()));
                let name = text[function..open].trim().to_owned();
                let close = closed(bytes, open);
                let mut params = Vec::new();
                for part in top_level(&text[open + 1..close]) {
                    let part = part.trim();
                    if part.is_empty() {
                        continue;
                    }
                    let mut halves = part.splitn(2, ':');
                    let param = halves.next().unwrap_or("").trim();
                    let kind = halves.next().unwrap_or("").trim();
                    assert!(
                        !kind.is_empty(),
                        "{}: `{name}({part})` is not a parameter this scan can read",
                        file.display()
                    );
                    params.push((param.to_owned(), kind.to_owned()));
                }
                out.insert(name, params);
                at = close;
            }
        }
        out
    }

    /// Every command call the front end writes, with the argument keys the literal sends. A
    /// call whose second argument is not an object literal is a scan failure rather than a
    /// skipped site, so a reader cannot quietly stop seeing half the seam.
    fn call_sites() -> Vec<(PathBuf, String, BTreeSet<String>)> {
        let root = root();
        let mut files = Vec::new();
        for dir in ["ui", "bridge"] {
            for extension in ["ts", "tsx"] {
                sources(&root.join("packages").join(dir), extension, &mut files);
            }
        }
        let mut sites = Vec::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let bytes = text.as_bytes();
            let mut at = 0;
            while let Some(offset) = text[at..].find(INVOKE) {
                let word_at = at + offset;
                let start = word_at + INVOKE.len();
                at = start;
                if word_at > 0
                    && matches!(
                        bytes[word_at - 1],
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'
                    )
                {
                    continue;
                }
                let Some(name) = first_string_argument(bytes, start, true) else {
                    continue;
                };
                let (open, close) = call_span(bytes, start, true).unwrap_or_else(|| {
                    panic!("{}: invoke({name:?}) has no argument list", file.display())
                });
                let parts: Vec<String> = top_level(&text[open + 1..close])
                    .into_iter()
                    .map(|part| part.trim().to_owned())
                    .filter(|part| !part.is_empty())
                    .collect();
                let mut keys = BTreeSet::new();
                match parts.as_slice() {
                    [_] => {}
                    [_, object] => {
                        let body = object
                            .strip_prefix('{')
                            .and_then(|tail| tail.strip_suffix('}'))
                            .unwrap_or_else(|| {
                                panic!(
                                    "{}: invoke({name:?}) sends `{object}`, which is not an \
                                     argument literal this scan can read",
                                    file.display()
                                )
                            });
                        for part in top_level(body) {
                            let part = part.trim();
                            if part.is_empty() {
                                continue;
                            }
                            if part.starts_with("...") {
                                panic!(
                                    "{}: invoke({name:?}) spreads its arguments, so this scan \
                                     cannot say what the call sends",
                                    file.display()
                                );
                            }
                            let key = part
                                .split(':')
                                .next()
                                .unwrap_or("")
                                .trim()
                                .trim_start_matches(['\'', '"']);
                            assert!(
                                key.bytes().all(|byte| byte.is_ascii_alphanumeric()
                                    || byte == b'_'
                                    || byte == b'$'),
                                "{}: `{part}` is not an argument key",
                                file.display()
                            );
                            keys.insert(key.to_owned());
                        }
                    }
                    others => panic!(
                        "{}: invoke({name:?}) sends {} arguments, and Tauri takes one object",
                        file.display(),
                        others.len() - 1
                    ),
                }
                sites.push((file.clone(), name, keys));
            }
        }
        sites
    }

    #[test]
    fn every_argument_the_front_end_sends_names_a_parameter() {
        let (signatures, sites) = (signatures(), call_sites());
        assert!(
            sites.len() > 50,
            "the walk found {} invoke sites, which cannot be every call the shell makes",
            sites.len()
        );
        let mut stray = Vec::new();
        for (file, name, keys) in &sites {
            let params = signatures.get(name).unwrap_or_else(|| {
                panic!("{}: `{name}` has no attributed definition", file.display())
            });
            for key in keys {
                if !params
                    .iter()
                    .any(|(param, kind)| !is_injected(kind) && camel(param) == *key)
                {
                    stray.push(format!("{name} → {key}"));
                }
            }
        }
        assert!(
            stray.is_empty(),
            "the front end sends {stray:?}, which no command of that name declares, so Tauri \
             answers that call with an argument error at runtime"
        );
    }

    #[test]
    fn every_parameter_is_sent_or_supplied_by_tauri() {
        let (signatures, sites) = (signatures(), call_sites());
        assert!(
            signatures.contains_key("send_message"),
            "the signature walk found {:?}, which cannot be this crate's commands",
            signatures.keys().collect::<Vec<_>>()
        );
        let mut starved = Vec::new();
        for (file, name, keys) in &sites {
            let params = signatures.get(name).unwrap_or_else(|| {
                panic!("{}: `{name}` has no attributed definition", file.display())
            });
            for (param, kind) in params {
                if !is_injected(kind) && !keys.contains(&camel(param)) {
                    starved.push(format!("{name} → {param}"));
                }
            }
        }
        assert!(
            starved.is_empty(),
            "{starved:?} are declared by a command the front end calls and never sent to it, so \
             Tauri refuses the call as missing an argument"
        );
    }

    /// The string an `APP_NAME` constant is set to, read from whichever half declares it.
    fn declared_name(source: &Path) -> String {
        let text = fs::read_to_string(source)
            .unwrap_or_else(|error| panic!("{}: {error}", source.display()));
        let at = text
            .find("APP_NAME")
            .unwrap_or_else(|| panic!("{} declares no app name", source.display()));
        let open = at + text[at..].find('"').expect("an app name in quotes");
        let tail = &text[open + 1..];
        let close = tail.find('"').expect("a closed app name");
        tail[..close].to_owned()
    }

    #[test]
    fn every_command_the_front_end_invokes_is_registered() {
        let (registered, invoked) = (registered(), invoked());
        assert!(
            invoked.contains("list_peers"),
            "the walk found {invoked:?}, which cannot be every command the shell calls"
        );
        let missing: Vec<&String> = invoked.difference(&registered).collect();
        assert!(
            missing.is_empty(),
            "the front end invokes {missing:?}, which this build never hands to Tauri, so the \
             call fails at runtime with a command-not-found the compiler cannot see"
        );
    }

    #[test]
    fn every_registered_command_is_invoked() {
        let (registered, invoked) = (registered(), invoked());
        assert!(
            registered.contains("list_peers"),
            "the handler list read as {registered:?}, which cannot be this build's commands"
        );
        let dead: Vec<&String> = registered.difference(&invoked).collect();
        assert!(
            dead.is_empty(),
            "Tauri is told about {dead:?} and nothing in the front end ever calls them"
        );
    }

    #[test]
    fn both_halves_call_the_app_by_the_same_name() {
        let rust = declared_name(&root().join("crates/alver-core/src/lib.rs"));
        let front = declared_name(&root().join("packages/bridge/src/index.ts"));
        assert_eq!(
            rust, front,
            "the windows are titled {rust:?} and the page and the About box say {front:?}"
        );
    }
}

/// The shapes that cross the command seam are written twice — a Rust struct the bridge
/// serialises, a TypeScript type the shell reads — and until this scan existed the only thing
/// holding them together was the comment that says they mirror each other. A field renamed on
/// one pane is `undefined` on the other, and no compiler on either side can see it.
///
/// Both panes are read from source, so the comparison is against what this build writes and
/// what this build reads, not against a copy of either typed into a test.
#[cfg(test)]
mod bridge_surface_tests {
    use super::seam_scan::{closed, root, sources, strip_comments};
    use std::collections::BTreeSet;
    use std::fs;
    use std::path::PathBuf;

    /// Every member the front-end contract declares. One named interface is the whole seam, so
    /// a member this scan cannot read as a method fails with its text instead of being skipped.
    fn declared() -> BTreeSet<String> {
        let path = root().join("packages/bridge/src/index.ts");
        let text = strip_comments(
            &fs::read_to_string(&path)
                .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
        );
        let mark = "export interface PlatformBridge";
        let at = text
            .find(mark)
            .expect("the front-end contract is one named interface");
        let open = at + text[at..].find('{').expect("the contract never opens");
        let end = closed(text.as_bytes(), open);
        let mut names = BTreeSet::new();
        let mut depth = 0usize;
        let mut start = open + 1;
        let mut index = start;
        while index < end {
            match text.as_bytes()[index] {
                b'{' | b'(' => depth += 1,
                b'}' | b')' => depth = depth.saturating_sub(1),
                b';' if depth == 0 => {
                    names.insert(member(&text[start..index]));
                    start = index + 1;
                }
                _ => {}
            }
            index += 1;
        }
        names.insert(member(&text[start..end]));
        names.remove("");
        names
    }

    /// One declaration inside the interface: `name(args): Return`, optionally `name?:`.
    fn member(entry: &str) -> String {
        let entry = entry.trim();
        if entry.is_empty() {
            return String::new();
        }
        let bytes = entry.as_bytes();
        let mut index = 0;
        while index < bytes.len() && (bytes[index].is_ascii_alphanumeric() || bytes[index] == b'_')
        {
            index += 1;
        }
        let name = &entry[..index];
        let rest = entry[index..].trim_start();
        let rest = rest.strip_prefix('?').unwrap_or(rest).trim_start();
        assert!(
            rest.starts_with('('),
            "{name:?} is a member of the bridge contract that is not a method: {entry}"
        );
        assert!(
            name.bytes()
                .next()
                .is_some_and(|first| first.is_ascii_lowercase()),
            "{name:?} is a bridge method that does not start lowercase"
        );
        name.to_owned()
    }

    /// Every `.name(` this file contains. The receiver is deliberately not checked: `bridge.x()`,
    /// `props.bridge.x()` and `next.x()` (where `next` is what `bridgeForRuntime()` resolved to)
    /// are all real call sites, and a list of allowed receiver words would be a guess about how
    /// screens are written that the next refactor outlives. The cost is the blind spot this file
    /// names elsewhere: an unrelated object with the same distinctive method name can mask a
    /// bridge method nobody calls, so this direction proves a name is reachable, not that the
    /// object it was reached through is the bridge.
    fn calls_in(text: &str) -> BTreeSet<String> {
        let bytes = text.as_bytes();
        let mut out = BTreeSet::new();
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'.' {
                let mut cursor = index + 1;
                if bytes.get(cursor) == Some(&b'?') {
                    cursor += 1;
                }
                let start = cursor;
                while cursor < bytes.len()
                    && (bytes[cursor].is_ascii_alphanumeric() || bytes[cursor] == b'_')
                {
                    cursor += 1;
                }
                let mut open = cursor;
                while open < bytes.len() && (bytes[open] as char).is_whitespace() {
                    open += 1;
                }
                if start < cursor && bytes.get(open) == Some(&b'(') {
                    out.insert(text[start..cursor].to_owned());
                }
                index = cursor.max(index + 1);
            } else {
                index += 1;
            }
        }
        out
    }

    fn screen_sources() -> Vec<PathBuf> {
        let root = root();
        let mut files = Vec::new();
        for extension in ["ts", "tsx"] {
            sources(&root.join("packages/ui/src"), extension, &mut files);
        }
        files
    }

    #[test]
    fn every_declared_bridge_method_is_called_by_a_screen() {
        let declared = declared();
        assert!(
            declared.len() >= 40,
            "the contract scan saw {} methods, which is not this app's bridge",
            declared.len()
        );
        let files = screen_sources();
        assert!(!files.is_empty(), "no screen sources were found to scan");
        let mut called = BTreeSet::new();
        let mut sites = 0usize;
        for path in &files {
            let text = strip_comments(
                &fs::read_to_string(path)
                    .unwrap_or_else(|error| panic!("{}: {error}", path.display())),
            );
            let found = calls_in(&text);
            sites += declared
                .iter()
                .filter(|name| found.contains(name.as_str()))
                .count();
            called.extend(found);
        }
        let orphans: Vec<&String> = declared
            .iter()
            .filter(|name| !called.contains(name.as_str()))
            .collect();
        assert!(
            sites > declared.len(),
            "the call scan matched {sites} contract names across {} files, which is fewer than \
             the {} methods declared — a scan that reads nothing reports nothing missing",
            files.len(),
            declared.len()
        );
        assert!(
            orphans.is_empty(),
            "{} bridge method(s) are declared in the contract and called by no screen: {orphans:?}",
            orphans.len()
        );
    }

    #[test]
    fn the_call_scan_finds_a_name_that_is_not_called() {
        // The scan is a set of property accesses, so the negative half has to be proven on a
        // string rather than on the tree: an unrelated word must not be mistaken for a call,
        // and a real one must not be missed because of an optional chain or a space.
        let found = calls_in(
            "bridge.pinMessage(id)\nfoo . bar()\na?.listPeers( )\nsendMessage(x)\nconst y = bridge.listPeers\n",
        );
        assert!(found.contains("pinMessage"));
        assert!(found.contains("listPeers"));
        assert!(!found.contains("bar"));
        assert!(
            !found.contains("sendMessage"),
            "a bare name with no dot in front of it is not a call site"
        );
    }
}

/// The fourth inert-setting hunt (items 14, 32 and 43 each found settings that were stored,
/// shown and serialised but decided nothing) became a count rather than a hunt: every field of
/// `AppSettings` is asked who reads it, on either pane, in the same shape item 44 used for the
/// store's key table. What this proves is that a field reaches code. What it cannot prove — and
/// what the manual pass over all forty fields in item 48 is for — is that the reading code acts
/// on the value instead of painting it back into a control.
#[cfg(test)]
mod settings_surface_tests {
    use super::seam_scan::{camel, closed, root, sources, strip_comments, top_level};
    use std::fs;
    use std::path::PathBuf;

    // Spliced, because this module reads the file that declares the struct and `AppSettings`
    // named here would be a field the scan could not see.
    const DECLARED: &str = concat!("pub str", "uct AppSettings {");

    /// The fields in the order the struct declares them, in the snake_case they are stored in.
    /// Anything inside the braces that is not `pub name: Type` fails rather than being skipped,
    /// so a reworked struct cannot quietly shrink the audit to whatever parses easily.
    fn declared() -> Vec<String> {
        let path = root().join("apps/desktop/src-tauri/src/platform.rs");
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let at = text
            .find(DECLARED)
            .unwrap_or_else(|| panic!("{}: no AppSettings to audit", path.display()));
        let open = at + DECLARED.len() - 1;
        let close = closed(text.as_bytes(), open);
        let mut out = Vec::new();
        for part in top_level(&strip_comments(&text[open + 1..close])) {
            let member = part.trim();
            if member.is_empty() {
                continue;
            }
            let rest = member.strip_prefix("pub ").unwrap_or_else(|| {
                panic!(
                    "{}: AppSettings member {member:?} is not a `pub name: Type` field",
                    path.display()
                )
            });
            let colon = rest.find(':').unwrap_or_else(|| {
                panic!(
                    "{}: AppSettings member {member:?} declares no type",
                    path.display()
                )
            });
            let name = rest[..colon].trim();
            assert!(
                !name.is_empty()
                    && name
                        .bytes()
                        .all(|byte| byte.is_ascii_lowercase() || byte == b'_'),
                "{}: `{name:?}` is not a settings field name this scan can spell",
                path.display()
            );
            out.push(name.to_owned());
        }
        out
    }

    /// How many times this text reads a name off something. A read is the name with a dot in
    /// front and not an `=` behind it, so `if settings.snap_to_edge` and `settings.theme == …`
    /// count while `guard.lan_encryption = …` does not, and neither does the declaration or the
    /// `Self { … }` literal that seeds it. A name already inside a longer one does not match, so
    /// `speakerOptions` is not a read of `speaker`.
    fn reads_in(text: &str, name: &str) -> usize {
        let bytes = text.as_bytes();
        let name_at = |at: usize| {
            bytes.get(at.wrapping_sub(1)) == Some(&b'.')
                && !matches!(bytes.get(at.wrapping_sub(2)), Some(b'.'))
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        };
        let mut found = 0;
        let mut at = 0;
        while let Some(offset) = text[at..].find(name) {
            let start = at + offset;
            let after = start + name.len();
            at = after;
            if !name_at(start) {
                continue;
            }
            if bytes
                .get(after)
                .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
            {
                continue;
            }
            let tail = text[after..].trim_start();
            if tail.starts_with('=') && !tail.starts_with("==") {
                continue;
            }
            found += 1;
        }
        found
    }

    /// Every source that could act on a preference: the shell, and the screens running inside
    /// its webviews. `sources` already skips `*.test.*`.
    fn pane(dirs: &[PathBuf], extensions: &[&str]) -> Vec<String> {
        let mut files = Vec::new();
        for dir in dirs {
            for extension in extensions {
                sources(dir, extension, &mut files);
            }
        }
        assert!(!files.is_empty(), "no sources to read");
        files
            .iter()
            .map(|file| {
                let text = fs::read_to_string(file)
                    .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
                strip_comments(&text)
            })
            .collect()
    }

    #[test]
    fn every_declared_setting_is_read_by_code_on_one_of_the_two_panes() {
        let root = root();
        let fields = declared();
        assert!(
            fields.len() >= 30,
            "the AppSettings scan found {} fields, which cannot be all of them",
            fields.len()
        );
        let shell = pane(
            &[
                root.join("apps/desktop/src-tauri/src"),
                root.join("crates/alver-core/src"),
                root.join("crates/alver-platform/src"),
            ],
            &["rs"],
        );
        let screens = pane(&[root.join("packages/ui/src")], &["ts", "tsx"]);
        let mut sites = 0;
        let mut unread = Vec::new();
        for field in &fields {
            let wire = camel(field);
            let in_shell: usize = shell.iter().map(|text| reads_in(text, field)).sum();
            let in_screens: usize = screens.iter().map(|text| reads_in(text, &wire)).sum();
            sites += in_shell + in_screens;
            if in_shell + in_screens == 0 {
                unread.push(field);
            }
        }
        // The same floor the other seam scans need: a walk that read no files would report no
        // unread fields, so the number of reads has to be larger than the number of fields.
        assert!(
            sites > fields.len(),
            "the reader scan matched {sites} sites across {} files, which is fewer than the {} \
             settings fields — a scan that reads nothing reports nothing missing",
            shell.len() + screens.len(),
            fields.len()
        );
        assert!(
            unread.is_empty(),
            "{} setting(s) are stored in settings.json and serialised to the shell, and no code \
             reads them: {unread:?}",
            unread.len()
        );
    }

    #[test]
    fn a_read_needs_a_dot_and_not_an_assignment() {
        // Proven on strings with invented names, so this module cannot be the reader that makes
        // a real field look used. The tree contains every shape the rule has to tell apart, but
        // a broken rule still finds *something* there.
        assert_eq!(
            reads_in("if settings.probe_days.is_empty()", "probe_days"),
            1
        );
        assert_eq!(
            reads_in("let x = a.probe_days == b.probe_days;", "probe_days"),
            2
        );
        assert_eq!(
            reads_in("guard.probe_flag = !passphrase.is_empty();", "probe_flag"),
            0
        );
        assert_eq!(reads_in("pub probe_days: Vec<String>,", "probe_days"), 0);
        assert_eq!(
            reads_in(
                "const probeOptionSet = withCurrent(settings.probe_one);",
                "probe_one"
            ),
            1,
            "a longer name that starts with the field is not a read of it"
        );
        assert_eq!(reads_in("let y = settings.probe_two;", "probe_2"), 0);
    }

    /// The backup passphrase limit is one rule with three homes: the gate in this crate, the
    /// number the Settings placeholder hands the sentence, and the English in the catalog. The
    /// front end cannot ask the platform for it, so the only defence against the three drifting
    /// apart is to compare the copies against the value this binary actually enforces.
    #[test]
    fn the_backup_passphrase_the_settings_promise_is_the_one_this_binary_enforces() {
        let root = root();
        let min = super::platform::BACKUP_PASSPHRASE_MIN_LEN;
        let platform = fs::read_to_string(root.join("apps/desktop/src-tauri/src/platform.rs"))
            .unwrap_or_else(|error| {
                panic!(
                    "{}: {error}",
                    root.join("apps/desktop/src-tauri/src/platform.rs")
                        .display()
                )
            });
        assert!(
            platform.contains("if passphrase.len() < BACKUP_PASSPHRASE_MIN_LEN"),
            "create_backup no longer gates on the constant, so the number this test reads is not \
             the number that refuses a short passphrase"
        );
        let screen = fs::read_to_string(root.join("packages/ui/src/screens/SettingsScreen.tsx"))
            .unwrap_or_else(|error| {
                panic!(
                    "{}: {error}",
                    root.join("packages/ui/src/screens/SettingsScreen.tsx")
                        .display()
                )
            });
        let declared = format!("const BACKUP_PASSPHRASE_MIN_LEN = {min};");
        assert!(
            screen.contains(&declared),
            "SettingsScreen.tsx does not declare the limit this binary enforces; it should \
             contain `{declared}`"
        );
        let asked = screen
            .find("text('backupPassphrasePlaceholder'")
            .unwrap_or_else(|| {
                panic!("the Settings pane no longer asks for backupPassphrasePlaceholder")
            });
        let call: String = screen[asked..].chars().take(240).collect();
        assert!(
            call.contains("min: BACKUP_PASSPHRASE_MIN_LEN"),
            "the placeholder sentence is handed a value that is not the limit this binary owns: {call}"
        );
    }

    // Spliced: this module reads the desktop sources, so a name written plainly here would be
    // found by its own scan.
    const DEAD_HELPERS: [(&str, &str); 2] = [
        (
            concat!("edit_message_", "in_window"),
            "nothing in this product calls",
        ),
        (concat!("transfer_", "policy"), "no product caller"),
    ];
    const EDIT_WINDOW_ROW: &str = concat!("message.edit_", "window_ms");

    /// Every desktop source, comments gone — test modules included, which is deliberate: the
    /// crate's own `#[cfg(test)]` markers sit on real items near the top of four files, so no
    /// cheap cut separates product from test here, and a helper reached only from a test is still
    /// a helper the document has to describe.
    fn desktop_code() -> String {
        let mut files = Vec::new();
        sources(&root().join("apps"), "rs", &mut files);
        assert!(
            files.len() > 5,
            "only {} desktop sources to scan",
            files.len()
        );
        let mut out = String::new();
        for file in files {
            let text = fs::read_to_string(&file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            out.push_str(&strip_comments(&text));
        }
        out
    }

    /// The protocol document now states a call-graph fact rather than a behaviour: two of the
    /// core store's helpers read a decision out of its settings rows, and the desktop reads the
    /// same two decisions out of `settings.json`, so the store-row halves are unreachable and the
    /// `message.edit_window_ms` row has no writer outside a test. A claim about what calls what
    /// rots in both directions — wiring the helper makes the sentence false, and deleting the
    /// sentence puts the dead half back where it was found, undocumented — so both halves are
    /// pinned here against the tree and the file.
    #[test]
    fn the_documented_dead_store_readings_stay_dead_and_stay_documented() {
        let protocol = fs::read_to_string(root().join("docs/PROTOCOL.md"))
            .unwrap_or_else(|error| panic!("docs/PROTOCOL.md: {error}"));
        let desktop = desktop_code();
        for (name, claim) in DEAD_HELPERS {
            assert!(
                !desktop.contains(name),
                "`{name}` is now named in desktop code, so docs/PROTOCOL.md is wrong to say \
                 {claim:?}. If it reached product code, the document has to describe the live \
                 path; if only a test reached it, reword that sentence and drop `{name}` from \
                 the DEAD_HELPERS table in this module — the claim may not be left standing \
                 while it is false."
            );
            assert!(
                protocol.contains(claim),
                "nothing in desktop code reaches `{name}`, yet docs/PROTOCOL.md no longer \
                 records that: the sentence saying {claim:?} has gone, so the dead half of this \
                 rule is undocumented again"
            );
        }
        assert!(
            !desktop.contains(EDIT_WINDOW_ROW) && !desktop.contains(concat!("EDIT_WINDOW_", "MS")),
            "desktop code now names the store's edit-window row, which is the row the document \
             says nothing writes — either it is a writer the document has to describe, or this \
             guard is being satisfied by an unrelated string"
        );
    }

    /// The source of one function, from its signature to the first brace that closes a line.
    /// Deliberately narrow: the assertions below are about a body, and a rule restated in the
    /// neighbouring comments or in this module's own strings must not be what satisfies them.
    fn one_fn(source: &str, marker: &str) -> String {
        let start = source
            .find(marker)
            .unwrap_or_else(|| panic!("{marker} is gone from this file"));
        let rest = &source[start..];
        let end = rest
            .find("\n}")
            .unwrap_or_else(|| panic!("no closing line for {marker}"));
        rest[..end].to_owned()
    }

    /// The Settings badge is a claim about the transport: `has_lan_passphrase` lights
    /// "Encrypted", and `lan_passphrase` hands the frame HMAC its key. Until item 55 both
    /// decided on their own whether a stored secret counts, in two files that no test joined —
    /// so the label could be lit over a session whose key is an empty file. Both now read
    /// through `platform::stored_passphrase`, and this is what keeps the join: a caller that
    /// goes back to deciding by itself fails here, and the unit test over a throwaway profile
    /// (`platform::tests::the_badge_answers_the_same_question_the_transport_asks`) is what
    /// decides the answer.
    #[test]
    fn the_badge_and_the_challenge_key_read_the_stored_passphrase_through_one_rule() {
        let shell = fs::read_to_string(root().join("apps/desktop/src-tauri/src/main.rs"))
            .unwrap_or_else(|error| panic!("main.rs: {error}"));
        let platform = fs::read_to_string(root().join("apps/desktop/src-tauri/src/platform.rs"))
            .unwrap_or_else(|error| panic!("platform.rs: {error}"));
        for (who, body) in [
            (
                concat!("lan_", "passphrase"),
                one_fn(&shell, concat!("pub(crate) fn lan_", "passphrase")),
            ),
            (
                concat!("has_", "lan_passphrase"),
                one_fn(&platform, concat!("pub fn has_", "lan_passphrase")),
            ),
        ] {
            assert!(
                body.contains("stored_passphrase"),
                "{who} no longer reads the secret store through the shared rule, so the badge \
                 and the challenge key are back to two answers about one stored passphrase"
            );
            assert!(
                !body.contains(concat!("is_", "empty")),
                "{who} decides by itself whether a stored passphrase counts. That question is \
                 `sealing_passphrase`'s; answering it a second time here is how the label and \
                 the HMAC start disagreeing"
            );
        }
    }
}

/// Two files decide whether "Settings → About installs the update" is a feature or a wish: the
/// bundle target list that has to produce a signed artifact per platform, and the capability list
/// that lets the shell restart. Neither is compiled, so until this scan existed both could be
/// broken with every local gate still green, and the breakage would only show on a runner.
#[cfg(test)]
mod release_config_tests {
    use super::seam_scan::root;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};

    fn json(relative: &str) -> Value {
        let path = root().join(relative);
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// The strings of a JSON array found by walking keys, and nothing when the key is absent or
    /// not an array — which the assertions below report as a missing entry, not a parse failure.
    fn strings(value: &Value, path: &[&str]) -> Vec<String> {
        let mut node = value;
        for key in path {
            node = &node[key];
        }
        match node.as_array() {
            Some(items) => items
                .iter()
                .filter_map(Value::as_str)
                .map(str::to_owned)
                .collect(),
            None => Vec::new(),
        }
    }

    #[test]
    fn every_platform_the_updater_can_reach_is_actually_bundled() {
        let config = json("apps/desktop/src-tauri/tauri.conf.json");
        assert_eq!(
            config["bundle"]["createUpdaterArtifacts"],
            Value::Bool(true),
            "without this the release writes no signature, and the updater refuses every bundle"
        );
        let targets = strings(&config, &["bundle", "targets"]);
        // Windows updates through the NSIS exe whose `.sig` ships beside it. macOS can only be
        // updated from an `.app.tar.gz`, which the bundler writes for the `app` target and not for
        // `dmg`: the published v0.1.2 manifest carried `windows-x86_64` and nothing for Apple
        // Silicon, so every macOS install reported "up to date" whatever the runner published.
        for required in ["nsis", "app"] {
            assert!(
                targets.iter().any(|target| target == required),
                "bundle.targets is {targets:?}, which cannot produce the {required} updater artifact"
            );
        }
        let endpoints = strings(&config, &["plugins", "updater", "endpoints"]);
        assert_eq!(
            endpoints.len(),
            1,
            "the updater should read one manifest, not fall back across several: {endpoints:?}"
        );
        assert!(
            endpoints[0].ends_with("/releases/latest/download/latest.json"),
            "the updater reads {endpoints:?}, which is not the manifest a release writes"
        );
        assert!(
            config["plugins"]["updater"]["pubkey"]
                .as_str()
                .is_some_and(|key| !key.is_empty()),
            "no public key is bundled, so nothing could be verified even if a target matched"
        );
    }

    #[test]
    fn the_permissions_the_install_flow_needs_are_granted_in_every_window() {
        let config = json("apps/desktop/src-tauri/capabilities/default.json");
        let permissions = strings(&config, &["permissions"]);
        // `updater:default` is what lets a webview call `check` and `download_and_install`; the
        // restart after them belongs to another plugin, and a missing entry here is a button that
        // throws rather than an app that updates.
        for required in ["updater:default", "process:allow-restart"] {
            assert!(
                permissions.iter().any(|granted| granted == required),
                "{required} is not granted, so Settings → About cannot finish an install: {permissions:?}"
            );
        }
        // The four labels this crate creates: `main` in this config's own `app.windows`, and the
        // three overlays in `build_overlay`. The install button can be pressed in any of them.
        let windows = strings(&config, &["windows"]);
        for label in ["main", "dock", "compact", "menu"] {
            assert!(
                windows.iter().any(|window| window == label),
                "the capability list omits {label}, so that window cannot reach the updater"
            );
        }
    }

    fn text(relative: &str) -> String {
        let path = root().join(relative);
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// The `key = "value"` under one `[section]` header, without a TOML dependency. The section is
    /// tracked line by line because a `Cargo.toml` is full of other `version = ` lines: dependency
    /// tables would answer for the workspace table if the scan did not know which one it was in.
    fn toml_value(path: &str, section: &str, key: &str) -> String {
        let mut inside = false;
        for line in text(path).lines() {
            let line = line.trim();
            if line.starts_with('[') {
                inside = line == format!("[{section}]");
                continue;
            }
            if inside {
                if let Some(rest) = line.strip_prefix(&format!("{key} = ")) {
                    return rest.trim_matches('"').to_owned();
                }
            }
        }
        panic!("{path} declares no {key} under [{section}]")
    }

    /// A `v` followed by three dotted numbers — the shape of a build number typed into a label
    /// instead of read from a manifest. `192.168.1.0` is a network and `0.55` is a progress bar, so
    /// the prefix is what makes this a claim about the running app rather than a number.
    fn typed_version(haystack: &str) -> Option<String> {
        let bytes = haystack.as_bytes();
        for index in 0..bytes.len() {
            if bytes[index] != b'v' || !bytes.get(index + 1).is_some_and(|c| c.is_ascii_digit()) {
                continue;
            }
            let mut cursor = index + 1;
            let mut parts = 0;
            loop {
                while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                    cursor += 1;
                }
                parts += 1;
                let dotted = bytes.get(cursor) == Some(&b'.')
                    && bytes.get(cursor + 1).is_some_and(|c| c.is_ascii_digit());
                if !dotted {
                    break;
                }
                cursor += 1;
            }
            if parts >= 3 {
                return Some(haystack[index..cursor].to_owned());
            }
        }
        None
    }

    #[test]
    fn every_layer_that_declares_a_version_declares_the_same_one() {
        let cargo = toml_value("Cargo.toml", "workspace.package", "version");
        let npm = json("package.json")["version"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        let desktop = json("apps/desktop/src-tauri/tauri.conf.json")["version"]
            .as_str()
            .unwrap_or("")
            .to_owned();
        // Three files, and they are not synonyms: `tauri.conf.json` is the version the bundler puts
        // in the artifact name and the updater compares a manifest against, while `package.json`
        // names nothing in the release path at all. A bump that misses the first publishes a build
        // that can never be told it is newer than itself.
        if cargo != desktop || npm != desktop {
            panic!(
                "the app version is typed in three files that have to agree and do not: Cargo.toml [workspace.package] = {cargo}, package.json = {npm}, apps/desktop/src-tauri/tauri.conf.json = {desktop}"
            );
        }
    }

    #[test]
    fn the_version_the_shell_prints_is_read_from_the_manifest_the_updater_obeys() {
        let settings = text("packages/ui/src/screens/SettingsScreen.tsx");
        let declaration = settings
            .lines()
            .find(|line| line.trim_start().starts_with("export const APP_VERSION"))
            .expect(
                "SettingsScreen no longer declares APP_VERSION, so find what the About page prints",
            );
        let declaration = declaration.trim();
        assert!(
            !declaration.chars().any(|c| c.is_ascii_digit()),
            "APP_VERSION types the number again ({declaration}): the About page must follow the config the updater obeys"
        );
        assert!(
            settings.contains("apps/desktop/src-tauri/tauri.conf.json"),
            "APP_VERSION is derived, but not from the desktop config whose version decides whether an update exists"
        );
        // The gallery is where a second copy can hide: it renders a badge, and a badge nobody
        // guards keeps flattering an old build after the release path has moved on.
        let gallery = text("packages/ui/src/screens/Gallery.tsx");
        assert!(
            gallery.contains("APP_VERSION"),
            "the gallery badge stopped showing the app version at all"
        );
        for file in [
            "packages/ui/src/screens/Gallery.tsx",
            "packages/ui/src/screens/SettingsScreen.tsx",
        ] {
            if let Some(typed) = typed_version(&text(file)) {
                panic!("{file} types a version ({typed}) where it should read APP_VERSION");
            }
        }
    }

    /// Every `.ts`/`.tsx` under one directory, test files included — the CI `ui` job greps this pane
    /// over the whole tree, and its fixtures are the only reason the step has an allowlist.
    fn pane_files(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in dir.read_dir().expect("packages/ui/src exists") {
            let path = entry.expect("readable directory entry").path();
            if path.is_dir() {
                pane_files(&path, into);
            } else if matches!(
                path.extension().and_then(|tail| tail.to_str()),
                Some("ts") | Some("tsx")
            ) {
                into.push(path);
            }
        }
    }

    /// The host of every `scheme://` in a line, cut at the first thing that cannot be a host.
    fn hosts(line: &str) -> Vec<String> {
        let bytes = line.as_bytes();
        let mut found = Vec::new();
        let mut at = 0usize;
        while let Some(offset) = line[at..].find("://") {
            let start = at + offset + 3;
            let mut end = start;
            while end < bytes.len()
                && !matches!(
                    bytes[end],
                    b'/' | b'\\' | b'"' | b'\'' | b'`' | b' ' | b'\t' | b')' | b'>' | b';' | b','
                )
            {
                end += 1;
            }
            found.push(String::from_utf8_lossy(&bytes[start..end]).into_owned());
            at = end;
        }
        found
    }

    /// The Rust target triples a release matrix builds, in the order the file lists them.
    fn matrix_triples(workflow: &str) -> Vec<String> {
        workflow
            .lines()
            .map(str::trim)
            .filter_map(|line| line.strip_prefix("target: "))
            .filter(|value| value.contains('-') && !value.contains("${{"))
            .map(str::to_owned)
            .collect()
    }

    #[test]
    fn the_release_workflow_builds_what_the_bundle_config_declares() {
        let workflow = text(".github/workflows/release.yml");
        let config = json("apps/desktop/src-tauri/tauri.conf.json");
        let targets = strings(&config, &["bundle", "targets"]);
        let triples = matrix_triples(&workflow);
        assert!(
            triples.len() >= 2,
            "the release matrix declares {triples:?}, which is not the two platforms this project ships"
        );
        // Each triple is only useful if the bundle section produces an artifact for it: the v0.1.2
        // release built Apple Silicon and bundled only the `dmg`, so the runner succeeded, the
        // manifest published, and no macOS client could ever be offered the update.
        for triple in &triples {
            let (family, required) = if triple.contains("windows") {
                ("windows", "nsis")
            } else if triple.contains("darwin") {
                ("darwin", "app")
            } else if triple.contains("linux") {
                panic!("the release matrix builds {triple}, and no bundle target or platform layer covers Linux");
            } else {
                panic!("unrecognised target triple {triple} in the release matrix");
            };
            let expected = if family == "windows" {
                "x86_64"
            } else {
                "aarch64"
            };
            assert!(
                triple.starts_with(&format!("{expected}-")),
                "{triple} is not the architecture this project releases: {family} builds {expected}, and the artifact names carry it"
            );
            assert!(
                targets.iter().any(|bundled| bundled == required),
                "the matrix builds {triple} but bundle.targets is {targets:?}, which yields no {required} artifact to sign"
            );
        }
        // A matrix entry nothing reads is worse than no matrix: the build would quietly produce the
        // runner's own architecture and name it after the one it did not build.
        assert!(
            workflow.contains("--target ${{ matrix.target }}"),
            "the matrix target is never passed to the builder, so the artifacts are the runner's own"
        );
        assert!(
            workflow.contains("runs-on: ${{ matrix.os }}"),
            "the matrix os is never used, so both builds land on one operating system"
        );
        // The two steps that decide whether a release is installable at all.
        assert!(
            workflow.contains("test -n \"$TAURI_SIGNING_PRIVATE_KEY\""),
            "nothing checks the signing secret, so a missing one publishes bundles every client refuses"
        );
        let configured = workflow.find("node scripts/configure-updater.mjs");
        let built = workflow.find("tauri-apps/tauri-action");
        match (configured, built) {
            (Some(before), Some(after)) if before < after => {}
            _ => panic!(
                "the endpoint must be rewritten before the build: configure-updater at {configured:?}, tauri-action at {built:?}"
            ),
        }
        assert!(
            workflow.contains("contents: write"),
            "without contents: write the release cannot be created, and the failure is the last step of a long build"
        );
    }

    #[test]
    fn no_ui_source_names_a_host_the_ci_allowlist_does_not_cover() {
        // The same two exceptions `.github/workflows/ci.yml` grants, spelled the same way: an SVG
        // namespace is not a fetch, and a reserved `.example` host (RFC 6761) cannot resolve.
        let allowed = |host: &str, line: &str| {
            host == "example"
                || host.ends_with(".example")
                || (host == "www.w3.org" && line.contains("www.w3.org/2000/svg"))
        };
        let mut files = Vec::new();
        pane_files(&root().join("packages/ui/src"), &mut files);
        assert!(
            files.len() > 60,
            "the pane walk returned {} files, which is not a scan of packages/ui/src",
            files.len()
        );
        let mut offenders = Vec::new();
        for file in &files {
            let contents = fs::read_to_string(file).unwrap_or_else(|error| {
                panic!("{}: {error}", file.display());
            });
            for (number, line) in contents.lines().enumerate() {
                let named = hosts(line);
                if named.is_empty() {
                    continue;
                }
                // A line is clean only when every host on it is allowed, so `https://evil.example`
                // beside a real host still reports the real one.
                for host in named {
                    if !allowed(&host, line) {
                        offenders.push(format!("{}:{} -> {host}", file.display(), number + 1));
                    }
                }
            }
        }
        assert!(
            offenders.is_empty(),
            "{} UI source line(s) name a host the CI `ui` job refuses: {offenders:?}",
            offenders.len()
        );
    }

    /// The `version:` pin of every `pnpm/action-setup` step, in the order the file lists them.
    fn action_setup_pins(file: &str, workflow: &str) -> Vec<String> {
        let lines: Vec<String> = workflow
            .lines()
            .map(|line| {
                let line = line.trim();
                line.strip_prefix("- ").unwrap_or(line).trim().to_owned()
            })
            .collect();
        let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
        let mut pins = Vec::new();
        for (index, line) in lines.iter().enumerate() {
            if !line.starts_with("uses: pnpm/action-setup@") {
                continue;
            }
            let found = lines[index + 1..]
                .iter()
                .take(3)
                .find_map(|next| next.strip_prefix("version:"));
            match found {
                Some(version) => pins.push(version.trim().to_owned()),
                None => panic!("{file} has a pnpm/action-setup step with no version pin"),
            }
        }
        pins
    }

    #[test]
    fn the_workflows_install_the_package_manager_the_lockfile_was_written_with() {
        let declared = json("package.json")["packageManager"]
            .as_str()
            .unwrap_or_default()
            .trim()
            .to_owned();
        let Some((tool, version)) = declared.split_once('@') else {
            panic!(
                "package.json declares packageManager {declared:?}, which is not `tool@version`"
            );
        };
        assert_eq!(
            tool, "pnpm",
            "package.json names {tool}, but every workflow step installs pnpm"
        );
        for file in [".github/workflows/ci.yml", ".github/workflows/release.yml"] {
            let workflow = text(file);
            let pins = action_setup_pins(file, &workflow);
            assert!(
                !pins.is_empty(),
                "{file} installs no package manager, so the runner picks one for itself"
            );
            // `version:` in the action overrides `packageManager`, and a different major resolves
            // this lockfile differently — the job then fails at `--frozen-lockfile`, or installs a
            // tree the local gates never saw.
            assert!(
                pins.iter().all(|pin| pin == version),
                "{file} installs pnpm {pins:?} while package.json's packageManager is {declared}"
            );
        }
    }
}

#[cfg(test)]
mod declared_surface_tests {
    use super::seam_scan::root;
    use serde_json::Value;
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Runtime dependencies that every clone installs and no source file imports. Named rather than
    /// deleted because dropping a package changes what the lockfile resolves for everyone, which is
    /// a maintainer's call and not a test's; the list is what stops it growing a third entry, and
    /// `no_dead_weight_entry_is_stale` is what stops it surviving the day one of them is used.
    const DECLARED_BUT_NOT_IMPORTED: [&str; 2] = ["framer-motion", "zustand"];

    fn manifest() -> Value {
        let path = root().join("package.json");
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        serde_json::from_str(&text).unwrap_or_else(|error| panic!("{}: {error}", path.display()))
    }

    /// Every `.ts`, `.tsx` and `.css` file the workspace ships or runs, tests included: a package
    /// used only by a test is still a package something real imports, and `docs/**` is excluded on
    /// purpose because prose that names a library is not code that loads it.
    fn walk(dir: &Path, into: &mut Vec<PathBuf>) {
        for entry in dir.read_dir().expect("workspace source directory") {
            let path = entry.expect("readable directory entry").path();
            let name = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();
            if ["node_modules", "target", "gen", "dist"].contains(&name.as_str()) {
                continue;
            }
            let extension = path.extension().and_then(|tail| tail.to_str());
            if path.is_dir() {
                walk(&path, into);
            } else if matches!(extension, Some("ts") | Some("tsx") | Some("css")) {
                into.push(path);
            }
        }
    }

    fn imported_by() -> String {
        let mut files = Vec::new();
        for dir in ["packages", "scripts"] {
            walk(&root().join(dir), &mut files);
        }
        for file in ["vite.config.ts", "playwright.config.ts", "eslint.config.js"] {
            files.push(root().join(file));
        }
        assert!(
            files.len() > 60,
            "the dependency walk found {} files, which is not the workspace",
            files.len()
        );
        files
            .iter()
            .map(|file| {
                fs::read_to_string(file)
                    .unwrap_or_else(|error| panic!("{}: {error}", file.display()))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn every_runtime_dependency_is_imported_somewhere_or_named_as_dead_weight() {
        let declared: Vec<String> = manifest()["dependencies"]
            .as_object()
            .expect("package.json declares a dependencies object")
            .keys()
            .cloned()
            .collect();
        assert!(
            declared.len() > 10,
            "only {} runtime dependencies are declared, which is not this manifest",
            declared.len()
        );
        let sources = imported_by();
        let unused: Vec<&str> = declared
            .iter()
            .map(String::as_str)
            .filter(|name| {
                !sources.contains(&format!("\"{name}")) && !sources.contains(&format!("'{name}"))
            })
            .collect();
        let expected: Vec<&str> = DECLARED_BUT_NOT_IMPORTED.to_vec();
        assert_eq!(
            unused, expected,
            "the set of installed-but-never-imported runtime dependencies changed: a new one is \
             weight every clone carries, and one of {expected:?} is now imported and belongs in \
             `dependencies` for real"
        );
    }

    #[test]
    fn no_dead_weight_entry_is_stale() {
        let declared: Vec<String> = manifest()["dependencies"]
            .as_object()
            .expect("package.json declares a dependencies object")
            .keys()
            .cloned()
            .collect();
        let sources = imported_by();
        for name in DECLARED_BUT_NOT_IMPORTED {
            assert!(
                declared.iter().any(|entry| entry == name),
                "{name} is listed as dead weight but the manifest no longer declares it"
            );
            assert!(
                !sources.contains(&format!("\"{name}")) && !sources.contains(&format!("'{name}")),
                "{name} is now imported, so the dead-weight list is what has gone stale"
            );
        }
    }

    #[test]
    fn every_path_a_script_runs_exists() {
        let scripts = manifest()["scripts"]
            .as_object()
            .expect("package.json declares a scripts object")
            .clone();
        assert!(
            scripts.len() > 8,
            "only {} scripts are declared, which is not this manifest",
            scripts.len()
        );
        let mut missing = Vec::new();
        for (name, command) in &scripts {
            let command = command.as_str().unwrap_or_default();
            // A path token is anything with a slash and a dot in it: `scripts/dev-peers.ts`,
            // `apps/desktop/src-tauri/tauri.conf.json`. Flags, bare `.` and an address like
            // `127.0.0.1` are excluded by needing the slash, and nothing in this manifest is.
            for token in command.split_whitespace() {
                if token.starts_with('-') || !token.contains('/') || !token.contains('.') {
                    continue;
                }
                if !root().join(token).exists() {
                    missing.push(format!("{name} -> {token}"));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "these `pnpm` scripts point at a file that is not in the tree: {missing:?}"
        );
    }
}

#[cfg(test)]
mod dto_shape_tests {
    use super::seam_scan::{
        call_span, camel, closed, first_string_argument, is_injected, root, sources,
        strip_comments, top_level,
    };
    use std::collections::{BTreeMap, BTreeSet};
    use std::fs;
    use std::path::{Path, PathBuf};

    // Spliced, because this file is one of the two panes: an unspliced mark would be the first
    // thing the walk reports, and the walk would report itself.
    const STRUCT: &str = concat!("str", "uct ");
    const SERDE: &str = concat!("#[ser", "de(");
    const ATTRIBUTE: &str = concat!("#[tauri::", "command]");
    const EMIT: &str = concat!("em", "it", "(");
    const TYPE: &str = concat!("export ty", "pe ");
    const INTERFACE: &str = concat!("export inter", "face ");
    const OPEN_MAP: &str = concat!("json", "!");

    /// The mirror, paired by the name each half answers to on the other side: `MessageDto` is
    /// the row the shell calls `ChatMessage`, `AnnouncementBubbleDto` its `DockAnnouncement`.
    /// The pairings are declared because the names genuinely differ, and every one of them is
    /// checked to exist and to be in use, so a rename on either pane fails here rather than as
    /// a blank thumbnail in the UI.
    const MIRROR: [(&str, &str); 22] = [
        ("PeerDto", "Peer"),
        ("IdentityDto", "Identity"),
        ("MessageDto", "ChatMessage"),
        ("AnnouncementBubbleDto", "DockAnnouncement"),
        ("LiveCallDto", "LiveCall"),
        ("OverlaySnapshot", "OverlaySnapshot"),
        ("GroupDto", "Group"),
        ("GroupPatch", "GroupPatch"),
        ("ReactionTallyDto", "ReactionTally"),
        ("PinnedMessageDto", "PinnedMessage"),
        ("SearchHitDto", "SearchHit"),
        ("UnreadStateDto", "UnreadState"),
        ("AttachmentDto", "Attachment"),
        ("AnnouncementDto", "Announcement"),
        ("CallEventDto", "CallEvent"),
        ("CallSignal", "CallSignal"),
        ("IncomingCallSignal", "IncomingCallSignal"),
        ("AppSettings", "AppSettings"),
        ("PermissionStatus", "PermissionStatus"),
        ("ConnectionReport", "ConnectionReport"),
        ("BackupResult", "BackupResult"),
        ("ExportResult", "ExportResult"),
    ];

    /// One pane of the mirror: where it is written and the members it declares. The third of a
    /// field is its type text, which is how a nested shape is followed across the seam.
    struct Pane {
        file: PathBuf,
        fields: Vec<(String, bool, String)>,
    }

    impl Pane {
        fn names(&self) -> BTreeSet<String> {
            self.fields
                .iter()
                .map(|(name, _, _)| name.clone())
                .collect()
        }
    }

    /// The word that starts at `at`, which is a name in both languages.
    fn word(text: &str, at: usize) -> String {
        let tail = &text[at..];
        let end = tail
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(tail.len());
        text[at..at + end].to_owned()
    }

    /// Where a name stops, so the reader can carry on from there.
    fn word_end(text: &str, at: usize) -> usize {
        let tail = &text[at..];
        let end = tail
            .find(|ch: char| !(ch.is_ascii_alphanumeric() || ch == '_'))
            .unwrap_or(tail.len());
        at + end
    }

    fn is_word(text: &str, at: usize) -> bool {
        at == 0
            || !matches!(
                text.as_bytes()[at - 1],
                b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'#' | b'/' | b'*'
            )
    }

    /// The value of `key = "…"` inside one serde attribute.
    fn serde_value(attribute: &str, key: &str) -> Option<String> {
        let at = attribute.find(key)?;
        let tail = &attribute[at + key.len()..];
        let eq = tail.find('=')?;
        let tail = tail[eq + 1..].trim_start();
        let quote = tail.chars().next().filter(|ch| *ch == '"')?;
        let tail = &tail[quote.len_utf8()..];
        let close = tail.find('"')?;
        Some(tail[..close].to_owned())
    }

    /// How a struct's field names reach the wire: the item attribute this build uses, or the
    /// field names as Rust wrote them. Anything else is a spelling this scan would have to
    /// guess at, so it says so instead of reporting a clean seam.
    fn rename_all(header: &str, file: &std::path::Path, name: &str) -> Option<String> {
        let start = header
            .rfind(['}', ';'])
            .map(|at| at + 1)
            .unwrap_or_default();
        let region = &header[start..];
        let mut found: Option<String> = None;
        let mut at = 0;
        while let Some(offset) = region[at..].find(SERDE) {
            let open = at + offset + 1;
            let close = closed(region.as_bytes(), open);
            let attribute = &region[open..=close];
            if let Some(value) = serde_value(attribute, "rename_all") {
                if found.as_ref().is_some_and(|prior| *prior != value) {
                    panic!("{}: `{name}` is renamed two ways", file.display());
                }
                found = Some(value);
            }
            at = close + 1;
        }
        match found.as_deref() {
            None | Some("camelCase") => found,
            Some(other) => panic!(
                "{}: `{name}` renames its fields {other:?}, which this scan cannot spell",
                file.display()
            ),
        }
    }

    /// Every struct in the desktop crate whose body this scan can read, with its field names as
    /// the wire sees them. A tuple or unit struct has no fields to compare and is remembered as
    /// skipped, so the crossing test can still object if one of them reaches the front end.
    fn rust_panes() -> (BTreeMap<String, Pane>, BTreeSet<String>) {
        let root = root();
        let mut files = Vec::new();
        sources(&root.join("apps/desktop/src-tauri/src"), "rs", &mut files);
        assert!(
            files.len() > 5,
            "only {} files in the desktop crate",
            files.len()
        );
        let mut out = BTreeMap::new();
        let mut skipped = BTreeSet::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let mut at = 0;
            while let Some(offset) = text[at..].find(STRUCT) {
                let start = at + offset;
                at = start + STRUCT.len();
                if !is_word(&text, start) {
                    continue;
                }
                let name = word(&text, at);
                if !name.starts_with(|ch: char| ch.is_ascii_uppercase()) {
                    continue;
                }
                let after = word_end(&text, at);
                let Some(brace) = text[after..].find('{') else {
                    continue;
                };
                let between = text[after..after + brace].trim();
                if !between.is_empty() && !(between.starts_with('<') && between.ends_with('>')) {
                    // No field list at all: a tuple or unit struct.
                    skipped.insert(name);
                    continue;
                }
                let brace_at = after + brace;
                let close = closed(text.as_bytes(), brace_at);
                let declared = rename_all(&text[..start], file, &name);
                let body = strip_comments(&text[brace_at + 1..close]);
                let mut fields = Vec::new();
                for part in top_level(&body) {
                    let mut rest = part.as_str();
                    let mut renamed: Option<String> = None;
                    loop {
                        let trimmed = rest.trim_start();
                        if !trimmed.starts_with('#') {
                            break;
                        }
                        let open = trimmed.find('[').unwrap_or_else(|| {
                            panic!("{}: `{name}` has a broken attribute", file.display())
                        });
                        let end = closed(trimmed.as_bytes(), open);
                        let attribute = &trimmed[open..=end];
                        for marker in ["skip", "flatten"] {
                            if attribute.contains(marker) {
                                panic!(
                                    "{}: a `{name}` field is `{attribute}`, so it may be absent \
                                     on the wire and this scan cannot say which rows carry it",
                                    file.display()
                                );
                            }
                        }
                        if let Some(value) = serde_value(attribute, "rename") {
                            renamed = Some(value);
                        }
                        rest = &trimmed[end + 1..];
                    }
                    let mut decl = rest.trim_start();
                    if let Some(tail) = decl.strip_prefix("pub") {
                        let tail = tail.trim_start();
                        decl = match tail.strip_prefix('(') {
                            Some(inner) => {
                                inner.split_once(')').map(|(_, rest)| rest).unwrap_or(tail)
                            }
                            None => tail,
                        }
                        .trim_start();
                    }
                    if decl.is_empty() {
                        continue;
                    }
                    let mut halves = decl.splitn(2, ':');
                    let field = halves.next().unwrap_or("").trim();
                    let kind = halves.next().unwrap_or("").trim().to_owned();
                    assert!(
                        !kind.is_empty()
                            && !field.is_empty()
                            && field.starts_with(|ch: char| ch.is_ascii_lowercase())
                            && field.bytes().all(|byte| byte.is_ascii_lowercase()
                                || byte.is_ascii_digit()
                                || byte == b'_'),
                        "{}: `{}` in `{name}` is not a field this scan can read",
                        file.display(),
                        decl.trim()
                    );
                    let wire = match (&renamed, declared.as_deref()) {
                        (Some(value), _) => value.clone(),
                        (None, Some("camelCase")) => camel(field),
                        (None, _) => field.to_owned(),
                    };
                    fields.push((wire, false, kind));
                }
                let prior = out.insert(
                    name.clone(),
                    Pane {
                        file: file.clone(),
                        fields,
                    },
                );
                assert!(
                    prior.is_none(),
                    "{}: `{name}` is read twice, so one of those is not the struct",
                    file.display()
                );
                at = close;
            }
        }
        (out, skipped)
    }

    /// Every object shape in the bridge's contract, with the member names the shell reads. A
    /// member that is not a field — a method, an index signature — stops the scan for that
    /// type, because a shape half read is a seam half audited.
    fn ts_panes() -> BTreeMap<String, Pane> {
        let path = root().join("packages/bridge/src/index.ts");
        let text =
            fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
        let mut out = BTreeMap::new();
        for mark in [TYPE, INTERFACE] {
            let mut at = 0;
            while let Some(offset) = text[at..].find(mark) {
                let start = at + offset + mark.len();
                at = start;
                let name = word(&text, start);
                if name.is_empty() {
                    continue;
                }
                let mut cursor = word_end(&text, start);
                let head = text[cursor..].trim_start();
                cursor += text[cursor..].len() - head.len();
                if let Some(tail) = head.strip_prefix('=') {
                    cursor += 1;
                    let tail = tail.trim_start();
                    cursor += text[cursor..].len() - tail.len();
                }
                if !text[cursor..].starts_with('{') {
                    continue; // a union or an alias: no field names to compare
                }
                let close = closed(text.as_bytes(), cursor);
                let body = strip_comments(&text[cursor + 1..close]);
                let mut fields = Vec::new();
                for part in body.split([';', '\n']) {
                    let member = part.trim();
                    if member.is_empty() {
                        continue;
                    }
                    let field = word(member, 0);
                    let rest = member[word_end(member, 0)..].trim_start();
                    let (optional, rest) = match rest.strip_prefix('?') {
                        Some(tail) => (true, tail.trim_start()),
                        None => (false, rest),
                    };
                    if !rest.starts_with(':') || field.is_empty() {
                        // A method or something else with no name to compare; a shape made
                        // only of these declares nothing and is not entered at all.
                        continue;
                    }
                    assert!(
                        field.starts_with(|ch: char| ch.is_ascii_alphabetic()),
                        "{}: `{}` in `{name}` is not a member this scan can read",
                        path.display(),
                        member
                    );
                    let kind = rest[1..].trim().to_owned();
                    fields.push((field, optional, kind));
                }
                if fields.is_empty() {
                    continue;
                }
                let prior = out.insert(
                    name.clone(),
                    Pane {
                        file: path.clone(),
                        fields,
                    },
                );
                assert!(
                    prior.is_none(),
                    "{}: the shell declares `{name}` twice",
                    path.display()
                );
                at = close;
            }
        }
        assert!(
            out.contains_key("ChatMessage"),
            "the contract read as {:?}, which cannot be the bridge's types",
            out.keys().collect::<Vec<_>>()
        );
        out
    }

    /// Every pane the mirror names, on both sides.
    fn both_panes() -> (BTreeMap<String, Pane>, BTreeMap<String, Pane>) {
        let (rust, _) = rust_panes();
        let typescript = ts_panes();
        for (here, there) in MIRROR {
            assert!(
                rust.contains_key(here),
                "the mirror names {here:?} on the Rust side and the desktop declares no such \
                 shape, so this build no longer sends it"
            );
            assert!(
                typescript.contains_key(there),
                "the mirror names {there:?} on the TypeScript side and the contract declares no \
                 such shape, so nothing in the front end reads it"
            );
        }
        (rust, typescript)
    }

    /// What the Rust side hands the front end: the shapes each registered command names in its
    /// return type, and the closure of everything that crosses.
    struct Seam {
        returns: BTreeMap<String, BTreeSet<String>>,
        crossing: BTreeSet<String>,
        open_maps: BTreeSet<String>,
    }

    /// The shapes that actually reach the front end: those named by a command signature, those
    /// built in the open at an event emit, and — following the field types — whatever they carry
    /// inside them.
    fn seam() -> Seam {
        let (panes, skipped) = rust_panes();
        let root = root();
        let mut files = Vec::new();
        sources(&root.join("apps/desktop/src-tauri/src"), "rs", &mut files);
        let mut queue: BTreeSet<String> = BTreeSet::new();
        let mut returns: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let mut open_maps: BTreeSet<String> = BTreeSet::new();
        for file in &files {
            let text = fs::read_to_string(file)
                .unwrap_or_else(|error| panic!("{}: {error}", file.display()));
            let bytes = text.as_bytes();
            let mut at = 0;
            while let Some(offset) = text[at..].find(EMIT) {
                let word_at = at + offset;
                let start = word_at + EMIT.len();
                at = start;
                // `app.emit(` is the call; `emitted(` is not, and neither is a word that ends
                // in these four letters. A dot before is the method receiver, not a name.
                if word_at > 0
                    && matches!(
                        bytes[word_at - 1],
                        b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_'
                    )
                {
                    continue;
                }
                // `start - 1` is the `(` the mark ends with.
                let Some(event) = first_string_argument(bytes, start - 1, false) else {
                    continue;
                };
                let Some((open, close)) = call_span(bytes, start - 1, false) else {
                    continue;
                };
                for part in top_level(&text[open + 1..close]).into_iter().skip(1) {
                    // The event pane gets the same rule as the command pane: a payload keyed in
                    // the open names nothing the mirror can read, so the front end alone decides
                    // what the listener's field is called.
                    if part.contains(OPEN_MAP) {
                        panic!(
                            "{}: the `{event}` payload `{}` is built in the open, so its keys are \
                             spelled on one side only; put them in a struct",
                            file.display(),
                            part.trim()
                        );
                    }
                    queue.extend(names_in(
                        &part,
                        &panes,
                        &skipped,
                        file,
                        &format!("the `{event}` payload"),
                    ));
                }
            }
            let mut at = 0;
            while let Some(offset) = text[at..].find(ATTRIBUTE) {
                let attribute = at + offset + ATTRIBUTE.len();
                let head = &text[attribute..];
                let function = attribute
                    + head.find("fn ").unwrap_or_else(|| {
                        panic!("{}: an attributed command with no function", file.display())
                    })
                    + 3;
                let rest = &text[function..];
                let open = function
                    + rest.find('(').unwrap_or_else(|| {
                        panic!(
                            "{}: `{}` has no parameter list",
                            file.display(),
                            word(&text, function)
                        )
                    });
                let close = closed(text.as_bytes(), open);
                let tail = &text[close..];
                let brace = close
                    + tail.find('{').unwrap_or_else(|| {
                        panic!(
                            "{}: a command after `{}` never reaches a body",
                            file.display(),
                            &text[open..=close]
                        )
                    });
                for part in top_level(&text[open + 1..close]) {
                    if is_injected(&part) {
                        continue;
                    }
                    // A `Value` argument is allowed, but it is counted: the mirror cannot read an
                    // open map, so every one of them is a seam key the front end spells alone.
                    if mentions_value(&part) {
                        open_maps.insert(word(&text, function));
                    }
                    queue.extend(names_in(
                        &part,
                        &panes,
                        &skipped,
                        file,
                        "a command parameter",
                    ));
                }
                let signature = &text[close + 1..brace];
                // A command that answers with a map built in the open has no name for the
                // mirror to read: the shell can spell any key it likes and no compiler on
                // either side notices. Every shape that crosses has to be a declared one.
                if mentions_value(signature) {
                    panic!(
                        "{}: `{}` returns {signature}, which is a shape with no name; declare \
                         the struct so the shell's type has something to mirror",
                        file.display(),
                        word(&text, function)
                    );
                }
                let answered = names_in(signature, &panes, &skipped, file, "a command return type");
                if returns
                    .insert(word(&text, function), answered.clone())
                    .is_some()
                {
                    panic!(
                        "{}: two commands are named `{}`",
                        file.display(),
                        word(&text, function)
                    );
                }
                queue.extend(answered);
                at = brace;
            }
        }
        assert!(
            returns.len() > 60,
            "the signature walk read {} commands, which is not this app's seam",
            returns.len()
        );
        let mut seen: BTreeSet<String> = BTreeSet::new();
        while let Some(name) = queue.iter().next().cloned() {
            queue.remove(&name);
            if !seen.insert(name.clone()) {
                continue;
            }
            let pane = panes.get(&name).unwrap_or_else(|| {
                panic!("`{name}` crosses the command seam and is not a struct this scan read")
            });
            for (_, _, kind) in &pane.fields {
                queue.extend(names_in(kind, &panes, &skipped, &pane.file, "a field type"));
            }
        }
        assert!(
            seen.len() > 10,
            "the signature walk found {seen:?}, which cannot be every shape this app sends"
        );
        Seam {
            returns,
            crossing: seen,
            open_maps,
        }
    }

    /// Whether one piece of type text names `Value` — the JSON map type — as a word of its own.
    fn mentions_value(signature: &str) -> bool {
        signature
            .split(|character: char| !character.is_ascii_alphanumeric() && character != '_')
            .any(|token| token == "Value")
    }

    /// The struct names one piece of type text mentions. A name the walk recognised as a
    /// fields-less struct is an error rather than a blind spot: it crosses the seam, and this
    /// scan cannot say what it carries.
    fn names_in(
        kind: &str,
        panes: &BTreeMap<String, Pane>,
        skipped: &BTreeSet<String>,
        file: &Path,
        where_: &str,
    ) -> BTreeSet<String> {
        let bytes = kind.as_bytes();
        let mut out = BTreeSet::new();
        let mut at = 0;
        while at < bytes.len() {
            if bytes[at].is_ascii_alphabetic() || bytes[at] == b'_' {
                let start = at;
                while at < bytes.len() && (bytes[at].is_ascii_alphanumeric() || bytes[at] == b'_') {
                    at += 1;
                }
                let token = &kind[start..at];
                if skipped.contains(token) {
                    panic!(
                        "{}: {where_} `{kind}` mentions `{token}`, which has no fields for this \
                         scan to compare",
                        file.display()
                    );
                }
                if panes.contains_key(token) {
                    out.insert(token.to_owned());
                }
            } else {
                at += 1;
            }
        }
        out
    }

    #[test]
    fn every_shape_the_desktop_sends_is_declared_in_the_contract() {
        let (rust, typescript) = both_panes();
        let mut stray = Vec::new();
        for (here, there) in MIRROR {
            for name in rust[here].names().difference(&typescript[there].names()) {
                stray.push(format!("{here} → {name}"));
            }
        }
        assert!(
            stray.is_empty(),
            "the desktop writes {stray:?}, which the shell's types do not declare, so nothing \
             in the front end can read it"
        );
    }

    #[test]
    fn every_field_the_shell_declares_arrives_from_the_desktop() {
        let (rust, typescript) = both_panes();
        let mut missing = Vec::new();
        for (here, there) in MIRROR {
            let sender = &rust[here];
            for (name, optional, _) in &typescript[there].fields {
                if !sender.names().contains(name) {
                    missing.push(format!(
                        "{there}.{name}{}",
                        if *optional { "?" } else { "" }
                    ));
                }
            }
        }
        assert!(
            missing.is_empty(),
            "{missing:?} are read by the shell and sent by no command, so they are undefined in \
             the desktop app however the UI uses them"
        );
    }

    /// The mirror is the whole seam, not a selection of it: a shape that reaches the front end
    /// and is not listed here is a shape nobody compares.
    #[test]
    fn every_shape_that_crosses_the_seam_is_in_the_mirror() {
        let crossing = seam().crossing;
        assert!(
            crossing.contains("MessageDto") && crossing.contains("AppSettings"),
            "the signature walk found {:?}, which cannot be this app's shapes",
            crossing
        );
        let paired: BTreeSet<String> = MIRROR.iter().map(|(here, _)| (*here).to_owned()).collect();
        let unlisted: Vec<&String> = crossing.difference(&paired).collect();
        let unused: Vec<&String> = paired.difference(&crossing).collect();
        assert!(
            unlisted.is_empty() && unused.is_empty(),
            "the mirror and the seam disagree: {unlisted:?} cross it unlisted, and {unused:?} \
             are listed but neither a command nor an event sends them"
        );
    }

    /// The one command that takes a map the mirror cannot read, listed so it cannot grow.
    ///
    /// `update_settings` applies a partial patch, so its argument is deliberately open: the shell
    /// sends only the keys the user touched and Rust deserialises what arrives into `AppSettings`.
    /// That is the one place at this seam where a key can be spelled on one side only, and it is
    /// covered by the settings round-trip tests rather than by the mirror. A second command taking
    /// a `Value` argument would be a second such place, and nobody has agreed to it.
    #[test]
    fn only_the_documented_command_takes_an_open_map() {
        let got = seam().open_maps;
        let want: BTreeSet<String> = ["update_settings"].into_iter().map(str::to_owned).collect();
        assert_eq!(
            got, want,
            "the seam now answers to a map that no mirror can check"
        );
    }

    /// The other half of the seam: what the bridge *claims* a command answers with. A command
    /// that stopped sending a row, or a call that reads one row as another, is a blank panel
    /// and no compiler error, because the two halves are written in different languages.
    #[test]
    fn every_command_answer_is_read_as_the_shape_that_sends_it() {
        const INVOKE: &str = concat!("in", "voke");
        let seam = seam();
        let typescript = ts_panes();
        let file = root().join("packages/bridge/src/tauri.ts");
        let text =
            fs::read_to_string(&file).unwrap_or_else(|error| panic!("{}: {error}", file.display()));
        let bytes = text.as_bytes();
        let mut claimed = 0usize;
        let mut anonymous = Vec::new();
        let mut unpaired = Vec::new();
        let mut stray = Vec::new();
        let mut unread = Vec::new();
        let mut at = 0;
        while let Some(offset) = text[at..].find(INVOKE) {
            let word_at = at + offset;
            let start = word_at + INVOKE.len();
            at = start;
            if word_at > 0
                && matches!(
                    bytes[word_at - 1],
                    b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'_' | b'.'
                )
            {
                continue;
            }
            // Only a call that names a shape makes a claim about one.
            if bytes.get(start) != Some(&b'<') {
                continue;
            }
            let close = closed(bytes, start);
            let generic = text[start + 1..close].trim().to_owned();
            let open = close
                + text[close..].find('(').unwrap_or_else(|| {
                    panic!(
                        "{}: `invoke<{generic}>` is not a call this scan can read",
                        file.display()
                    )
                });
            let command = first_string_argument(bytes, open, false).unwrap_or_else(|| {
                panic!("{}: `invoke<{generic}>` names no command", file.display())
            });
            claimed += 1;
            if generic.contains('{') {
                anonymous.push(format!("{command} answers {generic}"));
                continue;
            }
            let mut wants = BTreeSet::new();
            for token in generic
                .split(|ch: char| !ch.is_ascii_alphanumeric() && ch != '_')
                .filter(|token| !token.is_empty())
            {
                if !typescript.contains_key(token) {
                    continue;
                }
                match MIRROR.iter().find(|(_, there)| *there == token) {
                    Some((here, _)) => {
                        wants.insert((*here).to_owned());
                    }
                    None => unpaired.push(format!("{command} reads {token}")),
                }
            }
            let sent = seam.returns.get(&command).unwrap_or_else(|| {
                panic!(
                    "the bridge reads {command:?} as `{generic}` and the desktop registers no \
                     such command"
                )
            });
            for name in wants.difference(sent) {
                stray.push(format!("{command} reads {name}"));
            }
            for name in sent.difference(&wants) {
                unread.push(format!("{command} answers {name}"));
            }
        }
        assert!(
            claimed > 40,
            "the invoke walk read {claimed} claims, which is not this seam",
        );
        assert!(
            anonymous.is_empty(),
            "{anonymous:?} are answers the bridge describes with an object literal, so the \
             mirror has no name to read them against; declare the type"
        );
        assert!(
            unpaired.is_empty(),
            "{unpaired:?} are shapes the bridge reads that the mirror does not pair, so nothing \
             compares them"
        );
        assert!(
            stray.is_empty(),
            "{stray:?} are read out of commands that do not send them, so they arrive undefined"
        );
        assert!(
            unread.is_empty(),
            "{unread:?} are sent by a command and named by no reader, so the shell throws the \
             row away"
        );
    }
}
