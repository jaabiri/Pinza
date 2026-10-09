//! Every screenshot Pinza has hung, with the text read from it, so you can
//! search them later. Also where auto clean-up finds what to put away.

use crate::line::{kind_of, now, Kind};
use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

const LIMIT: usize = 3000;

#[derive(Serialize, Deserialize, Clone)]
pub struct Entry {
    pub path: PathBuf,
    pub added: u64,
    #[serde(default)]
    pub text: Option<String>,
    #[serde(default)]
    pub kept: bool,
}

#[derive(Serialize, Clone)]
pub struct Hit {
    pub path: String,
    pub name: String,
    pub added: u64,
    pub snippet: String,
    pub kind: Kind,
}

pub struct History {
    entries: Vec<Entry>,
    store: PathBuf,
}

impl History {
    pub fn load(store: PathBuf) -> Self {
        let entries = fs::read_to_string(&store).ok().and_then(|t| serde_json::from_str(&t).ok()).unwrap_or_default();
        Self { entries, store }
    }

    pub fn record(&mut self, path: &Path) {
        if self.entries.iter().any(|e| e.path == path) {
            return;
        }
        self.entries.push(Entry { path: path.to_path_buf(), added: now(), text: None, kept: false });
        if self.entries.len() > LIMIT {
            let extra = self.entries.len() - LIMIT;
            self.entries.drain(..extra);
        }
        self.save();
    }

    pub fn set_text(&mut self, path: &Path, text: String) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.path == path) {
            e.text = Some(text);
            self.save();
        }
    }

    pub fn set_kept(&mut self, path: &Path, kept: bool) {
        if let Some(e) = self.entries.iter_mut().find(|e| e.path == path) {
            e.kept = kept;
            self.save();
        }
    }

    pub fn forget(&mut self, path: &Path) {
        self.entries.retain(|e| e.path != path);
        self.save();
    }

    /// Pictures whose text has not been read yet, newest first.
    pub fn unread(&self) -> Vec<PathBuf> {
        self.entries
            .iter()
            .rev()
            .filter(|e| e.text.is_none() && kind_of(&e.path) != Kind::Video && e.path.is_file())
            .map(|e| e.path.clone())
            .collect()
    }

    /// Screenshots older than `hours` that you did not keep.
    pub fn due(&self, hours: u64) -> Vec<PathBuf> {
        let cutoff = now().saturating_sub(hours * 3600);
        self.entries.iter().filter(|e| !e.kept && e.added <= cutoff && e.path.is_file()).map(|e| e.path.clone()).collect()
    }

    /// Every word must appear in the text, the file name or the date.
    pub fn search(&self, query: &str, limit: usize) -> Vec<Hit> {
        let words: Vec<String> = query.split_whitespace().map(str::to_lowercase).collect();
        let mut hits = Vec::new();
        for e in self.entries.iter().rev() {
            if hits.len() >= limit {
                break;
            }
            if !e.path.is_file() {
                continue;
            }
            let name = e.path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let text = e.text.clone().unwrap_or_default();
            let haystack = format!("{}\n{}\n{}", name, text, date(e.added)).to_lowercase();
            if !words.iter().all(|w| haystack.contains(w.as_str())) {
                continue;
            }
            hits.push(Hit {
                path: e.path.to_string_lossy().into_owned(),
                name,
                added: e.added,
                snippet: snippet(&text, &words),
                kind: kind_of(&e.path),
            });
        }
        hits
    }

    fn save(&self) {
        if let Some(dir) = self.store.parent() {
            let _ = fs::create_dir_all(dir);
        }
        if let Ok(text) = serde_json::to_string(&self.entries) {
            let _ = fs::write(&self.store, text);
        }
    }
}

/// The bit of text around the first word that matched.
fn snippet(text: &str, words: &[String]) -> String {
    let flat: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let lower = flat.to_lowercase();
    let at = words.iter().filter_map(|w| lower.find(w.as_str())).min().unwrap_or(0);
    let chars: Vec<char> = flat.chars().collect();
    let start_char = lower[..at].chars().count().saturating_sub(30);
    let piece: String = chars.iter().skip(start_char).take(110).collect();
    if start_char > 0 {
        format!("…{piece}")
    } else {
        piece
    }
}

/// `2026-10-09`, in UTC, so a search for a day finds it.
pub fn date(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    // Howard Hinnant's days-to-civil.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates() {
        assert_eq!(date(0), "1970-01-01");
        assert_eq!(date(1_791_590_400), "2026-10-10");
    }

    #[test]
    fn snippets_start_near_the_match() {
        let text = "a ".repeat(50) + "needle here";
        let s = snippet(&text, &["needle".into()]);
        assert!(s.starts_with('…') && s.contains("needle"));
    }
}
