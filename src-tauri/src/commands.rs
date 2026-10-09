//! Everything the web view can ask for.

use crate::controller::{self, Mode, Rect, Reveal};
use crate::line::ItemView;
use crate::{actions, emit_items, AppState};
use serde::Serialize;
use std::path::PathBuf;
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

fn path_of(state: &AppState, id: u64) -> Option<PathBuf> {
    state.line.lock().unwrap().get(id).map(|i| i.path.clone())
}

#[tauri::command]
pub fn items(state: State<AppState>) -> Vec<ItemView> {
    state.line.lock().unwrap().views()
}

#[derive(Serialize)]
pub struct Settings {
    mode: Mode,
    sound: bool,
    edge: bool,
    revealed: bool,
    platform: &'static str,
}

#[tauri::command]
pub fn settings(state: State<AppState>) -> Settings {
    let revealed = state.ctl.lock().unwrap().revealed;
    let cfg = state.config.lock().unwrap();
    Settings { mode: state.mode, sound: cfg.sound, edge: cfg.reveal_at_edge, revealed, platform: std::env::consts::OS }
}

/// Click: copy the image.
#[tauri::command]
pub fn copy(app: AppHandle, state: State<AppState>, id: u64) {
    if let Some(path) = path_of(&state, id) {
        state.clipboard.copy(&path);
        let _ = app.emit("copied", id);
    }
}

/// Double click: open it in the default viewer.
#[tauri::command]
pub fn open(state: State<AppState>, id: u64) {
    if let Some(path) = path_of(&state, id) {
        actions::open(&path);
    }
}

/// Press and hold: mark it up.
#[tauri::command]
pub fn edit(state: State<AppState>, id: u64) {
    if let Some(path) = path_of(&state, id) {
        actions::edit(&path);
    }
}

#[tauri::command]
pub fn show_in_folder(state: State<AppState>, id: u64) {
    if let Some(path) = path_of(&state, id) {
        actions::show_in_folder(&path);
    }
}

/// The cross: off the line. The file stays where it is.
#[tauri::command]
pub fn take_down(app: AppHandle, id: u64) {
    take_down_item(&app, id);
}

fn take_down_item(app: &AppHandle, id: u64) {
    let removed = app.state::<AppState>().line.lock().unwrap().remove(id).is_some();
    if removed {
        let _ = app.emit("fell", id);
        emit_items(app);
        crate::tuck_if_empty(app);
    }
}

/// Off the line and into the trash.
#[tauri::command]
pub fn trash(app: AppHandle, state: State<AppState>, id: u64) {
    if let Some(path) = path_of(&state, id) {
        if actions::trash(&path) {
            take_down_item(&app, id);
        }
    }
}

pub fn clear_line(app: &AppHandle) {
    app.state::<AppState>().line.lock().unwrap().clear();
    let _ = app.emit("cleared", ());
    emit_items(app);
    crate::tuck_if_empty(app);
}

/// Right click: everything else, in a native menu.
#[tauri::command]
pub fn menu(app: AppHandle, window: WebviewWindow, id: u64, x: f64, y: f64) -> Result<(), String> {
    let m = |key: &str, label: &str| MenuItem::with_id(&app, format!("{key}:{id}"), label, true, None::<&str>);
    let menu = Menu::with_items(
        &app,
        &[
            &m("copy", "Copy").map_err(|e| e.to_string())?,
            &m("open", "Open").map_err(|e| e.to_string())?,
            &m("edit", "Mark Up").map_err(|e| e.to_string())?,
            &m("folder", show_in_folder_label()).map_err(|e| e.to_string())?,
            &PredefinedMenuItem::separator(&app).map_err(|e| e.to_string())?,
            &m("down", "Take Down").map_err(|e| e.to_string())?,
            &m("trash", trash_label()).map_err(|e| e.to_string())?,
        ],
    )
    .map_err(|e| e.to_string())?;
    // Keep the line down while the menu is open.
    app.state::<AppState>().ctl.lock().unwrap().inside = true;
    // At the click, in window coordinates: Wayland will not say where the
    // pointer is.
    window
        .popup_menu_at(&menu, tauri::LogicalPosition::new(x, y))
        .map_err(|e| e.to_string())
}

