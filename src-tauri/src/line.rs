//! What is hanging on the lines. The files never move: a line is only a view
//! onto them.

use base64::Engine;
use serde::{Deserialize, Serialize};
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const THUMB_PIXELS: u32 = 480;

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Image,
    Gif,
    Video,
}

pub struct Item {
    pub id: u64,
    pub path: PathBuf,
    pub pinned: bool,
    pub added: u64,
    pub kind: Kind,
    thumb: String,
    width: u32,
    height: u32,
}

/// What the web view needs to draw one photo.
#[derive(Serialize, Clone)]
pub struct ItemView {
    pub id: u64,
    pub name: String,
    pub path: String,
    pub thumb: String,
    pub width: u32,
    pub height: u32,
    pub pinned: bool,
    pub kind: Kind,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    path: PathBuf,
    #[serde(default)]
    pinned: bool,
    #[serde(default)]
    added: u64,
}

#[derive(Serialize, Deserialize, Default)]
struct StoreFile {
    active: String,
    lines: Vec<(String, Vec<Stored>)>,
}

pub struct Line {
    pub name: String,
    items: Vec<Item>,
}

pub struct Lines {
    lines: Vec<Line>,
    active: usize,
    next_id: u64,
    store: PathBuf,
    pub max_items: usize,
}

pub fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs())
}

impl Lines {
    /// Restores what was hanging last time, skipping files that are gone.
    pub fn load(store: PathBuf, names: Vec<String>, max_items: usize) -> Self {
        let mut lines = Self {
            lines: names.into_iter().map(|name| Line { name, items: Vec::new() }).collect(),
            active: 0,
            next_id: 1,
            store,
            max_items: max_items.max(1),
        };
        let text = fs::read_to_string(&lines.store).unwrap_or_default();
        let saved: StoreFile = serde_json::from_str(&text).unwrap_or_else(|_| {
            // The first version kept a plain list of paths.
            let paths: Vec<PathBuf> = serde_json::from_str(&text).unwrap_or_default();
            let first = lines.lines[0].name.clone();
            StoreFile {
                active: first.clone(),
                lines: vec![(first, paths.into_iter().map(|path| Stored { path, pinned: false, added: now() }).collect())],
            }
        });
        for (name, stored) in saved.lines {
            let Some(index) = lines.lines.iter().position(|l| l.name == name) else { continue };
            for s in stored {
                if s.path.is_file() {
                    if let Some((id, _)) = lines.hang(index, &s.path) {
                        let item = lines.get_mut(id).unwrap();
                        item.pinned = s.pinned;
                        item.added = if s.added > 0 { s.added } else { now() };
                    }
                }
            }
        }
        lines.active = lines.lines.iter().position(|l| l.name == saved.active).unwrap_or(0);
        lines.save();
        lines
    }

    pub fn names(&self) -> Vec<String> {
        self.lines.iter().map(|l| l.name.clone()).collect()
    }

    pub fn active(&self) -> usize {
        self.active
    }

    pub fn set_active(&mut self, index: usize) {
        if index < self.lines.len() && index != self.active {
            self.active = index;
            self.save();
        }
    }

    /// Photos on the line in view.
    pub fn len(&self) -> usize {
        self.lines[self.active].items.len()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.all().any(|i| i.path == path)
    }

    fn all(&self) -> impl Iterator<Item = &Item> {
        self.lines.iter().flat_map(|l| l.items.iter())
    }

    /// Hangs a photo on a line and returns its id, plus the ids that fell off
    /// the far end because the line was full. Kept photos never fall off.
    /// None if the file cannot be read yet (it may still be being written).
    pub fn hang(&mut self, line: usize, path: &Path) -> Option<(u64, Vec<u64>)> {
        if self.contains(path) || line >= self.lines.len() {
            return None;
        }
        let kind = kind_of(path);
        let (thumb, width, height) = match kind {
            Kind::Video => video_poster(path).unwrap_or((String::new(), 1920, 1080)),
            _ => thumbnail(path)?,
        };
        let id = self.next_id;
        self.next_id += 1;
        let max = self.max_items;
        let items = &mut self.lines[line].items;
        items.push(Item { id, path: path.to_path_buf(), pinned: false, added: now(), kind, thumb, width, height });
        let mut dropped = Vec::new();
        while items.len() > max {
            let Some(i) = items.iter().position(|i| !i.pinned) else { break };
            dropped.push(items.remove(i).id);
        }
        self.save();
        Some((id, dropped))
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.all().find(|i| i.id == id)
    }

    fn get_mut(&mut self, id: u64) -> Option<&mut Item> {
        self.lines.iter_mut().flat_map(|l| l.items.iter_mut()).find(|i| i.id == id)
    }

    pub fn remove(&mut self, id: u64) -> Option<Item> {
        for line in &mut self.lines {
            if let Some(i) = line.items.iter().position(|i| i.id == id) {
                let item = line.items.remove(i);
                self.save();
                return Some(item);
            }
        }
        None
    }

