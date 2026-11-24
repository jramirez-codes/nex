#!/usr/bin/env bash
# Build nex.app around a nex binary.
#
#   packaging/macos/make-app.sh BINARY OUT_DIR [VERSION] [HOST_DIR]
#
# Writes OUT_DIR/nex.app (replacing an existing one). VERSION defaults to the one in
# Cargo.toml next to this script, or to `BINARY --version` in a release tarball (pass "" to
# keep that default and still give HOST_DIR). HOST_DIR holds the nex-host-<target> binaries
# nex installs on ssh hosts; they go in Contents/Resources/host.
# The app is ad-hoc signed, which is enough to run it on this Mac. Set SIGN_IDENTITY to a
# "Developer ID Application: …" certificate to sign it for distribution instead (hardened
# runtime, nex.entitlements); make-dmg.sh then notarizes it.
set -euo pipefail

if [ $# -lt 2 ]; then
  echo "usage: $0 BINARY OUT_DIR [VERSION]" >&2
  exit 2
fi
BIN=$1
OUT=$2
HERE="$(cd "$(dirname "$0")" && pwd)"
VERSION=${3:-}
HOST_DIR=${4:-}
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
if [ -n "$HOST_DIR" ]; then
  mkdir -p "$STAGE/Contents/Resources/host"
  for f in "$HOST_DIR"/nex-host-*; do
    [ -f "$f" ] && install -m755 "$f" "$STAGE/Contents/Resources/host/"
  done
fi
sed "s/@VERSION@/$VERSION/g" "$HERE/Info.plist" > "$STAGE/Contents/Info.plist"
plutil -lint -s "$STAGE/Contents/Info.plist"
if [ -n "${SIGN_IDENTITY:-}" ]; then
  # Notarization checks every Mac executable inside, so the macOS nex-host builds get their
  # own signature first (the Linux ones aren't Mach-O and are only sealed as resources).
  for f in "$STAGE/Contents/Resources/host"/nex-host-*; do
    if [ -f "$f" ] && file "$f" | grep -q Mach-O; then
      codesign --force --options runtime --timestamp --sign "$SIGN_IDENTITY" "$f"
    fi
  done
  codesign --force --options runtime --timestamp --entitlements "$HERE/nex.entitlements" \
    --sign "$SIGN_IDENTITY" "$STAGE"
  codesign --verify --strict --verbose=2 "$STAGE"
else
  codesign --force --sign - "$STAGE"
fi

mkdir -p "$OUT"
rm -rf "$APP"
mv "$STAGE" "$APP"
echo "Built $APP (nex $VERSION)"
