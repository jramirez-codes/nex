//! The nex iPhone app. The Xcode project's `main` (xcode/main.c) calls [`nex_ios_main`] on
//! the main thread; everything else is nex itself (`nex::ios_main`, see `src/platform/ios/`).

/// Run the app. UIKit takes over the main thread, so this never returns.
#[cfg(target_os = "ios")]
#[unsafe(no_mangle)]
pub extern "C" fn nex_ios_main() {
    nex::ios_main();
}
