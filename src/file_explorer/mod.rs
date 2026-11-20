//! The file-explorer pane shown in the Files tab: a directory listing (as a list or a grid of
//! tiles) you can navigate with the mouse, keyboard or a touch screen. Folders open in place;
//! files open with the desktop's default app, or any app from "Open With".
//!
//! The path bar has Back / Forward / Up buttons and clickable breadcrumbs; clicking beside
//! them (or Ctrl+L) types a location instead, with Tab completion. Typing letters in the
//! listing jumps to the first matching name.
//!
//! Files can be copied, cut and pasted (Ctrl+C / Ctrl+X / Ctrl+V, the right-click menu, or the
//! action bar that appears while items are selected), and dragged onto folders, onto another
//! explorer pane, or onto a terminal (which inserts their paths). Hold Ctrl while dropping to
//! copy instead of move. Copies, moves and deletes run in the background with a progress bar
//! and a Cancel button in the action bar. Items can be renamed in place (F2), new folders and
//! files created, and a Properties dialog shows a file's details. On touch screens a tap opens,
//! a long press starts selecting, dragging scrolls, and a long press followed by a drag (or,
//! while selecting, dragging a selected item) moves the selection. Delete (the key, the
//! right-click menu or the action bar) asks for confirmation, then permanently deletes the
//! selection.

pub mod fs;
mod input;
mod items;
mod properties;
#[cfg(test)]
mod ui_test;
mod view;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::{Duration, Instant};

use egui::{Pos2, Rect};

use crate::terminal::tilde;

use fs::ops::read_entries;
use input::edit::{Find, OpenWith, Rename};
use input::Press;
use items::Layout;
use input::line_edit::LineEdit;
use properties::Properties;
use fs::transfer::{FS_GENERATION, Job};

pub use input::{ExplorerDrag, release_pos};
pub use items::{drag_image, paint_drag_ghost};
pub use fs::transfer::{Op, files_changed};

/// How often an idle listing is re-read so files created elsewhere show up (where the
/// folder can't be watched; see `platform::DirWatch`).
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// How many folders Back remembers.
const HISTORY: usize = 100;

/// Something an explorer asks the app to do (the app owns the other panes).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Split a terminal off this pane, starting in the folder.
    OpenTerminal(PathBuf),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    List,
    Grid,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// File size in bytes (`None` for directories).
    pub size: Option<u64>,
    pub is_symlink: bool,
    /// A file with an execute bit set.
    pub executable: bool,
}

pub struct FileExplorer {
    pub dir: PathBuf,
    pub entries: Vec<Entry>,
    /// Keyboard cursor (and the single selection when nothing is marked).
    pub selected: usize,
    /// Multi-selection, as paths in the current folder.
    pub marked: BTreeSet<PathBuf>,
    /// Touch-style selection mode: taps toggle items instead of opening them.
    pub selecting: bool,
    pub view: View,
    pub show_hidden: bool,
    pub error: Option<String>,
    /// The last press came from a touch screen: use larger hit targets.
    pub touch: bool,
    scroll: f32,
    /// Zoom level of this pane (1.0 = normal); see `zoom.rs`.
    zoom: f32,
    /// What to keep in place while zooming, applied on the next paint; see `zoom.rs`.
    zoom_anchor: Option<input::zoom::Anchor>,
    /// Scroll the selection into view on the next paint (after keyboard moves).
    reveal: bool,
    /// Columns and items per page, from the last paint (for keyboard movement).
    cols: usize,
    page_items: usize,
    layout: Option<Layout>,
    last_refresh: Instant,
    seen_generation: u64,
    /// Watching `dir` (the folder it was made for), so changes show up right away.
    watch: Option<(PathBuf, crate::platform::DirWatch)>,
    press: Option<Press>,
    /// Last known pointer position: a touch release also reports the pointer as gone, so the
    /// release frame needs this to know where a drop landed.
    last_pos: Option<Pos2>,
    last_click: Option<(usize, f64)>,
    jobs: Vec<Job>,
    status: Option<(String, bool, Instant)>,
    /// Items waiting for the user to confirm they should be permanently deleted.
    confirm_delete: Option<Vec<PathBuf>>,
    /// Folders visited before and after this one (Back / Forward), most recent last.
    back: Vec<PathBuf>,
    forward: Vec<PathBuf>,
    /// The location bar's text field, while typing a path.
    location: Option<LineEdit>,
    /// An item being renamed in place.
    rename: Option<Rename>,
    /// Where the rename field was drawn last frame (presses there stay in the field).
    rename_rect: Option<Rect>,
    /// Type-to-find: what's been typed so far.
    find: Option<Find>,
    /// The Properties dialog, and a request to open one (on the next paint).
    properties: Option<Properties>,
    pending_properties: Option<PathBuf>,
    /// Apps for the right-clicked file's "Open With" menu, loading in the background.
    open_with: Option<OpenWith>,
    /// The last right-click landed on an item (not empty space).
    menu_on_item: bool,
    /// The progress bar's drawn length, easing toward the jobs' real progress.
    shown_progress: f32,
    /// Requests for the app, taken after each paint.
    pub requests: Vec<Request>,
}

