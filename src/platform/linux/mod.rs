//! Linux backend: `/proc`, fontconfig, `xdg-open`, desktop entries, and Wayland drag and drop.

mod apps;
mod dnd;
mod fonts;
mod proc;
mod watch;

pub use super::unix::{
    config_dir, file_mode, format_local_time, home_dir, hostname, is_executable, owner_names, pty_options, pty_pid, shell_quote,
    symlink,
};
pub use apps::{apps_for, mime_type, open_with};
pub use dnd::Dnd;
pub use fonts::{FALLBACK_FAMILIES, find_font, nerd_mono_families};
pub use proc::proc_info;
pub use watch::{DirWatch, on_dir_change};

use super::{ShortcutStyle, WindowDefaults};

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Ctrl;

/// nex draws its own title bar and window buttons.
pub const WINDOW: WindowDefaults = WindowDefaults { decorations: false, controls: true, titlebar_inset: 0.0 };

/// Desktop launchers start nex in the home folder already, so there is nothing to detect.
/// The window keeps eframe's default icon (on macOS, the bundle's own icon wins).
pub fn app_icon() -> Option<egui::IconData> {
    None
}

/// nex draws its own window buttons here, so there is nothing to align (macOS only).
pub fn align_window_buttons(_frame: &eframe::Frame, _height: f32) {}

pub fn set_window_title(ctx: &egui::Context, _frame: &eframe::Frame, title: &str, _buttons_height: Option<f32>) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_owned()));
}

pub fn opened_from_desktop() -> bool {
    false
}

/// Linux sessions pass the locale on to every app.
pub fn shell_env() -> Vec<(String, String)> {
    Vec::new()
}

/// Open a URI with the desktop's default handler.
pub fn open_uri(uri: &std::ffi::OsStr) {
    super::unix::spawn_opener("xdg-open", uri);
}
