use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Settings kept in `config.json` next to the line. Written with the
/// defaults on first launch so it is easy to find and edit by hand.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Folders to watch. Empty means "where this system saves screenshots".
    pub folders: Vec<PathBuf>,
    /// How many photos fit on the line before the oldest falls off.
    pub max_items: usize,
    /// Global shortcut to show or hide the line. On Wayland it cannot be
    /// registered by an app: bind `pinza toggle` in your compositor instead.
    pub shortcut: String,
    pub sound: bool,
    /// Rest the pointer at the top edge of the screen to bring the line down.
    pub reveal_at_edge: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            max_items: 8,
            shortcut: "Ctrl+Alt+T".into(),
            sound: true,
            reveal_at_edge: true,
        }
    }
}

impl Config {
    pub fn path(dir: &Path) -> PathBuf {
        dir.join("config.json")
    }

    pub fn load(dir: &Path) -> Self {
        let path = Self::path(dir);
        match fs::read_to_string(&path) {
            Ok(text) => serde_json::from_str(&text).unwrap_or_else(|e| {
                log::warn!("Ignoring {}: {e}", path.display());
                Self::default()
            }),
            Err(_) => {
                let cfg = Self::default();
                cfg.save(dir);
                cfg
            }
        }
    }

    pub fn save(&self, dir: &Path) {
        let _ = fs::create_dir_all(dir);
        if let Ok(text) = serde_json::to_string_pretty(self) {
            if let Err(e) = fs::write(Self::path(dir), text) {
                log::warn!("Could not save settings: {e}");
            }
        }
    }
}