impl FileExplorer {
    pub fn new(dir: PathBuf) -> Self {
        let mut b = Self {
            dir,
            entries: Vec::new(),
            selected: 0,
            marked: BTreeSet::new(),
            selecting: false,
            view: View::default(),
            show_hidden: false,
            error: None,
            touch: false,
            scroll: 0.0,
            zoom: 1.0,
            zoom_anchor: None,
            reveal: false,
            cols: 1,
            page_items: 10,
            layout: None,
            last_refresh: Instant::now(),
            seen_generation: FS_GENERATION.load(Ordering::Relaxed),
            watch: None,
            press: None,
            last_pos: None,
            last_click: None,
            jobs: Vec::new(),
            status: None,
            confirm_delete: None,
            back: Vec::new(),
            forward: Vec::new(),
            location: None,
            rename: None,
            rename_rect: None,
            find: None,
            properties: None,
            pending_properties: None,
            open_with: None,
            menu_on_item: false,
            shown_progress: 0.0,
            requests: Vec::new(),
        };
        b.refresh();
        b
    }

    pub fn title(&self) -> String {
        tilde(&self.dir)
    }

    /// Re-read the directory, keeping the same entry selected if it still exists.
    pub fn refresh(&mut self) {
        let keep = self.entries.get(self.selected).map(|e| e.name.clone());
        self.last_refresh = Instant::now();
        if self.watch.as_ref().is_none_or(|(d, _)| *d != self.dir) {
            self.watch = crate::platform::DirWatch::new(&self.dir).map(|w| (self.dir.clone(), w));
        }
        match read_entries(&self.dir, self.show_hidden) {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e.to_string());
            }
        }
        match keep.and_then(|name| self.entries.iter().position(|e| e.name == name)) {
            Some(i) => self.selected = i,
            None => self.selected = self.selected.min(self.entries.len().saturating_sub(1)),
        }
        let entries = &self.entries;
        self.marked.retain(|p| entries.iter().any(|e| &e.path == p));
    }

    /// Refresh if the listing hasn't been read for a while, or a file operation finished.
    pub fn refresh_if_stale(&mut self) {
        let generation = FS_GENERATION.load(Ordering::Relaxed);
        if generation != self.seen_generation || self.last_refresh.elapsed() >= REFRESH_INTERVAL {
            self.seen_generation = generation;
            self.refresh();
        }
    }

    /// Open `dir`, remembering this folder for Back.
    pub fn navigate(&mut self, dir: PathBuf) {
        if dir != self.dir {
            self.back.push(self.dir.clone());
            if self.back.len() > HISTORY {
                self.back.remove(0);
            }
            self.forward.clear();
        }
        self.go_to(dir);
    }

    /// Show `dir` without touching the history.
    fn go_to(&mut self, dir: PathBuf) {
        let from = std::mem::replace(&mut self.dir, dir);
        self.entries.clear();
        self.marked.clear();
        self.selecting = false;
        self.selected = 0;
        self.scroll = 0.0;
        self.location = None;
        self.rename = None;
        self.find = None;
        self.refresh();
        self.reveal = true;
        self.select_towards(&from);
    }

    /// When `from` is inside the folder now shown, select the item that leads back to it
    /// (going up from `a/b/c` to `a` selects `b`).
    fn select_towards(&mut self, from: &Path) {
        let Some(name) = from.strip_prefix(&self.dir).ok().and_then(|rest| rest.components().next()) else { return };
        let name = name.as_os_str().to_string_lossy();
        if let Some(i) = self.entries.iter().position(|e| e.name == name) {
            self.selected = i;
        }
    }

    /// Go to the parent folder, selecting the folder we came from.
    pub fn up(&mut self) {
        if let Some(parent) = self.dir.parent().map(Path::to_path_buf) {
            self.navigate(parent);
        }
    }

    pub fn can_go_back(&self) -> bool {
        !self.back.is_empty()
    }

    pub fn can_go_forward(&self) -> bool {
        !self.forward.is_empty()
    }

    /// The folder before this one.
    pub fn go_back(&mut self) {
        if let Some(dir) = self.back.pop() {
            self.forward.push(self.dir.clone());
            self.go_to(dir);
        }
    }

    /// The folder Back left.
    pub fn go_forward(&mut self) {
        if let Some(dir) = self.forward.pop() {
            self.back.push(self.dir.clone());
            self.go_to(dir);
        }
    }

    /// A text field (location bar or rename) has the keyboard: keys and typed text go there
    /// before any shortcut.
    pub fn editing(&self) -> bool {
        self.location.is_some() || self.rename.is_some()
    }

    /// A dialog (delete confirmation, Properties) is open and reads the keyboard itself.
    pub fn modal_open(&self) -> bool {
        self.confirm_delete.is_some() || self.properties.is_some() || self.pending_properties.is_some()
    }

    /// Typed text: into the location bar or rename field, else type-to-find. Returns whether
    /// it was used.
    pub fn text(&mut self, text: &str) -> bool {
        if let Some(field) = self.location.as_mut() {
            field.insert(text);
        } else if let Some(r) = self.rename.as_mut() {
            r.field.insert(text);
        } else {
            let text: String = text.chars().filter(|c| !c.is_control()).collect();
            // Space marks items, unless it's part of a name being typed.
            if text.is_empty() || (text.trim().is_empty() && self.find.is_none()) {
                return false;
            }
            self.find_type(&text);
        }
        true
    }

    /// Ctrl+C / Ctrl+X while a text field has the keyboard: its selected text.
    pub fn edit_clipboard(&mut self, cut: bool) -> Option<String> {
        let field = match (self.location.as_mut(), self.rename.as_mut()) {
            (Some(f), _) => f,
            (None, Some(r)) => &mut r.field,
            (None, None) => return None,
        };
        let s = if cut { field.cut() } else { field.selected_text() };
        (!s.is_empty()).then_some(s)
    }

    /// Open a folder in place, or a file with the desktop's default app.
    pub fn activate(&mut self, index: usize) {
        let Some(entry) = self.entries.get(index) else { return };
        if entry.is_dir {
            let path = entry.path.clone();
            self.navigate(path);
        } else {
            crate::platform::open_uri(entry.path.as_os_str());
        }
    }

    pub fn move_selection(&mut self, delta: isize) {
        if self.entries.is_empty() {
            return;
        }
        let last = self.entries.len() as isize - 1;
        self.selected = (self.selected as isize + delta).clamp(0, last) as usize;
        self.reveal = true;
    }

    pub fn toggle_hidden(&mut self) {
        self.show_hidden = !self.show_hidden;
        self.refresh();
        self.reveal = true;
    }

    pub fn toggle_view(&mut self) {
        self.view = match self.view {
            View::List => View::Grid,
            View::Grid => View::List,
        };
        self.reveal = true;
    }

    pub fn select_all(&mut self) {
        self.marked = self.entries.iter().map(|e| e.path.clone()).collect();
    }

    pub fn clear_selection(&mut self) {
        self.marked.clear();
        self.selecting = false;
    }

    fn toggle_mark(&mut self, i: usize) {
        let Some(path) = self.entries.get(i).map(|e| e.path.clone()) else { return };
        if !self.marked.remove(&path) {
            self.marked.insert(path);
        }
        self.selected = i;
    }

    fn is_marked(&self, i: usize) -> bool {
        self.entries.get(i).is_some_and(|e| self.marked.contains(&e.path))
    }

    /// What an operation applies to: the marked items (in listing order), or the cursor's.
    pub fn targets(&self) -> Vec<PathBuf> {
        if self.marked.is_empty() {
            self.entries.get(self.selected).map(|e| vec![e.path.clone()]).unwrap_or_default()
        } else {
            self.entries.iter().filter(|e| self.marked.contains(&e.path)).map(|e| e.path.clone()).collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub(super) fn fixture(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("nex-files-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("beta")).unwrap();
        std::fs::create_dir_all(dir.join("Alpha")).unwrap();
        std::fs::create_dir_all(dir.join(".hidden_dir")).unwrap();
        std::fs::write(dir.join("zeta.txt"), "z").unwrap();
        std::fs::write(dir.join("Apple.md"), "apple").unwrap();
        std::fs::write(dir.join(".env"), "x").unwrap();
        dir
    }

    fn names(b: &FileExplorer) -> Vec<&str> {
        b.entries.iter().map(|e| e.name.as_str()).collect()
    }

    #[test]
    fn folders_first_then_case_insensitive() {
        let dir = fixture("sort");
        let b = FileExplorer::new(dir.clone());
        assert_eq!(names(&b), ["Alpha", "beta", "Apple.md", "zeta.txt"]);
        assert_eq!(b.entries[2].size, Some(5));
        assert_eq!(b.entries[0].size, None);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn hidden_files_toggle() {
        let dir = fixture("hidden");
        let mut b = FileExplorer::new(dir.clone());
        assert!(!names(&b).iter().any(|n| n.starts_with('.')));
        b.toggle_hidden();
        assert_eq!(names(&b), [".hidden_dir", "Alpha", "beta", ".env", "Apple.md", "zeta.txt"]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn up_selects_the_folder_we_came_from() {
        let dir = fixture("up");
        let mut b = FileExplorer::new(dir.join("beta"));
        b.up();
        assert_eq!(b.dir, dir);
        assert_eq!(b.entries[b.selected].name, "beta");
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn back_and_forward() {
        let dir = fixture("history");
        std::fs::create_dir(dir.join("beta/deep")).unwrap();
        let mut b = FileExplorer::new(dir.clone());
        assert!(!b.can_go_back());
        b.navigate(dir.join("beta"));
        b.navigate(dir.join("beta/deep"));
        b.go_back();
        assert_eq!(b.dir, dir.join("beta"));
        assert_eq!(b.entries[b.selected].name, "deep");
        b.go_back();
        assert_eq!(b.dir, dir);
        assert_eq!(b.entries[b.selected].name, "beta");
        assert!(!b.can_go_back());
        b.go_forward();
        assert_eq!(b.dir, dir.join("beta"));
        // Going somewhere new forgets the folders ahead.
        b.navigate(dir.join("Alpha"));
        assert!(!b.can_go_forward());
        b.go_back();
        assert_eq!(b.dir, dir.join("beta"));
        // Up counts as a step too.
        b.up();
        b.go_back();
        assert_eq!(b.dir, dir.join("beta"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn unreadable_dir_reports_error() {
        let b = FileExplorer::new(PathBuf::from("/definitely/not/a/real/dir"));
        assert!(b.error.is_some());
        assert!(b.entries.is_empty());
    }

    #[test]
    fn targets_follow_marks_in_listing_order() {
        let dir = fixture("targets");
        let mut b = FileExplorer::new(dir.clone());
        assert_eq!(b.targets(), [dir.join("Alpha")]);
        b.toggle_mark(3);
        b.toggle_mark(1);
        assert_eq!(b.targets(), [dir.join("beta"), dir.join("zeta.txt")]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
