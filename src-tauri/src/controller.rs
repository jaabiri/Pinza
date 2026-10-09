//! Bringing the line down and tucking it away, like an auto-hiding Dock.
//!
//! Two ways of knowing where the pointer is:
//! - `Mode::Poll` (macOS, Windows): ask the system for the cursor position a
//!   few times a second, and let clicks pass through except over photos.
//! - `Mode::Shape` (Linux): Wayland never tells an app where the pointer is,
//!   so the window shrinks its input region to the photos and the web view
//!   reports when the pointer comes and goes.

use crate::AppState;
use serde::{Deserialize, Serialize};
use std::thread;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize};

pub const WINDOW: &str = "line";
pub const PANEL_HEIGHT: f64 = 210.0;

const TICK: Duration = Duration::from_millis(40);
const EDGE_DWELL: Duration = Duration::from_millis(180);
const LEAVE_GRACE: Duration = Duration::from_millis(600);
const HIDE_AFTER_TUCK: Duration = Duration::from_millis(300);

#[derive(Clone, Copy, PartialEq, Eq, Serialize, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Poll,
    Shape,
}

#[derive(Deserialize, Clone, Copy, Debug, Default)]
pub struct Rect {
    pub x: f64,
    pub y: f64,
    pub w: f64,
    pub h: f64,
}

impl Rect {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x <= self.x + self.w && y >= self.y && y <= self.y + self.h
    }
}

pub enum Reveal {
    /// A new screenshot shows itself for a moment, then tucks away.
    Peek(Duration),
    /// Opened on purpose: stays until the pointer has visited and left.
    Pin,
    /// The pointer rested at the top edge.
    Hover,
}

#[derive(Default)]
pub struct Ctl {
    pub revealed: bool,
    pinned: bool,
    visited: bool,
    pub inside: bool,
    pub dragging: bool,
    peek_until: Option<Instant>,
    hot_since: Option<Instant>,
    away_since: Option<Instant>,
    ignoring: Option<bool>,
    pub hit_rects: Vec<Rect>,
    /// Window origin and scale, in physical pixels, from the last placement.
    origin: (f64, f64),
    size: (f64, f64),
    scale: f64,
    generation: u64,
}

pub fn reveal(app: &AppHandle, how: Reveal) {
    let st = app.state::<AppState>();
    let mut c = st.ctl.lock().unwrap();
    let now = Instant::now();
    match how {
        Reveal::Peek(d) => c.peek_until = Some(c.peek_until.map_or(now + d, |t| t.max(now + d))),
        Reveal::Pin => {
            c.pinned = true;
            c.visited = false;
        }
        Reveal::Hover => c.visited = true,
    }
    c.away_since = None;
    if c.revealed {
        return;
    }
    c.revealed = true;
    c.generation += 1;
    drop(c);
    if let Some(win) = app.get_webview_window(WINDOW) {
        place(app);
        let _ = win.show();
    }
    let _ = app.emit("reveal", true);
}

pub fn tuck(app: &AppHandle) {
    let st = app.state::<AppState>();
    let mut c = st.ctl.lock().unwrap();
    if !c.revealed {
        return;
    }
    c.revealed = false;
    c.pinned = false;
    c.visited = false;
    c.inside = false;
    c.peek_until = None;
    c.away_since = None;
    c.generation += 1;
    let generation = c.generation;
    drop(c);
    let _ = app.emit("reveal", false);

    // On macOS and Windows the window is put away once the slide is over. On
    // Linux it stays, a few pixels tall, to catch the pointer at the edge.
    if st.mode == Mode::Poll {
        let app = app.clone();
        thread::spawn(move || {
            thread::sleep(HIDE_AFTER_TUCK);
            let st = app.state::<AppState>();
            let c = st.ctl.lock().unwrap();
            if !c.revealed && c.generation == generation {
                if let Some(win) = app.get_webview_window(WINDOW) {
                    let _ = win.hide();
                }
            }
        });
    }
}

pub fn toggle(app: &AppHandle) {
    let revealed = app.state::<AppState>().ctl.lock().unwrap().revealed;
    if revealed {
        tuck(app);
    } else {
        reveal(app, Reveal::Pin);
    }
}

