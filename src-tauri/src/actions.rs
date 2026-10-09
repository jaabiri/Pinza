//! What you can do with one photo.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

/// The clipboard lives on its own thread for the whole run: on Linux the
/// copied image is served by whoever owns the clipboard, so it must outlive
/// the click that put it there.
pub struct Clipboard(mpsc::Sender<PathBuf>);

impl Clipboard {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<PathBuf>();
        thread::spawn(move || {
            let mut board = None;
            for path in rx {
                if board.is_none() {
                    board = arboard::Clipboard::new().map_err(|e| log::error!("No clipboard: {e}")).ok();
                }
                let Some(b) = board.as_mut() else { continue };
                if let Err(e) = copy_image(b, &path) {
                    log::error!("Could not copy {}: {e}", path.display());
                }
            }
        });
        Self(tx)
    }

    pub fn copy(&self, path: &Path) {
        let _ = self.0.send(path.to_path_buf());
    }
}

fn copy_image(board: &mut arboard::Clipboard, path: &Path) -> Result<(), String> {
    let img = image::open(path).map_err(|e| e.to_string())?.into_rgba8();
    let (w, h) = img.dimensions();
    board
        .set_image(arboard::ImageData { width: w as usize, height: h as usize, bytes: img.into_raw().into() })
        .map_err(|e| e.to_string())
}

/// Long press: open the photo to mark it up. Omarchy ships Satty, which
/// writes back to the same file so the line shows the edit.
pub fn edit(path: &Path) {
    #[cfg(target_os = "linux")]
    if crate::linux::has("satty") {
        let p = path.as_os_str();
        let _ = Command::new("satty")
            .arg("--filename")
            .arg(p)
            .arg("--output-filename")
            .arg(p)
            .arg("--early-exit")
            .spawn();
        return;
    }
    #[cfg(target_os = "macos")]
    {
        let _ = Command::new("open").args(["-a", "Preview"]).arg(path).spawn();
        return;
    }
    #[cfg(target_os = "windows")]
    {
        let _ = Command::new("mspaint").arg(path).spawn();
        return;
    }
    #[allow(unreachable_code)]
    {
        let _ = tauri_plugin_opener::open_path(path, None::<&str>);
    }
}

pub fn open(path: &Path) {
    if let Err(e) = tauri_plugin_opener::open_path(path, None::<&str>) {
        log::error!("Could not open {}: {e}", path.display());
    }
}

pub fn show_in_folder(path: &Path) {
    if let Err(e) = tauri_plugin_opener::reveal_item_in_dir(path) {
        log::error!("Could not reveal {}: {e}", path.display());
    }
}

pub fn trash(path: &Path) -> bool {
    match trash::delete(path) {
        Ok(()) => true,
        Err(e) => {
            log::error!("Could not move {} to the trash: {e}", path.display());
            false
        }
    }
}
