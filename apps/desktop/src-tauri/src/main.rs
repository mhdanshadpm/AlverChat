#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use serde::Serialize;
use std::{collections::HashMap, sync::Mutex};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use uuid::Uuid;

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
}
#[derive(Serialize, Clone)]
#[serde(rename_all = "camelCase")]
struct OverlaySnapshot {
    peers: Vec<PeerDto>,
    unread_total: u32,
    open: bool,
}
struct OverlayState {
    compact_open: Mutex<bool>,
    peers: Mutex<Vec<PeerDto>>,
    messages: Mutex<HashMap<String, Vec<MessageDto>>>,
}

fn peers() -> Vec<PeerDto> {
    vec![
        PeerDto {
            id: "arjun".into(),
            name: "Arjun Kumar".into(),
            initials: "AK".into(),
            color: "peach".into(),
            online: true,
            status: "Available".into(),
            preview: "Invoice updated. Please check.".into(),
            time: "11:24 AM".into(),
            unread: Some(3),
            group: false,
            typing: false,
            muted: false,
            pinned: true,
        },
        PeerDto {
            id: "billing".into(),
            name: "Billing Team".into(),
            initials: "BT".into(),
            color: "violet".into(),
            online: true,
            status: "12 members · 4 online".into(),
            preview: "Anu: Purchase entry completed.".into(),
            time: "11:20 AM".into(),
            unread: Some(5),
            group: true,
            typing: false,
            muted: false,
            pinned: true,
        },
        PeerDto {
            id: "fathima".into(),
            name: "Fathima P".into(),
            initials: "FP".into(),
            color: "rose".into(),
            online: true,
            status: "Typing…".into(),
            preview: "Can you share the stock report?".into(),
            time: "11:18 AM".into(),
            unread: None,
            group: false,
            typing: true,
            muted: false,
            pinned: false,
        },
        PeerDto {
            id: "sarath".into(),
            name: "Sarath S".into(),
            initials: "SS".into(),
            color: "green".into(),
            online: true,
            status: "Available".into(),
            preview: "Video ready, please review.".into(),
            time: "10:22 AM".into(),
            unread: Some(1),
            group: false,
            typing: false,
            muted: false,
            pinned: false,
        },
    ]
}
fn snapshot(app: &AppHandle) -> OverlaySnapshot {
    let state = app.state::<OverlayState>();
    let open = state.compact_open.lock().map(|v| *v).unwrap_or(false);
    let peers = state.peers.lock().map(|v| v.clone()).unwrap_or_default();
    let unread_total = peers.iter().map(|peer| peer.unread.unwrap_or(0)).sum();
    OverlaySnapshot {
        peers,
        unread_total,
        open,
    }
}
fn publish(app: &AppHandle) {
    let _ = app.emit("overlay-state", snapshot(app));
}

#[tauri::command]
fn app_name() -> &'static str {
    alver_core::APP_NAME
}
#[tauri::command]
fn list_peers(app: AppHandle) -> Vec<PeerDto> {
    app.state::<OverlayState>()
        .peers
        .lock()
        .map(|v| v.clone())
        .unwrap_or_default()
}
#[tauri::command]
fn list_messages(app: AppHandle, chat_id: String) -> Vec<MessageDto> {
    app.state::<OverlayState>()
        .messages
        .lock()
        .ok()
        .and_then(|messages| messages.get(&chat_id).cloned())
        .unwrap_or_default()
}
#[tauri::command]
fn send_message(app: AppHandle, chat_id: String, text: String) -> MessageDto {
    let msg = MessageDto {
        id: Uuid::now_v7().to_string(),
        sender: "You".into(),
        text,
        time: "Now".into(),
        mine: true,
        read: false,
        kind: "text".into(),
        file_name: None,
    };
    if let Ok(mut messages) = app.state::<OverlayState>().messages.lock() {
        messages.entry(chat_id).or_default().push(msg.clone());
    }
    publish(&app);
    msg
}
#[tauri::command]
fn set_theme(_theme: String) {}
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
    window
        .set_ignore_cursor_events(!has_hit)
        .map_err(|e| e.to_string())
}
#[tauri::command]
fn start_overlay_drag(window: WebviewWindow) -> Result<(), String> {
    window.start_dragging().map_err(|e| e.to_string())
}
#[tauri::command]
fn resize_dock(app: AppHandle, has_bubbles: bool) -> Result<(), String> {
    let Some(dock) = app.get_webview_window("dock") else {
        return Ok(());
    };
    let size = if has_bubbles {
        tauri::LogicalSize::new(440.0, 220.0)
    } else {
        tauri::LogicalSize::new(84.0, 84.0)
    };
    dock.set_size(size).map_err(|e| e.to_string())?;
    if let Some(monitor) = dock
        .current_monitor()
        .map_err(|e| e.to_string())?
        .or(app.primary_monitor().map_err(|e| e.to_string())?)
    {
        let area = monitor.work_area();
        let scale = monitor.scale_factor();
        let width = if has_bubbles { 440.0 } else { 84.0 };
        let x = area.position.x as f64 / scale + area.size.width as f64 / scale - width - 12.0;
        let y = area.position.y as f64 / scale + area.size.height as f64 / scale / 2.0
            - size.height / 2.0;
        dock.set_position(tauri::LogicalPosition::new(x, y))
            .map_err(|e| e.to_string())?;
    }
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
    let dock_w = (72.0 * monitor.scale_factor()) as i32;
    let left = area.position.x;
    let right = left + area.size.width as i32;
    let dock_width = dock
        .outer_size()
        .map(|size| size.width as i32)
        .unwrap_or(dock_w);
    let on_right = dock_pos.x + dock_width / 2 >= left + area.size.width as i32 / 2;
    let icon_left = if on_right {
        dock_pos.x + dock_width - dock_w
    } else {
        dock_pos.x + (12.0 * monitor.scale_factor()) as i32
    };
    let x = if on_right && right - (icon_left + dock_w) >= panel_w + gap {
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

fn main() {
    tauri::Builder::default()
        .manage(OverlayState {
            compact_open: Mutex::new(false),
            peers: Mutex::new(peers()),
            messages: Mutex::new(HashMap::new()),
        })
        .setup(|app| {
            let show = MenuItem::with_id(app, "show", "Open AlverChat", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show, &quit])?;
            let icon = tauri::image::Image::from_bytes(include_bytes!("../icons/icon.png"))?;
            TrayIconBuilder::new()
                .icon(icon)
                .menu(&tray_menu)
                .on_menu_event(|app, event| match event.id().as_ref() {
                    "show" => {
                        if let Some(main) = app.get_webview_window("main") {
                            let _ = main.show();
                            let _ = main.set_focus();
                        }
                    }
                    "quit" => app.exit(0),
                    _ => {}
                })
                .build(app)?;
            build_overlay(app, "dock", "dock", 84.0, 84.0, true)?;
            build_overlay(app, "compact", "compact", 404.0, 664.0, false)?;
            build_overlay(app, "menu", "menu", 264.0, 176.0, false)?;
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
            set_theme,
            overlay_snapshot,
            toggle_compact,
            hide_compact,
            open_full_app,
            set_overlay_hit_regions,
            start_overlay_drag,
            resize_dock,
            open_dock_menu,
            hide_menu,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AlverChat");
}
