//! Windows backend: a first pass that compiles. It is not a tested, supported build yet.
//!
//! What's left for a real port: foreground-process info (`proc.rs`), quoting for cmd /
//! PowerShell instead of POSIX `shell_quote`, and testing with ConPTY.

mod proc;

use std::path::{Path, PathBuf};

pub use super::fontdb::{find_font, nerd_mono_families};
pub use super::no_dnd::Dnd;
pub use super::no_watch::{DirWatch, on_dir_change};
pub use proc::proc_info;

use super::{ShortcutStyle, WindowDefaults};

pub const SHORTCUTS: ShortcutStyle = ShortcutStyle::Ctrl;

pub const WINDOW: WindowDefaults = WindowDefaults { decorations: false, controls: true, titlebar_inset: 0.0 };

pub const FALLBACK_FAMILIES: &[&str] = &["Consolas", "Segoe UI Symbol", "Cambria Math", "Microsoft YaHei"];

pub(super) const SANS_SERIF: &str = "Segoe UI";
pub(super) const MONOSPACE: &str = "Consolas";

/// The window keeps eframe's default icon (on macOS, the bundle's own icon wins).
pub fn app_icon() -> Option<egui::IconData> {
    None
}

/// nex draws its own window buttons here, so there is nothing to align (macOS only).
pub fn align_window_buttons(_frame: &eframe::Frame, _height: f32) {}

pub fn set_window_title(ctx: &egui::Context, _frame: &eframe::Frame, title: &str, _buttons_height: Option<f32>) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Title(title.to_owned()));
}

/// Reading files and images off the clipboard isn't done here yet.
pub fn clipboard_files() -> Option<super::ClipboardFiles> {
    None
}

/// The Windows heap returns freed memory on its own.
pub fn tune_allocator() {}
pub fn trim_heap() {}

pub fn opened_from_desktop() -> bool {
    false
}

pub fn shell_env() -> Vec<(String, String)> {
    Vec::new()
}

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

/// No "Open With" list on Windows yet; the explorer hides the menu then.
pub fn apps_for(_path: &Path) -> Vec<super::AppChoice> {
    Vec::new()
}

pub fn open_with(_app: &super::AppChoice, _path: &Path) {}

pub fn mime_type(_path: &Path) -> Option<String> {
    None
}

/// Windows has no POSIX permission bits.
pub fn file_mode(_meta: &std::fs::Metadata) -> Option<u32> {
    None
}

/// Windows marks programs by extension (`.exe`), which the explorer's icons already cover.
pub fn is_executable(_meta: &std::fs::Metadata) -> bool {
    false
}

pub fn owner_names(_meta: &std::fs::Metadata) -> Option<(String, String)> {
    None
}

/// `2026-11-15 14:03 UTC` (no time zone lookup here yet).
pub fn format_local_time(t: std::time::SystemTime) -> Option<String> {
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as i64;
    let (days, rem) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    // Days since 1970-01-01 to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    Some(format!("{year:04}-{month:02}-{day:02} {:02}:{:02} UTC", rem / 3600, rem % 3600 / 60))
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

/// Where to let nex use the microphone, after the OS refused it.
pub const MIC_PERMISSION_HINT: &str = "Allow it in Settings → Privacy & security → Microphone, then share again.";
