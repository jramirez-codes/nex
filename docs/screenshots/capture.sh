#!/usr/bin/env bash
# Regenerate docs/screenshots/explorer.png: tabs, split panes and a Ctrl+E file explorer with a
# multi-selection, driven through Hyprland.
# Needs Hyprland (hyprctl), grim, python3 and btop. Run from anywhere: docs/screenshots/capture.sh
set -euo pipefail

repo=$(cd "$(dirname "$0")/../.." && pwd)
out="$repo/docs/screenshots/explorer.png"
W=1440 H=880

cargo build --release --manifest-path "$repo/Cargo.toml" -q

tmp=$(mktemp -d)
pid=""
trap '[ -n "$pid" ] && kill "$pid" 2>/dev/null; rm -rf "$tmp"' EXIT

# A throwaway config (your own ~/.config/nex is left alone): an opaque window, and a bash
# with a tidy prompt that runs a different demo in each new shell, in the order they open.
mkdir -p "$tmp/nex"
cat >"$tmp/nex/config" <<EOF
font-size = 14
adjust-cell-height = 12%
background-opacity = 1.0
window-decorations = false
cursor-style-blink = false
command = bash --rcfile $tmp/rc
EOF
cat >"$tmp/rc" <<EOF
cd "$repo"
PS1='\[\e[1;35m\]nex\[\e[0m\] \[\e[2m\]on\[\e[0m\] \[\e[36m\]\$(git branch --show-current)\[\e[0m\] \[\e[1;32m\]❯\[\e[0m\] '
n=\$(ls "$tmp" | grep -c '^shell-' || true); mkdir "$tmp/shell-\$n"
clear
case \$n in
  0) # First tab, left pane.
    echo "\$ git log --oneline -8"; git -c color.ui=always log --oneline -8; echo
    echo "\$ ls src/file_explorer"; ls --color=always src/file_explorer ;;
  1) # Second tab, so the title bar shows more than one.
    exec btop ;;
  2) # Split off the explorer: starts in the folder it's showing.
    echo "\$ tree -L 1 --dirsfirst"; tree -C -L 1 --dirsfirst ;;
esac
EOF

XDG_CONFIG_HOME="$tmp" "$repo/target/release/nex" >/dev/null 2>&1 &
pid=$!

# Wait for the window, then float it at a fixed size in the middle of the monitor.
addr=""
for _ in $(seq 50); do
  addr=$(hyprctl clients -j | python3 -c "import json,sys
for c in json.load(sys.stdin):
    if c['pid']==$pid: print(c['address'])")
  [ -n "$addr" ] && break
  sleep 0.2
done
[ -n "$addr" ] || { echo "nex window never appeared" >&2; exit 1; }
w="address:$addr"
hyprctl --batch "dispatch setfloating $w; dispatch resizewindowpixel exact $W $H,$w; dispatch centerwindow; dispatch focuswindow $w" >/dev/null

key() { hyprctl dispatch sendshortcut "$1,$2,$w" >/dev/null; sleep "${3:-0.15}"; }

sleep 2.5                       # first shell and its demo
key CTRL_SHIFT T 1.5            # second tab (btop)
key ALT 1 0.5                   # back to the first tab
key CTRL E 0.8                  # file explorer split off the terminal, at its folder
key CTRL_SHIFT E 1.5            # split a terminal off the explorer (opens in its folder)
key CTRL_ALT Up 0.4             # focus back on the explorer
key "" Down; key "" Down        # src/
key "" Return 0.6               # open it
for _ in 1 2 3; do key "" Down; done
key "" Space; key "" Down; key "" Space; key "" Down; key "" Space   # mark three files
sleep 1.2                       # let the split glide settle

geom=$(hyprctl clients -j | python3 -c "import json,sys
c=[c for c in json.load(sys.stdin) if c['address']=='$addr'][0]
print('%d,%d %dx%d' % (*c['at'], *c['size']))")
grim -g "$geom" "$out"
echo "wrote $out ($geom)"
