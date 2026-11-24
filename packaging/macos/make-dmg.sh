#!/usr/bin/env bash
# Package a Developer ID-signed nex.app into a notarized, stapled DMG.
#
#   SIGN_IDENTITY="Developer ID Application: Name (TEAMID)" \
#     packaging/macos/make-dmg.sh path/to/nex.app OUT.dmg
#
# Build the app first with make-app.sh and the same SIGN_IDENTITY. Notarization uses a
# notarytool keychain profile, made once with:
#   xcrun notarytool store-credentials nex-notary
# (NOTARY_PROFILE overrides the name; NOTARY_KEYCHAIN names a keychain other than the login
# one that holds it, as in CI.) Set NOTARIZE=0 to skip notarizing, e.g. for a dry run.
set -euo pipefail

if [ $# -ne 2 ]; then
  echo "usage: $0 nex.app OUT.dmg" >&2
  exit 2
fi
APP=$1
DMG=$2
: "${SIGN_IDENTITY:?set SIGN_IDENTITY to your Developer ID Application certificate}"
PROFILE=(--keychain-profile "${NOTARY_PROFILE:-nex-notary}")
if [ -n "${NOTARY_KEYCHAIN:-}" ]; then
  PROFILE+=(--keychain "$NOTARY_KEYCHAIN")
fi

# The app must already carry a Developer ID signature with the hardened runtime.
codesign --verify --strict --verbose=2 "$APP"
flags="$(codesign -dv "$APP" 2>&1)"
case "$flags" in *flags=*runtime*) ;; *) false ;; esac || {
  echo "error: $APP isn't signed with the hardened runtime; rebuild it with SIGN_IDENTITY set" >&2
  exit 1
}

# The DMG holds the app and a link to /Applications to drag it onto.
STAGE="$(mktemp -d)"
trap 'rm -rf "$STAGE"' EXIT
ditto "$APP" "$STAGE/nex.app"
ln -s /Applications "$STAGE/Applications"
rm -f "$DMG"
hdiutil create -volname nex -srcfolder "$STAGE" -fs HFS+ -format UDZO -ov "$DMG"
codesign --force --timestamp --sign "$SIGN_IDENTITY" "$DMG"

if [ "${NOTARIZE:-1}" != 0 ]; then
  xcrun notarytool submit "$DMG" "${PROFILE[@]}" --wait
  xcrun stapler staple "$DMG"
  xcrun stapler validate "$DMG"
  spctl -a -t open --context context:primary-signature -vv "$DMG"
fi
echo "Built $DMG"
