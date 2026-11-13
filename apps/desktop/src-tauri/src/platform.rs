//! OS-facing behavior for the desktop shell: persisted app settings, global
//! hotkey, launch-at-login, native notifications with the Do-Not-Disturb
//! schedule, permission deep links, connection diagnostics and backups.
//!
//! Everything here is platform glue. Protocol and storage rules stay in
//! `alver-core`.

use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};
use tauri_plugin_notification::NotificationExt;
use tauri_plugin_opener::OpenerExt;

pub const SETTINGS_FILE: &str = "settings.json";
pub const PASSPHRASE_SERVICE: &str = "AlverChat";
pub const PASSPHRASE_ACCOUNT: &str = "lan-passphrase";

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

pub struct SettingsState(pub Arc<Mutex<AppSettings>>);

impl SettingsState {
    pub fn snapshot(&self) -> AppSettings {
        self.0.lock().map(|value| value.clone()).unwrap_or_default()
    }
}

fn settings_path(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_config_dir()
        .map_err(|error| error.to_string())?;
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

/// A chat stays quiet when the app is muted, muted per chat, or inside the
/// scheduled quiet window.
pub fn should_notify(settings: &AppSettings, chat_id: &str, now_ms: u64) -> bool {
    !settings.notifications_muted
        && !dnd_window_active(settings, now_ms)
        && !settings.muted_chats.iter().any(|id| id == chat_id)
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
        let visible = dock.is_visible().unwrap_or(false);
        if visible {
            let _ = dock.hide();
        } else {
            let _ = dock.show();
            let _ = dock.set_always_on_top(true);
        }
        let _ = app.emit("dock-visibility", !visible);
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

pub fn notify_message(app: &AppHandle, title: &str, body: &str) {
    let settings = app.state::<SettingsState>().snapshot();
    let preview = if settings.show_message_preview {
        body
    } else {
        "New message"
    };
    let mut builder = app.notification().builder().title(title).body(preview);
    if settings.notification_sound {
        builder = builder.sound("default");
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
fn deep_link(kind: &str) -> String {
    match kind {
        "notifications" => "x-apple-system-preferences://".into(),
        _ => "https://localhost/".into(),
    }
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

#[tauri::command]
pub fn update_settings(app: AppHandle, patch: AppSettings) -> Result<AppSettings, String> {
    let state = app.state::<SettingsState>();
    let previous = state.snapshot();
    {
        let mut guard = state
            .0
            .lock()
            .map_err(|_| "settings lock poisoned".to_string())?;
        *guard = patch;
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
    let entry = keyring::Entry::new(PASSPHRASE_SERVICE, PASSPHRASE_ACCOUNT)
        .map_err(|error| format!("Cannot access the credential store: {error}"))?;
    if passphrase.is_empty() {
        match entry.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => {}
            Err(error) => return Err(format!("Cannot clear the passphrase: {error}")),
        }
    } else {
        if passphrase.len() > 256 {
            return Err("The LAN passphrase is too long.".into());
        }
        entry
            .set_secret(passphrase.as_bytes())
            .map_err(|error| format!("Cannot save the passphrase: {error}"))?;
    }
    let state = app.state::<SettingsState>();
    let mut guard = state
        .0
        .lock()
        .map_err(|_| "settings lock poisoned".to_string())?;
    guard.lan_encryption = !passphrase.is_empty();
    persist(&app, &guard)?;
    Ok(!passphrase.is_empty())
}

#[tauri::command]
pub fn has_lan_passphrase() -> bool {
    keyring::Entry::new(PASSPHRASE_SERVICE, PASSPHRASE_ACCOUNT)
        .and_then(|entry| entry.get_secret())
        .map(|secret| !secret.is_empty())
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
    ConnectionReport {
        listening: port != 0,
        port,
        peers: peers.len(),
        online_peers: peers.iter().filter(|peer| peer.online).count(),
        adapters,
        discovery_port: alver_core::discovery::DISCOVERY_PORT,
        warnings,
    }
}

pub fn local_ipv4_addresses() -> Vec<String> {
    if_addrs::get_if_addrs()
        .map(|interfaces| {
            interfaces
                .iter()
                .filter(|interface| interface.ip().is_ipv4() && !interface.is_loopback())
                .map(|interface| format!("{} ({})", interface.ip(), interface.name))
                .collect()
        })
        .unwrap_or_default()
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackupResult {
    pub path: String,
    pub bytes: usize,
    pub created_ms: u64,
}

fn backup_directory(app: &AppHandle) -> Result<PathBuf, String> {
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("Backups");
    std::fs::create_dir_all(&directory).map_err(|error| error.to_string())?;
    Ok(directory)
}

fn bundle_plaintext(app: &AppHandle) -> Result<Vec<u8>, String> {
    let data_dir = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
    let settings = app.state::<SettingsState>().snapshot();
    let database = data_dir.join("messages.db");
    let database_bytes = std::fs::read(&database).unwrap_or_default();
    let payload = serde_json::json!({
        "version": 1,
        "app": alver_core::APP_NAME,
        "settings": settings,
        "databaseBase64": base64_bytes(&database_bytes),
    });
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
    if passphrase.len() < 8 {
        return Err("Use a backup passphrase of at least 8 characters.".into());
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

#[tauri::command]
pub fn restore_backup(app: AppHandle, path: String, passphrase: String) -> Result<(), String> {
    let candidate = Path::new(&path);
    let directory = backup_directory(&app)?;
    if !candidate.starts_with(&directory) {
        return Err("Only backups from the AlverChat backup folder can be restored.".into());
    }
    let bundle = std::fs::read(candidate).map_err(|error| error.to_string())?;
    let plaintext = alver_core::crypto::decrypt_backup(&passphrase, &bundle)?;
    let value: serde_json::Value =
        serde_json::from_slice(&plaintext).map_err(|error| error.to_string())?;
    let settings_value = value
        .get("settings")
        .cloned()
        .ok_or_else(|| "The backup has no settings.".to_string())?;
    let restored: AppSettings =
        serde_json::from_value(settings_value).map_err(|error| error.to_string())?;
    if let Some(encoded) = value.get("databaseBase64").and_then(|v| v.as_str()) {
        let bytes = unbase64_bytes(encoded)?;
        if !bytes.is_empty() {
            let database = app
                .path()
                .app_data_dir()
                .map_err(|error| error.to_string())?
                .join("messages.db");
            std::fs::write(&database, bytes).map_err(|error| error.to_string())?;
        }
    }
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
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?
        .join("Exports");
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
    let directory = app
        .path()
        .app_data_dir()
        .map_err(|error| error.to_string())?;
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
    fn hotkey_strings_become_shortcuts() {
        assert!(parse_hotkey("Ctrl+Alt+S").is_ok());
        assert!(parse_hotkey("Command+Alt+S").is_ok());
        assert!(parse_hotkey("Ctrl+Alt").is_err());
        assert!(parse_hotkey("").is_err());
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
}
