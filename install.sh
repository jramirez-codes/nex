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
else
  BIN=./nex
fi

PREFIX="${PREFIX:-$HOME/.local}"
CONFIG_DIR="${XDG_CONFIG_HOME:-$HOME/.config}/nex"

# `install -D` is GNU-only, so folders are made first (works with BSD install on macOS too).
install -d "$PREFIX/bin"
install -m755 "$BIN" "$PREFIX/bin/nex"
if [ "$(uname -s)" = Linux ]; then
  # Absolute Exec path: desktop launchers often don't inherit the shell's PATH (e.g. ~/.local/bin).
  install -d "$PREFIX/share/applications" "$PREFIX/share/icons/hicolor/scalable/apps"
  sed "s|^Exec=nex|Exec=$PREFIX/bin/nex|" assets/nex.desktop > "$PREFIX/share/applications/nex.desktop"
  chmod 644 "$PREFIX/share/applications/nex.desktop"
  install -m644 assets/nex.svg "$PREFIX/share/icons/hicolor/scalable/apps/nex.svg"
  command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true
elif [ "$(uname -s)" = Darwin ]; then
  if [ -z "${APP_DIR:-}" ]; then
    if [ -w /Applications ]; then APP_DIR=/Applications; else APP_DIR="$HOME/Applications"; fi
  fi
  packaging/macos/make-app.sh "$BIN" "$APP_DIR"
fi
if [ ! -e "$CONFIG_DIR/config" ]; then
  install -d "$CONFIG_DIR"
  install -m644 config.example "$CONFIG_DIR/config"
fi

echo "Installed nex to $PREFIX/bin/nex"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: $PREFIX/bin is not on your PATH";; esac
