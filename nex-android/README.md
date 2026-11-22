# nex-android

The nex Android app. The phone runs only nex's window. Every pane works on an ssh host, such as
your home server, and nothing else runs on the device. This crate just wraps nex as the shared
library NativeActivity loads (`android_main`). The code for the Android build itself is in
`src/platform/android/` and `nex::android_main` (`src/lib.rs`).

## Status

The port is happening in milestones (see the plan in the PR):

1. **The app starts** *(this one)*: the APK installs and opens to the connect screen
   (server, port, user). Connect doesn't connect yet.
2. **ssh**: on the first connection you type your password once. The app then makes a device
   key, adds it to `~/.ssh/authorized_keys` on the server with an expiry date, and logs in with
   that key from then on. Shells run under `nex-host`, so they survive switching apps.
3. Touch typing: the soft keyboard and an extra-keys bar (Esc, Ctrl, Tab, arrows), plus
   drag to scroll.
4. The Agents, Changes and Files panes over the same connection.

## Building

Build on a computer with the Android SDK; you can't build it on the server the phone connects to.
You need:

- Rust 1.88 or newer, with the Android target:
  `rustup target add aarch64-linux-android`
- The Android SDK, with a platform (API 34) and build-tools installed, plus the NDK (r26 or
  newer). Android Studio's SDK Manager can install all of these. Otherwise use
  `sdkmanager "platforms;android-34" "build-tools;34.0.0" "ndk;26.3.11579264"`.
- These environment variables:
  ```sh
  export ANDROID_HOME=$HOME/Android/Sdk                # wherever the SDK is
  export ANDROID_NDK_ROOT=$ANDROID_HOME/ndk/26.3.11579264
  ```
- cargo-apk: `cargo install cargo-apk`

Then, from the repository root, with the phone plugged in and USB debugging on:

```sh
cargo apk run -p nex-android          # build, install and start a debug build
adb logcat -s nex RustStdoutStderr    # nex's messages and any panic
```

`cargo apk build -p nex-android` only builds the APK, in
`target/debug/apk/nex.apk`, and `adb install -r target/debug/apk/nex.apk` installs it.

A debug build is slow to draw. For a fast one, build with `--release`. cargo-apk then wants a
signing key. Make one once:

```sh
keytool -genkey -v -keystore ~/.android/nex-release.jks -alias nex -keyalg RSA -keysize 2048 -validity 10000
```

and point cargo-apk at it with environment variables, so it doesn't end up in `Cargo.toml`:

```sh
export CARGO_APK_RELEASE_KEYSTORE=~/.android/nex-release.jks
export CARGO_APK_RELEASE_KEYSTORE_PASSWORD=...
cargo apk run -p nex-android --release
```

## What to check on the phone (milestone 1)

- The app opens full screen to the connect screen and doesn't close by itself.
- Tapping a field brings up the soft keyboard, and typing fills it in.
- Connect with an empty field says which field is missing. With everything filled in it says
  ssh isn't built in yet.
- Rotating the phone redraws the screen without restarting the app.
- Leaving the app and coming back still shows the screen. `adb logcat -s nex` shows no panic.

If a step fails, send the `adb logcat` output from around that moment.
