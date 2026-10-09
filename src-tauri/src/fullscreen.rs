//! Staying out of the way: no line over a full screen video, game or
//! presentation, or while the screen is being shared.
//!
//! - Hyprland: its event socket reports full screen and screen sharing.
//! - Windows: the shell says when a full screen app or presentation runs.
//! - macOS: the line never joins full screen Spaces.
//!
//! On macOS and Windows the window is also left out of screen sharing and
//! recordings by the system (see `content_protected` in main.rs).

use tauri::AppHandle;

pub fn start(app: AppHandle) {
    #[cfg(target_os = "linux")]
    hyprland::start(app);
    #[cfg(target_os = "windows")]
    windows_shell::start(app);
    #[cfg(not(any(target_os = "linux", target_os = "windows")))]
    let _ = app;
}

#[cfg(target_os = "linux")]
mod hyprland {
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixStream;
    use std::path::PathBuf;
    use std::process::Command;
    use std::time::Duration;
    use tauri::AppHandle;

    fn socket() -> Option<PathBuf> {
        let sig = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
        let mut candidates = Vec::new();
        if let Some(run) = std::env::var_os("XDG_RUNTIME_DIR") {
            candidates.push(PathBuf::from(run).join("hypr").join(&sig).join(".socket2.sock"));
        }
        candidates.push(PathBuf::from("/tmp/hypr").join(&sig).join(".socket2.sock"));
        candidates.into_iter().find(|p| p.exists())
    }

    /// Whether the focused window is full screen (not just maximised).
    fn active_is_full_screen() -> bool {
        let Ok(out) = Command::new("hyprctl").args(["activewindow", "-j"]).output() else { return false };
        let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out.stdout) else { return false };
        parse_full_screen(&v)
    }

    /// Newer Hyprland: `fullscreen` is 0 none, 1 maximised, 2 full screen.
    /// Older: `fullscreen` true plus `fullscreenMode` 0 full, 1 maximised.
    pub fn parse_full_screen(v: &serde_json::Value) -> bool {
        match &v["fullscreen"] {
            serde_json::Value::Number(n) => n.as_u64().unwrap_or(0) >= 2,
            serde_json::Value::Bool(b) => *b && v["fullscreenMode"].as_u64().unwrap_or(0) != 1,
            _ => false,
        }
    }

    pub fn start(app: AppHandle) {
        let Some(path) = socket() else { return };
        std::thread::spawn(move || {
            let (mut full, mut sharing) = (active_is_full_screen(), false);
            crate::controller::set_suppressed(&app, full || sharing);
            loop {
                let Ok(stream) = UnixStream::connect(&path) else {
                    std::thread::sleep(Duration::from_secs(5));
                    continue;
                };
                log::info!("Following Hyprland full screen and screen sharing");
                for line in BufReader::new(stream).lines().map_while(Result::ok) {
                    let Some((event, data)) = line.split_once(">>") else { continue };
                    match event {
                        "fullscreen" | "workspace" | "workspacev2" | "activewindowv2" | "focusedmon" => {
                            full = active_is_full_screen();
                        }
                        // screencast>>STATE,OWNER
                        "screencast" => sharing = data.starts_with('1'),
                        _ => continue,
                    }
                    crate::controller::set_suppressed(&app, full || sharing);
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        });
    }

    #[cfg(test)]
    mod tests {
        use serde_json::json;

        #[test]
        fn full_screen_but_not_maximised() {
            assert!(super::parse_full_screen(&json!({"fullscreen": 2})));
            assert!(!super::parse_full_screen(&json!({"fullscreen": 1})));
            assert!(super::parse_full_screen(&json!({"fullscreen": true, "fullscreenMode": 0})));
            assert!(!super::parse_full_screen(&json!({"fullscreen": true, "fullscreenMode": 1})));
            assert!(!super::parse_full_screen(&json!({})));
        }
    }
}

#[cfg(target_os = "windows")]
mod windows_shell {
    use std::time::Duration;
    use tauri::AppHandle;
    use windows::Win32::UI::Shell::{
        SHQueryUserNotificationState, QUNS_BUSY, QUNS_PRESENTATION_MODE, QUNS_RUNNING_D3D_FULL_SCREEN,
    };

    pub fn start(app: AppHandle) {
        std::thread::spawn(move || {
            let mut last = None;
            loop {
                let busy = unsafe { SHQueryUserNotificationState() }
                    .map(|s| s == QUNS_BUSY || s == QUNS_RUNNING_D3D_FULL_SCREEN || s == QUNS_PRESENTATION_MODE)
                    .unwrap_or(false);
                if last != Some(busy) {
                    crate::controller::set_suppressed(&app, busy);
                    last = Some(busy);
                }
                std::thread::sleep(Duration::from_millis(700));
            }
        });
    }
}
