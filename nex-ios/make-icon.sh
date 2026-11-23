#!/usr/bin/env bash
# Render assets/nex.svg into the iPhone app's icon (xcode/Assets.xcassets/AppIcon.appiconset/):
# one opaque 1024 px square, the tile's gradient edge to edge (iOS rounds the corners itself).
#
#   nex-ios/make-icon.sh
#
# Needs Google Chrome or Chromium (headless) to render the SVG, as nex-android/make-icon.sh does.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
SVG="$HERE/../assets/nex.svg"
OUT="$HERE/xcode/Assets.xcassets/AppIcon.appiconset/icon-1024.png"
CHROME="$(command -v google-chrome || command -v chromium || command -v chromium-browser || echo "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome")"
WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

# The tile fills the square, square-cornered and without its rim; the dog stays where it is.
sed -e 's|<rect x="8" y="8" width="112" height="112" rx="26"|<rect x="0" y="0" width="128" height="128"|' \
    -e '/<rect x="8.5"/d' \
    -e 's|<svg |<svg width="1024" height="1024" |' "$SVG" > "$WORK/icon.svg"
cat > "$WORK/icon.html" <<HTML
<html><body style="margin:0;background:#16141a"><img src="file://$WORK/icon.svg" width="1024" height="1024" style="display:block"></body></html>
HTML
"$CHROME" --headless --disable-gpu --hide-scrollbars --window-size=1024,1024 \
    --screenshot="$WORK/icon.png" "file://$WORK/icon.html" >/dev/null 2>&1
# App Store icons may not have an alpha channel.
sips -s format jpeg "$WORK/icon.png" --out "$WORK/icon.jpg" >/dev/null
sips -s format png "$WORK/icon.jpg" --out "$OUT" >/dev/null
echo "Wrote $OUT"