/// The line hangs along the top of the screen you are using: the one with
/// the pointer, which is where you just took the screenshot.
pub fn place(app: &AppHandle) {
    let st = app.state::<AppState>();
    let Some(win) = app.get_webview_window(WINDOW) else { return };

    #[cfg(target_os = "linux")]
    if st.layer_shell {
        // The compositor places a layer surface. Mapping it again moves it to
        // the output that has focus now; `reveal` shows it right after.
        let _ = win.hide();
        return;
    }

    let monitor = app
        .cursor_position()
        .ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten())
        .or_else(|| win.current_monitor().ok().flatten())
        .or_else(|| app.primary_monitor().ok().flatten());
    let Some(m) = monitor else { return };
    let area = m.work_area();
    let scale = m.scale_factor();
    let height = (PANEL_HEIGHT * scale).round();
    let _ = win.set_position(PhysicalPosition::new(area.position.x, area.position.y));
    let _ = win.set_size(PhysicalSize::new(area.size.width, height as u32));
    let mut c = st.ctl.lock().unwrap();
    c.origin = (area.position.x as f64, area.position.y as f64);
    c.size = (area.size.width as f64, height);
    c.scale = scale;
}

/// Whether the pointer rests at the top of a screen. On macOS that is the
/// whole menu bar, elsewhere the top couple of pixels.
fn at_top_edge(app: &AppHandle, x: f64, y: f64) -> bool {
    let Ok(Some(m)) = app.monitor_from_point(x, y) else { return false };
    let top = m.position().y as f64;
    let work_top = m.work_area().position.y as f64;
    y <= (top + 1.0).max(work_top - 1.0)
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn pointer(app: &AppHandle, inside: bool) {
    app.state::<AppState>().ctl.lock().unwrap().inside = inside;
}

/// Starts the loop that watches the pointer and decides when to tuck away.
pub fn start(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(TICK);
        tick(&app);
    });
}

fn tick(app: &AppHandle) {
    let st = app.state::<AppState>();
    let now = Instant::now();
    let edge = st.config.lock().unwrap().reveal_at_edge;
    let has_items = st.line.lock().unwrap().len() > 0;

    // Window calls may wait on the main thread, so none is made while the
    // lock is held.
    if st.mode == Mode::Poll {
        if let Ok(p) = app.cursor_position() {
            let revealed = st.ctl.lock().unwrap().revealed;
            if !revealed {
                let hot = edge && has_items && at_top_edge(app, p.x, p.y);
                let mut c = st.ctl.lock().unwrap();
                if !hot {
                    c.hot_since = None;
                } else if now - *c.hot_since.get_or_insert(now) >= EDGE_DWELL {
                    c.hot_since = None;
                    drop(c);
                    reveal(app, Reveal::Hover);
                    return;
                }
            } else {
                let mut c = st.ctl.lock().unwrap();
                let (ox, oy) = c.origin;
                let (w, h) = c.size;
                // The strip above the window counts too: on macOS that is the
                // menu bar the pointer came from.
                c.inside = p.x >= ox && p.x <= ox + w && p.y <= oy + h && p.y >= oy - 48.0;
                let s = if c.scale > 0.0 { c.scale } else { 1.0 };
                let (lx, ly) = ((p.x - ox) / s, (p.y - oy) / s);
                let over = c.dragging || c.hit_rects.iter().any(|r| r.contains(lx, ly));
                let changed = c.ignoring != Some(!over);
                c.ignoring = Some(!over);
                drop(c);
                if changed {
                    if let Some(win) = app.get_webview_window(WINDOW) {
                        let _ = win.set_ignore_cursor_events(!over);
                    }
                }
            }
        }
    }

    let mut c = st.ctl.lock().unwrap();
    if !c.revealed || c.dragging {
        return;
    }
    if c.inside {
        c.visited = true;
        c.away_since = None;
        return;
    }
    let peeking = c.peek_until.is_some_and(|t| now < t);
    let held = c.pinned && !c.visited;
    if peeking || held {
        c.away_since = None;
        return;
    }
    let since = *c.away_since.get_or_insert(now);
    if now - since >= LEAVE_GRACE {
        drop(c);
        tuck(app);
    }
}
