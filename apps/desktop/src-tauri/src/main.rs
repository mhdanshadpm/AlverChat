#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod drag;

use drag::{Edge, Rect};
use serde::Serialize;
use std::{collections::HashMap, fs, path::PathBuf, sync::Mutex};
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
    dock_side: Edge,
    snap_to_edge: bool,
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
    snap_to_edge: bool,
}
struct OverlayState {
    compact_open: Mutex<bool>,
    peers: Mutex<Vec<PeerDto>>,
    messages: Mutex<HashMap<String, Vec<MessageDto>>>,
    preferences: Mutex<OverlayPreferences>,
    preferences_path: PathBuf,
    active_drag: Mutex<Option<String>>,
    pointer_capture: Mutex<HashMap<String, bool>>,
    compact_detached: Mutex<bool>,
    cursor_capture: Mutex<HashMap<String, bool>>,
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
        snap_to_edge: state
            .preferences
            .lock()
            .map(|p| p.snap_to_edge)
            .unwrap_or(true),
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
            persist_preferences(&app, &prefs);
        }
    }
    publish(&app);
    Ok(())
}
#[tauri::command]
fn set_snap_to_edge(app: AppHandle, enabled: bool) {
    if let Ok(mut prefs) = app.state::<OverlayState>().preferences.lock() {
        prefs.snap_to_edge = enabled;
        persist_preferences(&app, &prefs);
    }
    publish(&app);
}

fn persist_preferences(app: &AppHandle, prefs: &OverlayPreferences) {
    if let Some(state) = app.try_state::<OverlayState>() {
        let _ = fs::write(
            &state.preferences_path,
            serde_json::to_vec_pretty(prefs).unwrap_or_default(),
        );
    }
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
        prefs_snapshot.snap_to_edge,
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
        persist_preferences(app, &prefs);
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
            persist_preferences(app, &prefs);
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
        .setup(|app| {
            let preferences_path = app.path().app_config_dir()?.join("overlay-settings.json");
            if let Some(parent) = preferences_path.parent() {
                fs::create_dir_all(parent)?;
            }
            let preferences = fs::read(&preferences_path)
                .ok()
                .and_then(|data| serde_json::from_slice(&data).ok())
                .unwrap_or(OverlayPreferences {
                    dock_position: None,
                    snap_to_edge: true,
                });
            app.manage(OverlayState {
                compact_open: Mutex::new(false),
                peers: Mutex::new(peers()),
                messages: Mutex::new(HashMap::new()),
                preferences: Mutex::new(preferences),
                preferences_path,
                active_drag: Mutex::new(None),
                pointer_capture: Mutex::new(HashMap::new()),
                compact_detached: Mutex::new(false),
                cursor_capture: Mutex::new(HashMap::new()),
            });
            let show = MenuItem::with_id(app, "show", "Open AlverChat", true, None::<&str>)?;
            let reset = MenuItem::with_id(
                app,
                "reset-position",
                "Reset icon position",
                true,
                None::<&str>,
            )?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let tray_menu = Menu::with_items(app, &[&show, &reset, &quit])?;
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
            set_theme,
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
            resize_dock,
            open_dock_menu,
            hide_menu,
            quit_app
        ])
        .run(tauri::generate_context!())
        .expect("failed to run AlverChat");
}
