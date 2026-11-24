#!/usr/bin/env bash
# Build a signed nex.ipa to install on registered iPhones (ad hoc), or to upload to TestFlight.
#
#   nex-ios/build-ipa.sh                # ad hoc: target/ios/nex.ipa
#   nex-ios/build-ipa.sh app-store      # for App Store Connect / TestFlight
#   nex-ios/build-ipa.sh upload         # app-store, and send it to App Store Connect
#
# The build number is `git rev-list --count HEAD`, so each commit uploads as a new build;
# set BUILD_NUMBER to override it (App Store Connect refuses a number it has already seen).
#
# Needs your Apple ID signed in to Xcode (Settings → Accounts) and your team ID in
# xcode/Local.xcconfig (see README.md). Xcode makes the certificate and provisioning profile
# itself; an ad hoc profile covers the devices registered on your team at the time, so
# register a new phone first and run this again.
set -euo pipefail

HERE="$(cd "$(dirname "$0")" && pwd)"
ROOT="$HERE/.."
METHOD="${1:-ad-hoc}"
DESTINATION=export
if [ "$METHOD" = upload ]; then
    METHOD=app-store
    DESTINATION=upload
fi
BUILD_NUMBER="${BUILD_NUMBER:-$(git -C "$HERE" rev-list --count HEAD)}"
OUT="$ROOT/target/ios"
export DEVELOPER_DIR="${DEVELOPER_DIR:-/Applications/Xcode.app/Contents/Developer}"

if ! grep -qs '^DEVELOPMENT_TEAM' "$HERE/xcode/Local.xcconfig"; then
    echo "error: put your team ID in nex-ios/xcode/Local.xcconfig, as: DEVELOPMENT_TEAM = ABCDE12345" >&2
    exit 1
fi
TEAM="$(sed -n 's/^DEVELOPMENT_TEAM *= *//p' "$HERE/xcode/Local.xcconfig" | tr -d '[:space:]')"

# nex-host for each kind of ssh host, bundled into the app (see build.rs). rust-lld links
# the static musl builds without a Linux toolchain.
for triple in x86_64-unknown-linux-musl aarch64-unknown-linux-musl; do
    rustup +1.88.0 target add "$triple" >/dev/null
    var="CARGO_TARGET_$(echo "$triple" | tr 'a-z-' 'A-Z_')_LINKER"
    env "$var=rust-lld" cargo +1.88.0 build --manifest-path "$ROOT/Cargo.toml" -p nex-host --release --target "$triple"
done
rustup +1.88.0 target add aarch64-apple-ios >/dev/null

mkdir -p "$OUT"
rm -rf "$OUT/nex.xcarchive" "$OUT/export"
xcodebuild -project "$HERE/xcode/nex.xcodeproj" -scheme nex -configuration Release \
    -destination 'generic/platform=iOS' -derivedDataPath "$HERE/xcode/build" \
    -archivePath "$OUT/nex.xcarchive" -allowProvisioningUpdates archive \
    CURRENT_PROJECT_VERSION="$BUILD_NUMBER"

cat > "$OUT/ExportOptions.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>destination</key>
	<string>$DESTINATION</string>
	<key>method</key>
	<string>$METHOD</string>
	<key>teamID</key>
	<string>$TEAM</string>
	<key>signingStyle</key>
	<string>automatic</string>
	<key>compileBitcode</key>
	<false/>
	<key>thinning</key>
	<string>&lt;none&gt;</string>
</dict>
</plist>
PLIST
xcodebuild -exportArchive -archivePath "$OUT/nex.xcarchive" -exportPath "$OUT/export" \
    -exportOptionsPlist "$OUT/ExportOptions.plist" -allowProvisioningUpdates
echo
if [ "$DESTINATION" = upload ]; then
    echo "Uploaded build $BUILD_NUMBER to App Store Connect; it shows up in TestFlight once processed"
else
    cp "$OUT/export/nex.ipa" "$OUT/nex.ipa"
    echo "Built $OUT/nex.ipa ($METHOD, build $BUILD_NUMBER)"
fi
