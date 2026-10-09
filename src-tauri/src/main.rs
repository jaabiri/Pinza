#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod commands;
mod config;
mod controller;
mod folders;
mod fullscreen;
mod history;
mod images;
#[cfg(target_os = "linux")]
mod linux;
mod line;
mod ocr;
mod share;
mod theme;
mod tray;
mod watcher;

use config::Config;
use controller::{Ctl, Mode, Reveal};
use history::History;
use line::Lines;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{mpsc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub struct AppState {
    pub lines: Mutex<Lines>,
    pub history: Mutex<History>,
    pub config: Mutex<Config>,
    pub ctl: Mutex<Ctl>,
    pub clipboard: actions::Clipboard,
    /// Pictures waiting to have their text read, for search.
    pub reader: mpsc::Sender<PathBuf>,
    pub mode: Mode,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub layer_shell: bool,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    /// The folders each line watches.
    pub folders: Vec<Vec<PathBuf>>,
}

#[derive(Serialize, Clone)]
struct Hung {
    id: u64,
}

#[derive(Serialize, Clone)]
pub struct LineInfo {
    names: Vec<String>,
    active: usize,
}

pub fn emit_items(app: &AppHandle) {
    let views = app.state::<AppState>().lines.lock().unwrap().views();
    let _ = app.emit("items", views);
}

pub fn emit_line(app: &AppHandle) {
    let _ = app.emit("line", line_info(app));
}

pub fn line_info(app: &AppHandle) -> LineInfo {
    let st = app.state::<AppState>();
    let lines = st.lines.lock().unwrap();
    LineInfo { names: lines.names(), active: lines.active() }
}

/// A short message under the line.
pub fn toast(app: &AppHandle, text: impl Into<String>) {
    let _ = app.emit("toast", text.into());
}

/// A new screenshot: hang it on its line and show it for a moment. Returns
/// false when the file cannot be read yet, so the watcher tries again.
pub fn hang(app: &AppHandle, line: usize, path: &Path) -> bool {
    let st = app.state::<AppState>();
    let (hung, switched) = {
        let mut lines = st.lines.lock().unwrap();
        if lines.contains(path) {
            return true;
        }
        let hung = lines.hang(line, path);
        let switched = hung.is_some() && lines.active() != line;
        if switched {
            lines.set_active(line);
        }
        (hung, switched)
    };
    let Some((id, _dropped)) = hung else { return false };
    log::info!("Hung {}", path.display());
    st.history.lock().unwrap().record(path);
    let _ = st.reader.send(path.to_path_buf());
    if switched {
        emit_line(app);
    }
    emit_items(app);
    let _ = app.emit("hung", Hung { id });
    controller::reveal(app, Reveal::Peek(Duration::from_millis(2500)));
    true
}

pub fn refresh(app: &AppHandle, path: &Path) {
    let changed = app.state::<AppState>().lines.lock().unwrap().refresh(path);
    if changed {
        emit_items(app);
        // The text may have changed with the edit.
        let st = app.state::<AppState>();
        st.history.lock().unwrap().set_text(path, String::new());
        let _ = st.reader.send(path.to_path_buf());
    }
}

pub fn prune(app: &AppHandle) {
    let changed = app.state::<AppState>().lines.lock().unwrap().prune();
    if changed {
        emit_items(app);
        tuck_if_empty(app);
    }
}

/// An empty line goes away by itself, unless it was opened on purpose.
pub fn tuck_if_empty(app: &AppHandle) {
    let app = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(700));
        let empty = app.state::<AppState>().lines.lock().unwrap().len() == 0;
        if empty {
            controller::tuck(&app);
        }
    });
}

