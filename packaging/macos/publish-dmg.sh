#!/usr/bin/env bash
# Add a notarized DMG to a GitHub release whose macOS build ran without signing secrets.
#
#   SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#     packaging/macos/publish-dmg.sh v0.16.0
#
# Downloads the release's universal-macos tarball, builds a Developer ID-signed nex.app from
# its binary and nex-host builds, notarizes it into nex-TAG-macos.dmg (make-dmg.sh), and
# uploads the DMG plus a re-zipped, signed and stapled nex-TAG-macos-app.zip, updating
# SHA256SUMS.txt to match. SIGN_KEYCHAIN names a keychain to unlock first, with its password
# in SIGN_KEYCHAIN_PASS_FILE. UPLOAD=0 builds without touching the release.
set -euo pipefail

if [ $# -ne 1 ]; then
  echo "usage: $0 TAG" >&2
  exit 2
fi
TAG=$1
: "${SIGN_IDENTITY:?set SIGN_IDENTITY to your Developer ID Application certificate}"
HERE="$(cd "$(dirname "$0")" && pwd)"

if [ -n "${SIGN_KEYCHAIN:-}" ]; then
  security unlock-keychain -p "$(cat "${SIGN_KEYCHAIN_PASS_FILE:?set SIGN_KEYCHAIN_PASS_FILE}")" "$SIGN_KEYCHAIN"
fi

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
name="nex-${TAG}-universal-macos"
gh release download "$TAG" -p "$name.tar.gz" -p SHA256SUMS.txt -D "$WORK"
(cd "$WORK" && shasum -a 256 -c SHA256SUMS.txt --ignore-missing)
tar -C "$WORK" -xzf "$WORK/$name.tar.gz"

"$HERE/make-app.sh" "$WORK/$name/nex" "$WORK/build" "${TAG#v}" "$WORK/$name/host"
dist="$WORK/dist"
mkdir -p "$dist"
"$HERE/make-dmg.sh" "$WORK/build/nex.app" "$dist/nex-${TAG}-macos.dmg"
xcrun stapler staple "$WORK/build/nex.app"
ditto -c -k --keepParent "$WORK/build/nex.app" "$dist/nex-${TAG}-macos-app.zip"

# SHA256SUMS.txt keeps every other file's line and gets fresh ones for these two.
(cd "$dist" && shasum -a 256 ./*) | sed 's#  \./#  #' > "$WORK/new-sums"
grep -v -e "nex-${TAG}-macos.dmg\$" -e "nex-${TAG}-macos-app.zip\$" "$WORK/SHA256SUMS.txt" \
  | cat - "$WORK/new-sums" | sort -k2 > "$dist/SHA256SUMS.txt"
cat "$dist/SHA256SUMS.txt"

if [ "${UPLOAD:-1}" != 0 ]; then
  gh release upload "$TAG" "$dist"/* --clobber
  echo "Uploaded the DMG, app zip and SHA256SUMS.txt to $TAG"
else
  mkdir -p dist && cp "$dist"/* dist/
  echo "Built into dist/ (not uploaded)"
fi
