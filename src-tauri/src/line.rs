//! What is hanging on the line. The files never move: the line is only a
//! view onto them.

use base64::Engine;
use serde::Serialize;
use std::fs;
use std::io::Cursor;
use std::path::{Path, PathBuf};

const THUMB_PIXELS: u32 = 480;

pub struct Item {
    pub id: u64,
    pub path: PathBuf,
    thumb: String,
    width: u32,
    height: u32,
}

/// What the web view needs to draw one photo.
#[derive(Serialize, Clone)]
pub struct ItemView {
    pub id: u64,
    pub name: String,
    pub thumb: String,
    pub width: u32,
    pub height: u32,
}

pub struct Line {
    items: Vec<Item>,
    next_id: u64,
    store: PathBuf,
    pub max_items: usize,
}

impl Line {
    /// Restores what was hanging last time, skipping files that are gone.
    pub fn load(store: PathBuf, max_items: usize) -> Self {
        let mut line = Self { items: Vec::new(), next_id: 1, store, max_items: max_items.max(1) };
        let paths: Vec<PathBuf> = fs::read_to_string(&line.store)
            .ok()
            .and_then(|t| serde_json::from_str(&t).ok())
            .unwrap_or_default();
        for path in paths {
            if path.is_file() {
                line.hang(&path);
            }
        }
        line
    }

    pub fn len(&self) -> usize {
        self.items.len()
    }

    pub fn contains(&self, path: &Path) -> bool {
        self.items.iter().any(|i| i.path == path)
    }

    /// Hangs a photo and returns its id, plus the ids that fell off the far
    /// end because the line was full. None if the file is not a readable
    /// image yet (it may still be being written).
    pub fn hang(&mut self, path: &Path) -> Option<(u64, Vec<u64>)> {
        if self.contains(path) {
            return None;
        }
        let (thumb, width, height) = thumbnail(path)?;
        let id = self.next_id;
        self.next_id += 1;
        self.items.push(Item { id, path: path.to_path_buf(), thumb, width, height });
        let mut dropped = Vec::new();
        while self.items.len() > self.max_items {
            dropped.push(self.items.remove(0).id);
        }
        self.save();
        Some((id, dropped))
    }

    pub fn get(&self, id: u64) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }

    pub fn remove(&mut self, id: u64) -> Option<Item> {
        let i = self.items.iter().position(|i| i.id == id)?;
        let item = self.items.remove(i);
        self.save();
        Some(item)
    }

    pub fn clear(&mut self) {
        self.items.clear();
        self.save();
    }

    /// Photos whose file was deleted or moved away fall off by themselves.
    pub fn prune(&mut self) -> bool {
        let before = self.items.len();
        self.items.retain(|i| i.path.is_file());
        let changed = self.items.len() != before;
        if changed {
            self.save();
        }
        changed
    }

    /// After an edit, the photo shows the new version.
    pub fn refresh(&mut self, path: &Path) -> bool {
        let Some(item) = self.items.iter_mut().find(|i| i.path == path) else { return false };
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
        self.items
            .iter()
            .map(|i| ItemView {
                id: i.id,
                name: i.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                thumb: i.thumb.clone(),
                width: i.width,
                height: i.height,
            })
            .collect()
    }

    fn save(&self) {
        let paths: Vec<&PathBuf> = self.items.iter().map(|i| &i.path).collect();
        if let Some(dir) = self.store.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string_pretty(&paths) {
            let _ = fs::write(&self.store, text);
        }
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

/// A PNG of the photo for the drag preview, written to the cache folder.
pub fn drag_preview(path: &Path, cache: &Path) -> Option<PathBuf> {
    let img = image::open(path).ok()?.thumbnail(160, 160);
    let _ = fs::create_dir_all(cache);
    let out = cache.join("drag-preview.png");
    img.save_with_format(&out, image::ImageFormat::Png).ok()?;
    Some(out)
}
