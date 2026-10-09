//! The icon in the menu bar, Waybar or the notification area.

use crate::{commands, controller, AppState};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem, Submenu};
use tauri::tray::TrayIconBuilder;
use tauri::{AppHandle, Emitter, Manager, Wry};
use tauri_plugin_autostart::ManagerExt;

const CLEAN_UP: [(u64, &str); 4] = [(0, "Never"), (24, "After a Day"), (24 * 7, "After a Week"), (24 * 30, "After a Month")];

pub fn build(app: &AppHandle) -> tauri::Result<()> {
    let menu = make_menu(app)?;
    let mut tray = TrayIconBuilder::with_id("pinza").tooltip("Pinza").menu(&menu);
    if let Some(icon) = app.default_window_icon() {
        tray = tray.icon(icon.clone());
    }
    tray.build(app)?;
    Ok(())
}

/// Ticks and line names change, so the menu is rebuilt after each choice.
fn rebuild(app: &AppHandle) {
    if let (Some(tray), Ok(menu)) = (app.tray_by_id("pinza"), make_menu(app)) {
        let _ = tray.set_menu(Some(menu));
    }
}

fn make_menu(app: &AppHandle) -> tauri::Result<Menu<Wry>> {
    let st = app.state::<AppState>();
    let cfg = st.config.lock().unwrap().clone();
    let at_login = app.autolaunch().is_enabled().unwrap_or(false);
    let shortcut = if cfg.shortcut.is_empty() { None } else { Some(cfg.shortcut.as_str()) };

    let menu = Menu::new(app)?;
    menu.append(&MenuItem::with_id(app, "tray:toggle", "Show the Line", true, shortcut)?)?;
    menu.append(&MenuItem::with_id(app, "tray:search", "Search Screenshots…", true, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "tray:clear", "Clear the Line", true, None::<&str>)?)?;

    let info = crate::line_info(app);
    if info.names.len() > 1 {
        let lines = Submenu::new(app, "Line", true)?;
        for (i, name) in info.names.iter().enumerate() {
            lines.append(&CheckMenuItem::with_id(app, format!("tray:line:{i}"), name, true, i == info.active, None::<&str>)?)?;
        }
        menu.append(&lines)?;
    }
    menu.append(&MenuItem::with_id(app, "tray:folder", "Open Screenshots Folder", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;

    let clean = Submenu::new(app, "Clean Up Screenshots", true)?;
    for (hours, label) in CLEAN_UP {
        clean.append(&CheckMenuItem::with_id(
            app,
            format!("tray:clean:{hours}"),
            label,
            true,
            cfg.clean_up_after_hours == hours,
            None::<&str>,
        )?)?;
    }
    menu.append(&clean)?;
    menu.append(&CheckMenuItem::with_id(app, "tray:sound", "Sounds", true, cfg.sound, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(app, "tray:edge", "Show at Top Edge", true, cfg.reveal_at_edge, None::<&str>)?)?;
    menu.append(&CheckMenuItem::with_id(
        app,
        "tray:full",
        "Hide in Full Screen and Sharing",
        true,
        cfg.hide_in_full_screen,
        None::<&str>,
    )?)?;
    menu.append(&CheckMenuItem::with_id(app, "tray:login", "Open at Login", true, at_login, None::<&str>)?)?;
    menu.append(&MenuItem::with_id(app, "tray:settings", "Edit Settings…", true, None::<&str>)?)?;
    menu.append(&PredefinedMenuItem::separator(app)?)?;
    menu.append(&MenuItem::with_id(app, "tray:quit", "Quit Pinza", true, None::<&str>)?)?;
    Ok(menu)
}

/// Handles a tray menu click. Returns false for ids that are not ours.
pub fn on_menu(app: &AppHandle, id: &str) -> bool {
    let Some(rest) = id.strip_prefix("tray:") else { return false };
    let st = app.state::<AppState>();
    let save = |change: &dyn Fn(&mut crate::config::Config)| {
        let mut cfg = st.config.lock().unwrap();
        change(&mut cfg);
        cfg.save(&st.config_dir);
    };
    match rest {
        "toggle" => controller::toggle(app),
        "search" => crate::open_search(app),
        "clear" => commands::clear_line(app),
        "folder" => {
            if let Some(dir) = st.folders.first().and_then(|f| f.first()) {
                let _ = tauri_plugin_opener::open_path(dir, None::<&str>);
            }
        }
        "sound" => save(&|c| c.sound = !c.sound),
        "edge" => save(&|c| c.reveal_at_edge = !c.reveal_at_edge),
        "full" => {
            save(&|c| c.hide_in_full_screen = !c.hide_in_full_screen);
            let on = st.config.lock().unwrap().hide_in_full_screen;
            if let Some(win) = app.get_webview_window(controller::WINDOW) {
                let _ = win.set_content_protected(on);
            }
            if !on {
                controller::set_suppressed(app, false);
            }
        }
        "login" => {
            let auto = app.autolaunch();
            let result = if auto.is_enabled().unwrap_or(false) { auto.disable() } else { auto.enable() };
            if let Err(e) = result {
                log::warn!("Could not change Open at Login: {e}");
            }
        }
        "settings" => {
            let path = crate::config::Config::path(&st.config_dir);
            let _ = tauri_plugin_opener::open_path(path, None::<&str>);
        }
        "quit" => app.exit(0),
        other => {
            if let Some(hours) = other.strip_prefix("clean:").and_then(|h| h.parse::<u64>().ok()) {
                save(&|c| c.clean_up_after_hours = hours);
                crate::clean_up(app);
            } else if let Some(i) = other.strip_prefix("line:").and_then(|i| i.parse::<i64>().ok()) {
                let active = st.lines.lock().unwrap().active() as i64;
                commands::switch_line(app, i - active);
            } else {
                return false;
            }
        }
    }
    let _ = app.emit("settings", ());
    rebuild(app);
    true
}
