//! OS-facing behavior for the desktop shell: persisted app settings, global
//! hotkey, launch-at-login, native notifications with the Do-Not-Disturb
//! schedule, permission deep links, connection diagnostics and backups.
//!
//! Everything here is platform glue. Protocol and storage rules stay in
//! `alver-core`.

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

pub const SETTINGS_FILE: &str = "settings.json";
pub const PASSPHRASE_ACCOUNT: &str = "lan-passphrase";
/// The shortest backup passphrase [`create_backup`] accepts; `settings_surface_tests` checks the
/// number the Settings placeholder prints against this one.
pub const BACKUP_PASSPHRASE_MIN_LEN: usize = 8;

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct AppSettings {
    // General
    pub theme: String,
    pub language: String,
    pub launch_at_login: bool,
    pub display_name: String,
    pub avatar_color: String,
    pub presence: String,
    pub custom_status: String,
    // Floating icon & bubbles
    pub bubbles_enabled: bool,
    pub idle_icon: String,
    pub icon_morph_enabled: bool,
    pub icon_morph_delay_seconds: u8,
    pub bubble_auto_hide_seconds: u8,
    pub screen_corner: String,
    pub opacity_percent: u8,
    pub snap_to_edge: bool,
    pub hotkey: String,
    // Notifications
    pub notification_sound: bool,
    pub notifications_muted: bool,
    pub dnd_enabled: bool,
    pub dnd_days: Vec<String>,
    pub dnd_from: String,
    pub dnd_to: String,
    pub muted_chats: Vec<String>,
    /// Chats the user pinned to the top of the list, in pin order.
    pub pinned_chats: Vec<String>,
    pub show_message_preview: bool,
    // Messages
    pub edit_window_minutes: u32,
    pub auto_download_limit_mb: u32,
    pub max_file_size_mb: u32,
    pub downloads_dir: String,
    // Calls
    pub microphone: String,
    pub speaker: String,
    pub camera: String,
    pub noise_suppression: bool,
    pub echo_cancellation: bool,
    pub ringtone: String,
    // Privacy & network
    pub lan_encryption: bool,
    pub who_sees_status: String,
    pub visible_on_network: bool,
    pub network_adapter: String,
    pub announcements_from_admins_only: bool,
}

