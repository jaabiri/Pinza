//! The icon in the menu bar, Waybar or the notification area.

use crate::{commands, controller, AppState};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager};
use tauri_plugin_autostart::ManagerExt;

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let st = app.state::<AppState>();
    let cfg = st.config.lock().unwrap().clone();
    let at_login = app.autolaunch().is_enabled().unwrap_or(false);
    let shortcut = if cfg.shortcut.is_empty() { None } else { Some(cfg.shortcut.as_str()) };
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "tray:toggle", "Show the Line", true, shortcut)?,
            &MenuItem::with_id(app, "tray:clear", "Clear the Line", true, None::<&str>)?,
            &MenuItem::with_id(app, "tray:folder", "Open Screenshots Folder", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &CheckMenuItem::with_id(app, "tray:sound", "Sounds", true, cfg.sound, None::<&str>)?,
            &CheckMenuItem::with_id(app, "tray:edge", "Show at Top Edge", true, cfg.reveal_at_edge, None::<&str>)?,
            &CheckMenuItem::with_id(app, "tray:login", "Open at Login", true, at_login, None::<&str>)?,
            &MenuItem::with_id(app, "tray:settings", "Edit Settings…", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "tray:quit", "Quit Pinza", true, None::<&str>)?,
        ],
    )?;
    let mut tray = TrayIconBuilder::with_id("pinza").tooltip("Pinza").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    #[cfg(target_os = "macos")]
    {
        tray = tray.icon_as_template(false);
    }
    tray.build(app)?;
    Ok(())
}

/// Handles a tray menu click. Returns false for ids that are not ours.
pub fn on_menu(app: &AppHandle, id: &str) -> bool {
    let st = app.state::<AppState>();
    match id {
        "tray:toggle" => controller::toggle(app),
        "tray:clear" => commands::clear_line(app),
        "tray:folder" => {
            if let Some(dir) = st.folders.first() {
                let _ = tauri_plugin_opener::open_path(dir, None::<&str>);
            }
        }
        "tray:sound" | "tray:edge" => {
            let mut cfg = st.config.lock().unwrap();
            if id == "tray:sound" {
                cfg.sound = !cfg.sound;
            } else {
                cfg.reveal_at_edge = !cfg.reveal_at_edge;
            }
            cfg.save(&st.config_dir);
            let _ = app.emit("settings", ());
        }
        "tray:login" => {
            let auto = app.autolaunch();
            let result = if auto.is_enabled().unwrap_or(false) { auto.disable() } else { auto.enable() };
            if let Err(e) = result {
                log::warn!("Could not change Open at Login: {e}");
            }
        }
        "tray:settings" => {
            let path = crate::config::Config::path(&st.config_dir);
            let _ = tauri_plugin_opener::open_path(path, None::<&str>);
        }
        "tray:quit" => app.exit(0),
        _ => return false,
    }
    true
}
