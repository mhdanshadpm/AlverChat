#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use uuid::Uuid;

#[derive(Serialize)]
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

#[derive(Serialize)]
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
}

#[tauri::command]
fn app_name() -> &'static str {
    alver_core::APP_NAME
}

#[tauri::command]
fn list_peers() -> Vec<PeerDto> {
    Vec::new()
}

#[tauri::command]
fn list_messages(_chat_id: String) -> Vec<MessageDto> {
    Vec::new()
}

#[tauri::command]
fn send_message(_chat_id: String, text: String) -> MessageDto {
    MessageDto {
        id: Uuid::now_v7().to_string(),
        sender: "You".into(),
        text,
        time: "Now".into(),
        mine: true,
        read: false,
        kind: "text".into(),
        file_name: None,
    }
}

#[tauri::command]
fn set_theme(_theme: String) {}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![
            app_name,
            list_peers,
            list_messages,
            send_message,
            set_theme
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AlverChat");
}