/// Reads the text in new screenshots, one at a time, in the background, so
/// they can be searched. Older ones are caught up at launch.
fn start_reader(app: AppHandle, rx: mpsc::Receiver<PathBuf>) {
    std::thread::spawn(move || {
        if !ocr::available() {
            log::info!("{}", ocr::missing_hint());
            return;
        }
        let backlog = app.state::<AppState>().history.lock().unwrap().unread();
        let mut queue: std::collections::VecDeque<PathBuf> = backlog.into();
        loop {
            let next = match queue.pop_front() {
                Some(p) => p,
                None => match rx.recv() {
                    Ok(p) => p,
                    Err(_) => return,
                },
            };
            while let Ok(p) = rx.try_recv() {
                queue.push_front(p);
            }
            if line::kind_of(&next) == line::Kind::Video || !next.is_file() {
                continue;
            }
            let st = app.state::<AppState>();
            let language = st.config.lock().unwrap().ocr_language.clone();
            match ocr::recognize(&next, &language) {
                Ok(lines) => st.history.lock().unwrap().set_text(&next, ocr::text_of(&lines)),
                Err(e) => log::warn!("Could not read {}: {e}", next.display()),
            }
            // Stay gentle on the machine.
            std::thread::sleep(Duration::from_millis(300));
        }
    });
}

/// Puts screenshots you did not keep in the trash after a while, so the
/// folder only holds what you meant to keep.
fn start_clean_up(app: AppHandle) {
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(30));
        loop {
            clean_up(&app);
            std::thread::sleep(Duration::from_secs(10 * 60));
        }
    });
}

pub fn clean_up(app: &AppHandle) {
    let st = app.state::<AppState>();
    let hours = st.config.lock().unwrap().clean_up_after_hours;
    if hours == 0 {
        return;
    }
    // Kept photos are marked in the history and never come up here.
    let due = st.history.lock().unwrap().due(hours);
    let mut cleaned = 0;
    for path in &due {
        if actions::trash(path) {
            st.history.lock().unwrap().forget(path);
            cleaned += 1;
        }
    }
    if cleaned > 0 {
        log::info!("Cleaned up {cleaned} screenshot(s)");
        prune(app);
    }
}

/// The search window, created the first time it is asked for.
pub fn open_search(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("search") {
        let _ = win.show();
        let _ = win.set_focus();
        return;
    }
    let built = WebviewWindowBuilder::new(app, "search", WebviewUrl::App("search.html".into()))
        .title("Search Screenshots")
        .inner_size(820.0, 560.0)
        .min_inner_size(480.0, 320.0)
        .center()
        .focused(true)
        .build();
    if let Err(e) = built {
        log::error!("Could not open search: {e}");
    }
}

/// Command line: `pinza toggle|show|hide|clear|search|quit`, handy for
/// compositor key bindings. A second launch hands its arguments to the
/// running one.
fn handle_args(app: &AppHandle, args: &[String]) {
    match args.get(1).map(String::as_str) {
        Some("toggle") => controller::toggle(app),
        Some("show") => controller::reveal(app, Reveal::Pin),
        Some("hide") => controller::tuck(app),
        Some("clear") => commands::clear_line(app),
        Some("search") => open_search(app),
        Some("next-line") => commands::switch_line(app, 1),
        Some("quit") => app.exit(0),
        _ => controller::reveal(app, Reveal::Pin),
    }
}

