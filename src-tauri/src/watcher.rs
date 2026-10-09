//! Notices new screenshots in the watched folders.

use crate::folders;
use notify::{RecursiveMode, Watcher};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, SystemTime};
use tauri::AppHandle;

/// Screenshot tools often write a temp file and rename it, or write in
/// several steps. Waiting for a quiet moment avoids reading half a file.
const SETTLE: Duration = Duration::from_millis(250);
const RETRIES: u32 = 8;

type Known = HashMap<PathBuf, SystemTime>;

pub fn start(app: AppHandle, dirs: Vec<PathBuf>) {
    thread::spawn(move || {
        let (tx, rx) = mpsc::channel();
        let mut watcher = match notify::recommended_watcher(move |res| {
            let _ = tx.send(res);
        }) {
            Ok(w) => w,
            Err(e) => {
                log::error!("Cannot watch folders: {e}");
                return;
            }
        };
        for dir in &dirs {
            match watcher.watch(dir, RecursiveMode::NonRecursive) {
                Ok(()) => log::info!("Watching {}", dir.display()),
                Err(e) => log::warn!("Cannot watch {}: {e}", dir.display()),
            }
        }

        // Only what appears after launch is new.
        let mut known: Known = dirs.iter().flat_map(|d| listing(d)).collect();
        let mut pending: HashMap<PathBuf, u32> = HashMap::new();

        loop {
            let first = rx.recv();
            if first.is_err() {
                return;
            }
            // Collect everything that arrives until things settle.
            while rx.recv_timeout(SETTLE).is_ok() {}
            scan(&app, &dirs, &mut known, &mut pending);
            while !pending.is_empty() {
                thread::sleep(SETTLE * 2);
                while rx.try_recv().is_ok() {}
                scan(&app, &dirs, &mut known, &mut pending);
            }
        }
    });
}

fn scan(app: &AppHandle, dirs: &[PathBuf], known: &mut Known, pending: &mut HashMap<PathBuf, u32>) {
    let mut now: Known = HashMap::new();
    let mut fresh: Vec<(PathBuf, SystemTime)> = Vec::new();
    for dir in dirs {
        for (path, modified) in listing(dir) {
            match known.get(&path) {
                None => {
                    if folders::is_candidate(&path, dir) {
                        fresh.push((path.clone(), modified));
                    }
                }
                Some(old) if *old != modified => crate::refresh(app, &path),
                _ => {}
            }
            now.insert(path, modified);
        }
    }
    fresh.sort_by_key(|(_, t)| *t);
    for (path, _) in fresh {
        if crate::hang(app, &path) {
            pending.remove(&path);
        } else {
            // Not readable yet. Try again shortly, but not forever.
            let tries = pending.entry(path.clone()).or_insert(0);
            *tries += 1;
            if *tries < RETRIES {
                now.remove(&path);
            } else {
                pending.remove(&path);
            }
        }
    }
    if known.keys().any(|p| !now.contains_key(p)) {
        crate::prune(app);
    }
    *known = now;
}

fn listing(dir: &Path) -> Vec<(PathBuf, SystemTime)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    entries
        .flatten()
        .filter_map(|e| {
            let meta = e.metadata().ok()?;
            if !meta.is_file() || !folders::is_image(&e.path()) {
                return None;
            }
            Some((e.path(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}
