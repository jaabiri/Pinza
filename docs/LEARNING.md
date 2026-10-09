# Learning Rust with Pinza

This guide walks through Pinza as a learning project. Each section names a
technique, shows where Pinza uses it (links go to the exact lines), explains
*why* it is done that way, and ends with links for a deep dive.

You don't need to read it in order. If you are new to Rust, follow the
[reading path](#a-reading-path) at the end.

**Contents**

1. [The shape of the app](#1-the-shape-of-the-app)
2. [Rust techniques](#2-rust-techniques)
   - [Ownership across threads: `Mutex` in shared state](#21-ownership-across-threads-mutex-in-shared-state)
   - [Channels and worker threads](#22-channels-and-worker-threads)
   - [Avoiding deadlocks: never hold a lock across a wait](#23-avoiding-deadlocks-never-hold-a-lock-across-a-wait)
   - [`Option` and `Result` instead of exceptions](#24-option-and-result-instead-of-exceptions)
   - [`let … else` and early returns](#25-let--else-and-early-returns)
   - [Closures and `move`](#26-closures-and-move)
   - [Lazy statics with `OnceLock`](#27-lazy-statics-with-oncelock)
   - [One codebase, three OSes: `#[cfg]`](#28-one-codebase-three-oses-cfg)
   - [Serde: config files that never break](#29-serde-config-files-that-never-break)
   - [Enums that carry data](#210-enums-that-carry-data)
   - [Iterators](#211-iterators)
   - [Unit tests next to the code](#212-unit-tests-next-to-the-code)
   - [`unsafe`, only where the OS demands it](#213-unsafe-only-where-the-os-demands-it)
3. [Tauri: a Rust backend with a web frontend](#3-tauri-a-rust-backend-with-a-web-frontend)
4. [The system APIs, per platform](#4-the-system-apis-per-platform)
   - [Linux / Wayland / Hyprland](#41-linux--wayland--hyprland)
   - [macOS](#42-macos)
   - [Windows 11](#43-windows-11)
5. [The crates, and what each is for](#5-the-crates-and-what-each-is-for)
6. [Algorithms worth knowing](#6-algorithms-worth-knowing)
7. [Building, testing, shipping](#7-building-testing-shipping)
8. [Exercises](#8-exercises)
9. [A reading path](#a-reading-path)

---

## 1. The shape of the app

```
┌────────────────────────── Rust (src-tauri/src) ──────────────────────────┐
│                                                                          │
│  watcher.rs ──new file──▶ main.rs::hang() ──▶ line.rs (Lines, thumbnails) │
│      ▲                        │                    │                     │
│  notify (OS file events)      │ emit("items")      └─▶ history.rs        │
│                               ▼                         (search, clean)  │
│  controller.rs ◀── cursor / GTK pointer events      ocr.rs, images.rs,   │
│  (reveal / tuck, click-through, keyboard)           share.rs, theme.rs,  │
│                                                     fullscreen.rs        │
│  commands.rs ◀──────── invoke("copy", {id}) ────────────────┐            │
└───────────────────────────────┬──────────────────────────────┼────────────┘
                     events ▼   │                              │ commands
┌────────────────────────── Web view (src) ────────────────────┴───────────┐
│  main.js: rope, clips, spring physics, gestures, keyboard, action bar    │
│  search.js: the search window                                            │
└──────────────────────────────────────────────────────────────────────────┘
```

The split is the core design decision. **Rust owns everything that touches
the system**: files, clipboard, windows, OCR, the network. **The web view
only draws** and reports what your hands did. They talk in two directions:

- JS → Rust: `invoke("command", args)` calls a `#[tauri::command]` function.
- Rust → JS: `app.emit("event", payload)` and JS `listen("event", …)`.

Start reading at [`main.rs::main`](../src-tauri/src/main.rs#L242), then
follow one screenshot through [`watcher.rs::scan`](../src-tauri/src/watcher.rs#L69)
→ [`main.rs::hang`](../src-tauri/src/main.rs#L80) →
[`main.js` `listen("items")`](../src/main.js#L788).

---

## 2. Rust techniques

### 2.1 Ownership across threads: `Mutex` in shared state

**Where:** [`AppState`](../src-tauri/src/main.rs#L30)

```rust
pub struct AppState {
    pub lines: Mutex<Lines>,
    pub history: Mutex<History>,
    pub config: Mutex<Config>,
    pub ctl: Mutex<Ctl>,
    ...
}
```

**Why:** Pinza runs many threads at once: the file watcher, the pointer
loop, the OCR reader, the clean-up timer, the phone server, and Tauri's own
command threads. Rust will not compile code where two threads could change
the same data unguarded. Wrapping each piece in a `Mutex` makes the rule
explicit: to touch `lines` you must `lock()` it, and the lock is released
when the guard goes out of scope (RAII). Tauri stores `AppState` once with
`app.manage(...)` ([main.rs#L341](../src-tauri/src/main.rs#L341)), and any
thread holding an `AppHandle` can get it back with `app.state::<AppState>()`.

There are four separate locks instead of one big one, so the OCR thread
writing to `history` never waits for the pointer loop reading `ctl`.

**Deep dive**
- The Rust Book, ch. 16, Fearless Concurrency: <https://doc.rust-lang.org/book/ch16-00-concurrency.html>
- `std::sync::Mutex`: <https://doc.rust-lang.org/std/sync/struct.Mutex.html>
- Mara Bos, *Rust Atomics and Locks* (free online): <https://marabos.nl/atomics/>

### 2.2 Channels and worker threads

**Where:**
- The clipboard thread: [`actions.rs` `Clipboard`](../src-tauri/src/actions.rs#L18)
- The OCR reader queue: [`main.rs::start_reader`](../src-tauri/src/main.rs#L140)
- The file watcher: [`watcher.rs`](../src-tauri/src/watcher.rs#L25)

```rust
pub struct Clipboard(mpsc::Sender<Clip>);
...
thread::spawn(move || {
    let mut board = None;           // lives as long as the thread
    for clip in rx { /* copy it */ }
});
```

**Why:** On Linux, whoever *owns* the clipboard must stay alive to serve
the data when you paste. If the clipboard object were dropped at the end of
the click handler, the copied image would vanish. So one thread owns it for
the whole run, and everyone else sends it messages through a channel. This
is the *actor* pattern: own the resource in one place and talk to it with
messages instead of sharing it.

`for clip in rx` ends by itself when every `Sender` is dropped, so there is
no shutdown code to write.

**Deep dive**
- `std::sync::mpsc`: <https://doc.rust-lang.org/std/sync/mpsc/>
- The Book, Using Message Passing: <https://doc.rust-lang.org/book/ch16-02-message-passing.html>
- Alice Ryhl, *Actors with Tokio* (async, but the same idea): <https://ryhl.io/blog/actors-with-tokio/>

### 2.3 Avoiding deadlocks: never hold a lock across a wait

**Where:** [`controller.rs::tick`](../src-tauri/src/controller.rs#L273): read
the comment at [line 279](../src-tauri/src/controller.rs#L279).

```rust
let revealed = st.ctl.lock().unwrap().revealed;   // lock, copy, unlock
if !revealed {
    let hot = at_top_edge(app, p.x, p.y);         // may wait for the main thread
    let mut c = st.ctl.lock().unwrap();           // lock again
    ...
```

**Why:** Some Tauri window calls (asking which monitor is under a point,
for example) are answered by the main UI thread. If the pointer thread held
`ctl` while waiting for the main thread, and the main thread meanwhile
wanted `ctl` (say, a tray click calling `toggle`), each would wait for the
other forever. The fix is a discipline: **copy what you need out of the
lock, release it, then make the slow call.** Notice also `drop(c)` before
`reveal(...)` and `tuck(...)` throughout `controller.rs`.

A related rule in [`commands.rs`](../src-tauri/src/commands.rs#L23):
non-`async` Tauri commands run *on the main thread*, so anything slow (OCR,
image work, your own commands) is moved to `background(...)` so the line
keeps animating.

**Deep dive**
- Dining philosophers, the classic deadlock: <https://en.wikipedia.org/wiki/Dining_philosophers_problem>
- Tauri, sync vs async commands: <https://v2.tauri.app/develop/calling-rust/#async-commands>

### 2.4 `Option` and `Result` instead of exceptions

**Where:** almost everywhere. Good examples:
- [`line.rs::thumbnail`](../src-tauri/src/line.rs#L286) returns `Option`: no thumbnail means "not readable yet, try later"
- [`images.rs::join`](../src-tauri/src/images.rs#L129) returns `Result<PathBuf, String>` and uses `?`
- [`line.rs` load](../src-tauri/src/line.rs#L88): `unwrap_or_else` falls back to the old file format

```rust
let img = image::open(path).ok()?;           // Result → Option, bail on error
let out = cmd.output().map_err(|e| e.to_string())?;   // change the error type, bail
```

**Why:** In Rust a function that can fail says so in its type, and the
compiler makes the caller deal with it. `?` is the short way to say "if
this failed, return the failure now". Pinza turns most errors into a
`String`, because they end up in a toast for the user. A library would use
a proper error enum (see `thiserror`).

**Deep dive**
- The Book, ch. 9, Error Handling: <https://doc.rust-lang.org/book/ch09-00-error-handling.html>
- `Option` combinators cheat sheet: <https://doc.rust-lang.org/std/option/enum.Option.html#implementations>
- `thiserror` / `anyhow`, the usual error crates: <https://docs.rs/thiserror> · <https://docs.rs/anyhow>

### 2.5 `let … else` and early returns

**Where:** [`commands.rs`](../src-tauri/src/commands.rs#L73), [`controller.rs::place`](../src-tauri/src/controller.rs), [`theme.rs`](../src-tauri/src/theme.rs#L32)

```rust
let Some((path, kind)) = item_of(&state, id) else { return };
```

**Why:** It keeps the happy path flat instead of nesting `if let` four
levels deep. The `else` branch must leave (`return`, `continue`, `break`),
which the compiler checks.

**Deep dive:** Rust by Example, `let-else`: <https://doc.rust-lang.org/rust-by-example/flow_control/let_else.html>

### 2.6 Closures and `move`

**Where:** every `thread::spawn(move || …)`, the drag callback in
[`commands.rs::start_drag`](../src-tauri/src/commands.rs#L383), GTK signal
handlers in [`linux.rs::watch_pointer`](../src-tauri/src/linux.rs#L88).

**Why:** A thread or callback may outlive the function that created it, so
it must *own* what it uses. `move` transfers ownership into the closure.
That's why you see `let app = app.clone();` just before: an `AppHandle` is
cheap to clone (it's a reference-counted handle), so each closure gets its
own.

**Deep dive**
- The Book, Closures: <https://doc.rust-lang.org/book/ch13-01-closures.html>
- `Fn`, `FnMut`, `FnOnce` explained: <https://doc.rust-lang.org/book/ch13-01-closures.html#moving-captured-values-out-of-closures-and-the-fn-traits>

### 2.7 Lazy statics with `OnceLock`

**Where:** compiled regexes in [`images.rs::patterns`](../src-tauri/src/images.rs#L13), the phone server in [`share.rs`](../src-tauri/src/share.rs#L25).

```rust
static P: OnceLock<Vec<Regex>> = OnceLock::new();
P.get_or_init(|| /* compile once */)
```

**Why:** Compiling a regex is slow, and the phone server should start only
the first time you share, then stay. `OnceLock` runs the initialiser exactly
once, safely, even if two threads race to it.

**Deep dive:** `std::sync::OnceLock`: <https://doc.rust-lang.org/std/sync/struct.OnceLock.html>

### 2.8 One codebase, three OSes: `#[cfg]`

**Where:**
- Per-OS modules: [`ocr.rs`](../src-tauri/src/ocr.rs#L72) has `mod imp` three times, one per OS
- Per-OS dependencies: [`Cargo.toml`](../src-tauri/Cargo.toml) (`[target.'cfg(target_os = "windows")'.dependencies]`)
- Per-OS lines inside a function: [`controller.rs::keyboard`](../src-tauri/src/controller.rs#L125)

```rust
#[cfg(target_os = "linux")]   mod imp { /* tesseract */ }
#[cfg(target_os = "macos")]   mod imp { /* Vision */ }
#[cfg(target_os = "windows")] mod imp { /* Windows.Media.Ocr */ }
pub fn recognize(...) { imp::recognize(...) }   // same API for all
```

**Why:** Code for other OSes isn't just skipped at runtime: it isn't
compiled at all, so a Linux build doesn't need Windows libraries. Each OS
gets a module with the same function names, and the rest of the app calls
one API.

The catch: you only know the macOS branch compiles when you build *for*
macOS. Pinza type-checks Windows from Linux with
`cargo check --target x86_64-pc-windows-msvc`, and leaves macOS to CI.

**Deep dive**
- Conditional compilation: <https://doc.rust-lang.org/reference/conditional-compilation.html>
- Platform-specific dependencies: <https://doc.rust-lang.org/cargo/reference/specifying-dependencies.html#platform-specific-dependencies>
- Rust platform support and targets: <https://doc.rust-lang.org/rustc/platform-support.html>

### 2.9 Serde: config files that never break

**Where:** [`config.rs`](../src-tauri/src/config.rs#L8), [`line.rs` `StoreFile`](../src-tauri/src/line.rs#L55)

```rust
#[derive(Serialize, Deserialize)]
#[serde(default)]          // any missing field takes its Default value
pub struct Config { ... }
```

**Why:** `#[serde(default)]` plus `impl Default` means an old
`config.json` missing new fields still loads, and new fields get sensible
values. `line.rs` goes further: if the file is in the *first* version's
format (a plain list of paths), `unwrap_or_else` converts it. This is
schema migration in a few lines.

**Deep dive**
- Serde: <https://serde.rs/> and its attributes: <https://serde.rs/attributes.html>
- `#[serde(default)]`: <https://serde.rs/container-attrs.html#default>

### 2.10 Enums that carry data

**Where:** [`actions.rs` `Clip`](../src-tauri/src/actions.rs), [`controller.rs` `Reveal`](../src-tauri/src/controller.rs), [`line.rs` `Kind`](../src-tauri/src/line.rs#L15)

```rust
pub enum Clip { Image(PathBuf), Text(String), Files(Vec<PathBuf>) }
pub enum Reveal { Peek(Duration), Pin, Hover }
```

**Why:** One message type for "anything the clipboard can hold", and the
`match` that handles it must cover every case. Add a variant and the
compiler shows you every place to update.

**Deep dive:** The Book, ch. 6, Enums and Pattern Matching: <https://doc.rust-lang.org/book/ch06-00-enums.html>

### 2.11 Iterators

**Where:** [`history.rs::search`](../src-tauri/src/history.rs), [`images.rs::join`](../src-tauri/src/images.rs#L129), [`line.rs::views`](../src-tauri/src/line.rs)

```rust
let images: Vec<DynamicImage> = paths.iter()
    .map(|p| image::open(p).map_err(|e| format!("{}: {e}", p.display())))
    .collect::<Result<_, _>>()?;      // stops at the first error
```

**Why:** `collect::<Result<Vec<_>, _>>()` turns "a list of results" into
"a result of a list". It's one of the most useful tricks in Rust.

**Deep dive**
- The Book, Iterators: <https://doc.rust-lang.org/book/ch13-02-iterators.html>
- `Iterator` docs (skim the method list once): <https://doc.rust-lang.org/std/iter/trait.Iterator.html>

### 2.12 Unit tests next to the code

**Where:** the `#[cfg(test)] mod tests` blocks at the end of
[`images.rs`](../src-tauri/src/images.rs), [`ocr.rs`](../src-tauri/src/ocr.rs),
[`history.rs`](../src-tauri/src/history.rs) and [`fullscreen.rs`](../src-tauri/src/fullscreen.rs).
Run them with `cargo test` in `src-tauri`.

**Why:** The parts worth testing are pure functions: parse Tesseract
output, find secrets in text, turn days into a date, read Hyprland's JSON.
Keeping them free of windows and files is what makes them testable: a
design choice, not an accident.

**Deep dive:** The Book, ch. 11, Writing Tests: <https://doc.rust-lang.org/book/ch11-00-testing.html>

### 2.13 `unsafe`, only where the OS demands it

**Where:** a single call, [`fullscreen.rs`](../src-tauri/src/fullscreen.rs#L113):
`unsafe { SHQueryUserNotificationState() }`.

**Why:** Calling a C function from Windows can't be checked by the
compiler, so `windows-rs` marks it `unsafe`. Pinza keeps it to one line.
Everything else is safe Rust, even though the app talks to three operating
systems, because the crates wrap the unsafe parts for you.

**Deep dive:** The Book, Unsafe Rust: <https://doc.rust-lang.org/book/ch20-01-unsafe-rust.html>

---

## 3. Tauri: a Rust backend with a web frontend

Tauri uses the OS's own web view (WebKitGTK on Linux, WKWebView on macOS,
WebView2 on Windows) instead of bundling Chromium like Electron. That is
why the release build is a few MB.

| Concept | Where in Pinza | Docs |
|:--|:--|:--|
| Builder, plugins, `setup` | [`main.rs#L242`](../src-tauri/src/main.rs#L242) | <https://v2.tauri.app/develop/> |
| Commands (`#[tauri::command]`, `invoke`) | [`commands.rs`](../src-tauri/src/commands.rs#L27), [`generate_handler!`](../src-tauri/src/main.rs#L257) | <https://v2.tauri.app/develop/calling-rust/> |
| Events (`emit` / `listen`) | `emit_items` in [`main.rs`](../src-tauri/src/main.rs), [`main.js#L781`](../src/main.js#L781) | <https://v2.tauri.app/develop/calling-frontend/> |
| Shared state (`manage`, `State<T>`) | [`main.rs#L341`](../src-tauri/src/main.rs#L341) | <https://v2.tauri.app/develop/state-management/> |
| Capabilities (what JS may call) | [`capabilities/default.json`](../src-tauri/capabilities/default.json) | <https://v2.tauri.app/security/capabilities/> |
| Content Security Policy | [`tauri.conf.json`](../src-tauri/tauri.conf.json) | <https://v2.tauri.app/security/csp/> |
| Asset protocol (`convertFileSrc`), for playing videos from disk | [`main.rs#L304`](../src-tauri/src/main.rs#L304), [`main.js`](../src/main.js) | <https://v2.tauri.app/reference/javascript/api/namespacecore/#convertfilesrc> |
| Windows built in code (transparent, always on top, no focus) | [`main.rs` `WebviewWindowBuilder`](../src-tauri/src/main.rs) | <https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindowBuilder.html> |
| Click-through (`set_ignore_cursor_events`) | [`controller.rs`](../src-tauri/src/controller.rs#L273) | <https://docs.rs/tauri/latest/tauri/window/struct.Window.html#method.set_ignore_cursor_events> |
| Tray icon and menus | [`tray.rs`](../src-tauri/src/tray.rs#L28) | <https://v2.tauri.app/learn/system-tray/> · <https://v2.tauri.app/learn/window-menu/> |
| Plugin: single instance (`pinza toggle` reaches the running app) | [`main.rs#L229`](../src-tauri/src/main.rs#L229) | <https://v2.tauri.app/plugin/single-instance/> |
| Plugin: global shortcut | `register_shortcut` in [`main.rs`](../src-tauri/src/main.rs) | <https://v2.tauri.app/plugin/global-shortcut/> |
| Plugin: autostart | [`tray.rs`](../src-tauri/src/tray.rs) | <https://v2.tauri.app/plugin/autostart/> |
| Plugin: opener (open files, reveal in folder) | [`actions.rs`](../src-tauri/src/actions.rs) | <https://v2.tauri.app/plugin/opener/> |
| Reaching the native window (`gtk_window()`) | [`linux.rs`](../src-tauri/src/linux.rs#L18) | <https://docs.rs/tauri/latest/tauri/webview/struct.WebviewWindow.html#method.gtk_window> |
| Bundles (.deb, .dmg, .msi) and CI | [`.github/workflows/pinza.yml`](../.github/workflows/pinza.yml) | <https://v2.tauri.app/distribute/> · <https://v2.tauri.app/distribute/pipelines/github/> |

**One pattern to copy:** the web view never gets file paths to act on
directly. It sends an *id* (`invoke("trash", { ids: [7] })`), and Rust looks
up the path in its own state. A compromised or buggy page can only ask for
things that are already on the line, which makes the IPC boundary much
easier to reason about.

---

## 4. The system APIs, per platform

### 4.1 Linux / Wayland / Hyprland

**Wayland layer shell: an overlay that isn't a window.**
Under Wayland an app can't place its own window, can't stay on top, and
can't avoid taking focus. Panels and bars (Waybar) use the *layer shell*
protocol instead: a surface anchored to screen edges, on a layer above
windows, with control over keyboard focus.
[`linux.rs::init_layer_shell`](../src-tauri/src/linux.rs#L18) turns Tauri's
GTK window into one: anchored top/left/right, `Layer::Overlay`, exclusive
zone 0 (sits under Waybar), and `KeyboardMode::None`.
- Protocol: <https://wayland.app/protocols/wlr-layer-shell-unstable-v1>
- gtk-layer-shell: <https://github.com/wmww/gtk-layer-shell> · Rust crate: <https://docs.rs/gtk-layer-shell>
- The Wayland Book (the best introduction to Wayland): <https://wayland-book.com/>

**Input regions: clicks through the transparent parts.**
Wayland never tells an app where the pointer is unless it's over the app.
So instead of polling, Pinza sets the surface's *input region* to just the
photos ([`linux.rs::set_input_region`](../src-tauri/src/linux.rs#L67)).
Everywhere else, clicks fall through to the app below. The JS side computes
the rectangles ([`main.js` `reportShapes`](../src/main.js#L632)).
- GDK `input_shape_combine_region`: <https://docs.gtk.org/gdk3/method.Window.input_shape_combine_region.html>
- Wayland `wl_surface.set_input_region`: <https://wayland.app/protocols/wayland#wl_surface:request:set_input_region>

**Pointer enter/leave from GTK.**
[`linux.rs::watch_pointer`](../src-tauri/src/linux.rs#L88) listens to GTK
crossing events. Note the `NotifyType::Inferior` check: moving from the
window into its own web view child also fires "leave", and that must be
ignored.
- GTK3 crossing events: <https://docs.gtk.org/gdk3/struct.EventCrossing.html>

**Keyboard on demand.** [`linux.rs::set_keyboard`](../src-tauri/src/linux.rs#L39)
switches the layer surface to `Exclusive` keyboard mode while you use the
line from the keyboard, and back to `None` after.

**Running GTK code on the right thread.** GTK objects aren't `Send`;
they live on the main thread. `run_on_main_thread` moves the work there
([`linux.rs#L41`](../src-tauri/src/linux.rs#L41)).
- Tauri `run_on_main_thread`: <https://docs.rs/tauri/latest/tauri/struct.AppHandle.html#method.run_on_main_thread>

**Hyprland IPC: knowing about full screen and screen sharing.**
Hyprland publishes events on a Unix socket (`.socket2.sock`), one line per
event such as `fullscreen>>1` or `screencast>>1,0`.
[`fullscreen.rs`](../src-tauri/src/fullscreen.rs#L31) reads it with a
`UnixStream` and asks `hyprctl activewindow -j` for details.
- Hyprland IPC: <https://wiki.hypr.land/IPC/>
- `std::os::unix::net::UnixStream`: <https://doc.rust-lang.org/std/os/unix/net/struct.UnixStream.html>

**Omarchy.** Screenshots come from `hyprshot` into `$OMARCHY_SCREENSHOT_DIR`,
editing uses Satty, and the theme lives in `~/.config/omarchy/current/theme`
([`theme.rs`](../src-tauri/src/theme.rs#L32), which reads `alacritty.toml`).
- Omarchy: <https://github.com/basecamp/omarchy>
- Satty: <https://github.com/gabm/Satty>

**Tesseract OCR.** `tesseract file stdout tsv` prints one row per word with
its bounding box; [`ocr.rs::parse_tsv`](../src-tauri/src/ocr.rs#L96) groups
them back into lines.
- Command line usage: <https://tesseract-ocr.github.io/tessdoc/Command-Line-Usage.html>
- Improving accuracy: <https://tesseract-ocr.github.io/tessdoc/ImproveQuality.html>

**Native drag and drop.** The [`drag`](https://docs.rs/drag) crate starts a
real OS drag with GTK on Linux, so you can drop a screenshot into any app
([`commands.rs::start_drag`](../src-tauri/src/commands.rs#L383)).
- GTK3 drag and drop: <https://docs.gtk.org/gtk3/drag-and-drop.html>

### 4.2 macOS

- **No Dock icon:** `ActivationPolicy::Accessory` (a menu bar app) in [`main.rs`](../src-tauri/src/main.rs).
  Apple: <https://developer.apple.com/documentation/appkit/nsapplication/activationpolicy-swift.enum/accessory>
- **On every Space but not in full screen:** Tauri's `visible_on_all_workspaces` sets the window's collection behaviour.
  Apple: <https://developer.apple.com/documentation/appkit/nswindow/collectionbehavior-swift.struct>
- **Left out of screen sharing and screenshots:** `content_protected(true)` sets `NSWindow.sharingType = .none`.
  Apple: <https://developer.apple.com/documentation/appkit/nswindow/sharingtype-swift.property>
- **Reading text with Vision, without Objective-C in Rust:** [`ocr.rs`](../src-tauri/src/ocr.rs#L129) runs a small AppleScriptObjC script through `osascript` that calls `VNRecognizeTextRequest`. Calling a system framework from a script is a handy trick when writing bindings isn't worth it. For real bindings, see `objc2`.
  Vision: <https://developer.apple.com/documentation/vision/vnrecognizetextrequest> ·
  AppleScriptObjC: <https://developer.apple.com/library/archive/releasenotes/ScriptingAutomation/RN-AppleScriptObjC/> ·
  objc2: <https://docs.rs/objc2>
- **Where screenshots go:** `defaults read com.apple.screencapture location` in [`folders.rs`](../src-tauri/src/folders.rs).
- **Real screenshots on the Desktop** carry an extended attribute, read with the `xattr` crate: <https://docs.rs/xattr>

### 4.3 Windows 11

- **WinRT from Rust with `windows-rs`:** [`ocr.rs` Windows `run`](../src-tauri/src/ocr.rs#L202) opens the file with `StorageFile`, decodes it with `BitmapDecoder`, and runs `OcrEngine`. `.join()` waits for each async WinRT operation.
  windows-rs: <https://github.com/microsoft/windows-rs> · API search: <https://microsoft.github.io/windows-docs-rs/>
  OcrEngine: <https://learn.microsoft.com/en-us/uwp/api/windows.media.ocr.ocrengine>
- **Is a full screen app or presentation running?** `SHQueryUserNotificationState`, polled in [`fullscreen.rs`](../src-tauri/src/fullscreen.rs#L113).
  <https://learn.microsoft.com/en-us/windows/win32/api/shellapi/nf-shellapi-shqueryusernotificationstate>
- **Left out of screen capture:** `content_protected(true)` uses `SetWindowDisplayAffinity(WDA_EXCLUDEFROMCAPTURE)`.
  <https://learn.microsoft.com/en-us/windows/win32/api/winuser/nf-winuser-setwindowdisplayaffinity>
- **Not stealing focus:** `focusable(false)` makes a no-activate window (`WS_EX_NOACTIVATE`).
  <https://learn.microsoft.com/en-us/windows/win32/winmsg/extended-window-styles>
- **Snipping Tool's folders:** `Pictures\Screenshots` and `Videos\Screen Recordings`, found through `dirs` (Known Folders).
  <https://learn.microsoft.com/en-us/windows/win32/shell/knownfolderid>
- **WebView2**, the web view Tauri uses on Windows: <https://learn.microsoft.com/en-us/microsoft-edge/webview2/>

---

## 5. The crates, and what each is for

| Crate | Used for | Read |
|:--|:--|:--|
| `tauri` | App, windows, IPC, tray | <https://docs.rs/tauri> |
| `serde`, `serde_json` | Config, saved lines, history, IPC payloads | <https://serde.rs/> |
| `notify` | File system events (inotify, FSEvents, ReadDirectoryChangesW) | <https://docs.rs/notify> |
| `image` | Decoding, thumbnails, pixelating, joining | <https://docs.rs/image> |
| `arboard` | Clipboard: images, text, file lists | <https://docs.rs/arboard> |
| `trash` | Moving files to the system trash | <https://docs.rs/trash> |
| `drag` | Native drag out of the window | <https://docs.rs/drag> |
| `regex` | Finding secrets to redact | <https://docs.rs/regex> |
| `qrcode` | The Send to Phone QR code as SVG | <https://docs.rs/qrcode> |
| `rand` | The random link token | <https://docs.rs/rand> |
| `base64` | Thumbnails as `data:` URLs | <https://docs.rs/base64> |
| `dirs` | Pictures, Videos, config folders per OS | <https://docs.rs/dirs> |
| `gtk`, `gtk-layer-shell` | Linux window internals | <https://gtk-rs.org/gtk3-rs/stable/latest/docs/gtk/> · <https://docs.rs/gtk-layer-shell> |
| `windows` | WinRT OCR, shell state | <https://docs.rs/windows> |
| `xattr` | macOS screenshot tag | <https://docs.rs/xattr> |
| `log`, `env_logger` | `RUST_LOG=pinza=debug` | <https://docs.rs/env_logger> |

How to read a crate's docs quickly: open docs.rs, read the crate-level page
(usually it has an example), then search (`S` key) for the one type you
need. The "Source" links show you how it's really done.

---

## 6. Algorithms worth knowing

- **Damped spring for the swing** ([`main.js` `frame`](../src/main.js#L231)): each photo has an angle and a velocity; every frame, acceleration = −stiffness·angle − damping·velocity. Splitting each frame into 4 small steps keeps it stable when a frame is slow. This is semi-implicit Euler integration.
  <https://en.wikipedia.org/wiki/Semi-implicit_Euler_method> · Gaffer on Games, *Integration Basics*: <https://gafferongames.com/post/integration_basics/>
- **The rope's sag**: a parabola through the screen edges, 4·sag·f·(1−f). A real hanging rope is a catenary, but a parabola is indistinguishable at this size.
  <https://en.wikipedia.org/wiki/Catenary>
- **Debouncing file events** ([`watcher.rs`](../src-tauri/src/watcher.rs)): tools write a file in several steps, so Pinza waits until events stop for 250 ms, and until a recording hasn't changed for 3 s.
- **Matching words to a regex across word boxes** ([`images.rs::sensitive_areas`](../src-tauri/src/images.rs#L43)): rebuild the line's text, remember each word's byte span, run the regex, and blur every word whose span overlaps a match. Neighbouring hits merge into one area.
- **Pixelation that can't be undone** ([`images.rs::pixelate`](../src-tauri/src/images.rs#L90)): shrink the area, then enlarge it with nearest-neighbour. Blur alone can sometimes be reversed; big blocks can't.
- **Days since 1970 to a calendar date** ([`history.rs::date`](../src-tauri/src/history.rs#L142)), without a date library: Howard Hinnant's algorithm.
  <https://howardhinnant.github.io/date_algorithms.html#civil_from_days>
- **Finding your LAN address** ([`share.rs::lan_ip`](../src-tauri/src/share.rs#L117)): "connect" a UDP socket to any address and read the local end. Nothing is sent; the OS just picks the outgoing interface.
- **A tiny HTTP server** ([`share.rs::handle`](../src-tauri/src/share.rs#L72)): read the request line, check the token, write a status line, headers and the file. Worth reading next to the spec: <https://www.rfc-editor.org/rfc/rfc9112#name-message-format>

---

## 7. Building, testing, shipping

```sh
cd pinza/src-tauri
cargo build                     # debug build
cargo test                      # unit tests
cargo clippy                    # lints: read every one, they teach a lot
cargo check --target x86_64-pc-windows-msvc   # does the Windows code compile?
RUST_LOG=pinza=debug cargo run  # see what it watches and hangs
cd .. && npm run build          # installers
```

- **Cargo profiles** ([`Cargo.toml`](../src-tauri/Cargo.toml)): release uses `lto`, `opt-level = "s"` and `strip` for a small binary; dev builds still optimise dependencies (`[profile.dev.package."*"]`), because decoding images unoptimised is ten times slower.
  <https://doc.rust-lang.org/cargo/reference/profiles.html>
- **Clippy**: <https://doc.rust-lang.org/clippy/>
- **CI builds for three OSes** with `tauri-action`: [`.github/workflows/pinza.yml`](../.github/workflows/pinza.yml) · <https://github.com/tauri-apps/tauri-action>

---

## 8. Exercises

Ordered from small to ambitious. Each touches a different technique.

1. **Add a setting.** Make the peek time (2.5 s) configurable. Add a field to
   `Config` with a default, read it in `main.rs::hang`. *(serde defaults, shared state)*
2. **A new redaction pattern.** Add French phone numbers or your own
   company's ticket ids to `images.rs::patterns`, with a test. *(regex, tests)*
3. **Copy as JPEG.** Add a `Clip::Jpeg` variant and an action that copies a
   smaller JPEG. *(enums, match exhaustiveness, `image` encoders)*
4. **Count words in search results.** Show how many matches each result has. *(iterators)*
5. **Replace the `String` errors** in `images.rs` with a `thiserror` enum. *(error design)*
6. **Make the phone link single-use.** Delete the token after the first
   successful download. *(`Mutex<HashMap>`, the HTTP handler)*
7. **Async OCR.** Replace the reader thread with a Tokio task and a
   `tokio::sync::mpsc` channel (Tauri already runs Tokio:
   `tauri::async_runtime::spawn`). Compare the two designs. *(async Rust)*
   <https://tokio.rs/tokio/tutorial>
8. **macOS OCR in Rust.** Replace the AppleScript bridge with `objc2-vision`
   bindings. *(FFI, objc2)*

---

## A reading path

If you are new to Rust, this order works well with Pinza open next to you:

1. *The Rust Programming Language* (the Book), chapters 1–10. Free: <https://doc.rust-lang.org/book/>
   Then read `config.rs`, `folders.rs` and `line.rs`. They use nothing beyond those chapters.
2. *Rust by Example*, to see each feature in small, runnable form: <https://doc.rust-lang.org/rust-by-example/>
3. Book chapters 13 (closures, iterators) and 16 (concurrency). Then read
   `watcher.rs`, `actions.rs` and `controller.rs`.
4. *Rustlings*, small exercises that make the borrow checker familiar: <https://github.com/rust-lang/rustlings>
5. The Tauri v2 guides on commands, events and state: <https://v2.tauri.app/develop/>. Then read `commands.rs` and `main.js` together.
6. Pick one platform section above and follow its links into the system
   docs. That's where the most interesting knowledge is.
7. Going further: *Rust Atomics and Locks* (<https://marabos.nl/atomics/>),
   *Effective Rust* (<https://effective-rust.com/>), and the Rust API
   Guidelines (<https://rust-lang.github.io/api-guidelines/>).

Pinza started as a port of [Tendedero](https://github.com/alejandrobujan/tendedero),
a macOS app in Swift. Reading the two side by side (`LinePanel.swift` next
to `controller.rs`, `PeggedView.swift` next to `main.js`) is a good way to
see how the same ideas translate between languages and platforms.
