//! Windows backend: a first pass that compiles. It is not a tested, supported build yet.
//!
//! What's left for a real port: foreground-process info (`proc.rs`), quoting for cmd /
//! PowerShell instead of POSIX `shell_quote`, and testing with ConPTY.

mod proc;

use std::path::{Path, PathBuf};

pub use super::fontdb::{find_font, nerd_mono_families};
pub use super::no_dnd::Dnd;
pub use proc::proc_info;

use super::{ShortcutStyle, WindowDefaults};

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Ctrl;

pub const WINDOW: WindowDefaults = WindowDefaults { decorations: false, controls: true, titlebar_inset: 0.0 };

pub const FALLBACK_FAMILIES: &[&str] = &["Consolas", "Segoe UI Symbol", "Cambria Math", "Microsoft YaHei"];

pub(super) const SANS_SERIF: &str = "Segoe UI";
pub(super) const MONOSPACE: &str = "Consolas";

/// `%APPDATA%\nex`.
pub fn config_dir() -> Option<PathBuf> {
    std::env::var_os("APPDATA").filter(|v| !v.is_empty()).map(|d| PathBuf::from(d).join("nex"))
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("USERPROFILE").filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn hostname() -> String {
    std::env::var("COMPUTERNAME").ok().filter(|h| !h.is_empty()).unwrap_or_else(|| "localhost".into())
}

pub fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    if target.is_dir() {
        std::os::windows::fs::symlink_dir(target, link)
    } else {
        std::os::windows::fs::symlink_file(target, link)
    }
}

/// Double quotes when needed (cmd and PowerShell both accept them for plain paths).
pub fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    if !s.is_empty() && !s.contains([' ', '&', '(', ')', '\'', ';', ',']) { s.into_owned() } else { format!("\"{s}\"") }
}

/// Open a URI with its default app.
pub fn open_uri(uri: &std::ffi::OsStr) {
    let _ = std::process::Command::new("cmd")
        .args(["/C", "start", ""])
        .arg(uri)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

pub fn pty_options(
    shell: Option<alacritty_terminal::tty::Shell>,
    working_directory: Option<PathBuf>,
    env: std::collections::HashMap<String, String>,
) -> alacritty_terminal::tty::Options {
    alacritty_terminal::tty::Options { shell, working_directory, drain_on_exit: true, env, escape_args: true }
}

pub fn pty_pid(pty: &alacritty_terminal::tty::Pty) -> u32 {
    pty.child_watcher().pid().map_or(0, |p| p.get())
}
