//! macOS backend: libproc, `open`, a font index, native traffic lights, and starting from
//! nex.app.

mod launch;
mod proc;
mod window;

pub use super::fontdb::{find_font, nerd_mono_families};
pub use super::no_dnd::Dnd;
pub use super::unix::{config_dir, home_dir, hostname, pty_options, pty_pid, shell_quote, symlink};
pub use launch::{opened_from_desktop, shell_env};
pub use proc::proc_info;
pub use window::align_window_buttons;

use super::{ShortcutStyle, WindowDefaults};

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Cmd;

/// The system's own traffic lights, drawn over nex's title bar (see `main.rs`).
pub const WINDOW: WindowDefaults = WindowDefaults { decorations: true, controls: false, titlebar_inset: 72.0 };

/// Menlo draws symbols to the cell width; the rest cover symbols, math and CJK.
pub const FALLBACK_FAMILIES: &[&str] = &["Menlo", "Apple Symbols", "STIX Two Math", "PingFang SC", "Hiragino Sans"];

/// Families the font index treats as `Family::SansSerif` / `Family::Monospace`.
pub(super) const SANS_SERIF: &str = "Helvetica Neue";
pub(super) const MONOSPACE: &str = "Menlo";

/// Open a URI with its default app.
pub fn open_uri(uri: &std::ffi::OsStr) {
    super::unix::spawn_opener("open", uri);
}
