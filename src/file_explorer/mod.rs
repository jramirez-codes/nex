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
//!
//! An explorer opened from a terminal in an `ssh` session shows that session's folder on the
//! host (see `fs/remote.rs`): listings load in the background, and file operations run there.
//! Files can't be copied between the host and this machine, or opened with a local app.
//!
//! Folders are read off the UI thread wherever nobody is waiting for the answer: a folder on
//! an ssh host always, and one on this machine when it is re-read because time passed or
//! files changed (`refresh_if_stale`, which the app calls for the explorers on screen only).
//! The listing shown, its selection and its scroll position stay as they are until the new
//! one arrives. Opening a folder here reads it right away.

pub mod fs;
mod input;
#[cfg(test)]
mod tests;
mod view;

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, OnceLock, mpsc};
use std::time::{Duration, Instant};

use egui::{Pos2, Rect};

use crate::remote::RemoteError;
use crate::remote::exec::{ExecError, Transport};
use crate::terminal::tilde;

use fs::ops::read_entries;
use fs::remote::Listing;
use input::edit::{Find, OpenWith, Rename};
use input::Press;
use view::items::Layout;
use input::line_edit::LineEdit;
use view::properties::Properties;
use fs::transfer::{FS_GENERATION, Job};

pub use input::{ExplorerDrag, release_pos};
pub use view::items::{drag_image, paint_drag_ghost};
pub use fs::transfer::{Op, files_changed};

/// How often an idle listing is re-read so files created elsewhere show up (where the
/// folder can't be watched; see `platform::DirWatch`).
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// The same for a folder on an ssh host (each read is a trip there).
const REMOTE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);
/// How many folders Back remembers.
const HISTORY: usize = 100;

/// Something an explorer asks the app to do (the app owns the other panes).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Split a terminal off this pane, starting in the folder (on the explorer's host, for
    /// one on an ssh host).
    OpenTerminal(PathBuf),
    /// The ssh host wants a password nex's own connections can't type (its name).
    NeedsLogin(String),
}

/// What an explorer on an ssh host keeps: how to reach it, and what its listings said.
struct Remote {
    transport: Arc<Transport>,
    /// The home folder there, once a listing has said.
    home: Option<PathBuf>,
    ctx: egui::Context,
    /// What to select once the next listing arrives.
    after: Option<After>,
    /// A login failure was reported to the app (once until a listing works again).
    told_login: bool,
}

/// What reading a folder in the background gave: this machine's, or an ssh host's.
enum Answer {
    Local(std::io::Result<Vec<Entry>>),
    Remote(Result<Listing, ExecError>),
}

/// What to do with a listing still on its way, once it's there.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::file_explorer) enum After {
    /// Select the item leading back to this folder (see `select_towards`).
    Towards(PathBuf),
    Select(PathBuf),
    /// Select it and start renaming it, the whole name selected (a new item).
    Rename(PathBuf),
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
    /// Zoom level of this pane (1.0 = normal); see `input/zoom.rs`.
    zoom: f32,
    /// What to keep in place while zooming, applied on the next paint; see `input/zoom.rs`.
    zoom_anchor: Option<input::zoom::Anchor>,
    /// Scroll the selection into view on the next paint (after keyboard moves).
    reveal: bool,
    /// Columns and items per page, from the last paint (for keyboard movement).
    cols: usize,
    page_items: usize,
    layout: Option<Layout>,
    last_refresh: Instant,
    seen_generation: u64,
    /// The listing being read in the background, and which request it answers.
    listing: Option<mpsc::Receiver<(u64, Answer)>>,
    listing_generation: u64,
    /// The context to repaint when a background listing is in: the one this was last
    /// painted in (an explorer on an ssh host has it from the start).
    repaint: Arc<OnceLock<egui::Context>>,
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
    /// One of them is a folder (not a link to one), worked out when the dialog was asked for.
    delete_has_folder: bool,
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
    /// The ssh host the folder is on (`None`: this machine).
    remote: Option<Remote>,
}

impl FileExplorer {
    pub fn new(dir: PathBuf) -> Self {
        Self::make(dir, None)
    }

    /// An explorer showing `dir` on the host `transport` reaches. `dir` may start with `~`
    /// (it's read as the host's home folder).
    pub fn new_remote(ctx: &egui::Context, transport: Arc<Transport>, dir: PathBuf) -> Self {
        let remote = Remote { transport, home: None, ctx: ctx.clone(), after: None, told_login: false };
        Self::make(dir, Some(remote))
    }

