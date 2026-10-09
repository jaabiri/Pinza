<p align="center"><img src="assets/icon.png" width="96" alt=""></p>

<h1 align="center">Pinza</h1>

<p align="center">
  Your screenshots, pegged to a line across the top of the screen.<br>
  One Rust + Tauri app for <b>Omarchy / Hyprland</b>, <b>macOS</b> and <b>Windows 11</b>.
</p>

<p align="center"><img src="docs/line.png" alt="Eight screenshots in glass frames hang from a thin line along the top of the screen, each held by a small aluminium clip."></p>

Take a screenshot the way you always do. It hangs on a thin line along the
top of the screen for a moment, then the line tucks away. Rest the pointer at
the top edge and it slides back down. Move away and it's gone.

Pinza is a cross-platform port of [Tendedero](https://github.com/alejandrobujan/tendedero)
by Alejandro Buján, a macOS app written in Swift. The idea, the gestures and
the look are his. Tendedero's licence covers its code but not its name or
icon, so this port has its own.

## Gestures

| | |
|:--|:--|
| Click | Copy the image |
| Double click | Open it in your image viewer |
| Press and hold | Mark it up: Satty on Omarchy, Preview on macOS, Paint on Windows |
| Drag into an app or folder | Hand over a copy. It stays on the line |
| Click the cross | Take it down. The file stays where it is |
| Right click | Copy, Open, Mark Up, Show in Folder, Take Down, Move to Trash |
| Rest the pointer at the top edge | Bring the line down |
| <kbd>Ctrl</kbd> <kbd>Alt</kbd> <kbd>T</kbd> | Show or hide the line (see Hyprland below) |

A full line holds 8 photos; the oldest falls off the far end. Delete or move
a file and its photo falls off by itself. Nothing is uploaded: there is no
network code at all.

## Where screenshots come from

Pinza never takes screenshots. It watches the folder your system saves them to.

| System | Shortcut you already use | Folder watched |
|:--|:--|:--|
| Omarchy (Hyprland) | <kbd>Print</kbd> (hyprshot) | `$OMARCHY_SCREENSHOT_DIR`, else `~/Pictures` and `~/Pictures/Screenshots` |
| Other Linux | Your screenshot tool | `~/Pictures/Screenshots`, `~/Pictures` |
| macOS | <kbd>⌘</kbd><kbd>⇧</kbd><kbd>3</kbd> / <kbd>4</kbd> / <kbd>5</kbd> | The folder set in <kbd>⌘</kbd><kbd>⇧</kbd><kbd>5</kbd> → Options (usually the Desktop, where only real screenshots count) |
| Windows 11 | <kbd>Win</kbd><kbd>Shift</kbd><kbd>S</kbd>, <kbd>Win</kbd><kbd>PrtScn</kbd> | `Pictures\Screenshots` (also under OneDrive) |

Only files that appear after Pinza starts are hung, so an existing Pictures
folder is left alone. To watch other folders, list them in the settings.

## Install

Pre-built installers come from the **Pinza** GitHub Actions workflow: open the
latest run and download `pinza-Linux`, `pinza-macOS` or `pinza-Windows`.
Pushing a tag like `pinza-v0.1.0` also attaches them to a draft release.
The builds are not signed yet. See the notes for each system below.

### Omarchy / Hyprland

```sh
git clone <this repo> && cd <repo>/pinza
scripts/install-linux.sh
```

The script installs the libraries with pacman (`webkit2gtk-4.1`, `gtk-layer-shell`,
`libayatana-appindicator`), builds a release binary into `~/.local/bin/pinza`
and adds a desktop entry. Then add to `~/.config/hypr/hyprland.conf`:

```ini
exec-once = pinza
bind = CTRL ALT, T, exec, pinza toggle
```

On Wayland, apps can't grab global shortcuts, so Hyprland handles the key and
runs `pinza toggle`, which passes it to the running Pinza. `pinza show`,
`pinza hide`, `pinza clear` and `pinza quit` work too. See
[`docs/hyprland.conf`](docs/hyprland.conf) for optional blur rules.

On Wayland, Pinza is a *layer-shell* surface: it sits under Waybar, never
takes keyboard focus, and takes the pointer only over the photos. No window
rules are needed. The tray icon shows up in Waybar's tray.

### macOS

Open the `.dmg` and drag Pinza to Applications. Until the app is signed,
right-click → Open the first time, or run
`xattr -dr com.apple.quarantine /Applications/Pinza.app`.
Pinza lives in the menu bar, with no Dock icon. Rest the pointer in the menu
bar to bring the line down.

### Windows 11

Run the `-setup.exe` or `.msi`. SmartScreen warns about unsigned apps:
choose *More info → Run anyway*. WebView2 is already part of Windows 11.
Snipping Tool saves screenshots automatically by default. If yours doesn't,
turn on *Automatically save screenshots* in Snipping Tool settings.

## Settings

The tray menu has *Sounds*, *Show at Top Edge*, *Clear the Line* and
*Edit Settings…*, which opens `config.json`:

```json
{
  "folders": [],
  "max_items": 8,
  "shortcut": "Ctrl+Alt+T",
  "sound": true,
  "reveal_at_edge": true
}
```

It lives in `~/.config/app.pinza.Pinza/` (Linux),
`~/Library/Application Support/app.pinza.Pinza/` (macOS) or
`%APPDATA%\app.pinza.Pinza\` (Windows). Restart Pinza after editing folders
or the shortcut.

## Build from source

You need Rust (stable) and Node 18+ (only for the Tauri CLI; the web side has
no build step). Linux also needs the libraries listed in
`scripts/install-linux.sh`.

```sh
cd pinza
npm install
npm run dev        # run it
npm run build      # installers in src-tauri/target/release/bundle/
```

Plain `cargo run --manifest-path src-tauri/Cargo.toml` works too.
`RUST_LOG=pinza=debug` shows what it is watching and hanging.

## How it works

| File | Role |
|:--|:--|
| `src-tauri/src/main.rs` | Startup, the window, the command line |
| `src-tauri/src/controller.rs` | Bringing the line down and tucking it away |
| `src-tauri/src/watcher.rs` | Notices new screenshots |
| `src-tauri/src/folders.rs` | Where each system saves screenshots |
| `src-tauri/src/line.rs` | What is hanging, thumbnails, persistence |
| `src-tauri/src/actions.rs` | Copy, open, mark up, trash |
| `src-tauri/src/commands.rs` | What the web view can ask for; drag-out |
| `src-tauri/src/linux.rs` | Layer shell and input regions on Wayland |
| `src-tauri/src/tray.rs` | The tray / menu bar icon |
| `src/main.js` | The rope, the clips, the swing, your gestures |
| `src/styles.css` | Glass frames and aluminium clips |

The window is a transparent strip, always on top, that never takes focus.
To let clicks through everywhere except the photos:

- **macOS and Windows:** Rust polls the cursor 25 times a second. It turns
  click-through off only while the pointer is over a photo, and brings the line
  down when the pointer rests at the top of a screen.
- **Linux:** Wayland never tells an app where the pointer is, so the window's
  input region shrinks to the photos, and GTK reports the pointer coming and
  going. While the line is tucked away, a 4 px strip along the top catches
  the pointer.

## Status

- Tested on Linux: Wayland with layer shell (wlroots) and X11. Hanging,
  peeking, edge reveal, click to copy, cross, trash, long press to Satty,
  drag-out, the 8-photo limit, `pinza toggle` and click-through.
- macOS and Windows are compiled in CI but not yet tried on real machines.
- Compared with Tendedero, not yet ported: the flight from the capture spot
  to the line, hiding when an app is in full screen, the macOS "inbox" mode
  that takes over screenshot settings, and translations.
