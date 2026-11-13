#!/usr/bin/env bash
# Build nex and install it for the current user (~/.local).
set -euo pipefail
cd "$(dirname "$0")"

cargo build --release
install -Dm755 target/release/nex "$HOME/.local/bin/nex"
install -Dm644 assets/nex.desktop "$HOME/.local/share/applications/nex.desktop"
install -Dm644 assets/nex.svg "$HOME/.local/share/icons/hicolor/scalable/apps/nex.svg"
if [ ! -e "${XDG_CONFIG_HOME:-$HOME/.config}/nex/config" ]; then
  install -Dm644 config.example "${XDG_CONFIG_HOME:-$HOME/.config}/nex/config"
fi
command -v update-desktop-database >/dev/null && update-desktop-database -q "$HOME/.local/share/applications" || true

echo "Installed nex to ~/.local/bin/nex"
case ":$PATH:" in *":$HOME/.local/bin:"*) ;; *) echo "Note: ~/.local/bin is not on your PATH";; esac
