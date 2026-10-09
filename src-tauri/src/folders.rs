//! Where each system puts screenshots. Pinza never takes screenshots itself:
//! you keep your usual shortcut and the line picks up whatever lands there.

use crate::config::Config;
use std::path::{Path, PathBuf};

const IMAGE_EXTENSIONS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "tif", "tiff", "bmp"];
pub const VIDEO_EXTENSIONS: &[&str] = &["mp4", "mov", "webm", "mkv", "m4v"];

/// The folders each line watches, in the order of `Config::line_specs`.
pub fn line_folders(cfg: &Config) -> Vec<Vec<PathBuf>> {
    cfg.line_specs()
        .iter()
        .enumerate()
        .map(|(i, spec)| {
            if i == 0 {
                let mut v = screenshot_folders(&spec.folders);
                for dir in recording_folders(&spec.folders) {
                    if !v.contains(&dir) {
                        v.push(dir);
                    }
                }
                v
            } else {
                spec.folders.iter().map(|p| expand_tilde(p)).filter(|p| p.is_dir()).collect()
            }
        })
        .collect()
}

/// Screen recordings, when the folders are not set by hand.
fn recording_folders(configured: &[PathBuf]) -> Vec<PathBuf> {
    if !configured.is_empty() {
        return Vec::new();
    }
    let mut v = Vec::new();
    #[cfg(target_os = "linux")]
    {
        // Omarchy's screen recorder writes to $OMARCHY_SCREENRECORD_DIR or Videos.
        if let Some(dir) = std::env::var_os("OMARCHY_SCREENRECORD_DIR").filter(|d| !d.is_empty()) {
            v.push(expand_tilde(Path::new(&dir)));
        }
        if let Some(videos) = dirs::video_dir() {
            v.push(videos.join("Screencasts"));
            v.push(videos);
        }
    }
    #[cfg(target_os = "windows")]
    if let Some(videos) = dirs::video_dir() {
        // Snipping Tool's recordings.
        v.push(videos.join("Screen Recordings"));
    }
    // macOS saves recordings next to screenshots.
    v.retain(|d| d.is_dir());
    v
}

pub fn screenshot_folders(configured: &[PathBuf]) -> Vec<PathBuf> {
    let wanted: Vec<PathBuf> = if configured.is_empty() {
        default_folders()
    } else {
        configured.iter().map(|p| expand_tilde(p)).collect()
    };
    let mut out: Vec<PathBuf> = Vec::new();
    for dir in wanted {
        if dir.is_dir() && !out.contains(&dir) {
            out.push(dir);
        }
    }
    if out.is_empty() {
        if let Some(fallback) = fallback_folder() {
            let _ = std::fs::create_dir_all(&fallback);
            out.push(fallback);
        }
    }
    out
}

/// Omarchy saves with `hyprshot` into `$OMARCHY_SCREENSHOT_DIR`, falling back
/// to the XDG Pictures folder. GNOME, KDE and others use Pictures/Screenshots.
#[cfg(target_os = "linux")]
fn default_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    for var in ["OMARCHY_SCREENSHOT_DIR", "XDG_SCREENSHOTS_DIR"] {
        if let Some(dir) = std::env::var_os(var).filter(|d| !d.is_empty()) {
            v.push(expand_tilde(Path::new(&dir)));
        }
    }
    let pictures = std::env::var_os("XDG_PICTURES_DIR")
        .filter(|d| !d.is_empty())
        .map(|d| expand_tilde(Path::new(&d)))
        .or_else(dirs::picture_dir);
    if let Some(p) = pictures {
        v.push(p.join("Screenshots"));
        v.push(p);
    }
    v
}

/// macOS keeps the location in the screencapture preferences. macOS 27 reads
/// `location-screenshot`, earlier versions `location`.
#[cfg(target_os = "macos")]
fn default_folders() -> Vec<PathBuf> {
    let read = |key: &str| -> Option<PathBuf> {
        let out = std::process::Command::new("defaults")
            .args(["read", "com.apple.screencapture", key])
            .output()
            .ok()?;
        let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
        (out.status.success() && !s.is_empty()).then(|| expand_tilde(Path::new(&s)))
    };
    let mut v: Vec<PathBuf> = ["location-screenshot", "location"].iter().filter_map(|k| read(k)).collect();
    if let Some(desktop) = dirs::desktop_dir() {
        v.push(desktop);
    }
    v.truncate(1);
    v
}

/// Windows 11 Snipping Tool (Win+Shift+S) and Win+PrtScn save to
/// Pictures\Screenshots, which OneDrive may have moved.
#[cfg(target_os = "windows")]
fn default_folders() -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Some(p) = dirs::picture_dir() {
        v.push(p.join("Screenshots"));
    }
    if let Some(h) = dirs::home_dir() {
        v.push(h.join("OneDrive").join("Pictures").join("Screenshots"));
        v.push(h.join("Pictures").join("Screenshots"));
    }
    v
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn default_folders() -> Vec<PathBuf> {
    dirs::picture_dir().into_iter().collect()
}

fn fallback_folder() -> Option<PathBuf> {
    dirs::picture_dir()
        .or_else(|| dirs::home_dir().map(|h| h.join("Pictures")))
        .map(|p| p.join("Screenshots"))
}

pub fn expand_tilde(p: &Path) -> PathBuf {
    match (p.strip_prefix("~"), dirs::home_dir()) {
        (Ok(rest), Some(home)) => home.join(rest),
        _ => p.to_path_buf(),
    }
}

pub fn is_video(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| VIDEO_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()))
}

/// Pictures and screen recordings, not hidden or temporary files.
pub fn is_media(path: &Path) -> bool {
    is_image(path) || (is_video(path) && !is_hidden(path))
}

fn is_hidden(path: &Path) -> bool {
    path.file_name().and_then(|n| n.to_str()).is_none_or(|n| n.starts_with('.'))
}

pub fn is_image(path: &Path) -> bool {
    let hidden = path
        .file_name()
        .and_then(|n| n.to_str())
        .is_none_or(|n| n.starts_with('.'));
    let ext_ok = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| IMAGE_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    !hidden && ext_ok
}

/// On the macOS Desktop only real screenshots count, tagged by the system with
/// an extended attribute. Anywhere else any new image counts.
pub fn is_candidate(path: &Path, folder: &Path) -> bool {
    if !is_media(path) {
        return false;
    }
    #[cfg(target_os = "macos")]
    if dirs::desktop_dir().as_deref() == Some(folder) {
        return matches!(xattr::get(path, "com.apple.metadata:kMDItemIsScreenCapture"), Ok(Some(_)));
    }
    let _ = folder;
    true
}