impl Default for AppSettings {
    fn default() -> Self {
        Self {
            theme: "system".into(),
            language: "en".into(),
            launch_at_login: false,
            display_name: String::new(),
            avatar_color: "violet".into(),
            presence: "Available".into(),
            custom_status: String::new(),
            bubbles_enabled: true,
            idle_icon: "logo".into(),
            icon_morph_enabled: true,
            icon_morph_delay_seconds: 5,
            bubble_auto_hide_seconds: 7,
            screen_corner: "right-center".into(),
            opacity_percent: 90,
            snap_to_edge: true,
            hotkey: default_hotkey(),
            notification_sound: true,
            notifications_muted: false,
            dnd_enabled: true,
            dnd_days: ["Mon", "Tue", "Wed", "Thu", "Fri"]
                .iter()
                .map(|day| (*day).to_string())
                .collect(),
            dnd_from: "22:00".into(),
            dnd_to: "07:00".into(),
            muted_chats: Vec::new(),
            pinned_chats: Vec::new(),
            show_message_preview: true,
            edit_window_minutes: 15,
            auto_download_limit_mb: 50,
            max_file_size_mb: 500,
            downloads_dir: String::new(),
            microphone: "default".into(),
            speaker: "default".into(),
            camera: "default".into(),
            noise_suppression: true,
            echo_cancellation: true,
            ringtone: "alverchat".into(),
            lan_encryption: false,
            who_sees_status: "everyone".into(),
            visible_on_network: true,
            network_adapter: "auto".into(),
            announcements_from_admins_only: false,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn default_hotkey() -> String {
    "Command+Alt+S".into()
}
#[cfg(not(target_os = "macos"))]
pub fn default_hotkey() -> String {
    "Ctrl+Alt+S".into()
}

/// The two toggle rows the tray menu shows, built from the settings the notification
/// path reads. Callers must refresh the tray whenever either flag changes — Settings
/// writes them directly, so a stale label would promise the opposite of what happens.
pub fn tray_toggle_titles(settings: &AppSettings) -> (String, String) {
    let word = |value: bool| if value { "On" } else { "Off" };
    (
        format!(
            "Mute all notifications: {}",
            word(settings.notifications_muted)
        ),
        format!("Do Not Disturb: {}", word(settings.dnd_enabled)),
    )
}

pub struct SettingsState(pub Arc<Mutex<AppSettings>>);

impl SettingsState {
    pub fn snapshot(&self) -> AppSettings {
        self.0.lock().map(|value| value.clone()).unwrap_or_default()
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = crate::instance_config_dir(app)?;
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory.join(SETTINGS_FILE))
}

pub fn load_settings(app: &AppHandle) -> AppSettings {
    settings_path(app)
        .ok()
        .and_then(|path| std::fs::read_to_string(path).ok())
        .and_then(|raw| serde_json::from_str(&raw).ok())
        .unwrap_or_default()
}

pub fn persist(app: &AppHandle, settings: &AppSettings) -> Result<(), String> {
    let path = settings_path(app)?;
    let bytes = serde_json::to_vec_pretty(settings).map_err(|error| error.to_string())?;
    std::fs::write(path, bytes).map_err(|error| error.to_string())
}

/// Minutes after midnight for `HH:MM`; `None` when the string is malformed.
pub fn minutes_of_day(value: &str) -> Option<u32> {
    let (hour, minute) = value.trim().split_once(':')?;
    let hour: u32 = hour.parse().ok()?;
    let minute: u32 = minute.parse().ok()?;
    if hour > 23 || minute > 59 {
        return None;
    }
    Some(hour * 60 + minute)
}

/// True when `now_ms` falls inside the configured quiet window. The window may
/// cross midnight, so the comparison wraps instead of requiring start < end.
pub fn dnd_window_active(settings: &AppSettings, now_ms: u64) -> bool {
    if !settings.dnd_enabled {
        return false;
    }
    let (start, end) = match (
        minutes_of_day(&settings.dnd_from),
        minutes_of_day(&settings.dnd_to),
    ) {
        (Some(start), Some(end)) if start != end => (start, end),
        _ => return false,
    };
    let local = match Local.timestamp_millis_opt(now_ms as i64).single() {
        Some(moment) => moment,
        None => return false,
    };
    if !settings
        .dnd_days
        .iter()
        .any(|day| day_abbrev(&local.format("%a").to_string()) == day.as_str())
    {
        return false;
    }
    let minute_of_day = local.format("%H").to_string().parse::<u32>().unwrap_or(0) * 60
        + local.format("%M").to_string().parse::<u32>().unwrap_or(0);
    if start < end {
        minute_of_day >= start && minute_of_day < end
    } else {
        minute_of_day >= start || minute_of_day < end
    }
}

fn day_abbrev(raw: &str) -> String {
    match raw {
        "Mon" | "Tue" | "Wed" | "Thu" | "Fri" | "Sat" | "Sun" => raw.to_string(),
        _ => String::new(),
    }
}

/// True when the whole device is quiet, which is what the floating icon shows as its
/// Do-Not-Disturb state. A per-chat mute is deliberately absent: it silences one thread,
/// not the icon. `should_notify` below reads the same rule, so the ring and the alert
/// cannot disagree.
#[must_use]
pub fn icon_is_quiet(settings: &AppSettings, now_ms: u64) -> bool {
    settings.notifications_muted || dnd_window_active(settings, now_ms)
}

/// A chat stays quiet when the app is muted, muted per chat, or inside the
/// scheduled quiet window.
pub fn should_notify(settings: &AppSettings, chat_id: &str, now_ms: u64) -> bool {
    !icon_is_quiet(settings, now_ms) && !settings.muted_chats.iter().any(|id| id == chat_id)
}

fn parse_hotkey(combo: &str) -> Result<Shortcut, String> {
    let mut modifiers = Modifiers::empty();
    let mut key: Option<Code> = None;
    for part in combo
        .split('+')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let lowered = part.to_lowercase();
        match lowered.as_str() {
            "cmd" | "command" | "meta" | "super" => modifiers |= Modifiers::SUPER,
            "ctrl" | "control" => modifiers |= Modifiers::CONTROL,
            "alt" | "option" | "opt" => modifiers |= Modifiers::ALT,
            "shift" => modifiers |= Modifiers::SHIFT,
            other => key = Some(code_from_name(other)?),
        }
    }
    let key = key.ok_or_else(|| "A shortcut needs one non-modifier key.".to_string())?;
    Ok(Shortcut::new(Some(modifiers), key))
}

fn code_from_name(name: &str) -> Result<Code, String> {
    let upper: String = name.to_uppercase();
    let code = match upper.as_str() {
        "S" => Code::KeyS,
        "D" => Code::KeyD,
        "C" => Code::KeyC,
        "V" => Code::KeyV,
        "K" => Code::KeyK,
        "A" => Code::KeyA,
        "E" => Code::KeyE,
        "M" => Code::KeyM,
        "SPACE" => Code::Space,
        other if other.len() == 1 && other.as_bytes()[0].is_ascii_alphabetic() => {
            return Err(format!("Unsupported shortcut key: {other}"))
        }
        _ => return Err(format!("Unsupported shortcut key: {name}")),
    };
    Ok(code)
}

pub fn register_hotkey(app: &AppHandle, combo: &str) -> Result<String, String> {
    let shortcut = parse_hotkey(combo)?;
    let _ = app.global_shortcut().unregister_all();
    app.global_shortcut()
        .on_shortcut(shortcut, |app, _shortcut, event| {
            if event.state == ShortcutState::Pressed {
                toggle_dock(app);
            }
        })
        .map_err(|error| format!("Cannot register the global shortcut: {error}"))?;
    Ok(combo.to_string())
}

fn toggle_dock(app: &AppHandle) {
    if let Some(dock) = app.get_webview_window("dock") {
        if dock.is_visible().unwrap_or(false) {
            let _ = dock.hide();
        } else {
            let _ = dock.show();
            let _ = dock.set_always_on_top(true);
        }
    }
}

pub fn apply_autostart(app: &AppHandle, enabled: bool) -> Result<(), String> {
    let launcher = app.autolaunch();
    if enabled {
        launcher.enable().map_err(|error| error.to_string())?;
    } else {
        launcher.disable().map_err(|error| error.to_string())?;
    }
    Ok(())
}

/// A toast on Windows is branded by the `AppUserModelID` of the installed shortcut, not by
/// the strings we pass, so a message toast reads as a stock Windows notification. The dock
/// bubble is AlverChat's own alert surface, so it takes over whenever it can actually show
/// the message.
fn message_alert_uses_dock(is_windows: bool, bubbles_enabled: bool, dock_visible: bool) -> bool {
    is_windows && bubbles_enabled && dock_visible
}

pub fn notify_message(app: &AppHandle, title: &str, body: &str) {
    let settings = app.state::<SettingsState>().snapshot();
    let preview = if settings.show_message_preview {
        body
    } else {
        "New message"
    };
    let dock = app.get_webview_window("dock");
    let dock_visible = dock
        .as_ref()
        .and_then(|dock| dock.is_visible().ok())
        .unwrap_or(false);
    if message_alert_uses_dock(cfg!(windows), settings.bubbles_enabled, dock_visible) {
        // The payload is the sound flag: the bubble itself already comes from `overlay-state`.
        if let Some(dock) = dock {
            let _ = dock.emit("message-alert", settings.notification_sound);
        }
        return;
    }
    let mut builder = app.notification().builder().title(title).body(preview);
    if settings.notification_sound {
        // notify-rust matches the Windows sound name case-sensitively, and an unparseable
        // name becomes `<audio silent="true">` instead of the default notification sound.
        builder = builder.sound(if cfg!(windows) { "Default" } else { "default" });
    }
    let _ = builder.show();
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionStatus {
    pub kind: String,
    pub label: String,
    pub hint: String,
    pub state: String,
    pub deep_link: String,
}

#[cfg(target_os = "macos")]
fn deep_link(kind: &str) -> String {
    match kind {
        "local-network" => "x-apple.systempreferences:com.apple.settings.PrivacySecurity.extension?Privacy_LocalNetwork".into(),
        "camera" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Camera".into(),
        "microphone" => "x-apple.systempreferences:com.apple.preference.security?Privacy_Microphone".into(),
        "notifications" => "x-apple.systempreferences:com.apple.preference.notifications".into(),
        "login-items" => "x-apple.systempreferences:com.apple.preference.security?Privacy_LaunchItems".into(),
        _ => "x-apple.systempreferences:com.apple.preference.security".into(),
    }
}

#[cfg(target_os = "windows")]
fn deep_link(kind: &str) -> String {
    match kind {
        "local-network" | "firewall" => "ms-settings:network-status".into(),
        "camera" => "ms-settings:privacy-webcam".into(),
        "microphone" => "ms-settings:privacy-microphone".into(),
        "notifications" => "ms-settings:notifications".into(),
        "login-items" => "ms-settings:startup".into(),
        _ => "ms-settings:privacy".into(),
    }
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
fn deep_link(_kind: &str) -> String {
    // Linux has no portable settings URL scheme; the desktop environment decides.
    // An empty link tells `open_permission_settings` to explain that instead of
    // opening an unrelated page in the browser.
    String::new()
}

#[cfg(target_os = "macos")]
pub fn firewall_steps() -> Vec<String> {
    vec![
        "Open System Settings, then Privacy & Security.".into(),
        "Click Local Network and enable AlverChat.".into(),
        "If a firewall is active, allow incoming connections for AlverChat.".into(),
    ]
}

#[cfg(target_os = "windows")]
pub fn firewall_steps() -> Vec<String> {
    vec![
        "Open Windows Security, then Firewall & network protection.".into(),
        "Click Allow an app through firewall.".into(),
        "Find AlverChat in the list and allow it for Private networks (LAN).".into(),
    ]
}

#[cfg(not(any(target_os = "macos", target_os = "windows")))]
pub fn firewall_steps() -> Vec<String> {
    vec![
        "Open your distribution firewall tool.".into(),
        "Allow inbound UDP 5353 and the AlverChat discovery port.".into(),
    ]
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

#[tauri::command]
pub fn get_settings(state: tauri::State<'_, SettingsState>) -> AppSettings {
    state.snapshot()
}

/// Overlays a partial update on the stored settings.
///
/// The UI sends only the fields it changed; treating that object as complete
/// would reset every other preference to its default.
fn merge_settings(current: &AppSettings, patch: &Value) -> Result<AppSettings, String> {
    let mut merged = serde_json::to_value(current).map_err(|error| error.to_string())?;
    if let (Some(base), Some(patch)) = (merged.as_object_mut(), patch.as_object()) {
        for (key, value) in patch {
            base.insert(key.clone(), value.clone());
        }
    }
    serde_json::from_value(merged).map_err(|error| format!("Invalid settings update: {error}"))
}

#[tauri::command]
pub fn update_settings(app: AppHandle, patch: Value) -> Result<AppSettings, String> {
    let state = app.state::<SettingsState>();
    let previous = state.snapshot();
    {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        *guard = merge_settings(&guard, &patch)?;
    }
    let current = state.snapshot();
    persist(&app, &current)?;
    if current.hotkey != previous.hotkey {
        match register_hotkey(&app, &current.hotkey) {
            Ok(_) => {}
            Err(error) => {
                let _ = app.emit("hotkey-error", error);
                let _ = register_hotkey(&app, &default_hotkey());
            }
        }
    }
    if current.launch_at_login != previous.launch_at_login {
        let _ = apply_autostart(&app, current.launch_at_login);
    }
    if current.notifications_muted != previous.notifications_muted
        || current.dnd_enabled != previous.dnd_enabled
    {
        crate::refresh_tray(&app);
    }
    if current.screen_corner != previous.screen_corner {
        let _ = crate::apply_screen_corner(&app, &current.screen_corner);
    }
    if current.display_name != previous.display_name
        || current.visible_on_network != previous.visible_on_network
        || current.network_adapter != previous.network_adapter
    {
        crate::republish_lan_beacon(&app);
    }
    // The overlays only see settings through the snapshot, and nothing else emits here,
    // so a settings write has to repaint them or the change waits for unrelated traffic.
    crate::refresh_overlay_quiet(&app);
    crate::publish(&app);
    Ok(current)
}

#[tauri::command]
pub fn set_hotkey(app: AppHandle, combo: String) -> Result<String, String> {
    let registered = register_hotkey(&app, &combo)?;
    {
        let state = app.state::<SettingsState>();
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        guard.hotkey = registered.clone();
        persist(&app, &guard)?;
    }
    Ok(registered)
}

#[tauri::command]
pub fn set_launch_at_login(app: AppHandle, enabled: bool) -> Result<bool, String> {
    apply_autostart(&app, enabled)?;
    let state = app.state::<SettingsState>();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    guard.launch_at_login = enabled;
    persist(&app, &guard)?;
    Ok(app.autolaunch().is_enabled().unwrap_or(enabled))
}

#[tauri::command]
pub fn is_dnd_active(app: AppHandle) -> bool {
    let settings = app.state::<SettingsState>().snapshot();
    let now_ms = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64;
    dnd_window_active(&settings, now_ms)
}

#[tauri::command]
pub fn set_lan_passphrase(app: AppHandle, passphrase: String) -> Result<bool, String> {
    if passphrase.len() > 256 {
        return Err("The LAN passphrase is too long.".into());
    }
    let secrets = crate::secrets::Secrets::open(&app)?;
    if passphrase.is_empty() {
        secrets.delete(PASSPHRASE_ACCOUNT)?;
    } else {
        secrets.set(PASSPHRASE_ACCOUNT, passphrase.as_bytes())?;
    }
    let state = app.state::<SettingsState>();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    guard.lan_encryption = !passphrase.is_empty();
    persist(&app, &guard)?;
    crate::forget_lan_passphrase(&app);
    crate::reset_authenticated_sessions(&app);
    Ok(!passphrase.is_empty())
}

#[tauri::command]
pub fn has_lan_passphrase(app: AppHandle) -> bool {
    crate::secrets::Secrets::open(&app)
        .and_then(|secrets| secrets.get(PASSPHRASE_ACCOUNT))
        .map(|secret| secret.is_some_and(|secret| !secret.is_empty()))
        .unwrap_or(false)
}

#[tauri::command]
pub fn permission_status(kind: String) -> PermissionStatus {
    let normalized = kind.to_lowercase().replace('_', "-");
    let (label, hint) = match normalized.as_str() {
        "local-network" => (
            "Local network access",
            "Allow AlverChat to discover and connect to other devices on your local network.",
        ),
        "camera" => ("Camera", "Needed for video calls."),
        "microphone" => ("Microphone", "Needed for voice and video calls."),
        "notifications" => ("Notifications", "Used for message and call alerts."),
        "firewall" => (
            "Firewall",
            "Allow AlverChat to accept connections on your private network.",
        ),
        other => ("Permission", other),
    };
    PermissionStatus {
        kind: normalized.clone(),
        label: label.to_string(),
        hint: hint.to_string(),
        // The OS does not expose a portable "is this granted" query, so the
        // onboarding screen treats unknown as "needs action" and shows the
        // system-settings deep link instead.
        state: "unknown".into(),
        deep_link: deep_link(&normalized),
    }
}

#[tauri::command]
pub fn open_permission_settings(app: AppHandle, kind: String) -> Result<(), String> {
    let status = permission_status(kind);
    if status.deep_link.is_empty() {
        return Err(
            "This platform has no settings deep link; open your desktop's privacy settings manually."
                .into(),
        );
    }
    app.opener()
        .open_url(&status.deep_link, None::<String>)
        .map_err(|error| format!("Cannot open the system settings: {error}"))
}

#[tauri::command]
pub fn open_firewall_guide(app: AppHandle) -> Result<Vec<String>, String> {
    let steps = firewall_steps();
    app.opener()
        .open_url(deep_link("firewall"), None::<String>)
        .unwrap_or_default();
    Ok(steps)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectionReport {
    pub listening: bool,
    pub port: u16,
    pub peers: usize,
    pub online_peers: usize,
    pub adapters: Vec<String>,
    pub discovery_port: u16,
    /// Whether the signed broadcast that carries this machine's beacon without mDNS is
    /// open right now. The port number alone would not say: a second instance on the same
    /// host cannot bind it, and then only mDNS is working.
    pub discovery_active: bool,
    pub warnings: Vec<String>,
}

#[tauri::command]
pub fn run_connection_test(app: AppHandle) -> ConnectionReport {
    let state = app.state::<crate::OverlayState>();
    let peers = state
        .peers
        .lock()
        .map(|peers| peers.clone())
        .unwrap_or_default();
    let port = state
        .endpoints
        .lock()
        .map(|_| crate::listener_port())
        .unwrap_or(0);
    let discovery_active = app
        .try_state::<crate::DiscoveryState>()
        .map(|discovery| {
            discovery
                .udp_open
                .load(std::sync::atomic::Ordering::Acquire)
        })
        .unwrap_or(false);
    let adapters = local_ipv4_addresses();
    let mut warnings = Vec::new();
    if port == 0 {
        warnings.push("AlverChat is not listening for peer connections.".into());
    }
    if peers.is_empty() {
        warnings.push(
            "No peers discovered yet. Check that other devices run AlverChat on this network."
                .into(),
        );
    }
    if adapters.is_empty() {
        warnings.push("No local network adapter with a private address was found.".into());
    }
    if !discovery_active {
        warnings.push(
            "The UDP discovery fallback is not listening, so peers are found by mDNS alone.".into(),
        );
    }
    ConnectionReport {
        listening: port != 0,
        port,
        peers: peers.len(),
        online_peers: peers.iter().filter(|peer| peer.online).count(),
        adapters,
        discovery_port: alver_core::discovery::DISCOVERY_PORT,
        discovery_active,
        warnings,
    }
}

fn local_interfaces() -> Vec<if_addrs::Interface> {
    if_addrs::get_if_addrs()
        .unwrap_or_default()
        .into_iter()
        .filter(|interface| interface.ip().is_ipv4() && !interface.is_loopback())
        .collect()
}

pub fn local_ipv4_addresses() -> Vec<String> {
    local_interfaces()
        .iter()
        .map(|interface| format!("{} ({})", interface.ip(), interface.name))
        .collect()
}

/// The same adapters as addresses, for the code that has to advertise one of them.
pub fn local_ipv4s() -> Vec<std::net::IpAddr> {
    local_interfaces()
        .iter()
        .map(|interface| interface.ip())
        .collect()
}

/// Settings → Network adapter, resolved against the adapters this host has right now.
/// `None` means advertise everywhere, which is what "Automatic" means and what a saved
/// address that no longer exists means too: a machine that switched Wi-Fi must not go
/// invisible because it is looking for an interface it left behind.
pub fn adapter_choice(choice: &str, available: &[std::net::IpAddr]) -> Option<std::net::IpAddr> {
    let trimmed = choice.trim();
    if trimmed.is_empty() || trimmed.eq_ignore_ascii_case("auto") {
        return None;
    }
    // The picker stores the report's label, "192.168.1.10 (en0)"; accept either order.
    let head = trimmed.split([' ', '(']).next().unwrap_or(trimmed);
    let tail = trimmed
        .trim_end_matches(')')
        .rsplit('(')
        .next()
        .unwrap_or("");
    for token in [head, tail] {
        if let Ok(address) = token.parse::<std::net::IpAddr>() {
            return available.contains(&address).then_some(address);
        }
    }
    None
}

/// The adapters the discovery fallback can broadcast from: each IPv4 interface's own
/// address and the subnet broadcast the system reports for it, when it reports one.
pub fn local_ipv4_beacons() -> Vec<(std::net::Ipv4Addr, Option<std::net::Ipv4Addr>)> {
    local_interfaces()
        .iter()
        .filter_map(|interface| match &interface.addr {
            if_addrs::IfAddr::V4(address) => Some((address.ip, address.broadcast)),
            if_addrs::IfAddr::V6(_) => None,
        })
        .collect()
}

/// Where the UDP discovery fallback sends this machine's beacon, and with which address
/// written into the signed payload.
///
/// A datagram aimed at `255.255.255.255` leaves only through the default route, so each
/// subnet's own broadcast address is used where the interface reports one, and an interface
/// that reports none still gets the limited address instead of being skipped. The host in
/// the beacon is the interface's own address, because a peer that cannot run mDNS is exactly
/// the peer that has no way to turn a `.local.` name into a route.
///
/// Settings → Network adapter narrows the list to the one interface it names. A saved
/// address this machine no longer holds means "automatic" rather than silence — the same
/// rule [`adapter_choice`] applies to the announcement — so a laptop that left the office
/// keeps announcing at home.
pub fn discovery_targets(
    adapters: &[(std::net::Ipv4Addr, Option<std::net::Ipv4Addr>)],
    choice: &str,
    port: u16,
) -> Vec<(std::net::IpAddr, std::net::SocketAddr)> {
    let addresses: Vec<std::net::IpAddr> = adapters
        .iter()
        .map(|(address, _)| std::net::IpAddr::V4(*address))
        .collect();
    let chosen = adapter_choice(choice, &addresses);
    let mut seen = std::collections::HashSet::new();
    adapters
        .iter()
        .filter(|(address, _)| chosen.is_none_or(|picked| picked == std::net::IpAddr::V4(*address)))
        .filter_map(|(address, broadcast)| {
            let destination = std::net::SocketAddr::from((
                broadcast.unwrap_or(std::net::Ipv4Addr::BROADCAST),
                port,
            ));
            seen.insert(destination)
                .then_some((std::net::IpAddr::V4(*address), destination))
        })
        .collect()
}

/// Settings → "Who sees my status". Only an explicit "nobody" hides anything: a blank
/// value, or one written by an older build, keeps the default of letting peers see it.
pub fn status_is_private(who_sees_status: &str) -> bool {
    who_sees_status.trim().eq_ignore_ascii_case("nobody")
}

/// The profile's presence label, reduced to the token the presence dots are drawn from.
/// The wire carries the token rather than the label so a translated profile cannot change
/// how a peer is drawn, and an unrecognized label reads as available.
pub fn presence_token(presence: &str) -> &'static str {
    let value = presence.trim().to_ascii_lowercase();
    if value.contains("meeting") || value.contains("busy") || value.contains("dnd") {
        "busy"
    } else if value.contains("away") {
        "away"
    } else {
        "online"
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub path: String,
    pub bytes: usize,
    pub created_ms: u64,
}

fn backup_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = crate::instance_data_dir(app)?.join("Backups");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

/// Secrets an archive carries beside the database, as `(bundle field, secret account)`.
///
/// A copy of `messages.db` without its key is a box nobody can open: on another device the
/// restore would write a file that build cannot read. The container is sealed with the user's
/// backup passphrase, so the profile can travel whole — and a restore moves into a new body
/// with the same fingerprint, which is what keeps the peers that pinned it.
const BUNDLED_SECRETS: [(&str, &str); 3] = [
    ("databaseKeyBase64", crate::secrets::DATABASE_KEY),
    ("identityKeyBase64", crate::secrets::IDENTITY_KEY),
    ("lanPassphraseBase64", PASSPHRASE_ACCOUNT),
];

fn bundle_plaintext(app: &AppHandle) -> Result<Vec<u8>, String> {
    let database = crate::instance_data_dir(app)?.join("messages.db");
    let database_bytes = std::fs::read(&database).unwrap_or_default();
    let secrets = crate::secrets::Secrets::open(app)?;
    let settings = app.state::<SettingsState>().snapshot();
    bundle_payload(&settings, database_bytes, &secrets)
}

/// The archive body: settings, the raw database, and the keys that make that database readable.
fn bundle_payload(
    settings: &AppSettings,
    database_bytes: Vec<u8>,
    secrets: &crate::secrets::Secrets,
) -> Result<Vec<u8>, String> {
    let mut payload = serde_json::json!({
        "version": 2,
        "app": alver_core::APP_NAME,
        "settings": settings,
        "databaseBase64": base64_bytes(&database_bytes),
    });
    if let Some(object) = payload.as_object_mut() {
        for (field, account) in BUNDLED_SECRETS {
            if let Some(secret) = secrets.get(account)? {
                object.insert((*field).into(), Value::String(base64_bytes(&secret)));
            }
        }
    }
    serde_json::to_vec(&payload).map_err(|error| error.to_string())
}

fn base64_bytes(bytes: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(bytes.len().div_ceil(3) * 4);
    for chunk in bytes.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = *chunk.get(1).unwrap_or(&0) as u32;
        let b2 = *chunk.get(2).unwrap_or(&0) as u32;
        let group = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[(group >> 18) as usize & 63] as char);
        out.push(ALPHABET[(group >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 {
            ALPHABET[(group >> 6) as usize & 63] as char
        } else {
            '='
        });
        out.push(if chunk.len() > 2 {
            ALPHABET[group as usize & 63] as char
        } else {
            '='
        });
    }
    out
}

fn unbase64_bytes(text: &str) -> Result<Vec<u8>, String> {
    let clean: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    if !clean.len().is_multiple_of(4) {
        return Err("The backup payload is not valid base64.".into());
    }
    let mut out = Vec::with_capacity(clean.len() / 4 * 3);
    for chunk in clean.as_bytes().chunks(4) {
        let mut acc: u32 = 0;
        let mut padding = 0usize;
        for (index, byte) in chunk.iter().enumerate() {
            let value = match byte {
                b'=' => {
                    padding += 1;
                    0
                }
                _ => alver_index(*byte).ok_or("The backup payload is not valid base64.")?,
            };
            acc |= (value as u32) << (18 - index * 6);
        }
        out.push((acc >> 16) as u8);
        if padding < 2 {
            out.push((acc >> 8) as u8);
        }
        if padding < 1 {
            out.push(acc as u8);
        }
    }
    Ok(out)
}

fn alver_index(byte: u8) -> Option<u8> {
    match byte {
        b'A'..=b'Z' => Some(byte - b'A'),
        b'a'..=b'z' => Some(byte - b'a' + 26),
        b'0'..=b'9' => Some(byte - b'0' + 52),
        b'+' => Some(62),
        b'/' => Some(63),
        _ => None,
    }
}

#[tauri::command]
pub fn create_backup(app: AppHandle, passphrase: String) -> Result<BackupResult, String> {
    if passphrase.len() < BACKUP_PASSPHRASE_MIN_LEN {
        return Err(format!(
            "Use a backup passphrase of at least {} characters.",
            BACKUP_PASSPHRASE_MIN_LEN
        ));
    }
    let plaintext = bundle_plaintext(&app)?;
    let bundle = alver_core::crypto::encrypt_backup(&passphrase, &plaintext)?;
    let directory = backup_directory(&app)?;
    let name = format!(
        "alverchat-backup-{}.alverbak",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let path = directory.join(name);
    std::fs::write(&path, &bundle).map_err(|error| error.to_string())?;
    Ok(BackupResult {
        path: path.display().to_string(),
        bytes: bundle.len(),
        created_ms: std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
    })
}

#[tauri::command]
pub fn list_backups(app: AppHandle) -> Result<Vec<BackupResult>, String> {
    let directory = backup_directory(&app)?;
    let mut items = Vec::new();
    for entry in std::fs::read_dir(directory).map_err(|error| error.to_string())? {
        let entry = entry.map_err(|error| error.to_string())?;
        let path = entry.path();
        if path.extension().and_then(|ext| ext.to_str()) == Some("alverbak") {
            let created_ms = entry
                .metadata()
                .and_then(|meta| meta.modified())
                .ok()
                .and_then(|modified| modified.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|since| since.as_millis() as u64)
                .unwrap_or_default();
            items.push(BackupResult {
                path: path.display().to_string(),
                bytes: entry
                    .metadata()
                    .map(|meta| meta.len() as usize)
                    .unwrap_or(0),
                created_ms,
            });
        }
    }
    items.sort_by_key(|item| std::cmp::Reverse(item.created_ms));
    Ok(items)
}

/// Writes an archive into a profile and proves the result opens, putting the profile back the
/// way it was if it does not. `verify` reopens the store the way startup does, which is the only
/// check that the keys inside an archive really belong to the database inside it.
fn install_payload(
    plaintext: &[u8],
    database: &Path,
    secrets: &crate::secrets::Secrets,
    verify: &dyn Fn() -> Result<(), String>,
) -> Result<AppSettings, String> {
    let value: serde_json::Value =
        serde_json::from_slice(plaintext).map_err(|error| error.to_string())?;
    let settings_value = value
        .get("settings")
        .cloned()
        .ok_or_else(|| "The backup has no settings.".to_string())?;
    let restored: AppSettings =
        serde_json::from_value(settings_value).map_err(|error| error.to_string())?;
    let previous_database = std::fs::read(database).unwrap_or_default();
    let previous_secrets = BUNDLED_SECRETS
        .iter()
        .map(|(_, account)| secrets.get(account).map(|secret| (*account, secret)))
        .collect::<Result<Vec<_>, String>>()?;
    for (field, account) in BUNDLED_SECRETS {
        // An archive made before keys travelled with a backup keeps this device's own keys; the
        // store open below is what catches a database they cannot read.
        if let Some(encoded) = value.get(field).and_then(|value| value.as_str()) {
            secrets.set(account, &unbase64_bytes(encoded)?)?;
        }
    }
    if let Some(encoded) = value.get("databaseBase64").and_then(|v| v.as_str()) {
        let bytes = unbase64_bytes(encoded)?;
        if !bytes.is_empty() {
            std::fs::write(database, bytes).map_err(|error| error.to_string())?;
        }
    }
    // Restoring is the one place a wrong key turns into a profile that will not start.
    if let Err(error) = verify() {
        if previous_database.is_empty() {
            let _ = std::fs::remove_file(database);
        } else {
            let _ = std::fs::write(database, &previous_database);
        }
        for (account, secret) in &previous_secrets {
            match secret {
                Some(secret) => {
                    let _ = secrets.set(account, secret);
                }
                None => {
                    let _ = secrets.delete(account);
                }
            }
        }
        return Err(format!(
            "That backup's database does not open with the keys inside it, so nothing changed: {error}"
        ));
    }
    Ok(restored)
}

#[tauri::command]
pub fn restore_backup(app: AppHandle, path: String, passphrase: String) -> Result<(), String> {
    let candidate = Path::new(&path);
    let directory = backup_directory(&app)?;
    if !candidate.starts_with(&directory) {
        return Err("Only backups from the AlverChat backup folder can be restored.".into());
    }
    let bundle = std::fs::read(candidate).map_err(|error| error.to_string())?;
    let plaintext = alver_core::crypto::decrypt_backup(&passphrase, &bundle)?;
    let database = crate::instance_data_dir(&app)?.join("messages.db");
    let secrets = crate::secrets::Secrets::open(&app)?;
    let restored = install_payload(&plaintext, &database, &secrets, &|| {
        crate::open_message_store(&app).map(|_| ())
    })?;
    crate::forget_lan_passphrase(&app);
    let state = app.state::<SettingsState>();
    {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        *guard = restored;
        persist(&app, &guard)?;
    }
    let _ = app.emit("settings-changed", ());
    Ok(())
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportResult {
    pub path: String,
    pub messages: usize,
}

#[tauri::command]
pub fn export_data(app: AppHandle) -> Result<ExportResult, String> {
    let directory = crate::instance_data_dir(&app)?.join("Exports");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    let state = app.state::<crate::OverlayState>();
    let mut chats = serde_json::Map::new();
    let mut count = 0usize;
    {
        let store = state
            .store
            .lock()
            .map_err(|_| "store lock poisoned".to_string())?;
        for peer in store.known_peers().unwrap_or_default() {
            let messages = store.messages_for_chat(&peer.id).unwrap_or_default();
            count += messages.len();
            chats.insert(
                peer.id,
                serde_json::json!({
                    "displayName": peer.name,
                    "messages": messages.iter().map(|message| serde_json::json!({
                        "id": message.id.to_string(),
                        "sender": message.sender,
                        "body": message.body,
                        "tombstone": message.tombstone,
                        "clock": { "physicalMs": message.clock.physical_ms, "logical": message.clock.logical },
                    })).collect::<Vec<_>>(),
                }),
            );
        }
    }
    let name = format!(
        "alverchat-export-{}.json",
        Local::now().format("%Y%m%d-%H%M%S")
    );
    let path = directory.join(name);
    let payload = serde_json::json!({
        "app": alver_core::APP_NAME,
        "version": 1,
        "identity": crate::identity_summary(&app),
        "chats": chats,
    });
    std::fs::write(
        &path,
        serde_json::to_vec_pretty(&payload).map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    Ok(ExportResult {
        path: path.display().to_string(),
        messages: count,
    })
}

#[tauri::command]
pub fn reveal_path(app: AppHandle, path: String) -> Result<(), String> {
    let directory = crate::instance_data_dir(&app)?;
    let candidate = Path::new(&path);
    if !candidate.starts_with(&directory) {
        return Err("Only AlverChat data files can be revealed.".into());
    }
    app.opener()
        .reveal_item_in_dir(candidate)
        .map_err(|error| error.to_string())
}

#[tauri::command]
pub fn set_muted_chat(app: AppHandle, chat_id: String, muted: bool) -> Result<Vec<String>, String> {
    let state = app.state::<SettingsState>();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    guard.muted_chats.retain(|id| id != &chat_id);
    if muted {
        guard.muted_chats.push(chat_id);
    }
    persist(&app, &guard)?;
    Ok(guard.muted_chats.clone())
}

#[tauri::command]
pub fn set_pinned_chat(
    app: AppHandle,
    chat_id: String,
    pinned: bool,
) -> Result<Vec<String>, String> {
    let state = app.state::<SettingsState>();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    guard.pinned_chats.retain(|id| id != &chat_id);
    if pinned {
        guard.pinned_chats.push(chat_id);
    }
    persist(&app, &guard)?;
    Ok(guard.pinned_chats.clone())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_clock_times() {
        assert_eq!(minutes_of_day("07:00"), Some(420));
        assert_eq!(minutes_of_day("22:30"), Some(1350));
        assert_eq!(minutes_of_day("24:00"), None);
        assert_eq!(minutes_of_day("7pm"), None);
    }

    #[test]
    fn the_adapter_choice_falls_back_to_every_interface() {
        let available: Vec<std::net::IpAddr> = ["192.168.1.10", "10.0.0.4"]
            .iter()
            .map(|text| text.parse().unwrap())
            .collect();
        assert_eq!(adapter_choice("auto", &available), None);
        assert_eq!(adapter_choice("", &available), None);
        assert_eq!(adapter_choice("not-an-address", &available), None);
        // A saved choice from a network this machine left must not hide it.
        assert_eq!(adapter_choice("192.168.9.9 (en0)", &available), None);
    }

    #[test]
    fn the_adapter_choice_reads_the_label_the_connection_test_writes() {
        let available: Vec<std::net::IpAddr> = ["192.168.1.10", "10.0.0.4"]
            .iter()
            .map(|text| text.parse().unwrap())
            .collect();
        let en0: std::net::IpAddr = "192.168.1.10".parse().unwrap();
        assert_eq!(adapter_choice("192.168.1.10 (en0)", &available), Some(en0));
        assert_eq!(adapter_choice("en0 (192.168.1.10)", &available), Some(en0));
        assert_eq!(adapter_choice("192.168.1.10", &available), Some(en0));
    }

    fn v4(text: &str) -> std::net::Ipv4Addr {
        text.parse().expect("an IPv4 address")
    }

    #[test]
    fn each_adapter_broadcasts_to_its_own_subnet_with_its_own_address() {
        let adapters = [(v4("192.168.1.10"), Some(v4("192.168.1.255")))];
        assert_eq!(
            discovery_targets(&adapters, "auto", 47_986),
            vec![(
                std::net::IpAddr::V4(v4("192.168.1.10")),
                std::net::SocketAddr::from((v4("192.168.1.255"), 47_986))
            )]
        );
    }

    #[test]
    fn an_interface_with_no_reported_broadcast_still_announces() {
        let adapters = [(v4("10.0.0.4"), None)];
        assert_eq!(
            discovery_targets(&adapters, "auto", 47_986),
            vec![(
                std::net::IpAddr::V4(v4("10.0.0.4")),
                std::net::SocketAddr::from((v4("255.255.255.255"), 47_986))
            )]
        );
    }

    #[test]
    fn a_named_adapter_narrows_the_broadcast_to_itself() {
        let adapters = [
            (v4("192.168.1.10"), Some(v4("192.168.1.255"))),
            (v4("10.0.0.4"), Some(v4("10.0.0.255"))),
        ];
        assert_eq!(
            discovery_targets(&adapters, "10.0.0.4 (en5)", 47_986),
            vec![(
                std::net::IpAddr::V4(v4("10.0.0.4")),
                std::net::SocketAddr::from((v4("10.0.0.255"), 47_986))
            )]
        );
        // Both adapters on one subnet would answer the same broadcast twice.
        assert_eq!(
            discovery_targets(
                &[
                    (v4("192.168.1.10"), Some(v4("192.168.1.255"))),
                    (v4("192.168.1.11"), Some(v4("192.168.1.255"))),
                ],
                "auto",
                47_986
            )
            .len(),
            1
        );
        // A saved choice from a network this machine left keeps it announcing anyway.
        assert_eq!(
            discovery_targets(&adapters, "172.16.0.9 (en0)", 47_986).len(),
            2
        );
        assert!(discovery_targets(&[], "auto", 47_986).is_empty());
    }

    #[test]
    fn only_an_explicit_nobody_hides_the_status() {
        assert!(status_is_private("nobody"));
        assert!(status_is_private(" Nobody "));
        assert!(!status_is_private("everyone"));
        assert!(!status_is_private(""));
    }

    #[test]
    fn presence_labels_reduce_to_the_dot_states() {
        assert_eq!(presence_token("Available"), "online");
        assert_eq!(presence_token("Away"), "away");
        assert_eq!(presence_token("Busy"), "busy");
        assert_eq!(presence_token("In a meeting"), "busy");
        assert_eq!(presence_token(""), "online");
    }

    #[test]
    fn partial_update_keeps_the_other_settings() {
        let current = AppSettings {
            display_name: "Anu".into(),
            theme: "dark".into(),
            ..Default::default()
        };
        let merged = merge_settings(&current, &serde_json::json!({"theme": "light"})).unwrap();
        assert_eq!(merged.theme, "light");
        assert_eq!(merged.display_name, "Anu");
        assert!(merge_settings(&current, &serde_json::json!({"opacityPercent": 500})).is_err());
    }

    #[test]
    fn windows_message_alerts_go_to_the_dock_bubble() {
        assert!(message_alert_uses_dock(true, true, true));
        assert!(!message_alert_uses_dock(true, false, true));
        assert!(!message_alert_uses_dock(true, true, false));
        assert!(!message_alert_uses_dock(false, true, true));
    }

    #[test]
    fn quiet_window_wraps_midnight() {
        let settings = AppSettings {
            dnd_days: vec![
                "Mon".into(),
                "Tue".into(),
                "Wed".into(),
                "Thu".into(),
                "Fri".into(),
            ],
            ..Default::default()
        };
        // 2024-01-03 is a Wednesday.
        let inside = Local
            .with_ymd_and_hms(2024, 1, 3, 23, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let outside = Local
            .with_ymd_and_hms(2024, 1, 3, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let weekend = Local
            .with_ymd_and_hms(2024, 1, 6, 23, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        assert!(dnd_window_active(&settings, inside));
        assert!(!dnd_window_active(&settings, outside));
        assert!(!dnd_window_active(&settings, weekend));
        let off = AppSettings {
            dnd_enabled: false,
            ..settings
        };
        assert!(!dnd_window_active(&off, inside));
    }

    #[test]
    fn muted_chats_are_quiet() {
        let settings = AppSettings {
            dnd_enabled: false,
            ..Default::default()
        };
        assert!(should_notify(&settings, "peer-1", 0));
        let muted = AppSettings {
            muted_chats: vec!["peer-1".into()],
            ..settings.clone()
        };
        assert!(!should_notify(&muted, "peer-1", 0));
        assert!(should_notify(&muted, "peer-2", 0));
    }

    #[test]
    fn the_icon_is_quiet_exactly_when_alerts_stop() {
        // The dock's Do-Not-Disturb ring is built from `icon_is_quiet`, so it must agree
        // with the rule that actually suppresses the alert — otherwise the icon promises
        // quiet while a message still chimes, or the reverse.
        let night = Local
            .with_ymd_and_hms(2024, 1, 3, 23, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let midday = Local
            .with_ymd_and_hms(2024, 1, 3, 12, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis() as u64;
        let dnd = AppSettings::default();
        assert!(icon_is_quiet(&dnd, night));
        assert!(!icon_is_quiet(&dnd, midday));
        let muted = AppSettings {
            notifications_muted: true,
            dnd_enabled: false,
            ..Default::default()
        };
        assert!(icon_is_quiet(&muted, midday));
        for (settings, at) in [(&dnd, night), (&dnd, midday), (&muted, midday)] {
            assert_eq!(
                should_notify(settings, "peer-1", at),
                !icon_is_quiet(settings, at)
            );
        }
        // A single muted thread keeps the icon loud: the device is not quiet.
        let one_chat = AppSettings {
            dnd_enabled: false,
            muted_chats: vec!["peer-1".into()],
            ..Default::default()
        };
        assert!(!icon_is_quiet(&one_chat, midday));
        assert!(!should_notify(&one_chat, "peer-1", midday));
        assert!(should_notify(&one_chat, "peer-2", midday));
    }

    #[test]
    fn hotkey_strings_become_shortcuts() {
        assert!(parse_hotkey("Ctrl+Alt+S").is_ok());
        assert!(parse_hotkey("Command+Alt+S").is_ok());
        assert!(parse_hotkey("Ctrl+Alt").is_err());
        assert!(parse_hotkey("").is_err());
    }

    #[test]
    fn the_default_hotkey_can_always_be_registered() {
        // A default that fails to parse would put the app into the fall-back loop at
        // startup, so the shipped combo has to survive the same parser the recorder uses.
        assert!(parse_hotkey(&default_hotkey()).is_ok());
        assert!(parse_hotkey(&AppSettings::default().hotkey).is_ok());
    }

    #[test]
    fn tray_toggle_labels_match_what_notifications_do() {
        let muted = AppSettings {
            notifications_muted: true,
            dnd_enabled: false,
            ..Default::default()
        };
        assert_eq!(
            tray_toggle_titles(&muted),
            (
                "Mute all notifications: On".into(),
                "Do Not Disturb: Off".into(),
            )
        );
        assert!(!should_notify(&muted, "peer-1", 0));

        let audible = AppSettings {
            notifications_muted: false,
            ..muted
        };
        assert_eq!(
            tray_toggle_titles(&audible).0,
            "Mute all notifications: Off"
        );
        assert!(should_notify(&audible, "peer-1", 0));

        // The Settings screen writes one-flag patches, and the tray label has to follow
        // the patched value rather than the stored one it was built from.
        let patched = merge_settings(
            &AppSettings::default(),
            &serde_json::json!({ "notificationsMuted": true }),
        )
        .unwrap();
        assert_eq!(tray_toggle_titles(&patched).0, "Mute all notifications: On");
    }

    #[test]
    fn base64_round_trips() {
        for size in [0usize, 1, 2, 3, 4, 17, 256] {
            let bytes: Vec<u8> = (0..size).map(|index| (index * 37) as u8).collect();
            let encoded = base64_bytes(&bytes);
            assert_eq!(unbase64_bytes(&encoded).unwrap(), bytes, "size {size}");
        }
        assert!(unbase64_bytes("abc").is_err());
    }

    #[test]
    fn settings_survive_json_round_trip() {
        let settings = AppSettings::default();
        let encoded = serde_json::to_string(&settings).unwrap();
        let decoded: AppSettings = serde_json::from_str(&encoded).unwrap();
        assert_eq!(settings, decoded);
    }

    #[test]
    fn partial_settings_json_uses_defaults() {
        let decoded: AppSettings = serde_json::from_str(r#"{"theme":"dark"}"#).unwrap();
        assert_eq!(decoded.theme, "dark");
        assert_eq!(decoded.opacity_percent, 90);
        assert!(decoded.bubbles_enabled);
    }

    /// A profile directory with its own keys and one database written with them.
    fn profile_with_keys(root: &Path, key: &[u8; 32], body: &str) -> crate::secrets::Secrets {
        std::fs::create_dir_all(root).unwrap();
        let secrets = crate::secrets::Secrets::files(root.to_path_buf());
        secrets.set(crate::secrets::DATABASE_KEY, key).unwrap();
        secrets
            .set(crate::secrets::IDENTITY_KEY, &[key[0]; 32])
            .unwrap();
        secrets.set(PASSPHRASE_ACCOUNT, b"lan-passphrase").unwrap();
        let database = root.join("messages.db");
        let mut store = alver_core::storage::Store::open(database.to_str().unwrap(), key).unwrap();
        store
            .save_message(&alver_core::Message {
                id: uuid::Uuid::now_v7(),
                chat_id: "chat-restore".into(),
                sender: "local".into(),
                body: body.into(),
                clock: alver_core::Hlc {
                    physical_ms: 99,
                    logical: 1,
                },
                tombstone: false,
                forwarded: false,
            })
            .unwrap();
        drop(store);
        secrets
    }

    fn archived_bundle(secrets: &crate::secrets::Secrets, root: &Path) -> Vec<u8> {
        bundle_payload(
            &AppSettings {
                display_name: "Anu".into(),
                theme: "dark".into(),
                ..Default::default()
            },
            std::fs::read(root.join("messages.db")).unwrap(),
            secrets,
        )
        .unwrap()
    }

    fn chat_bodies(database: &Path, key: &[u8; 32]) -> Vec<String> {
        let store = alver_core::storage::Store::open(database.to_str().unwrap(), key).unwrap();
        let bodies = store
            .messages_for_chat("chat-restore")
            .unwrap()
            .into_iter()
            .map(|message| message.body)
            .collect();
        drop(store);
        bodies
    }

    #[test]
    fn a_key_carrying_backup_restores_into_another_profile() {
        let root = std::env::temp_dir().join(format!("alverchat-restore-{}", uuid::Uuid::now_v7()));
        let source = root.join("source");
        let target = root.join("target");
        let source_key = [3u8; 32];
        let target_key = [9u8; 32];

        let source_secrets = profile_with_keys(&source, &source_key, "The invoice is ready.");
        let plaintext = archived_bundle(&source_secrets, &source);

        // A second device that never saw these keys, with a chat of its own.
        let target_secrets = profile_with_keys(&target, &target_key, "Something else entirely.");
        let target_database = target.join("messages.db");
        let verify = || match target_secrets.get(crate::secrets::DATABASE_KEY) {
            Ok(Some(secret)) => {
                let Ok(key) = <[u8; 32]>::try_from(secret) else {
                    return Err("the database key is not 32 bytes".to_string());
                };
                alver_core::storage::Store::open(target_database.to_str().unwrap(), &key)
                    .map(|_| ())
            }
            Ok(None) => Err("this profile has no database key".to_string()),
            Err(error) => Err(error),
        };

        let restored =
            install_payload(&plaintext, &target_database, &target_secrets, &verify).unwrap();
        assert_eq!(restored.display_name, "Anu");
        assert_eq!(restored.theme, "dark");

        // The archive's key replaced the local one, which is what makes its database readable.
        assert_eq!(
            target_secrets
                .get(crate::secrets::DATABASE_KEY)
                .unwrap()
                .as_deref(),
            Some(source_key.as_slice())
        );
        assert_eq!(
            target_secrets.get(PASSPHRASE_ACCOUNT).unwrap().as_deref(),
            Some(b"lan-passphrase".as_slice())
        );
        assert_eq!(
            chat_bodies(&target_database, &source_key),
            ["The invoice is ready."]
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_restore_that_cannot_open_its_database_changes_nothing() {
        let root =
            std::env::temp_dir().join(format!("alverchat-rollback-{}", uuid::Uuid::now_v7()));
        let source = root.join("source");
        let target = root.join("target");
        let target_key = [9u8; 32];

        let source_secrets = profile_with_keys(&source, &[3u8; 32], "Archived chat.");
        let plaintext = archived_bundle(&source_secrets, &source);
        let target_secrets = profile_with_keys(&target, &target_key, "Keep me.");
        let target_database = target.join("messages.db");
        let before = std::fs::read(&target_database).unwrap();

        // A store that will not open is what a key that does not match the database does.
        let error = install_payload(&plaintext, &target_database, &target_secrets, &|| {
            Err("file is not a database".to_string())
        })
        .unwrap_err();
        assert!(error.contains("nothing changed"), "{error}");

        assert_eq!(std::fs::read(&target_database).unwrap(), before);
        assert_eq!(
            target_secrets
                .get(crate::secrets::DATABASE_KEY)
                .unwrap()
                .as_deref(),
            Some(target_key.as_slice())
        );
        assert_eq!(chat_bodies(&target_database, &target_key), ["Keep me."]);
        let _ = std::fs::remove_dir_all(&root);
    }
}
