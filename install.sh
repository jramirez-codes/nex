#!/usr/bin/env bash
# Install nex for the current user (~/.local by default; override with PREFIX=...).
# Also adds a desktop entry and icon on Linux, or nex.app on macOS (in /Applications, or
# ~/Applications if that isn't writable; override with APP_DIR=...).
# Works from a release tarball (uses the prebuilt ./nex) or a source checkout (builds it).
set -euo pipefail
cd "$(dirname "$0")"

if [ -f Cargo.toml ]; then
  cargo build --release --locked
  BIN=target/release/nex
  HOST_DIR=target/host   # if you've built nex-host there (see nex-host/README.md)
else
  BIN=./nex
  HOST_DIR=host
fi

PREFIX="${PREFIX:-$HOME/.local}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/nex"

# `install -D` is GNU-only, so folders are made first (works with BSD install on macOS too).
install -d "$PREFIX/bin"
install -m755 "$BIN" "$PREFIX/bin/nex"
# nex-host for each kind of ssh host, which nex installs on hosts its panes connect to. The
# macOS app carries its own copy too (make-app.sh); this one serves $PREFIX/bin/nex.
if ls "$HOST_DIR"/nex-host-* >/dev/null 2>&1; then
  install -d "$PREFIX/share/nex/host"
  for f in "$HOST_DIR"/nex-host-*; do install -m755 "$f" "$PREFIX/share/nex/host/"; done
fi
if [ "$(uname -s)" = Linux ]; then
  # Absolute Exec path: desktop launchers often don't inherit the shell's PATH (e.g. ~/.local/bin).
  install -d "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/scalable/apps"
  for entry in nex nex-files; do
    sed "s|^Exec=nex|Exec=$PREFIX/bin/nex|" "assets/$entry.desktop" > "$PREFIX/share/applications/$entry.desktop"
    chmod 644 "$PREFIX/share/applications/$entry.desktop"
  done
  install -m644 assets/nex.svg "$PREFIX/share/icons/hicolor/scalable/apps/nex.svg"
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
  # Opt in to answering other apps' "Show in Folder" (D-Bus FileManager1) with nex.
  if [ "${NEX_FILE_MANAGER:-0}" = 1 ]; then
    SERVICES="${XDG_DATA_HOME:-$HOME/.local/share}/dbus-1/services"
    install -d "$SERVICES"
    sed "s|^Exec=nex|Exec=$PREFIX/bin/nex|" assets/org.freedesktop.FileManager1.service > "$SERVICES/org.freedesktop.FileManager1.service"
    chmod 644 "$SERVICES/org.freedesktop.FileManager1.service"
  fi
elif [ "$(uname -s)" = Darwin ]; then
  if [ -z "${APP_DIR:-}" ]; then
    if [ -w /Applications ]; then APP_DIR=/Applications; else APP_DIR="$HOME/Applications"; fi
  fi
  packaging/macos/make-app.sh "$BIN" "$APP_DIR" "" "$HOST_DIR"
fi
if [ ! -e "$CONFIG_DIR/config" ]; then
  install -d "$CONFIG_DIR"
  install -m644 config.example "$CONFIG_DIR/config"
fi

echo "Installed nex to $PREFIX/bin/nex"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: $PREFIX/bin is not on your PATH";; esac
