#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod drag;
mod file_transfer;
mod platform;
mod secrets;
mod websocket;

use alver_core::{
    discovery::{MdnsDiscovery, PeerBeacon},
    identity::Identity,
    payloads::kind,
    storage::Store,
    transport::{initiator_handshake, responder_handshake, RemoteIdentity, SecureStream},
    Envelope, Hlc, Message, PROTOCOL_VERSION,
};
use drag::{Edge, Rect};
use mdns_sd::{RecvTimeoutError, ServiceEvent};
use rand_core::{OsRng, RngCore};
use serde::Serialize;
use serde_json::Value;
use std::{
    collections::{HashMap, HashSet},
    fs, io,
    net::{TcpListener, TcpStream, ToSocketAddrs},
    path::PathBuf,
    sync::atomic::{AtomicU16, AtomicUsize, Ordering},
    sync::{Arc, Mutex},
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
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
pub fn identity_summary(app: &AppHandle) -> serde_json::Value {
    let identity = &app.state::<OverlayState>().identity;
    serde_json::json!({
        "peerId": identity.peer_id(),
        "fingerprint": identity.fingerprint(),
    })
}

/// Tray label for the current Do-Not-Disturb choice.
const TRAY_ID: &str = "alverchat-tray";

fn refresh_tray(app: &AppHandle) {
    if let Some(tray) = app.tray_by_id(TRAY_ID) {
        if let Ok(menu) = rebuild_tray_menu(app) {
            let _ = tray.set_menu(Some(menu));
        }
    }
}

fn dnd_menu_title(app: &AppHandle) -> String {
    let state = app.state::<platform::SettingsState>();
    let settings = state.snapshot();
    if settings.dnd_enabled {
        "Do Not Disturb: On"
    } else {
        "Do Not Disturb: Off"
    }
    .to_string()
}

fn rebuild_tray_menu(app: &AppHandle) -> Result<tauri::menu::Menu<tauri::Wry>, String> {
    let settings = app.state::<platform::SettingsState>().snapshot();
    let item = |id: &str, title: String| {
        MenuItem::with_id(app, id, title, true, None::<&str>).map_err(|error| error.to_string())
    };
    let show = item("show", "Open AlverChat".to_string())?;
    let mute = item(
        "mute",
        format!(
            "Mute all notifications: {}",
            if settings.notifications_muted {
                "On"
            } else {
                "Off"
            }
        ),
    )?;
    let dnd = item("dnd", dnd_menu_title(app))?;
    let reset = item("reset-position", "Reset icon position".to_string())?;
    let quit = item("quit", "Quit".to_string())?;
    Menu::with_items(app, &[&show, &mute, &dnd, &reset, &quit]).map_err(|error| error.to_string())
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
    preview: String,
    time: String,
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
struct OverlayState {
    compact_open: Mutex<bool>,
    peers: Mutex<Vec<PeerDto>>,
    endpoints: Mutex<HashMap<String, PeerBeacon>>,
    retrying_peers: Mutex<HashSet<String>>,
    /// When each peer's "typing…" flag lapses, so a dropped frame cannot strand it.
    typing_until: Mutex<HashMap<String, u64>>,
    /// Live progress of the file transfers this process is driving, by transfer id.
    transfers: Mutex<HashMap<Uuid, file_transfer::LiveTransfer>>,
    /// Transfers with a receive driver already running, so two clicks cannot pull twice.
    pulling: Mutex<HashSet<Uuid>>,
    identity: Arc<Identity>,
    peer_sessions: Arc<AtomicUsize>,
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
            let preview = store
                .messages_for_chat(&stored.id)?
                .last()
                .map(|message| {
                    if message.tombstone {
                        "This message was deleted".into()
                    } else {
                        message.body.clone()
                    }
                })
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
                preview,
                time: String::new(),
                unread: (unread > 0).then_some(unread as u32),
                group: false,
                typing: false,
                muted: false,
                pinned: false,
            })
        })
        .collect()
}
fn snapshot(app: &AppHandle) -> OverlaySnapshot {
    let state = app.state::<OverlayState>();
    let open = state.compact_open.lock().map(|v| *v).unwrap_or(false);
    let peers = state.peers.lock().map(|v| v.clone()).unwrap_or_default();
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
    }
}
fn publish(app: &AppHandle) {
    let _ = app.emit("overlay-state", snapshot(app));
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
        preview: String::new(),
        time: String::new(),
        unread: None,
        group: false,
        typing: false,
        muted: false,
        pinned: false,
    };
    if let Some(existing) = peers.iter_mut().find(|peer| peer.id == peer_id) {
        peer.preview.clone_from(&existing.preview);
        peer.time.clone_from(&existing.time);
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
    app.state::<OverlayState>()
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?
        .trust_peer(&remote.peer_id, &remote.public_key)?;

    loop {
        let envelope: Envelope<Value> = session.receive()?;
        let file_frame = file_transfer::handled_kind(&envelope.kind);
        if envelope.v != PROTOCOL_VERSION
            || envelope.from != remote.peer_id
            || envelope.to != identity.peer_id()
            || (!file_frame && !matches!(envelope.kind.as_str(), "message" | "typing"))
        {
            return Err("incoming message envelope is invalid".into());
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
        if envelope.kind == "typing" {
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
        let now_ms = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        if envelope.hlc.physical_ms > now_ms.saturating_add(5 * 60 * 1000) {
            return Err("incoming message timestamp is too far in the future".into());
        }
        let message = Message {
            id: envelope.id,
            chat_id: remote.peer_id.clone(),
            sender: remote.peer_id.clone(),
            body: body.to_owned(),
            clock: envelope.hlc,
            tombstone: false,
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
                if is_new {
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
            if !focused && platform::should_notify(&settings, &remote.peer_id, now_ms) {
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
        let _ = app.emit("messages-changed", remote.peer_id.clone());
    }
}

/// The receipt every accepted frame gets back, so the sender can mark its outbox row.
fn receipt_for(identity: &Identity, peer_id: &str, frame_id: Uuid, at_ms: u64) -> Envelope<Value> {
    Envelope {
        v: PROTOCOL_VERSION,
        id: Uuid::now_v7(),
        kind: "receipt".into(),
        from: identity.peer_id(),
        to: peer_id.to_owned(),
        hlc: Hlc {
            physical_ms: at_ms,
            logical: 0,
        },
        payload: serde_json::json!({"message_id": frame_id.to_string(), "status": "delivered"}),
    }
}

fn connect_to_peer(beacon: &PeerBeacon) -> Result<TcpStream, String> {
    let addresses = (beacon.host.as_str(), beacon.port)
        .to_socket_addrs()
        .map_err(|error| format!("Cannot resolve peer address: {error}"))?
        .collect::<Vec<_>>();
    if addresses.is_empty() {
        return Err("Peer address did not resolve on the local network".into());
    }
    let mut last_error = None;
    for address in addresses {
        match TcpStream::connect_timeout(&address, Duration::from_secs(5)) {
            Ok(stream) => {
                stream
                    .set_nodelay(true)
                    .map_err(|error| error.to_string())?;
                return Ok(stream);
            }
            Err(error) => last_error = Some(error),
        }
    }
    Err(format!(
        "Cannot connect to peer on the local network: {}",
        last_error
            .map(|error| error.to_string())
            .unwrap_or_else(|| "no usable address".into())
    ))
}

fn deliver_envelope(
    app: &AppHandle,
    beacon: &PeerBeacon,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    send_frame(app, beacon, envelope)?;
    app.state::<OverlayState>()
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?
        .acknowledge_outbox(envelope.id)?;
    let _ = app.emit("messages-changed", envelope.to.clone());
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
    let session = initiator_handshake(stream, &state.identity, &expected)?;
    state
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?
        .trust_peer(&expected.peer_id, &expected.public_key)?;
    Ok(session)
}

/// Handshakes with one beacon, sends a single frame and waits for its receipt.
pub(crate) fn send_frame(
    app: &AppHandle,
    beacon: &PeerBeacon,
    envelope: &Envelope<Value>,
) -> Result<(), String> {
    let my_id = app.state::<OverlayState>().identity.peer_id();
    let mut session = open_session(app, beacon)?;
    session.send(envelope)?;
    let receipt: Envelope<Value> = session.receive()?;
    let frame_id = envelope.id.to_string();
    if receipt.v != PROTOCOL_VERSION
        || receipt.kind != "receipt"
        || receipt.from != beacon.peer_id
        || receipt.to != my_id
        || receipt.payload.get("status").and_then(Value::as_str) != Some("delivered")
        || receipt.payload.get("message_id").and_then(Value::as_str) != Some(frame_id.as_str())
    {
        return Err("peer returned an invalid delivery receipt".into());
    }
    Ok(())
}

fn start_outbox_retry(app: AppHandle, beacon: PeerBeacon) {
    let peer_id = beacon.peer_id.clone();
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
                    break;
                };
                let mut failed = false;
                for envelope in pending {
                    if deliver_envelope(&app, &beacon, &envelope).is_err() {
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
    mdns: MdnsDiscovery,
    identity: Arc<Identity>,
    host: String,
    port: u16,
}

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
    if let Err(error) = discovery.mdns.publish(&beacon, &[]) {
        eprintln!("Could not republish LAN announcement: {error}");
    }
}

fn start_peer_discovery(
    app: &mut tauri::App,
    identity: Arc<Identity>,
    port: u16,
) -> Result<(), String> {
    let mdns = MdnsDiscovery::new()?;
    let host = format!("alverchat-{}.local.", &identity.peer_id()[..12]);
    let display_name = advertised_display_name(app.handle(), &identity);
    let beacon = PeerBeacon::signed(&identity, display_name, host.clone(), port);
    mdns.publish(&beacon, &[])?;
    app.manage(DiscoveryState {
        mdns: mdns.clone(),
        identity: identity.clone(),
        host,
        port,
    });
    let events = mdns.browse()?;
    let self_id = beacon.peer_id.clone();
    let app = app.handle().clone();
    std::thread::spawn(move || {
        let _daemon = mdns;
        let mut discovered_services = HashMap::<String, String>::new();
        loop {
            match events.recv_timeout(std::time::Duration::from_secs(1)) {
                Ok(ServiceEvent::ServiceResolved(service)) => {
                    let Some(beacon) = PeerBeacon::from_mdns(&service) else {
                        continue;
                    };
                    if beacon.peer_id == self_id {
                        continue;
                    }
                    discovered_services
                        .insert(service.get_fullname().to_owned(), beacon.peer_id.clone());
                    upsert_discovered_peer(&app, beacon.clone());
                    start_outbox_retry(app.clone(), beacon);
                }
                Ok(ServiceEvent::ServiceRemoved(fullname, _)) => {
                    if let Some(peer_id) = discovered_services.remove(&fullname) {
                        if let Ok(mut endpoints) = app.state::<OverlayState>().endpoints.lock() {
                            endpoints.remove(&peer_id);
                        }
                        if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
                            if let Some(peer) = peers.iter_mut().find(|peer| peer.id == peer_id) {
                                peer.online = false;
                                peer.status =
                                    "Offline · messages will retry when the peer returns".into();
                            }
                        }
                        publish(&app);
                    }
                }
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout) => expire_typing(&app),
                Err(RecvTimeoutError::Disconnected) => break,
            }
        }
    });
    Ok(())
}

#[tauri::command]
fn app_name() -> &'static str {
    alver_core::APP_NAME
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
    if let Ok(mut store) = app.state::<OverlayState>().store.lock() {
        let _ = store.mark_chat_read(&chat_id, "local", now_ms());
    }
    if let Ok(mut peers) = app.state::<OverlayState>().peers.lock() {
        if let Some(peer) = peers.iter_mut().find(|peer| peer.id == chat_id) {
            peer.unread = None;
        }
    }
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

#[tauri::command]
fn edit_message_local(app: AppHandle, message_id: String, text: String) -> Result<bool, String> {
    let state = app.state::<OverlayState>();
    let id = Uuid::parse_str(&message_id).map_err(|e| e.to_string())?;
    let text = text.trim();
    if text.is_empty() || text.len() > 16_384 {
        return Err("Message must contain 1-16384 bytes".into());
    }
    let now_ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    let clock = Hlc {
        physical_ms: now_ms,
        logical: 0,
    };
    let mut store = state
        .store
        .lock()
        .map_err(|_| "message store lock poisoned")?;
    store.edit_message(id, "local", text, clock)
}

#[tauri::command]
fn get_my_identity(app: AppHandle) -> serde_json::Value {
    let state = app.state::<OverlayState>();
    let peer_id = state.identity.peer_id();
    let key_bytes = state.identity.verifying_key().to_bytes();
    let fingerprint = alver_core::peer_id(&key_bytes);
    serde_json::json!({
        "peerId": peer_id,
        "fingerprint": fingerprint,
    })
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
/// label so several installs can share one machine without merging their
/// identity, keychain entries or databases.
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
            let settings = platform::load_settings(app.handle());
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
            let store = open_message_store(app.handle()).map_err(std::io::Error::other)?;
            let initial_peers = known_peers(&store).map_err(std::io::Error::other)?;
            let identity = Arc::new(identity_key(app.handle()).map_err(std::io::Error::other)?);
            app.manage(OverlayState {
                compact_open: Mutex::new(false),
                peers: Mutex::new(initial_peers),
                endpoints: Mutex::new(HashMap::new()),
                retrying_peers: Mutex::new(HashSet::new()),
                typing_until: Mutex::new(HashMap::new()),
                transfers: Mutex::new(HashMap::new()),
                pulling: Mutex::new(HashSet::new()),
                identity: identity.clone(),
                peer_sessions: Arc::new(AtomicUsize::new(0)),
                store: Mutex::new(store),
                preferences: Mutex::new(preferences),
                preferences_path,
                active_drag: Mutex::new(None),
                pointer_capture: Mutex::new(HashMap::new()),
                compact_detached: Mutex::new(false),
                cursor_capture: Mutex::new(HashMap::new()),
            });
            let transport = start_secure_listener(app.handle().clone(), identity.clone())
                .and_then(|port| start_peer_discovery(app, identity, port));
            match transport {
                Ok(()) => {}
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
                    "quit" => app.exit(0),
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
            app_name,
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
            commands::set_avatar,
            commands::get_avatar
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AlverChat");
}
