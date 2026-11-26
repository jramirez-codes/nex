# nex-android

The nex Android app. The phone runs only nex's window. Every pane works on an ssh host, such as
your home server, and nothing else runs on the device. This crate just wraps nex as the shared
library NativeActivity loads (`android_main`). The code for the Android build itself is in
`src/platform/android/` and `nex::android_main` (`src/lib.rs`).

## Status

The port is happening in milestones (see the plan in the PR):

1. **The app starts** *(done)*: the APK installs and opens to the connect screen.
2. **ssh** *(this one)*: on the first connection you type your password once. The app then
   makes a device key, adds it to `~/.ssh/authorized_keys` on the server with an expiry date
   (`device-key-days`, 30 by default), and logs in with that key from then on. Every pane's
   shell is a `nex-host` session on the server, so it survives switching apps and network
   changes: the pane reconnects and the screen comes back.
3. *(on hold)* Touch typing: an extra-keys bar (Esc, Ctrl, Tab, arrows), keeping the terminal
   above the keyboard, and drag to scroll.
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
- A JDK (`javac`): `build.rs` compiles the app's bit of Java (the view the on-screen keyboard
  types into, `src/platform/android/java/`) with it and the SDK's `d8`.

The app installs `nex-host` on the server itself, so it has to carry a build of it for your
server's kind of machine. Build that first. `build.rs` picks it up, and the APK build warns if
it's missing:

```sh
rustup target add x86_64-unknown-linux-musl            # an x86_64 server; aarch64-… for ARM
cargo build -p nex-host --release --target x86_64-unknown-linux-musl
```

If the server already has the same nex-host in `~/.nex/bin` (version and protocol, see
`~/.nex/bin/nex-host --version`), nothing is installed.

Then, from the repository root, with the phone plugged in and USB debugging on:

```sh
# Newer phones use 16 KB memory pages and warn about a library linked for 4 KB ones.
# cargo-apk ignores target-specific rustflags, so this has to be plain RUSTFLAGS.
export RUSTFLAGS="-C link-arg=-Wl,-z,max-page-size=16384"
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

## What to check on the phone (milestone 2)

1. **First connection.** Fill in server, port and user, then tap Connect.
   - It asks for your password. After you type it, it shows the server's key fingerprint.
   - Compare the fingerprint with `ssh-keygen -lf /etc/ssh/ssh_host_ed25519_key.pub` on the
     server, then tap Trust and connect.
   - Pasting works in every field: long-press it and tap Paste. Text copied in another app
     comes in.
   - A shell opens.
   - On the server, `~/.ssh/authorized_keys` ends with an `expiry-time="…" ssh-ed25519 … nex-device-…`
     line. Hosts with OpenSSH older than 8.7 get the line without `expiry-time`.
2. **Typing.** Tapping the terminal brings up the keyboard, and what you type reaches the
   shell, swipe (glide) typing included. A round keyboard button sits in the bottom-right
   corner, above the keyboard when it's up, on terminals and on the Agents tab:
   - Tap it to put the keyboard away or bring it back. Put away (by the button, the back
     gesture or the keyboard's hide key), the keyboard stays down.
   - Drag it as a joystick to send arrow keys. They repeat while you hold, faster the further
     you push.
   - Double-tap it to arm Ctrl for the next key. Double-tap, then `c`, sends `^C`.
     While it's armed, the focused pane's × is a pin (like holding Shift on a desktop).
   There's no Esc key yet (milestone 3, on hold).
3. **Next launch.** Close the app from the recents screen and open it again. It connects
   without asking for anything, and the shell you left open comes back as it was.
4. **Switching away.** Leave the app for a few minutes, or turn Wi-Fi off and on, then go back.
   - The pane says it lost the connection and is reconnecting.
   - Then the screen comes back.
   - A command that was running (`top`, say) is still running.
   - The same after locking the phone (power button) and unlocking it: the pane is live
     again within a few seconds (`adb logcat -s nex` shows the reconnect).
5. **New tabs.** The app stays in portrait with the tabs along the top. The new-tab button
   there opens another shell. Closing a pane ends its
   shell: `~/.nex/bin/nex-host list` on the server no longer shows it.
6. **No panics.** `adb logcat -s nex` shows none.

## What to check on the phone: downloading a file

1. Open a Files pane on the host and long-press a file. The menu has **Download to phone**
   (not for a folder). Pick it: the action bar shows "Downloading “…”" with a progress bar,
   then "Saved “…” to Downloads".
2. The Files app (or Downloads) shows the file, byte for byte the same as on the host
   (`sha256sum` there; compare with a checksum app or `adb shell sha256sum /sdcard/Download/…`).
3. Download the same file again: it arrives as "name (2).ext", and the first one is still there.
4. Properties of a file has a **Download** button that does the same.
5. Cancel during a big download: nothing half-written stays in Downloads.
6. A file you can't read on the host (`chmod 000`) says "Permission denied on the host".
7. On Android 10 or older the save is refused, and the action bar says why.

The same thing runs on a desktop with `nex --remote=you@server`, which is quicker for checking
the server side.

If a step fails, send the `adb logcat` output from around that moment.
