# nex-ios

The nex iPhone app. It works the same as the Android app (see `nex-android/README.md`): the
phone runs only nex's window, and every pane works on an ssh host such as your home server.
This crate builds nex as a static library (`libnex_ios.a`). A small Xcode project in `xcode/`
links that library into the app and signs it. The iOS code itself lives in
`src/platform/ios/` and `nex::ios_main` (`src/lib.rs`).

Differences from Android:

- **Drawing uses wgpu (Metal).** eframe's OpenGL backend doesn't run on iOS.
- **No Java-style input view.** winit's own view receives what the on-screen keyboard types
  (`UIKeyInput`), and egui shows and hides the keyboard.
- **The window covers the whole screen.** `platform/ios/screen.rs` tells winit's window that
  its safe area is empty, then reports the free part of the screen (clear of the notch, the
  home indicator and the keyboard) through `content_rect_px`, as Android does.

## What you need

- A Mac with Xcode (15 or newer) and the iOS platform installed.
- Rust 1.88 with the iOS targets:
  `rustup +1.88.0 target add aarch64-apple-ios aarch64-apple-ios-sim`
- For a real iPhone: an Apple Developer Program membership.

The app installs `nex-host` on the server itself, so it carries static Linux builds of it.
`build-ipa.sh` builds them for you, linking with `rust-lld`, so no Linux toolchain is needed.

## Try it in the Simulator

Open `xcode/nex.xcodeproj` in Xcode, pick an iPhone simulator and press Run. Or from a
terminal:

```sh
export DEVELOPER_DIR=/Applications/Xcode.app/Contents/Developer
xcodebuild -project nex-ios/xcode/nex.xcodeproj -scheme nex -sdk iphonesimulator \
    -derivedDataPath nex-ios/xcode/build CODE_SIGNING_ALLOWED=NO build
xcrun simctl boot "iPhone 15"; open -a Simulator
xcrun simctl install booted nex-ios/xcode/build/Build/Products/Debug-iphonesimulator/nex.app
xcrun simctl launch --console-pty booted io.github.jramirez-codes.nex   # nex's messages
```

## Build an .ipa for testers (ad hoc)

Do these once:

1. **Sign in.** In Xcode, open **Settings → Accounts**, click **+**, choose **Apple ID**, and
   sign in with the account that has the developer membership.
2. **Add your team ID.** Find the 10-character team ID under **Membership** at
   developer.apple.com/account, then write it into `nex-ios/xcode/Local.xcconfig` (this file
   is not committed):
   ```
   DEVELOPMENT_TEAM = ABCDE12345
   ```
3. **Register every tester's iPhone.** At developer.apple.com, go to
   **Certificates, IDs & Profiles → Devices → +** and enter the phone's UDID. A phone plugged
   into your Mac with Xcode open can also be added from **Window → Devices and Simulators**.

Then build:

```sh
nex-ios/build-ipa.sh        # writes target/ios/nex.ipa
```

Xcode creates the signing certificate and an ad hoc provisioning profile for you. The profile
lists the phones that were registered when it was made, so after registering a new phone, run
the script again and send the new `.ipa`.

**Installing the .ipa:**

- **On a phone plugged into your Mac:** open Finder, select the phone, and drag `nex.ipa`
  onto its window. Or use Xcode → Window → Devices and Simulators → **+** under
  Installed Apps.
- **On a friend's phone, remotely:** upload `nex.ipa` to an ad hoc distribution site such as
  diawi.com. Your friend opens the link it gives you in Safari on their (registered) iPhone
  and taps Install.

On iOS 16 and newer the phone may first ask to turn on **Developer Mode**, under
Settings → Privacy & Security.

## TestFlight

`nex-ios/build-ipa.sh app-store` builds an `.ipa` for App Store Connect. Testers then install
from a TestFlight link, and no UDIDs are needed. App Store Connect only accepts builds made
with a recent Xcode, so this may need a newer Xcode (and macOS) than ad hoc does.

## Not done yet

- **Links** don't open in Safari (`open_uri`).
- **Sharing the mic** with the host (hold the keyboard button to talk) builds, but hasn't been
  tried on a phone.
- **iOS stops the app's ssh connections** soon after you switch away. As on Android, panes
  reconnect when you come back.