    /// Takes down everything on the line in view except what you kept.
    pub fn clear(&mut self) {
        self.lines[self.active].items.retain(|i| i.pinned);
        self.save();
    }

    /// Kept photos stay on the line and are never cleaned up.
    pub fn toggle_pin(&mut self, id: u64) -> Option<(PathBuf, bool)> {
        let item = self.get_mut(id)?;
        item.pinned = !item.pinned;
        let out = (item.path.clone(), item.pinned);
        self.save();
        Some(out)
    }

    /// Photos whose file was deleted or moved away fall off by themselves.
    pub fn prune(&mut self) -> bool {
        let mut changed = false;
        for line in &mut self.lines {
            let before = line.items.len();
            line.items.retain(|i| i.path.is_file());
            changed |= line.items.len() != before;
        }
        if changed {
            self.save();
        }
        changed
    }

    /// After an edit, the photo shows the new version.
    pub fn refresh(&mut self, path: &Path) -> bool {
        let Some(item) = self.lines.iter_mut().flat_map(|l| l.items.iter_mut()).find(|i| i.path == path) else {
            return false;
        };
        if item.kind == Kind::Video {
            return true;
        }
        match thumbnail(path) {
            Some((thumb, w, h)) => {
                item.thumb = thumb;
                item.width = w;
                item.height = h;
                true
            }
            None => false,
        }
    }

    pub fn views(&self) -> Vec<ItemView> {
        self.lines[self.active]
            .items
            .iter()
            .map(|i| ItemView {
                id: i.id,
                name: i.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                path: i.path.to_string_lossy().into_owned(),
                thumb: i.thumb.clone(),
                width: i.width,
                height: i.height,
                pinned: i.pinned,
                kind: i.kind,
            })
            .collect()
    }

    fn save(&self) {
        let file = StoreFile {
            active: self.lines[self.active].name.clone(),
            lines: self
                .lines
                .iter()
                .map(|l| {
                    let stored = l
                        .items
                        .iter()
                        .map(|i| Stored { path: i.path.clone(), pinned: i.pinned, added: i.added })
                        .collect();
                    (l.name.clone(), stored)
                })
                .collect(),
        };
        if let Some(dir) = self.store.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&file) {
            let _ = fs::write(&self.store, text);
        }
    }
}

pub fn kind_of(path: &Path) -> Kind {
    match path.extension().and_then(|e| e.to_str()).map(|e| e.to_ascii_lowercase()).as_deref() {
        Some("gif") => Kind::Gif,
        Some(e) if crate::folders::VIDEO_EXTENSIONS.contains(&e) => Kind::Video,
        _ => Kind::Image,
    }
}

/// A small PNG as a data URL, plus the original size.
fn thumbnail(path: &Path) -> Option<(String, u32, u32)> {
    let img = image::open(path).ok()?;
    let (w, h) = (img.width(), img.height());
    let small = img.thumbnail(THUMB_PIXELS, THUMB_PIXELS);
    let mut png = Vec::new();
    small.write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    Some((format!("data:image/png;base64,{b64}"), w, h))
}

/// A frame from early in a recording, when ffmpeg is installed. Without it
/// the web view shows the first frame itself, if it can decode the video.
fn video_poster(path: &Path) -> Option<(String, u32, u32)> {
    let out = std::process::Command::new("ffmpeg")
        .args(["-loglevel", "error", "-ss", "0.3", "-i"])
        .arg(path)
        .args(["-frames:v", "1", "-f", "image2pipe", "-vcodec", "png", "-"])
        .output()
        .ok()?;
    if !out.status.success() || out.stdout.is_empty() {
        return None;
    }
    let img = image::load_from_memory(&out.stdout).ok()?;
    let (w, h) = (img.width(), img.height());
    let mut png = Vec::new();
    img.thumbnail(THUMB_PIXELS, THUMB_PIXELS).write_to(&mut Cursor::new(&mut png), image::ImageFormat::Png).ok()?;
    let b64 = base64::engine::general_purpose::STANDARD.encode(png);
    Some((format!("data:image/png;base64,{b64}"), w, h))
}

/// Every file on the lines, for the search history.
impl Lines {
    pub fn paths(&self) -> Vec<PathBuf> {
        self.all().map(|i| i.path.clone()).collect()
    }
}

/// A PNG of the photo for the drag preview, written to the cache folder.
pub fn drag_preview(path: &Path, cache: &Path) -> Option<PathBuf> {
    if kind_of(path) == Kind::Video {
        return None;
    }
    let img = image::open(path).ok()?.thumbnail(160, 160);
    let _ = fs::create_dir_all(cache);
    let out = cache.join("drag-preview.png");
    img.save_with_format(&out, image::ImageFormat::Png).ok()?;
    Some(out)
}

/// `name.png`, or `name 2.png` and so on if that is taken.
pub fn unique_path(dir: &Path, stem: &str, ext: &str) -> PathBuf {
    let mut candidate = dir.join(format!("{stem}.{ext}"));
    let mut n = 2;
    while candidate.exists() {
        candidate = dir.join(format!("{stem} {n}.{ext}"));
        n += 1;
    }
    candidate
}
