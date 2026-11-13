#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

#[tauri::command]
fn app_name() -> &'static str {
    alver_core::APP_NAME
}

fn main() {
    tauri::Builder::default()
        .invoke_handler(tauri::generate_handler![app_name])
        .run(tauri::generate_context!())
        .expect("failed to run AlverChat");
}