    fn make(dir: PathBuf, remote: Option<Remote>) -> Self {
        let repaint = Arc::new(OnceLock::new());
        if let Some(r) = &remote {
            let _ = repaint.set(r.ctx.clone());
        }
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
            listing: None,
            listing_generation: 0,
            repaint,
            watch: None,
            press: None,
            last_pos: None,
            last_click: None,
            jobs: Vec::new(),
            status: None,
            confirm_delete: None,
            delete_has_folder: false,
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
            remote,
        };
        b.refresh();
        b
    }

    pub fn title(&self) -> String {
        match &self.remote {
            Some(r) => format!("{}: {}", r.transport.label(), self.shown_path(&self.dir)),
            None => tilde(&self.dir),
        }
    }

    /// The host this explorer's folder is on, if it isn't this machine.
    pub fn transport(&self) -> Option<&Arc<Transport>> {
        self.remote.as_ref().map(|r| &r.transport)
    }

    /// Which machine the paths are on (see `Transport::key`; `None`: this one). Files only
    /// move between explorers on the same one.
    pub fn origin(&self) -> Option<String> {
        self.transport().map(|t| t.key())
    }

    /// The home folder, on the machine the folder is on.
    pub fn home(&self) -> Option<PathBuf> {
        match &self.remote {
            Some(r) => r.home.clone(),
            None => crate::platform::home_dir(),
        }
    }

    /// `path` for people: under the home folder as `~/…`.
    pub(in crate::file_explorer) fn shown_path(&self, path: &Path) -> String {
        if self.remote.is_none() {
            return tilde(path);
        }
        match self.home().and_then(|h| path.strip_prefix(h).ok().map(Path::to_path_buf)) {
            Some(rest) if rest.as_os_str().is_empty() => "~".into(),
            Some(rest) => format!("~/{}", rest.display()),
            None => path.display().to_string(),
        }
    }

    /// The context background jobs on an ssh host repaint (one made locally has its own).
    pub(in crate::file_explorer) fn ctx_for_jobs(&self) -> egui::Context {
        self.remote.as_ref().map(|r| r.ctx.clone()).unwrap_or_default()
    }

    /// The folder is still being read (on an ssh host; one on this machine keeps showing
    /// what it has while it is re-read).
    pub fn loading(&self) -> bool {
        self.remote.is_some() && self.listing.is_some()
    }

    /// Re-read the directory, keeping the same entry selected if it still exists. A folder on
    /// this machine is read right away (someone opened it and is waiting); one on an ssh
    /// host in the background.
    pub fn refresh(&mut self) {
        self.last_refresh = Instant::now();
        if self.remote.is_some() {
            self.start_listing();
            return;
        }
        self.watch_dir();
        // A background read on its way is older than this one: forget it.
        self.listing = None;
        self.listing_generation += 1;
        let entries = read_entries(&self.dir, self.show_hidden).map_err(|e| e.to_string());
        self.set_entries(entries);
    }

    /// Re-read the directory without holding up the UI thread, on this machine too: the
    /// listing shown, its selection and scroll position stay until the new one is in.
    pub(in crate::file_explorer) fn refresh_in_background(&mut self) {
        self.last_refresh = Instant::now();
        if self.remote.is_none() {
            self.watch_dir();
        }
        self.start_listing();
    }

    /// Watch the folder shown, if it isn't watched yet.
    fn watch_dir(&mut self) {
        if self.watch.as_ref().is_none_or(|(d, _)| *d != self.dir) {
            self.watch = crate::platform::DirWatch::new(&self.dir).map(|w| (self.dir.clone(), w));
        }
    }

    /// Read the folder on another thread; [`poll_listing`](Self::poll_listing) takes the
    /// answer. A listing already on its way is forgotten.
    fn start_listing(&mut self) {
        self.listing_generation += 1;
        let (generation, dir, show_hidden) = (self.listing_generation, self.dir.clone(), self.show_hidden);
        let (transport, repaint) = (self.transport().cloned(), self.repaint.clone());
        let (tx, rx) = mpsc::channel();
        self.listing = Some(rx);
        std::thread::spawn(move || {
            let answer = match &transport {
                Some(t) => Answer::Remote(fs::remote::list(t, &dir)),
                None => Answer::Local(read_entries(&dir, show_hidden)),
            };
            let _ = tx.send((generation, answer));
            // Looked up after sending: a paint that sets it later also polls later.
            if let Some(ctx) = repaint.get() {
                ctx.request_repaint();
            }
        });
    }

    /// Show a listing read in the background once it's there.
    pub(in crate::file_explorer) fn poll_listing(&mut self) {
        let Some(rx) = &self.listing else { return };
        let (generation, answer) = match rx.try_recv() {
            Ok(answer) => answer,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.listing = None;
                return;
            }
        };
        if generation != self.listing_generation {
            return;
        }
        self.listing = None;
        let listing = match answer {
            Answer::Local(entries) => {
                self.set_entries(entries.map_err(|e| e.to_string()));
                return;
            }
            Answer::Remote(listing) => listing,
        };
        let Some(r) = self.remote.as_mut() else { return };
        let after = r.after.take();
        let entries = match listing {
            Ok(l) => {
                r.home = Some(l.home);
                r.told_login = false;
                // What was asked for may have been `~/…`: the host says where that is.
                self.dir = l.dir;
                let hidden = self.show_hidden;
                Ok(l.entries.into_iter().filter(|e| hidden || !e.name.starts_with('.')).collect())
            }
            Err(ExecError::Remote(RemoteError::NeedsLogin)) => {
                let host = r.transport.label();
                if !std::mem::replace(&mut r.told_login, true) {
                    self.requests.push(Request::NeedsLogin(host.clone()));
                }
                Err(format!("nex can't log in to {host} without a password"))
            }
            Err(e) => Err(e.to_string()),
        };
        self.set_entries(entries);
        match after {
            Some(After::Towards(from)) => self.select_towards(&from),
            Some(After::Select(path)) => self.select_path(&path),
            Some(After::Rename(path)) => self.select_and_name(&path),
            None => {}
        }
    }

    /// What to do once the listing on its way (or the next one) arrives.
    pub(in crate::file_explorer) fn after_listing(&mut self, after: After) {
        if let Some(r) = self.remote.as_mut() {
            r.after = Some(after);
        }
    }

    /// Show `entries` (or why the folder couldn't be read), keeping the same entry selected
    /// if it's still there.
    fn set_entries(&mut self, entries: Result<Vec<Entry>, String>) {
        let keep = self.entries.get(self.selected).map(|e| e.name.clone());
        match entries {
            Ok(entries) => {
                self.entries = entries;
                self.error = None;
            }
            Err(e) => {
                self.entries.clear();
                self.error = Some(e);
            }
        }
        match keep.and_then(|name| self.entries.iter().position(|e| e.name == name)) {
            Some(i) => self.selected = i,
            None => self.selected = self.selected.min(self.entries.len().saturating_sub(1)),
        }
        let entries = &self.entries;
        self.marked.retain(|p| entries.iter().any(|e| &e.path == p));
    }

    /// Refresh, in the background, if the listing hasn't been read for a while or a file
    /// operation finished. Also takes a listing that has arrived.
    pub fn refresh_if_stale(&mut self) {
        self.poll_listing();
        let generation = FS_GENERATION.load(Ordering::Relaxed);
        let interval = if self.remote.is_some() { REMOTE_REFRESH_INTERVAL } else { REFRESH_INTERVAL };
        // A slow host or disk: one read at a time, unless files changed meanwhile.
        let due = self.last_refresh.elapsed() >= interval && self.listing.is_none();
        if generation != self.seen_generation || due {
            self.seen_generation = generation;
            self.refresh_in_background();
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
        if self.remote.is_some() {
            self.after_listing(After::Towards(from));
        } else {
            self.select_towards(&from);
        }
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

    /// Put the cursor on `item` (an entry of this folder) and scroll it into view, showing
    /// hidden files first if it is one.
    pub fn reveal_item(&mut self, item: &Path) {
        if item.parent() != Some(self.dir.as_path()) {
            return;
        }
        if !self.show_hidden && item.file_name().is_some_and(|n| n.as_encoded_bytes().starts_with(b".")) {
            self.show_hidden = true;
            self.refresh();
        }
        if self.loading() {
            self.after_listing(After::Select(item.to_path_buf()));
        } else {
            self.select_path(item);
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

    /// Whether Escape would close something here rather than reach the listing's own keys:
    /// the location bar or a rename being edited, type-to-find, or a selection of marked
    /// items. The phone's Back closes these first, as Escape does.
    pub fn escape_closes(&self) -> bool {
        self.location.is_some() || self.rename.is_some() || self.find.is_some() || !self.marked.is_empty() || self.selecting
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
        } else if let Some(r) = &self.remote {
            let msg = format!("Files on {} can't be opened here: open a terminal in this folder instead", r.transport.label());
            self.set_status(msg, true);
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
