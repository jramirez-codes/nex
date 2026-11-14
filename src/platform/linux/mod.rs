//! Linux backend: `/proc`, fontconfig, `xdg-open`, and Wayland drag and drop.

mod dnd;
mod fonts;
mod proc;

pub use super::unix::{config_dir, home_dir, hostname, pty_options, pty_pid, shell_quote, symlink};
pub use dnd::Dnd;
pub use fonts::{FALLBACK_FAMILIES, find_font, nerd_mono_families};
pub use proc::proc_info;

use super::{ShortcutStyle, WindowDefaults};

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Ctrl;

/// nex draws its own title bar and window buttons.
pub const WINDOW: WindowDefaults = WindowDefaults { decorations: false, controls: true, titlebar_inset: 0.0 };

/// Open a URI with the desktop's default handler.
pub fn open_uri(uri: &std::ffi::OsStr) {
    super::unix::spawn_opener("xdg-open", uri);
}
