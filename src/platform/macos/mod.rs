//! macOS backend: libproc, `open`, a font index, native traffic lights, and starting from
//! nex.app.

mod apps;
mod clipboard;
mod launch;
mod memory;
mod menu;
mod proc;
mod window;

pub use super::fontdb::{find_font, nerd_mono_families};
pub use super::no_dnd::Dnd;
pub use super::no_watch::{DirWatch, on_dir_change};
pub use super::unix::{
    FileLock, config_dir, file_mode, format_local_time, home_dir, hostname, is_executable, lock_file, owner_names, pty_options,
    pty_pid, shell_quote, symlink,
};
pub use apps::{apps_for, mime_type, open_with};
pub use clipboard::clipboard_files;
pub use launch::{opened_from_desktop, shell_env};
pub use memory::{trim_heap, tune_allocator};
pub use proc::{proc_info, process_start};
pub use window::{align_window_buttons, set_window_title};

use super::{ShortcutStyle, WindowDefaults};

/// Panes run shells on this machine.
pub const LOCAL_SHELLS: bool = true;

/// A physical keyboard.
pub const SOFT_KEYBOARD: bool = false;

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Cmd;

/// The system's own traffic lights, drawn over nex's title bar (see `main.rs`).
pub const WINDOW: WindowDefaults = WindowDefaults { decorations: true, controls: false, titlebar_inset: 72.0, min_size: [420.0, 260.0], transparent: true, sidebar_tabs: false };

/// An empty icon: eframe then leaves the Dock icon alone, so nex.app keeps its own `nex.icns`
/// (with no icon set, eframe swaps in the egui logo once the app is running).
pub fn app_icon() -> Option<egui::IconData> {
    Some(egui::IconData::default())
}

/// Menlo draws symbols to the cell width; the rest cover symbols, math and CJK.
pub const FALLBACK_FAMILIES: &[&str] = &["Menlo", "Apple Symbols", "STIX Two Math", "PingFang SC", "Hiragino Sans"];

/// Families the font index treats as `Family::SansSerif` / `Family::Monospace`.
pub(super) const SANS_SERIF: &str = "Helvetica Neue";
/// fontdb already scans the system's font folders.
pub(super) const FONT_DIRS: &[&str] = &[];
pub(super) const MONOSPACE: &str = "Menlo";

/// Showing files for other apps over D-Bus is Linux-only (see `platform/linux/file_manager/`).
pub fn file_manager_service(_window_args: &[String]) -> Result<(), String> {
    Err("--file-manager-service is only available on Linux".into())
}

/// Open a URI with its default app.
pub fn open_uri(uri: &std::ffi::OsStr) {
    super::unix::spawn_opener("open", uri);
}

/// Where to let nex use the microphone, after the OS refused it.
pub const MIC_PERMISSION_HINT: &str = "Allow it in System Settings → Privacy & Security → Microphone, then share again.";
