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
//!   no_watch.rs   folder-watching stub (explorers re-read their folder every few seconds)
//!   winit_app.rs  eframe's winit app on nex's own event loop, noting paste keys
//!   linux/        mod.rs · proc.rs (/proc) · fonts.rs (fontconfig) · dnd/ (Wayland) · watch.rs (inotify)
//!                 · file_manager/ (D-Bus FileManager1)
//!                 · memory.rs (glibc arenas, malloc_trim)
//!   macos/        mod.rs · proc.rs (libproc) · memory.rs (malloc pressure relief)
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
//! | `proc_info(pid)` | a shell's foreground process name, arguments and cwd ([`ProcInfo`]) |
//! | `clipboard_files()` | an image or copied files on the clipboard ([`ClipboardFiles`]), where `Dnd` can't read it |
//! | `pty_options(..)` / `pty_pid(..)` | build the PTY options, read the child's pid |
//! | `shell_env()` | variables the shell needs but may not inherit (`LANG` on macOS) |
//! | `opened_from_desktop()` | started by Finder / a launcher rather than a shell |
//! | `open_uri(uri)` | open a link or file with the desktop's default app |
//! | `file_manager_service()` | `nex --file-manager-service`: show files for other apps (Linux: D-Bus `FileManager1`) |
//! | `apps_for(path)` / `open_with(app, path)` | the explorer's "Open With" list ([`AppChoice`]) |
//! | `mime_type(path)` | a file's MIME type, for Properties |
//! | `file_mode(meta)` / `is_executable(meta)` / `owner_names(meta)` | permission bits and owner |
//! | `format_local_time(t)` | a timestamp as local `YYYY-MM-DD HH:MM` |
//! | `config_dir()` / `home_dir()` / `hostname()` | where things are, who we are |
//! | `symlink(src, dst)` / `shell_quote(path)` | file explorer helpers |
//! | `find_font(..)` / `nerd_mono_families()` | system font lookup ([`FontFace`]) |
//! | `Dnd` | drag and drop with other apps, where winit doesn't do it |
//! | `DirWatch` / `on_dir_change(f)` | being told when a folder's contents change |
//! | `tune_allocator()` / `trim_heap()` | allocator setup at launch; return freed heap to the OS |
//! | `MIC_PERMISSION_HINT` | where to allow microphone access, after the OS refused it |

use std::path::PathBuf;

#[cfg(unix)]
mod unix;

#[cfg(not(target_os = "linux"))]
mod fontdb;
#[cfg(not(target_os = "linux"))]
mod no_dnd;
#[cfg(not(target_os = "linux"))]
mod no_watch;

mod winit_app;
pub use winit_app::{WinitApp, vsync};

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
    /// Its command line (`ssh -p 2222 user@host`), empty where the OS doesn't say.
    pub argv: Vec<String>,
}

/// What the clipboard holds that isn't text: copied files, or an image (as PNG).
/// Windows has no backend that reads these yet, so nothing constructs them there.
#[cfg_attr(windows, allow(dead_code))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClipboardFiles {
    Paths(Vec<PathBuf>),
    Png(Vec<u8>),
}

/// The last paste key (Ctrl+V / Cmd+V, with or without Shift) seen by the window, for the
/// app to notice a paste that egui made no `Paste` event for (the clipboard held no text).
/// 0: none, 1: without Shift, 2: with Shift.
static PASTE_KEY: std::sync::atomic::AtomicU8 = std::sync::atomic::AtomicU8::new(0);

/// A paste key went down (see `winit_app`).
pub fn paste_key_pressed(shift: bool) {
    PASTE_KEY.store(if shift { 2 } else { 1 }, std::sync::atomic::Ordering::Relaxed);
}

/// Whether a paste key went down since the last call, and if so whether Shift was held.
pub fn take_paste_key() -> Option<bool> {
    match PASTE_KEY.swap(0, std::sync::atomic::Ordering::Relaxed) {
        0 => None,
        n => Some(n == 2),
    }
}

/// An app offered in the explorer's "Open With" menu.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AppChoice {
    pub name: String,
    /// What starts it: a desktop entry's `Exec=` line (Linux) or the app bundle (macOS).
    pub command: String,
    /// The desktop's default app for the file's type.
    pub default: bool,
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

/// A picture of what is being dragged, for the OS to show under the pointer once a drag
/// leaves the window (where nex can no longer draw it). Only the Linux backend hands it to
/// the OS so far; elsewhere it is built and dropped.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub struct DragImage {
    /// Premultiplied RGBA, row by row.
    pub pixels: Vec<[u8; 4]>,
    /// Width and height in pixels.
    pub size: [usize; 2],
    /// Pixels per logical pixel of the window (a Wayland buffer scale).
    pub scale: i32,
    /// Where the pointer is on the image, in logical pixels from its top-left corner.
    pub hotspot: [i32; 2],
}
