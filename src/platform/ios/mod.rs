//! iOS backend: nex as a thin client, as on Android. Nothing runs on the phone except the
//! window; every pane works on an ssh host (`LOCAL_SHELLS` is false). iOS lets an app write
//! only inside its own container, so [`ios_setup`] points `HOME` and the XDG folders there.
//!
//! The on-screen keyboard types into a hidden text view of nex's own (`keyboard.rs`), which
//! iOS treats as a real text field (holding delete repeats); its typing reaches nex through
//! [`typed_text`], as on Android. `screen.rs` makes the window cover the whole screen and
//! reports the part the notch, home indicator and keyboard leave free, as on Android.

mod clipboard;
mod haptics;
mod keyboard;
mod mic;
mod screen;

use std::path::Path;

pub use super::fontdb::{find_font, nerd_mono_families};
pub use super::no_dnd::Dnd;
pub use super::no_watch::{DirWatch, on_dir_change};
pub use super::unix::{
    FileLock, config_dir, file_mode, format_local_time, home_dir, hostname, is_executable, lock_file, owner_names, pty_options, pty_pid, shell_quote,
    symlink,
};
pub use clipboard::{clipboard_text, mirror_copies};
pub use haptics::haptic;
pub use keyboard::{sync_soft_keyboard, typed_text};
pub use mic::{ask_mic, mic_allowed, prepare_mic};
pub use screen::{content_rect_px, soft_keyboard_visible};

use super::{ProcInfo, ShortcutStyle, WindowDefaults};

/// No shells on the phone: panes are ssh sessions to a host.
pub const LOCAL_SHELLS: bool = false;

/// The on-screen keyboard: nex asks for it while a terminal has focus.
pub const SOFT_KEYBOARD: bool = true;

/// No system Back key: a keyboard's Browser Back key does nothing.
pub const SYSTEM_BACK: bool = false;

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Ctrl;

/// Full screen, as on Android: no decorations or window buttons, and title-bar tabs that swipe.
pub const WINDOW: WindowDefaults =
    WindowDefaults { decorations: false, controls: false, titlebar_inset: 0.0, min_size: [0.0, 0.0], transparent: false, scroll_tabs: true };

/// iOS's own fonts cover symbols, emoji and CJK.
pub const FALLBACK_FAMILIES: &[&str] = &["Menlo", "Apple Symbols", "Apple Color Emoji", "PingFang SC", "Hiragino Sans"];

pub(super) const SANS_SERIF: &str = "Helvetica Neue";
pub(super) const MONOSPACE: &str = "Menlo";
/// fontdb has no list of iOS's font folders.
pub(super) const FONT_DIRS: &[&str] = &["/System/Library/Fonts"];

/// Run once by `ios_main`, before anything reads a path or spawns a thread of its own.
pub fn ios_setup() {
    // iOS sets `HOME` to the app's container, whose top level can't be written to;
    // `Library/Application Support` can, and isn't shown to the user in Files.
    let Some(container) = home_dir() else {
        eprintln!("nex: iOS gave the app no HOME; nothing will be saved");
        return;
    };
    screen::install();
    let data = container.join("Library/Application Support/nex");
    let _ = std::fs::create_dir_all(&data);
    // SAFETY: called first thing in `ios_main`, before nex starts any thread of its own.
    unsafe {
        std::env::set_var("HOME", &data);
        std::env::set_var("XDG_CONFIG_HOME", data.join("config"));
        std::env::set_var("XDG_STATE_HOME", data.join("state"));
    }
}

/// The window keeps eframe's default icon; the home-screen icon comes from the app bundle.
pub fn app_icon() -> Option<egui::IconData> {
    None
}

/// No window buttons to align (macOS only).
pub fn align_window_buttons(_frame: &eframe::Frame, _height: f32) {}

/// iOS shows the app's name, not a window title.
pub fn set_window_title(_ctx: &egui::Context, _frame: &eframe::Frame, _title: &str, _buttons_height: Option<f32>) {}

/// Started from the home screen, always.
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

/// Files and images on the clipboard aren't read here (text goes through [`clipboard_text`]).
pub fn clipboard_files() -> Option<super::ClipboardFiles> {
    None
}

/// iOS's allocator needs no tuning.
pub fn tune_allocator() {}
pub fn trim_heap() {}

/// Showing files for other apps over D-Bus is Linux-only (see `platform/linux/file_manager/`).
pub fn file_manager_service(_window_args: &[String]) -> Result<(), String> {
    Err("--file-manager-service is only available on Linux".into())
}

/// System notifications aren't sent on iOS yet (they need UNUserNotificationCenter and the
/// user's permission).
pub fn notify(_title: &str, _body: &str) {}

/// Opening links in Safari needs `UIApplication.openURL`, which isn't wired up yet.
pub fn open_uri(uri: &std::ffi::OsStr) {
    eprintln!("nex: opening {} isn't supported on iOS yet", uri.to_string_lossy());
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
pub const MIC_PERMISSION_HINT: &str = "Allow it in Settings → Privacy & Security → Microphone → nex, then share again.";
