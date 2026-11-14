//! Everything that differs between operating systems, behind one small API.
//!
//! The rest of nex never checks `target_os`, reads `/proc`, runs OS commands or uses
//! `std::os::*`: it calls `crate::platform::…`. Each OS has a backend folder exposing the same
//! items, and this file picks one. Porting to a new OS means adding a folder; the compiler
//! lists anything it is missing.
//!
//! ```text
//! platform/
//!   mod.rs        this file: the shared types and the backend switch
//!   unix.rs       POSIX pieces shared by Linux and macOS (paths, symlinks, shell quoting)
//!   fontdb.rs     font lookup by scanning font folders (macOS, Windows)
//!   no_dnd.rs     drag-and-drop stub for OSes where winit already handles it
//!   linux/        mod.rs · proc.rs (/proc) · fonts.rs (fontconfig) · dnd/ (Wayland)
//!   macos/        mod.rs · proc.rs (libproc)
//!   windows/      mod.rs · proc.rs (not yet implemented)
//! ```
//!
//! Every backend's `mod.rs` provides:
//!
//! | Item | What it is |
//! |---|---|
//! | `SHORTCUTS` | which [`ShortcutStyle`] the keymap uses |
//! | `WINDOW` | the [`WindowDefaults`] for the window and its title bar |
//! | `FALLBACK_FAMILIES` | system fonts tried for glyphs the grid font lacks |
//! | `proc_info(pid)` | a shell's foreground process name and cwd ([`ProcInfo`]) |
//! | `pty_options(..)` / `pty_pid(..)` | build the PTY options, read the child's pid |
//! | `open_uri(uri)` | open a link or file with the desktop's default app |
//! | `config_dir()` / `home_dir()` / `hostname()` | where things are, who we are |
//! | `symlink(src, dst)` / `shell_quote(path)` | file explorer helpers |
//! | `find_font(..)` / `nerd_mono_families()` | system font lookup ([`FontFace`]) |
//! | `Dnd` | drag and drop with other apps, where winit doesn't do it |

use std::path::PathBuf;

#[cfg(unix)]
mod unix;

#[cfg(not(target_os = "linux"))]
mod fontdb;
#[cfg(not(target_os = "linux"))]
mod no_dnd;

#[cfg(target_os = "linux")]
mod linux;
#[cfg(target_os = "linux")]
use linux as backend;

#[cfg(target_os = "macos")]
mod macos;
#[cfg(target_os = "macos")]
use macos as backend;

#[cfg(target_os = "windows")]
mod windows;
#[cfg(target_os = "windows")]
use windows as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
compile_error!("nex has no platform backend for this OS yet: add one under src/platform/");

pub use backend::*;

/// What a terminal's shell is running right now, for pane headers and new-pane cwd.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcInfo {
    /// Name of the foreground process (`zsh`, `nvim`, `node`).
    pub name: Option<String>,
    /// Its working directory.
    pub cwd: Option<PathBuf>,
}

/// A font file found on the system (`index` picks a face inside a `.ttc` collection).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FontFace {
    pub path: PathBuf,
    pub index: u32,
}

/// A font family to look up: one by name, or the system's default of a kind.
#[derive(Clone, Copy, Debug)]
pub enum Family<'a> {
    Named(&'a str),
    Monospace,
    SansSerif,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Semibold,
    Bold,
}

/// How keyboard shortcuts and modifier keys behave.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ShortcutStyle {
    /// Linux / Windows: Ctrl+Shift chords for app actions, Ctrl+C reaches the shell, Alt
    /// prefixes ESC, and Ctrl copies instead of moving while dragging files.
    Ctrl,
    /// macOS: Cmd chords for app actions, Option types composed characters, and Option
    /// copies while dragging files (as in Finder).
    // Elsewhere only the keymap tests use it.
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    Cmd,
}

impl ShortcutStyle {
    /// The key app shortcuts are built on, as shown in tooltips.
    pub fn primary_key_name(self) -> &'static str {
        match self {
            ShortcutStyle::Ctrl => "Ctrl",
            ShortcutStyle::Cmd => "Cmd",
        }
    }

    pub fn alt_key_name(self) -> &'static str {
        match self {
            ShortcutStyle::Ctrl => "Alt",
            ShortcutStyle::Cmd => "Option",
        }
    }

    /// Alt/Option + key sends ESC + key to the shell (the "meta" convention).
    pub fn alt_sends_esc(self) -> bool {
        self == ShortcutStyle::Ctrl
    }

    /// The modifier held while dragging files that means "copy" rather than "move".
    pub fn copy_drag(self, mods: &egui::Modifiers) -> bool {
        match self {
            ShortcutStyle::Ctrl => mods.ctrl,
            ShortcutStyle::Cmd => mods.alt,
        }
    }
}

/// Window chrome defaults (each can still be changed in the config).
#[derive(Clone, Copy, Debug)]
pub struct WindowDefaults {
    /// Ask the OS for its window decorations.
    pub decorations: bool,
    /// Draw nex's own close / minimize / maximize buttons.
    pub controls: bool,
    /// With decorations on, the OS draws its buttons over nex's title bar (macOS traffic
    /// lights), and the tabs start this far from the left edge to clear them.
    pub titlebar_inset: f32,
}

impl WindowDefaults {
    /// The OS draws its window buttons on top of nex's title bar (instead of a title bar of its own).
    pub fn overlays_titlebar(&self) -> bool {
        self.titlebar_inset > 0.0
    }
}

/// `NEX_DEBUG_DND=1` logs each step of a drag between windows to stderr.
pub fn dnd_log(args: std::fmt::Arguments) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("NEX_DEBUG_DND").is_some()) {
        let t = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
        eprintln!("nex dnd {} {}.{:03}: {args}", std::process::id(), t.as_secs() % 100, t.subsec_millis());
    }
}
