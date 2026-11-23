#!/usr/bin/env bash
# Xcode's "Build Rust" phase: build nex-ios for the platform Xcode is building for and leave
# libnex_ios.a where the link step looks (OTHER_LDFLAGS in project.pbxproj).
set -euo pipefail


case "$PLATFORM_NAME" in
    iphonesimulator) TARGET=aarch64-apple-ios-sim ;;
    iphoneos) TARGET=aarch64-apple-ios ;;
    *) echo "error: nex-ios builds for iphoneos or iphonesimulator, not $PLATFORM_NAME" >&2; exit 1 ;;
esac
# Release builds (Archive) are optimized; Debug ones build fast.
if [ "$CONFIGURATION" = "Release" ]; then PROFILE=release; FLAG=--release; else PROFILE=debug; FLAG=; fi

ROOT="$SRCROOT/../.."
# Xcode exports hundreds of variables (SDKROOT, LD, the iOS deployment target ...) meant for
# its own target; cargo's build scripts run on the Mac and break on them. So cargo starts
# from a clean environment, keeping only which Xcode to use and the iOS version to build for.
env -i HOME="$HOME" USER="$USER" PATH="$HOME/.cargo/bin:/usr/bin:/bin:/usr/sbin:/sbin" \
    DEVELOPER_DIR="$DEVELOPER_DIR" IPHONEOS_DEPLOYMENT_TARGET="$IPHONEOS_DEPLOYMENT_TARGET" \
    cargo +1.88.0 build --manifest-path "$ROOT/Cargo.toml" -p nex-ios --target "$TARGET" $FLAG
mkdir -p "$BUILT_PRODUCTS_DIR"
cp "$ROOT/target/$TARGET/$PROFILE/libnex_ios.a" "$BUILT_PRODUCTS_DIR/libnex_ios.a"