fn main() {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("pinza=info")).init();

    // WebKitGTK's DMA-BUF renderer draws transparent windows black on some
    // NVIDIA setups.
    #[cfg(target_os = "linux")]
    if std::env::var_os("WEBKIT_DISABLE_DMABUF_RENDERER").is_none() {
        std::env::set_var("WEBKIT_DISABLE_DMABUF_RENDERER", "1");
    }

    tauri::Builder::default()
        .plugin(tauri_plugin_single_instance::init(|app, args, _cwd| handle_args(app, &args)))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_global_shortcut::Builder::new().build())
        .plugin(tauri_plugin_autostart::Builder::new().build())
        .invoke_handler(tauri::generate_handler![
            commands::items,
            commands::settings,
            commands::copy,
            commands::copy_text,
            commands::open,
            commands::edit,
            commands::show_in_folder,
            commands::take_down,
            commands::trash,
            commands::pin,
            commands::redact,
            commands::join,
            commands::share,
            commands::run_action,
            commands::menu,
            commands::start_drag,
            commands::hit_rects,
            commands::input_region,
            commands::edge,
            commands::activity,
            commands::hide,
            commands::next_line,
            commands::open_search,
            commands::search,
            commands::hang_again,
            commands::open_path,
        ])
        .on_menu_event(|app, event| commands::on_menu(app, event.id().as_ref()))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let first_run = !Config::path(&config_dir).exists();
            let config = Config::load(&config_dir);
            let names = config.line_specs().into_iter().map(|s| s.name).collect();
            let lines = Lines::load(config_dir.join("line.json"), names, config.max_items);
            let mut history = History::load(config_dir.join("history.json"));
            // What hangs from an earlier version is searchable too.
            for path in lines.paths() {
                history.record(&path);
            }
            let folders = folders::line_folders(&config);

            // Videos and moving GIFs play straight from their folders.
            let scope = app.asset_protocol_scope();
            for dir in folders.iter().flatten() {
                let _ = scope.allow_directory(dir, false);
            }

            let win = WebviewWindowBuilder::new(app, controller::WINDOW, WebviewUrl::App("index.html".into()))
                .title("Pinza")
                .transparent(true)
                .decorations(false)
                .shadow(false)
                .resizable(false)
                .always_on_top(true)
                .visible_on_all_workspaces(true)
                .skip_taskbar(true)
                .focused(false)
                .focusable(false)
                .accept_first_mouse(true)
                // Left out of screen sharing, recordings and screenshots
                // (macOS, Windows).
                .content_protected(config.hide_in_full_screen)
                .inner_size(1280.0, controller::PANEL_HEIGHT)
                .visible(false)
                .build()?;

            // PINZA_POINTER=poll tries the macOS/Windows behaviour on X11.
            #[cfg(target_os = "linux")]
            let (mode, layer_shell) = match std::env::var("PINZA_POINTER").as_deref() {
                Ok("poll") if !linux::is_wayland() => (Mode::Poll, false),
                _ => (Mode::Shape, linux::init_layer_shell(&win)),
            };
            #[cfg(not(target_os = "linux"))]
            let (mode, layer_shell) = (Mode::Poll, false);
            log::info!("Pointer mode {mode:?}, layer shell {layer_shell}");

            let shortcut = config.shortcut.clone();
            let theme_setting = config.theme.clone();
            let (reader, reader_rx) = mpsc::channel();
            app.manage(AppState {
                lines: Mutex::new(lines),
                history: Mutex::new(history),
                config: Mutex::new(config),
                ctl: Mutex::new(Ctl::default()),
                clipboard: actions::Clipboard::new(),
                reader,
                mode,
                layer_shell,
                config_dir,
                cache_dir,
                folders: folders.clone(),
            });

            let handle = app.handle().clone();
            tray::build(&handle)?;
            register_shortcut(&handle, &shortcut);
            watcher::start(handle.clone(), folders);
            controller::start(handle.clone());
            fullscreen::start(handle.clone());
            theme::watch(handle.clone(), theme_setting);
            start_reader(handle.clone(), reader_rx);
            start_clean_up(handle.clone());

            // On Linux the window stays mapped, a few pixels tall in effect,
            // to notice the pointer at the top edge.
            if mode == Mode::Shape {
                controller::place(&handle);
                let _ = win.show();
                #[cfg(target_os = "linux")]
                {
                    linux::set_input_region(&win, Vec::new());
                    linux::watch_pointer(&win, handle.clone());
                }
            }

            // The first time, the line shows itself so you know it is there.
            if first_run {
                controller::reveal(&handle, Reveal::Peek(Duration::from_secs(5)));
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building Pinza")
        .run(|_app, event| {
            // Closing the search window is not quitting: the line lives on.
            if let tauri::RunEvent::ExitRequested { api, code: None, .. } = event {
                api.prevent_exit();
            }
        });
}

fn register_shortcut(app: &AppHandle, shortcut: &str) {
    use tauri_plugin_global_shortcut::{GlobalShortcutExt, ShortcutState};
    if shortcut.trim().is_empty() {
        return;
    }
    #[cfg(target_os = "linux")]
    if linux::is_wayland() {
        log::info!("Wayland: bind `pinza toggle` in your compositor for a shortcut");
        return;
    }
    let result = app.global_shortcut().on_shortcut(shortcut, |app, _, event| {
        if event.state == ShortcutState::Pressed {
            controller::toggle(app);
        }
    });
    if let Err(e) = result {
        log::warn!("Could not register the shortcut {shortcut}: {e}");
    }
}
