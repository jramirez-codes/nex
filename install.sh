#!/usr/bin/env bash
# Install nex for the current user (~/.local by default; override with PREFIX=...).
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

install -Dm755 "$BIN" "$PREFIX/bin/nex"
# Absolute Exec path: desktop launchers often don't inherit the shell's PATH (e.g. ~/.local/bin).
install -d "$PREFIX/share/applications"
sed "s|^Exec=nex|Exec=$PREFIX/bin/nex|" assets/nex.desktop > "$PREFIX/share/applications/nex.desktop"
chmod 644 "$PREFIX/share/applications/nex.desktop"
install -Dm644 assets/nex.svg "$PREFIX/share/icons/hicolor/scalable/apps/nex.svg"
if [ ! -e "$CONFIG_DIR/config" ]; then
  install -Dm644 config.example "$CONFIG_DIR/config"
fi
command -v update-desktop-database >/dev/null && update-desktop-database -q "$PREFIX/share/applications" || true

echo "Installed nex to $PREFIX/bin/nex"
case ":$PATH:" in *":$PREFIX/bin:"*) ;; *) echo "Note: $PREFIX/bin is not on your PATH";; esac
