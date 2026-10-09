//! What you can do with one photo.

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc;
use std::thread;

pub enum Clip {
    Image(PathBuf),
    Text(String),
    /// A screen recording goes on the clipboard as a file.
    Files(Vec<PathBuf>),
}

/// The clipboard lives on its own thread for the whole run: on Linux the
/// copied image is served by whoever owns the clipboard, so it must outlive
/// the click that put it there.
pub struct Clipboard(mpsc::Sender<Clip>);

impl Clipboard {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel::<Clip>();
        thread::spawn(move || {
            let mut board = None;
            for clip in rx {
                if board.is_none() {
                    board = arboard::Clipboard::new().map_err(|e| log::error!("No clipboard: {e}")).ok();
                }
                let Some(b) = board.as_mut() else { continue };
                let result = match clip {
                    Clip::Image(path) => copy_image(b, &path),
                    Clip::Text(text) => b.set_text(text).map_err(|e| e.to_string()),
                    Clip::Files(paths) => b.set().file_list(&paths).map_err(|e| e.to_string()),
                };
                if let Err(e) = result {
                    log::error!("Could not copy: {e}");
                }
            }
        });
        Self(tx)
    }

    pub fn put(&self, clip: Clip) {
        let _ = self.0.send(clip);
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

/// Runs one of your own actions on a file. Arguments are passed as they
/// are, without a shell, with `{path}`, `{name}` and `{dir}` filled in.
/// Returns what it printed.
pub fn run_custom(command: &[String], path: &Path) -> Result<String, String> {
    let (program, args) = command.split_first().ok_or("This action has no command")?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let dir = path.parent().map(|d| d.to_string_lossy().into_owned()).unwrap_or_default();
    let fill = |s: &str| {
        s.replace("{path}", &path.to_string_lossy()).replace("{name}", &name).replace("{dir}", &dir)
    };
    let program = crate::folders::expand_tilde(Path::new(&fill(program)));
    let out = Command::new(program)
        .args(args.iter().map(|a| fill(a)))
        .output()
        .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        Err(if err.is_empty() { format!("exited with {}", out.status) } else { err })
    }
}

/// The first link in some text, if any.
pub fn find_link(text: &str) -> Option<String> {
    text.split_whitespace().find(|w| w.starts_with("http://") || w.starts_with("https://")).map(str::to_string)
}
