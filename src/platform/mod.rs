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
//!   unix.rs       POSIX pieces shared by Linux, macOS and Android (paths, symlinks, shell quoting)
//!   fontdb.rs     font lookup by scanning font folders (macOS, Windows, Android)
//!   no_dnd.rs     drag-and-drop stub for OSes where winit already handles it
//!   no_watch.rs   folder-watching stub (explorers re-read their folder every few seconds)
//!   winit_app.rs  eframe's winit app on nex's own event loop, noting paste keys
//!   glide.rs      the space typed after a glided (swiped) word ([`AutoSpace`], Android)
//!   text_edit.rs  a text field's change as Backspaces and typing ([`edit_between`], iOS)
//!   linux/        mod.rs · proc.rs (/proc) · fonts.rs (fontconfig) · dnd/ (Wayland) · watch.rs (inotify)
//!                 · file_manager/ (D-Bus FileManager1) · apps.rs (Open With, desktop entries)
//!                 · clipboard.rs · memory.rs (glibc arenas, malloc_trim)
//!   macos/        mod.rs · proc.rs (libproc) · memory.rs (malloc pressure relief)
//!                 · apps.rs (Open With) · clipboard.rs · launch.rs (started from nex.app)
//!                 · menu.rs (app menu) · window.rs (traffic lights, title)
//!   windows/      mod.rs · proc.rs (not yet implemented)
//!   android/      mod.rs (app folders, no local shells: a thin client to ssh hosts)
//!   ios/          mod.rs (the same thin client) · screen.rs (safe area, keyboard frame)
//!                 · keyboard.rs (the view the keyboard types into) · clipboard.rs · haptics.rs
//!                 · mic.rs (permission, audio session)
//! ```
//!
//! Every backend's `mod.rs` provides:
//!
//! | Item | What it is |
//! |---|---|
//! | `SHORTCUTS` | which [`ShortcutStyle`] the keymap uses |
//! | `WINDOW` | the [`WindowDefaults`] for the window and its title bar |
//! | `SOFT_KEYBOARD` | typing goes through an on-screen keyboard, shown while a terminal has focus |
//! | `SYSTEM_BACK` | the OS has a Back button or gesture (Android), which nex sees as `Key::BrowserBack`: it closes what Escape would, else goes back a tab |
//! | `LOCAL_SHELLS` | whether panes can run shells on this machine (not on Android: every pane works on an ssh host) |
//! | `FALLBACK_FAMILIES` | system fonts tried for glyphs the grid font lacks |
//! | `proc_info(pid)` | a shell's foreground process name, arguments and cwd ([`ProcInfo`]) |
//! | `foreground_job(pid)` | just [`ProcInfo::job`], cheaply: whether the foreground process changed |
//! | `process_start(pid)` | when a process started, as Claude Code's session file records it |
//! | `content_rect_px()` | the part of the window system bars leave free (Android: the window covers the whole screen) |
//! | `typed_text()` / `sync_soft_keyboard(ctx)` / `soft_keyboard_visible()` / `haptic(kind)` | text the on-screen keyboard typed or pasted, showing or hiding it, whether it's up, and haptic feedback |
//! | `clipboard_text()` / `mirror_copies(ctx)` | the system clipboard where egui has none (Android): pasting into text fields, and copies mirrored to it |
//! | `clipboard_files()` | an image or copied files on the clipboard ([`ClipboardFiles`]), where `Dnd` can't read it |
//! | `pty_options(..)` / `pty_pid(..)` | build the PTY options, read the child's pid |
//! | `shell_env()` | variables the shell needs but may not inherit (`LANG` on macOS) |
//! | `opened_from_desktop()` | started by Finder / a launcher rather than a shell |
//! | `open_uri(uri)` | open a link or file with the desktop's default app |
//! | `notify(title, body)` | a system notification, in the background (Linux: `notify-send`; macOS: `osascript`; a no-op on Windows, Android and iOS) |
//! | `file_manager_service(window_args)` | `nex --file-manager-service`: show files for other apps (Linux: D-Bus `FileManager1`) |
//! | `apps_for(path)` / `open_with(app, path)` | the explorer's "Open With" list ([`AppChoice`]) |
//! | `mime_type(path)` | a file's MIME type, for Properties |
//! | `file_mode(meta)` / `is_executable(meta)` / `owner_names(meta)` | permission bits and owner |
//! | `format_local_time(t)` | a timestamp as local `YYYY-MM-DD HH:MM` |
//! | `config_dir()` / `home_dir()` / `hostname()` | where things are, who we are |
//! | `FileLock` / `lock_file(path)` | an exclusive advisory lock on a file, held until dropped ([`LockFailed`]) |
//! | `symlink(src, dst)` / `shell_quote(path)` | file explorer helpers |
//! | `find_font(..)` / `nerd_mono_families()` | system font lookup ([`FontFace`]) |
//! | `Dnd` | drag and drop with other apps, where winit doesn't do it |
//! | `DirWatch` / `on_dir_change(f)` | being told when a folder's contents change |
//! | `tune_allocator()` / `trim_heap()` | allocator setup at launch; return freed heap to the OS |
//! | `MIC_PERMISSION_HINT` | where to allow microphone access, after the OS refused it |
//! | `mic_allowed()` / `ask_mic()` | whether nex may record from the microphone, and asking the user (phones; desktops ask when the mic opens) |
//! | `prepare_mic()` | get the OS ready to record, just before the mic opens (iOS: the audio session; [`MicRefused`]) |
//! | `app_icon()` | the window icon, where the OS takes it from the window |
//! | `set_window_title(..)` / `align_window_buttons(..)` | the window title; centering native title-bar buttons (macOS) |
//!
//! Shared by every OS, defined in this folder rather than per backend:
//!
//! | Item | What it is |
//! |---|---|
//! | `WinitApp` / `vsync(..)` | eframe on nex's own winit event loop (`winit_app.rs`) |
//! | `paste_key_pressed(..)` / `take_paste_key()` | the paste shortcut, noted by `winit_app.rs` before egui sees it |

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

