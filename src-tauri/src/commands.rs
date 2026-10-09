//! Everything the web views can ask for.

use crate::actions::{self, Clip};
use crate::controller::{self, Mode, Rect, Reveal};
use crate::history::Hit;
use crate::line::{ItemView, Kind};
use crate::{emit_items, emit_line, toast, AppState};
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::{AppHandle, Emitter, Manager, State, WebviewWindow};

fn item_of(state: &AppState, id: u64) -> Option<(PathBuf, Kind)> {
    state.lines.lock().unwrap().get(id).map(|i| (i.path.clone(), i.kind))
}

fn path_of(state: &AppState, id: u64) -> Option<PathBuf> {
    item_of(state, id).map(|(p, _)| p)
}

/// Slow work (reading text, writing images, running your commands) runs off
/// the main thread so the line keeps moving.
fn background(f: impl FnOnce() + Send + 'static) {
    std::thread::spawn(f);
}

#[tauri::command]
pub fn items(state: State<AppState>) -> Vec<ItemView> {
    state.lines.lock().unwrap().views()
}

#[derive(Serialize)]
pub struct Settings {
    mode: Mode,
    sound: bool,
    edge: bool,
    revealed: bool,
    platform: &'static str,
    theme: crate::theme::Theme,
    actions: Vec<String>,
    line: crate::LineInfo,
    can_read_text: bool,
}

#[tauri::command]
pub fn settings(app: AppHandle, state: State<AppState>) -> Settings {
    let revealed = state.ctl.lock().unwrap().revealed;
    let cfg = state.config.lock().unwrap().clone();
    Settings {
        mode: state.mode,
        sound: cfg.sound,
        edge: cfg.reveal_at_edge,
        revealed,
        platform: std::env::consts::OS,
        theme: crate::theme::current(&cfg.theme),
        actions: cfg.actions.iter().map(|a| a.name.clone()).collect(),
        line: crate::line_info(&app),
        can_read_text: crate::ocr::available(),
    }
}

/// Click: copy the image, or the file for a recording.
#[tauri::command]
pub fn copy(app: AppHandle, state: State<AppState>, id: u64) {
    if let Some((path, kind)) = item_of(&state, id) {
        state.clipboard.put(if kind == Kind::Video { Clip::Files(vec![path]) } else { Clip::Image(path) });
        let _ = app.emit("copied", id);
    }
}

/// Alt-click: copy the text in the picture.
#[tauri::command]
pub fn copy_text(app: AppHandle, state: State<AppState>, id: u64) {
    let Some((path, kind)) = item_of(&state, id) else { return };
    if kind == Kind::Video {
        toast(&app, "Recordings have no text to copy");
        return;
    }
    let language = state.config.lock().unwrap().ocr_language.clone();
    let _ = app.emit("busy", id);
    background(move || {
        let st = app.state::<AppState>();
        match crate::ocr::recognize(&path, &language) {
            Ok(lines) => {
                let text = crate::ocr::text_of(&lines);
                st.history.lock().unwrap().set_text(&path, text.clone());
                if text.trim().is_empty() {
                    toast(&app, "No text found");
                } else {
                    let words = text.split_whitespace().count();
                    st.clipboard.put(Clip::Text(text));
                    let _ = app.emit("copied", id);
                    toast(&app, format!("Copied {words} word{}", if words == 1 { "" } else { "s" }));
                }
            }
            Err(e) => toast(&app, e),
        }
        let _ = app.emit("idle", id);
    });
}

/// Double click: open it in the default viewer or player.
#[tauri::command]
pub fn open(state: State<AppState>, id: u64) {
    if let Some(path) = path_of(&state, id) {
        actions::open(&path);
    }
}

