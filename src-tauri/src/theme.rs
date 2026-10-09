//! Colours. "auto" follows the system light or dark mode, and on Omarchy the
//! current theme, so the clips, rope and glass match the rest of the desktop.

use serde::Serialize;
use std::path::PathBuf;
use tauri::{AppHandle, Emitter};

#[derive(Serialize, Clone, Default, Debug, PartialEq)]
pub struct Theme {
    /// "light", "dark" or "system".
    pub mode: String,
    pub background: Option<String>,
    pub foreground: Option<String>,
    pub accent: Option<String>,
}

fn omarchy_dir() -> Option<PathBuf> {
    let dir = dirs::config_dir()?.join("omarchy").join("current");
    dir.is_dir().then_some(dir)
}

pub fn current(setting: &str) -> Theme {
    let system = || Theme { mode: "system".into(), ..Default::default() };
    match setting {
        "light" | "dark" => Theme { mode: setting.into(), ..Default::default() },
        "omarchy" | "auto" => omarchy().unwrap_or_else(system),
        _ => system(),
    }
}

/// Reads the colours Omarchy gives the terminal, which every theme has.
fn omarchy() -> Option<Theme> {
    let theme = omarchy_dir()?.join("theme");
    let text = std::fs::read_to_string(theme.join("alacritty.toml")).ok()?;
    let mut section = String::new();
    let (mut bg, mut fg, mut accent) = (None, None, None);
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') {
            section = line.trim_matches(|c| c == '[' || c == ']').trim().to_string();
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let value = value.trim().trim_matches(|c| c == '"' || c == '\'').replace("0x", "#");
        if !value.starts_with('#') {
            continue;
        }
        match (section.as_str(), key.trim()) {
            ("colors.primary", "background") => bg = Some(value),
            ("colors.primary", "foreground") => fg = Some(value),
            ("colors.normal", "blue") => accent = Some(value),
            _ => {}
        }
    }
    let light = theme.join("light.mode").exists();
    Some(Theme { mode: if light { "light" } else { "dark" }.into(), background: bg, foreground: fg, accent })
}

/// Tells the line when the Omarchy theme changes.
pub fn watch(app: AppHandle, setting: String) {
    let Some(dir) = omarchy_dir() else { return };
    std::thread::spawn(move || {
        use notify::{RecursiveMode, Watcher};
        let (tx, rx) = std::sync::mpsc::channel();
        let Ok(mut watcher) = notify::recommended_watcher(move |e| {
            let _ = tx.send(e);
        }) else {
            return;
        };
        if watcher.watch(&dir, RecursiveMode::NonRecursive).is_err() {
            return;
        }
        let mut last = current(&setting);
        while rx.recv().is_ok() {
            while rx.recv_timeout(std::time::Duration::from_millis(400)).is_ok() {}
            let now = current(&setting);
            if now != last {
                log::info!("Theme changed: {now:?}");
                let _ = app.emit("theme", now.clone());
                last = now;
            }
        }
    });
}
