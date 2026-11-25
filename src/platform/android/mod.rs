//! Android backend: nex as a thin client. Nothing runs on the phone except the window; every
//! pane works on an ssh host (`LOCAL_SHELLS` is false). Android also sets up none of a unix
//! process's usual surroundings, so [`android_setup`] gives nex a `HOME` and a `TMPDIR` in the
//! app's private storage, and sends stdout and stderr to logcat (see `log.rs`).

mod clipboard;
mod keyboard;
mod log;

use std::path::Path;
use std::sync::OnceLock;

pub use clipboard::{clipboard_text, mirror_copies};
pub use keyboard::{ask_mic, haptic, mic_allowed, soft_keyboard_visible, sync_soft_keyboard, typed_text};
pub use super::fontdb::{find_font, nerd_mono_families};
pub use super::no_dnd::Dnd;
pub use super::no_watch::{DirWatch, on_dir_change};
pub use super::unix::{
    FileLock, config_dir, file_mode, format_local_time, home_dir, hostname, is_executable, lock_file, owner_names, pty_options,
    pty_pid, shell_quote, symlink,
};

use super::{ProcInfo, ShortcutStyle, WindowDefaults};

/// No shells on the phone: panes are ssh sessions to a host.
pub const LOCAL_SHELLS: bool = false;

/// The on-screen keyboard: nex asks for it while a terminal has focus.
pub const SOFT_KEYBOARD: bool = true;

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Ctrl;

/// Full screen: no OS decorations, no window buttons, any size the screen is, and tabs
/// two and a half to a screen that swipe sideways.
pub const WINDOW: WindowDefaults = WindowDefaults {
    decorations: false,
    controls: false,
    titlebar_inset: 0.0,
    min_size: [0.0, 0.0],
    transparent: false,
    scroll_tabs: true,
};

/// Android's own fonts cover symbols, emoji and CJK.
pub const FALLBACK_FAMILIES: &[&str] = &["Droid Sans Mono", "Noto Sans Symbols", "Noto Sans Symbols 2", "Noto Color Emoji", "Noto Sans CJK JP"];

pub(super) const SANS_SERIF: &str = "Roboto";
pub(super) const MONOSPACE: &str = "Droid Sans Mono";
/// fontdb has no list of Android's font folders.
pub(super) const FONT_DIRS: &[&str] = &["/system/fonts", "/product/fonts"];

/// The activity, for the screen area the system bars leave free ([`content_rect_px`]).
static APP: OnceLock<winit::platform::android::activity::AndroidApp> = OnceLock::new();

/// Run once by `android_main`, before anything reads a path or spawns a thread of its own.
pub fn android_setup(app: &winit::platform::android::activity::AndroidApp) {
    log::to_logcat();
    let _ = APP.set(app.clone());
    keyboard::install();
    let Some(data) = app.internal_data_path() else {
        eprintln!("nex: Android gave the app no data folder; nothing will be saved");
        return;
    };
    let tmp = data.join("tmp");
    let _ = std::fs::create_dir_all(&tmp);
    // SAFETY: called first thing in `android_main`, while no other thread of nex reads the
    // environment (android-activity's own threads never touch these variables).
    unsafe {
        std::env::set_var("HOME", &data);
        std::env::set_var("TMPDIR", &tmp);
        std::env::set_var("XDG_CONFIG_HOME", data.join("config"));
        std::env::set_var("XDG_STATE_HOME", data.join("state"));
    }
}

/// The part of the window, in pixels, that the status bar, navigation bar and keyboard leave
/// free: `[left, top, right, bottom]`. The window itself covers the whole screen.
pub fn content_rect_px() -> Option<[f32; 4]> {
    let r = APP.get()?.content_rect();
    (r.right > r.left && r.bottom > r.top).then_some([r.left as f32, r.top as f32, r.right as f32, r.bottom as f32])
}

/// The window keeps eframe's default icon; the launcher icon comes from the APK.
pub fn app_icon() -> Option<egui::IconData> {
    None
}

/// No window buttons to align (macOS only).
pub fn align_window_buttons(_frame: &eframe::Frame, _height: f32) {}

/// Android shows the app's name, not a window title.
pub fn set_window_title(_ctx: &egui::Context, _frame: &eframe::Frame, _title: &str, _buttons_height: Option<f32>) {}

/// Started from the launcher, always.
pub fn opened_from_desktop() -> bool {
    true
}

/// No local shells, so nothing to pass them.
pub fn shell_env() -> Vec<(String, String)> {
    Vec::new()
}

/// There are no local processes to look at.
pub fn proc_info(_pid: u32) -> ProcInfo {
    ProcInfo::default()
}

pub fn foreground_job(_pid: u32) -> Option<u32> {
    None
}

pub fn process_start(_pid: u32) -> Option<String> {
    None
}

/// Files and images on the clipboard aren't read here (text goes through egui).
pub fn clipboard_files() -> Option<super::ClipboardFiles> {
    None
}

/// Android's allocator needs no tuning.
pub fn tune_allocator() {}
pub fn trim_heap() {}

/// Showing files for other apps over D-Bus is Linux-only (see `platform/linux/file_manager/`).
pub fn file_manager_service(_window_args: &[String]) -> Result<(), String> {
    Err("--file-manager-service is only available on Linux".into())
}

/// System notifications aren't sent on Android yet (they need NotificationManager through
/// JNI, a channel, and the POST_NOTIFICATIONS permission).
pub fn notify(_title: &str, _body: &str) {}

/// Opening links in the browser needs an Intent (through JNI), which isn't wired up yet.
pub fn open_uri(uri: &std::ffi::OsStr) {
    eprintln!("nex: opening {} isn't supported on Android yet", uri.to_string_lossy());
}

/// Files live on the ssh host, so there are no local apps to open them with.
pub fn apps_for(_path: &Path) -> Vec<super::AppChoice> {
    Vec::new()
}

pub fn open_with(_app: &super::AppChoice, _path: &Path) {}

pub fn mime_type(_path: &Path) -> Option<String> {
    None
}

/// Where to let nex use the microphone, after the OS refused it.
pub const MIC_PERMISSION_HINT: &str = "Allow it in Settings → Apps → nex → Permissions → Microphone, then share again.";
