#!/usr/bin/env bash
# Builds Pinza from source and installs it for the current user.
# Tested target: Omarchy (Arch + Hyprland). Works on other distros once the
# libraries below are installed.
set -euo pipefail
cd "$(dirname "$0")/.."

if command -v pacman >/dev/null; then
  sudo pacman -S --needed --noconfirm rust webkit2gtk-4.1 gtk3 gtk-layer-shell libayatana-appindicator
elif command -v apt-get >/dev/null; then
  sudo apt-get install -y cargo libwebkit2gtk-4.1-dev libgtk-3-dev libgtk-layer-shell-dev \
    libayatana-appindicator3-dev librsvg2-dev
elif command -v dnf >/dev/null; then
  sudo dnf install -y cargo webkit2gtk4.1-devel gtk3-devel gtk-layer-shell-devel libappindicator-gtk3-devel
fi

cargo build --release --manifest-path src-tauri/Cargo.toml --features tauri/custom-protocol

bin="$HOME/.local/bin"
apps="$HOME/.local/share/applications"
icons="$HOME/.local/share/icons/hicolor/128x128/apps"
mkdir -p "$bin" "$apps" "$icons"
install -m 755 src-tauri/target/release/pinza "$bin/pinza"
install -m 644 src-tauri/icons/128x128.png "$icons/pinza.png"
cat > "$apps/pinza.desktop" <<DESKTOP
[Desktop Entry]
Type=Application
Name=Pinza
Comment=Your screenshots, pegged to a line across the top of the screen
Exec=$bin/pinza
Icon=pinza
Categories=Utility;Graphics;
StartupNotify=false
DESKTOP

cat <<'NEXT'

Pinza is installed in ~/.local/bin/pinza.

On Hyprland (Omarchy), add to ~/.config/hypr/hyprland.conf:

    exec-once = pinza
    bind = CTRL ALT, T, exec, pinza toggle

then run `hyprctl reload` and start it once with `pinza &`.
NEXT
