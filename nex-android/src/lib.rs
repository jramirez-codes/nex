//! The nex Android app. Android starts it through NativeActivity, which calls `android_main`;
//! everything else is nex itself (`nex::android_main`, see `src/platform/android/`).

#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
fn android_main(app: winit::platform::android::activity::AndroidApp) {
    nex::android_main(app);
}