mod glide;
#[cfg_attr(not(target_os = "android"), allow(unused_imports))]
pub use glide::AutoSpace;

mod text_edit;
#[cfg_attr(not(target_os = "ios"), allow(unused_imports))]
pub use text_edit::edit_between;

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

#[cfg(target_os = "android")]
mod android;
#[cfg(target_os = "android")]
use android as backend;

#[cfg(target_os = "ios")]
mod ios;
#[cfg(target_os = "ios")]
use ios as backend;

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows", target_os = "android", target_os = "ios")))]
compile_error!("nex has no platform backend for this OS yet: add one under src/platform/");

pub use backend::*;

/// Desktop windows have no system bars or on-screen keyboard drawn over them.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn content_rect_px() -> Option<[f32; 4]> {
    None
}

/// Text the on-screen keyboard sent outside of key events.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(not(any(target_os = "android", target_os = "ios")), allow(dead_code))]
pub enum Typed {
    /// Typed (a key, a glided word, a suggestion): first `delete` characters are taken back
    /// with Backspace (the keyboard changed its mind about a word it's still composing), then
    /// `text` goes in as if typed.
    Text { delete: usize, text: String },
    /// Several characters committed at once from the keyboard's clipboard: a paste.
    Paste(String),
    /// Return, where the keyboard types it as text rather than a key event (iOS).
    #[cfg_attr(not(target_os = "ios"), allow(dead_code))]
    Enter,
}

/// A touch of haptic feedback, for controls that should feel physical.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Haptic {
    /// A tap on a button.
    Tap,
    /// One step of a continuous control (each arrow the joystick sends).
    Tick,
    /// A mode switched on (Ctrl armed).
    Confirm,
    /// A mode used up or switched off.
    Release,
}

/// Only the phones have an on-screen keyboard that types outside of key events.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn typed_text() -> Vec<Typed> {
    Vec::new()
}

/// Desktops have no haptics to play.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn haptic(_kind: Haptic) {}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn sync_soft_keyboard(_ctx: &egui::Context) {}

/// Whether the on-screen keyboard is up, as far as the system has said.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn soft_keyboard_visible() -> Option<bool> {
    None
}

/// Desktops ask for the microphone themselves when it's opened, or don't ask at all.
#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn mic_allowed() -> bool {
    true
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
pub fn ask_mic() {}

/// Why the OS couldn't be made ready to record ([`prepare_mic`]).
#[derive(Debug)]
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub enum MicRefused {
    /// The user said no (or turned it off in Settings).
    Denied,
    Failed(String),
}

/// Elsewhere opening the mic is all it takes.
#[cfg(not(target_os = "ios"))]
pub fn prepare_mic() -> Result<(), MicRefused> {
    Ok(())
}

/// Why [`lock_file`] didn't take its lock.
#[derive(Debug)]
#[cfg_attr(not(unix), allow(dead_code))] // Windows grants every lock
pub enum LockFailed {
    /// Another process holds it.
    Held,
    /// The lock file (or its folder) couldn't be created or opened.
    Open(std::io::Error),
    /// The file opened but locking it failed for another reason.
    Lock(std::io::Error),
}

/// What a terminal's shell is running right now, for pane headers and new-pane cwd.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ProcInfo {
    /// Name of the foreground process (`zsh`, `nvim`, `node`).
    pub name: Option<String>,
    /// Its working directory.
    pub cwd: Option<PathBuf>,
    /// Its command line (`ssh -p 2222 user@host`), empty where the OS doesn't say.
    pub argv: Vec<String>,
    /// The foreground job's pid when it is something other than the shell itself (`node`
    /// started from the prompt); `None` while the shell is at its prompt.
    pub job: Option<u32>,
}

/// What the clipboard holds that isn't text: copied files, or an image (as PNG).
/// Windows and Android have no backend that reads these yet, so nothing constructs them there.
#[cfg_attr(any(windows, target_os = "android", target_os = "ios"), allow(dead_code))]
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
    /// The smallest the window may be made, in points.
    pub min_size: [f32; 2],
    /// Ask for a window with an alpha channel (for `background-opacity`).
    pub transparent: bool,
    /// Title-bar tabs keep one width (two and a half to a screen) and the strip swipes sideways,
    /// rather than shrinking to fit (phones).
    pub scroll_tabs: bool,
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
