//! Notices new screenshots and screen recordings in the watched folders.

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
/// A recording is done once the file has not changed for this long.
const VIDEO_SETTLED: Duration = Duration::from_secs(3);

type Known = HashMap<PathBuf, SystemTime>;

/// `lines[i]` lists the folders whose new files hang on line `i`.
pub fn start(app: AppHandle, lines: Vec<Vec<PathBuf>>) {
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
        // A folder listed for two lines belongs to the first.
        let mut dirs: Vec<(PathBuf, usize)> = Vec::new();
        for (line, folders) in lines.iter().enumerate() {
            for dir in folders {
                if dirs.iter().any(|(d, _)| d == dir) {
                    continue;
                }
                match watcher.watch(dir, RecursiveMode::NonRecursive) {
                    Ok(()) => log::info!("Watching {} for line {}", dir.display(), line + 1),
                    Err(e) => log::warn!("Cannot watch {}: {e}", dir.display()),
                }
                dirs.push((dir.clone(), line));
            }
        }

        // Only what appears after launch is new.
        let mut known: Known = dirs.iter().flat_map(|(d, _)| listing(d)).collect();
        let mut pending: HashMap<PathBuf, u32> = HashMap::new();

        loop {
            if rx.recv().is_err() {
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

fn scan(app: &AppHandle, dirs: &[(PathBuf, usize)], known: &mut Known, pending: &mut HashMap<PathBuf, u32>) {
    let mut now: Known = HashMap::new();
    let mut fresh: Vec<(PathBuf, SystemTime, usize)> = Vec::new();
    for (dir, line) in dirs {
        for (path, modified) in listing(dir) {
            match known.get(&path) {
                None => {
                    if folders::is_candidate(&path, dir) {
                        fresh.push((path.clone(), modified, *line));
                    }
                }
                Some(old) if *old != modified => crate::refresh(app, &path),
                _ => {}
            }
            now.insert(path, modified);
        }
    }
    fresh.sort_by_key(|(_, t, _)| *t);
    for (path, modified, line) in fresh {
        // A recording grows until it is stopped: wait until it settles.
        if folders::is_video(&path) {
            let settled = SystemTime::now().duration_since(modified).unwrap_or_default() >= VIDEO_SETTLED;
            if !settled {
                pending.insert(path.clone(), 0);
                now.remove(&path);
                continue;
            }
        }
        if crate::hang(app, line, &path) {
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
    pending.retain(|p, _| p.exists());
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
            if !meta.is_file() || !folders::is_media(&e.path()) {
                return None;
            }
            Some((e.path(), meta.modified().unwrap_or(SystemTime::UNIX_EPOCH)))
        })
        .collect()
}