fn show_in_folder_label() -> &'static str {
    if cfg!(target_os = "macos") {
        "Show in Finder"
    } else if cfg!(target_os = "windows") {
        "Show in File Explorer"
    } else {
        "Show in Folder"
    }
}

fn trash_label() -> &'static str {
    if cfg!(target_os = "windows") {
        "Move to Recycle Bin"
    } else {
        "Move to Trash"
    }
}

pub fn on_menu(app: &AppHandle, menu_id: &str) {
    if crate::tray::on_menu(app, menu_id) {
        return;
    }
    let Some((key, id)) = menu_id.split_once(':') else { return };
    let Ok(id) = id.parse::<u64>() else { return };
    let state = app.state::<AppState>();
    match key {
        "copy" => copy(app.clone(), state, id),
        "open" => open(state, id),
        "edit" => edit(state, id),
        "folder" => show_in_folder(state, id),
        "down" => take_down_item(app, id),
        "trash" => trash(app.clone(), state, id),
        _ => {}
    }
}

/// Dragging a photo out hands the file to whatever it is dropped on. It
/// stays on the line. Plain (not async) commands run on the main thread,
/// which is where a drag has to start.
#[tauri::command]
pub fn start_drag(app: AppHandle, window: WebviewWindow, state: State<AppState>, id: u64) -> Result<(), String> {
    let path = path_of(&state, id).ok_or("gone")?;
    let preview = crate::line::drag_preview(&path, &state.cache_dir).unwrap_or_else(|| path.clone());
    state.ctl.lock().unwrap().dragging = true;
    let _ = window.set_ignore_cursor_events(false);

    let done = app.clone();
    let on_drop = move |result: drag::DragResult, _pos: drag::CursorPosition| {
        log::info!("Drag ended: {result:?}");
        done.state::<AppState>().ctl.lock().unwrap().dragging = false;
        let _ = done.emit("drag-end", id);
    };
    let item = drag::DragItem::Files(vec![path]);
    let image = drag::Image::File(preview);
    #[cfg(target_os = "linux")]
    let res = {
        let gtk = window.gtk_window().map_err(|e| e.to_string())?;
        drag::start_drag(&gtk, item, image, on_drop, drag::Options::default())
    };
    #[cfg(not(target_os = "linux"))]
    let res = drag::start_drag(&window, item, image, on_drop, drag::Options::default());

    res.map_err(|e| {
        state.ctl.lock().unwrap().dragging = false;
        let _ = app.emit("drag-end", id);
        e.to_string()
    })
}

/// Where the photos are, in window coordinates, for letting clicks through
/// everywhere else (macOS, Windows).
#[tauri::command]
pub fn hit_rects(state: State<AppState>, rects: Vec<Rect>) {
    state.ctl.lock().unwrap().hit_rects = rects;
}

/// The part of the window that takes the pointer (Linux).
#[tauri::command]
pub fn input_region(window: WebviewWindow, state: State<AppState>, rects: Vec<Rect>) {
    #[cfg(target_os = "linux")]
    if state.mode == Mode::Shape {
        crate::linux::set_input_region(&window, rects);
    }
    #[cfg(not(target_os = "linux"))]
    let _ = (window, state, rects);
}

/// The pointer rested on the strip at the top edge (Linux).
#[tauri::command]
pub fn edge(app: AppHandle, state: State<AppState>) {
    if state.line.lock().unwrap().len() > 0 && state.config.lock().unwrap().reveal_at_edge {
        controller::reveal(&app, Reveal::Hover);
    }
}
