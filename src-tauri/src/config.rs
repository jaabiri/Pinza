use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

/// Settings kept in `config.json`. Written with the defaults on first launch
/// so it is easy to find and edit by hand.
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(default)]
pub struct Config {
    /// Folders to watch for the first line. Empty means "where this system
    /// saves screenshots and screen recordings".
    pub folders: Vec<PathBuf>,
    /// Extra lines, each watching its own folders, e.g. one for work.
    pub lines: Vec<LineSpec>,
    /// How many photos fit on a line before the oldest falls off. Kept
    /// photos never fall off.
    pub max_items: usize,
    /// Global shortcut to show or hide the line. On Wayland it cannot be
    /// registered by an app: bind `pinza toggle` in your compositor instead.
    pub shortcut: String,
    pub sound: bool,
    /// Rest the pointer at the top edge of the screen to bring the line down.
    pub reveal_at_edge: bool,
    /// Move screenshots Pinza picked up to the trash after this many hours,
    /// unless you kept them. 0 turns it off.
    pub clean_up_after_hours: u64,
    /// Stay out of the way while an app is full screen or the screen is shared.
    pub hide_in_full_screen: bool,
    /// "auto", "light", "dark" or "omarchy" (follow the Omarchy theme).
    pub theme: String,
    /// Your own commands, offered for every photo. `{path}`, `{name}` and
    /// `{dir}` are replaced. If one prints a link, the link is copied.
    pub actions: Vec<ActionSpec>,
    /// Language for reading text in screenshots with Tesseract (Linux), e.g.
    /// "eng" or "eng+spa". macOS and Windows use the system languages.
    pub ocr_language: String,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct LineSpec {
    pub name: String,
    pub folders: Vec<PathBuf>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
#[serde(default)]
pub struct ActionSpec {
    pub name: String,
    pub command: Vec<String>,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            folders: Vec::new(),
            lines: Vec::new(),
            max_items: 8,
            shortcut: "Ctrl+Alt+T".into(),
            sound: true,
            reveal_at_edge: true,
            clean_up_after_hours: 0,
            hide_in_full_screen: true,
            theme: "auto".into(),
            actions: Vec::new(),
            ocr_language: "eng".into(),
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

    /// Every line with its name: the main one first, then the extra ones.
    pub fn line_specs(&self) -> Vec<LineSpec> {
        let mut specs = vec![LineSpec { name: "Screenshots".into(), folders: self.folders.clone() }];
        for (i, l) in self.lines.iter().enumerate() {
            let name = if l.name.trim().is_empty() { format!("Line {}", i + 2) } else { l.name.clone() };
            specs.push(LineSpec { name, folders: l.folders.clone() });
        }
        specs
    }
}
