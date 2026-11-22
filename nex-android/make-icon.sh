#!/usr/bin/env bash
# Render assets/nex.svg into the app's adaptive launcher icon (nex-android/res/):
# the dark tile's gradient as the background layer, the dog as the foreground layer, at
# every density. The launcher masks the layers to its own shape (a circle, a squircle).
#
#   nex-android/make-icon.sh
#
# Needs Google Chrome or Chromium (headless) to render the SVG; ImageMagick's own SVG
# renderer drops the logo's gradients and clip path.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SVG="$HERE/../assets/nex.svg"
RES="$HERE/res"
CHROME="$(command -v google-chrome || command -v chromium || command -v chromium-browser)"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# Foreground: the logo without its tile. An adaptive icon's layers are 108dp, and only the
# middle 66dp is sure to show, so the dog (about 78 x 86 of the logo's 128 units, centred
# near (64, 68)) gets a 164-unit square around it, which keeps the ears inside even the
# tighter masks (Samsung's squircle).
grep -v '<rect ' "$SVG" | sed 's|viewBox="0 0 128 128"|viewBox="-18 -14 164 164"|' > "$WORK/fg.svg"

# Background: the tile's gradient, edge to edge.
cat > "$WORK/bg.svg" <<'EOF'
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 108 108">
  <linearGradient id="bg" x1="0" y1="0" x2="1" y2="1">
    <stop offset="0" stop-color="#3a3346"/>
    <stop offset="1" stop-color="#16141a"/>
  </linearGradient>
  <rect width="108" height="108" fill="url(#bg)"/>
</svg>
EOF

render() { # render SVG PX OUT
    "$CHROME" --headless --disable-gpu --hide-scrollbars --default-background-color=00000000 \
        --window-size="$2,$2" --screenshot="$3" "file://$1" >/dev/null 2>&1
}

for density in mdpi:108 hdpi:162 xhdpi:216 xxhdpi:324 xxxhdpi:432; do
    name="${density%%:*}" px="${density##*:}"
    mkdir -p "$RES/mipmap-$name"
    render "$WORK/fg.svg" "$px" "$RES/mipmap-$name/ic_launcher_foreground.png"
    render "$WORK/bg.svg" "$px" "$RES/mipmap-$name/ic_launcher_background.png"
done

mkdir -p "$RES/mipmap-anydpi-v26"
cat > "$RES/mipmap-anydpi-v26/ic_launcher.xml" <<'EOF'
<?xml version="1.0" encoding="utf-8"?>
<adaptive-icon xmlns:android="http://schemas.android.com/apk/res/android">
    <background android:drawable="@mipmap/ic_launcher_background"/>
    <foreground android:drawable="@mipmap/ic_launcher_foreground"/>
</adaptive-icon>
EOF
echo "Wrote $RES"