/// Press and hold: mark it up. A recording simply opens.
#[tauri::command]
pub fn edit(state: State<AppState>, id: u64) {
    if let Some((path, kind)) = item_of(&state, id) {
        if kind == Kind::Video {
            actions::open(&path);
        } else {
            actions::edit(&path);
        }
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
pub fn take_down(app: AppHandle, ids: Vec<u64>) {
    for id in ids {
        take_down_item(&app, id);
    }
}

fn take_down_item(app: &AppHandle, id: u64) {
    let removed = app.state::<AppState>().lines.lock().unwrap().remove(id).is_some();
    if removed {
        let _ = app.emit("fell", id);
        emit_items(app);
        crate::tuck_if_empty(app);
    }
}

/// Off the line and into the trash.
#[tauri::command]
pub fn trash(app: AppHandle, state: State<AppState>, ids: Vec<u64>) {
    for id in ids {
        if let Some(path) = path_of(&state, id) {
            if actions::trash(&path) {
                state.history.lock().unwrap().forget(&path);
                take_down_item(&app, id);
            }
        }
    }
}

/// Keep: stays on the line and is never cleaned up.
#[tauri::command]
pub fn pin(app: AppHandle, state: State<AppState>, id: u64) {
    let toggled = state.lines.lock().unwrap().toggle_pin(id);
    if let Some((path, kept)) = toggled {
        state.history.lock().unwrap().set_kept(&path, kept);
        emit_items(&app);
        toast(&app, if kept { "Kept" } else { "No longer kept" });
    }
}

/// Blurs emails, numbers, keys and the like into a new copy, which then
/// hangs on the line by itself.
#[tauri::command]
pub fn redact(app: AppHandle, state: State<AppState>, id: u64) {
    let Some((path, kind)) = item_of(&state, id) else { return };
    if kind != Kind::Image {
        toast(&app, "Only still pictures can be redacted");
        return;
    }
    let language = state.config.lock().unwrap().ocr_language.clone();
    let _ = app.emit("busy", id);
    background(move || {
        match crate::images::redact(&path, &language) {
            Ok((Some(_), n)) => toast(&app, format!("Blurred {n} item{} in a copy", if n == 1 { "" } else { "s" })),
            Ok((None, _)) => toast(&app, "Nothing sensitive found"),
            Err(e) => toast(&app, e),
        }
        let _ = app.emit("idle", id);
    });
}

/// Several pictures joined into one, which then hangs on the line.
#[tauri::command]
pub fn join(app: AppHandle, state: State<AppState>, ids: Vec<u64>, vertical: bool) {
    let paths: Vec<PathBuf> =
        ids.iter().filter_map(|id| item_of(&state, *id)).filter(|(_, k)| *k != Kind::Video).map(|(p, _)| p).collect();
    if paths.len() < 2 {
        toast(&app, "Pick two or more pictures to join");
        return;
    }
    toast(&app, "Joining…");
    background(move || {
        if let Err(e) = crate::images::join(&paths, vertical) {
            toast(&app, e);
        }
    });
}

#[derive(Serialize, Clone)]
pub struct Shared {
    id: u64,
    url: String,
    svg: String,
    minutes: u64,
}

/// Send to phone: a QR code for a link on the local network.
#[tauri::command]
pub fn share(state: State<AppState>, id: u64) -> Result<Shared, String> {
    let path = path_of(&state, id).ok_or("gone")?;
    let s = crate::share::share(&path)?;
    Ok(Shared { id, url: s.url, svg: s.svg, minutes: s.minutes })
}

/// One of your own actions from the settings.
#[tauri::command]
pub fn run_action(app: AppHandle, state: State<AppState>, id: u64, index: usize) {
    let Some(path) = path_of(&state, id) else { return };
    let Some(action) = state.config.lock().unwrap().actions.get(index).cloned() else { return };
    let _ = app.emit("busy", id);
    background(move || {
        match actions::run_custom(&action.command, &path) {
            Ok(out) => match actions::find_link(&out) {
                Some(link) => {
                    app.state::<AppState>().clipboard.put(Clip::Text(link));
                    toast(&app, format!("{}: link copied", action.name));
                }
                None => toast(&app, format!("{}: done", action.name)),
            },
            Err(e) => toast(&app, format!("{}: {e}", action.name)),
        }
        let _ = app.emit("idle", id);
    });
}

pub fn clear_line(app: &AppHandle) {
    app.state::<AppState>().lines.lock().unwrap().clear();
    let _ = app.emit("cleared", ());
    emit_items(app);
    crate::tuck_if_empty(app);
}

pub fn switch_line(app: &AppHandle, delta: i64) {
    {
        let st = app.state::<AppState>();
        let mut lines = st.lines.lock().unwrap();
        let n = lines.names().len() as i64;
        if n < 2 {
            return;
        }
        let next = (lines.active() as i64 + delta).rem_euclid(n) as usize;
        lines.set_active(next);
    }
    emit_line(app);
    emit_items(app);
    controller::reveal(app, Reveal::Pin);
}

#[tauri::command]
pub fn next_line(app: AppHandle, delta: i64) {
    switch_line(&app, delta);
}

/// Right click on macOS and Windows: everything else, in a native menu.
#[tauri::command]
pub fn menu(
    app: AppHandle,
    window: WebviewWindow,
    state: State<AppState>,
    id: u64,
    selected: Vec<u64>,
    x: f64,
    y: f64,
) -> Result<(), String> {
    let e = |e: tauri::Error| e.to_string();
    let item = |key: &str, label: &str| MenuItem::with_id(&app, format!("{key}:{id}"), label, true, None::<&str>);
    let pinned = state.lines.lock().unwrap().get(id).is_some_and(|i| i.pinned);
    let menu = Menu::new(&app).map_err(e)?;
    if selected.len() > 1 {
        let list = selected.iter().map(u64::to_string).collect::<Vec<_>>().join(",");
        let many = |key: &str, label: &str| MenuItem::with_id(&app, format!("{key}:{list}"), label, true, None::<&str>);
        menu.append(&many("join-v", &format!("Join {} Top to Bottom", selected.len())).map_err(e)?).map_err(e)?;
        menu.append(&many("join-h", &format!("Join {} Side by Side", selected.len())).map_err(e)?).map_err(e)?;
        menu.append(&many("down-many", &format!("Take Down {}", selected.len())).map_err(e)?).map_err(e)?;
        menu.append(&PredefinedMenuItem::separator(&app).map_err(e)?).map_err(e)?;
    }
    for (key, label) in [
        ("copy", "Copy"),
        ("text", "Copy Text"),
        ("open", "Open"),
        ("edit", "Mark Up"),
        ("redact", "Redact"),
        ("pin", if pinned { "Don't Keep" } else { "Keep" }),
        ("share", "Send to Phone"),
        ("folder", show_in_folder_label()),
    ] {
        menu.append(&item(key, label).map_err(e)?).map_err(e)?;
    }
    let custom = state.config.lock().unwrap().actions.clone();
    if !custom.is_empty() {
        let sub = Submenu::new(&app, "Actions", true).map_err(e)?;
        for (i, a) in custom.iter().enumerate() {
            sub.append(&MenuItem::with_id(&app, format!("act{i}:{id}"), &a.name, true, None::<&str>).map_err(e)?)
                .map_err(e)?;
        }
        menu.append(&sub).map_err(e)?;
    }
    menu.append(&PredefinedMenuItem::separator(&app).map_err(e)?).map_err(e)?;
    menu.append(&item("down", "Take Down").map_err(e)?).map_err(e)?;
    menu.append(&item("trash", trash_label()).map_err(e)?).map_err(e)?;
    // Keep the line down while the menu is open.
    state.ctl.lock().unwrap().inside = true;
    window.popup_menu_at(&menu, tauri::LogicalPosition::new(x, y)).map_err(e)
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
    let Some((key, ids)) = menu_id.split_once(':') else { return };
    let ids: Vec<u64> = ids.split(',').filter_map(|s| s.parse().ok()).collect();
    let Some(&id) = ids.first() else { return };
    let state = app.state::<AppState>();
    match key {
        "copy" => copy(app.clone(), state, id),
        "text" => copy_text(app.clone(), state, id),
        "open" => open(state, id),
        "edit" => edit(state, id),
        "redact" => redact(app.clone(), state, id),
        "pin" => pin(app.clone(), state, id),
        "share" => match share(state, id) {
            Ok(s) => {
                let _ = app.emit("shared", s);
            }
            Err(e) => toast(app, e),
        },
        "folder" => show_in_folder(state, id),
        "down" => take_down_item(app, id),
        "trash" => trash(app.clone(), state, vec![id]),
        "join-v" => join(app.clone(), state, ids, true),
        "join-h" => join(app.clone(), state, ids, false),
        "down-many" => take_down(app.clone(), ids),
        k if k.starts_with("act") => {
            if let Ok(i) = k[3..].parse() {
                run_action(app.clone(), state, id, i);
            }
        }
        _ => {}
    }
}

/// Dragging photos out hands the files to whatever they are dropped on.
/// They stay on the line. Plain (not async) commands run on the main
/// thread, which is where a drag has to start.
#[tauri::command]
pub fn start_drag(app: AppHandle, window: WebviewWindow, state: State<AppState>, ids: Vec<u64>) -> Result<(), String> {
    let paths: Vec<PathBuf> = ids.iter().filter_map(|id| path_of(&state, *id)).collect();
    let first = paths.first().cloned().ok_or("gone")?;
    let image = match crate::line::drag_preview(&first, &state.cache_dir) {
        Some(preview) => drag::Image::File(preview),
        None => drag::Image::Raw(include_bytes!("../icons/128x128.png").to_vec()),
    };
    state.ctl.lock().unwrap().dragging = true;
    let _ = window.set_ignore_cursor_events(false);

    let done = app.clone();
    let on_drop = move |result: drag::DragResult, _pos: drag::CursorPosition| {
        log::info!("Drag ended: {result:?}");
        done.state::<AppState>().ctl.lock().unwrap().dragging = false;
        let _ = done.emit("drag-end", ());
    };
    let item = drag::DragItem::Files(paths);
    #[cfg(target_os = "linux")]
    let res = {
        let gtk = window.gtk_window().map_err(|e| e.to_string())?;
        drag::start_drag(&gtk, item, image, on_drop, drag::Options::default())
    };
    #[cfg(not(target_os = "linux"))]
    let res = drag::start_drag(&window, item, image, on_drop, drag::Options::default());

    res.map_err(|e| {
        state.ctl.lock().unwrap().dragging = false;
        let _ = app.emit("drag-end", ());
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
    if state.lines.lock().unwrap().len() > 0 && state.config.lock().unwrap().reveal_at_edge {
        controller::reveal(&app, Reveal::Hover);
    }
}

/// A key or a click: keep the line down a little longer.
#[tauri::command]
pub fn activity(app: AppHandle) {
    controller::activity(&app);
}

/// Escape.
#[tauri::command]
pub fn hide(app: AppHandle) {
    controller::tuck(&app);
}

#[tauri::command]
pub fn open_search(app: AppHandle) {
    crate::open_search(&app);
}

#[tauri::command]
pub fn search(state: State<AppState>, query: String) -> Vec<Hit> {
    state.history.lock().unwrap().search(&query, 120)
}

/// From search: back on the line, and on the clipboard.
#[tauri::command]
pub fn hang_again(app: AppHandle, state: State<AppState>, path: String) -> Result<(), String> {
    let path = PathBuf::from(path);
    if !path.is_file() {
        return Err("That file is gone".into());
    }
    let active = state.lines.lock().unwrap().active();
    crate::hang(&app, active, &path);
    controller::reveal(&app, Reveal::Peek(std::time::Duration::from_millis(2500)));
    if crate::line::kind_of(&path) == Kind::Video {
        state.clipboard.put(Clip::Files(vec![path]));
    } else {
        state.clipboard.put(Clip::Image(path));
    }
    Ok(())
}

#[tauri::command]
pub fn open_path(path: String) {
    actions::open(Path::new(&path));
}
