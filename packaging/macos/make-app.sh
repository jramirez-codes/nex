#!/usr/bin/env bash
# Build nex.app around a nex binary.
#
#   packaging/macos/make-app.sh BINARY OUT_DIR [VERSION]
#
# Writes OUT_DIR/nex.app (replacing an existing one). VERSION defaults to the one in
# Cargo.toml next to this script, or to `BINARY --version` in a release tarball.
# The app is ad-hoc signed (not notarized), which is enough to run it on this Mac.
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "usage: $0 BINARY OUT_DIR [VERSION]" >&2
  exit 2
fi
BIN=$1
OUT=$2
HERE="$(cd "$(dirname "$0")" && pwd)"
VERSION=${3:-}
if [ -z "$VERSION" ] && [ -f "$HERE/../../Cargo.toml" ]; then
  VERSION="$(grep -m1 '^version' "$HERE/../../Cargo.toml" | cut -d '"' -f2)"
fi
if [ -z "$VERSION" ]; then
  VERSION="$("$BIN" --version | awk '{print $2}')"
fi

APP="$OUT/nex.app"
STAGE="$(mktemp -d)/nex.app"
trap 'rm -rf "$(dirname "$STAGE")"' EXIT

mkdir -p "$STAGE/Contents/MacOS" "$STAGE/Contents/Resources"
install -m755 "$BIN" "$STAGE/Contents/MacOS/nex"
install -m644 "$HERE/nex.icns" "$STAGE/Contents/Resources/nex.icns"
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist" > "$STAGE/Contents/Info.plist"
plutil -lint -s "$STAGE/Contents/Info.plist"
codesign --force --sign - "$STAGE"

mkdir -p "$OUT"
rm -rf "$APP"
mv "$STAGE" "$APP"
echo "Built $APP (nex $VERSION)"
