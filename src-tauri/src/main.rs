#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod actions;
mod commands;
mod config;
mod controller;
mod folders;
mod line;
#[cfg(target_os = "linux")]
mod linux;
mod tray;
mod watcher;

use config::Config;
use controller::{Ctl, Mode, Reveal};
use line::Line;
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager, WebviewUrl, WebviewWindowBuilder};

pub struct AppState {
    pub line: Mutex<Line>,
    pub config: Mutex<Config>,
    pub ctl: Mutex<Ctl>,
    pub clipboard: actions::Clipboard,
    pub mode: Mode,
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub layer_shell: bool,
    pub config_dir: PathBuf,
    pub cache_dir: PathBuf,
    pub folders: Vec<PathBuf>,
}

#[derive(Serialize, Clone)]
struct Hung {
    id: u64,
}

pub fn emit_items(app: &AppHandle) {
    let views = app.state::<AppState>().line.lock().unwrap().views();
    let _ = app.emit("items", views);
}

/// A new screenshot: hang it and show it for a moment. Returns false when
/// the file cannot be read yet, so the watcher tries again.
pub fn hang(app: &AppHandle, path: &Path) -> bool {
    let st = app.state::<AppState>();
    let hung = {
        let mut line = st.line.lock().unwrap();
        if line.contains(path) {
            return true;
        }
        line.hang(path)
    };
    let Some((id, _dropped)) = hung else { return false };
    log::info!("Hung {}", path.display());
    emit_items(app);
    let _ = app.emit("hung", Hung { id });
    controller::reveal(app, Reveal::Peek(Duration::from_millis(2500)));
    true
}

pub fn refresh(app: &AppHandle, path: &Path) {
    let changed = app.state::<AppState>().line.lock().unwrap().refresh(path);
    if changed {
        emit_items(app);
    }
}

pub fn prune(app: &AppHandle) {
    let changed = app.state::<AppState>().line.lock().unwrap().prune();
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
        let empty = app.state::<AppState>().line.lock().unwrap().len() == 0;
        if empty {
            controller::tuck(&app);
        }
    });
}

/// Command line: `pinza toggle|show|hide|clear`, handy for compositor key
/// bindings. A second launch hands its arguments to the running one.
fn handle_args(app: &AppHandle, args: &[String]) {
    match args.get(1).map(String::as_str) {
        Some("toggle") => controller::toggle(app),
        Some("show") => controller::reveal(app, Reveal::Pin),
        Some("hide") => controller::tuck(app),
        Some("clear") => commands::clear_line(app),
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
            commands::open,
            commands::edit,
            commands::show_in_folder,
            commands::take_down,
            commands::trash,
            commands::menu,
            commands::start_drag,
            commands::hit_rects,
            commands::input_region,
            commands::edge,
        ])
        .on_menu_event(|app, event| commands::on_menu(app, event.id().as_ref()))
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let cache_dir = app.path().app_cache_dir()?;
            let first_run = !Config::path(&config_dir).exists();
            let config = Config::load(&config_dir);
            let line = Line::load(config_dir.join("line.json"), config.max_items);
            let folders = folders::screenshot_folders(&config);

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
            app.manage(AppState {
                line: Mutex::new(line),
                config: Mutex::new(config),
                ctl: Mutex::new(Ctl::default()),
                clipboard: actions::Clipboard::new(),
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
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                // Closing the window is not quitting: the line lives in the tray.
                if code.is_none() {
                    api.prevent_exit();
                }
                let _ = app;
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
