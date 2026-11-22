//! The Changes pane: a read-only `git diff` viewer that lives in the split tree beside
//! terminals. A list of changed files (staged and not) on the left, the selected file's
//! unified diff on the right. It follows the pane it was opened from: focusing another
//! terminal shows that terminal's repository.
//!
//! `git` (running it, parsing status and diffs), `view` (drawing and the mouse), and here:
//! the state, background loading and keys.

pub mod git;
mod view;
#[cfg(test)]
mod tests;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver};
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use crate::layout::PaneId;

use git::{FileChange, FileDiff, Status};

/// A finished diff load: its generation, the file, and the diff (or why it failed).
type DiffResult = (u64, Sel, Result<FileDiff, String>);

/// How often the status is re-read while the pane is on screen.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// What the pane asks the app for (it can't open panes itself).
#[derive(Clone, Debug, PartialEq)]
pub enum Request {
    /// Show this file in a Files pane split off this one.
    Reveal(PathBuf),
}

/// Which changes the list shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Filter {
    All,
    Unstaged,
    Staged,
}

/// Which column the keyboard drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Column {
    List,
    Diff,
}

/// A file in the list: its path, and whether it's the staged or the work-tree side.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sel {
    pub path: String,
    pub staged: bool,
}

impl Sel {
    fn of(f: &FileChange) -> Sel {
        Sel { path: f.path.clone(), staged: f.staged }
    }
}

/// A scroll position that eases toward its target (keyboard jumps), or follows it at once
/// (the wheel, which egui already smooths).
#[derive(Clone, Copy, Debug, Default)]
struct Scroll {
    pos: f32,
    target: f32,
}

impl Scroll {
    fn set(&mut self, v: f32) {
        self.pos = v;
        self.target = v;
    }

    fn by(&mut self, d: f32) {
        self.set(self.target + d);
    }

    fn clamp(&mut self, max: f32) {
        let max = max.max(0.0);
        self.pos = self.pos.clamp(0.0, max);
        self.target = self.target.clamp(0.0, max);
    }

    /// Move toward the target; true while still moving.
    fn step(&mut self, ctx: &egui::Context) -> bool {
        if (self.target - self.pos).abs() < 0.5 || !crate::ui::glide::animations(ctx) {
            self.pos = self.target;
            return false;
        }
        let dt = ctx.input(|i| i.stable_dt).min(0.05);
        self.pos += (self.target - self.pos) * (dt * 16.0).min(1.0);
        ctx.request_repaint();
        true
    }
}

pub struct GitDiff {
    /// The folder whose repository is shown (the followed pane's working directory).
    pub dir: PathBuf,
    /// The pane this was opened from; closing returns focus there.
    pub origin: Option<PaneId>,
    ctx: egui::Context,
    status: Option<Arc<Status>>,
    status_job: Option<Receiver<(u64, Status)>>,
    status_gen: u64,
    last_status: Option<Instant>,
    selected: Option<Sel>,
    /// The diff on screen and the file it belongs to.
    diff: Option<(Sel, Arc<Result<FileDiff, String>>)>,
    diff_job: Option<Receiver<DiffResult>>,
    diff_gen: u64,
    pub filter: Filter,
    /// Collapsed list sections: staged, unstaged.
    collapsed: [bool; 2],
    focus: Column,
    list_scroll: f32,
    diff_scroll: Scroll,
    diff_scroll_x: f32,
    /// Scroll the selected row into view on the next frame.
    reveal: bool,
    /// Narrow panes show one column at a time: the diff instead of the list.
    show_detail: bool,
    /// View state kept between frames (animations, last layout).
    view: view::ViewState,
    pub requests: Vec<Request>,
}

impl GitDiff {
    pub fn new(ctx: &egui::Context, dir: PathBuf) -> GitDiff {
        let mut d = GitDiff {
            dir,
            origin: None,
            ctx: ctx.clone(),
            status: None,
            status_job: None,
            status_gen: 0,
            last_status: None,
            selected: None,
            diff: None,
            diff_job: None,
            diff_gen: 0,
            filter: Filter::All,
            collapsed: [false; 2],
            focus: Column::List,
            list_scroll: 0.0,
            diff_scroll: Scroll::default(),
            diff_scroll_x: 0.0,
            reveal: false,
            show_detail: false,
            view: view::ViewState::new(),
            requests: Vec::new(),
        };
        d.refresh();
        d
    }

    /// The pane's title: the repository's folder name.
    pub fn title(&self) -> String {
        let root = match self.status.as_deref() {
            Some(Status::Ready(s)) => s.root.as_path(),
            _ => self.dir.as_path(),
        };
        let name = root.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| root.display().to_string());
        format!("{name} · Changes")
    }

    /// Show the repository holding `dir` instead (the followed pane moved, or another pane
    /// was focused). The current view stays up until the new status arrives.
    pub fn follow(&mut self, dir: PathBuf) {
        if dir != self.dir {
            self.dir = dir;
            self.refresh();
        }
    }

    /// Re-read the status if it's due (called every poll while the pane is on screen).
    pub fn refresh_if_stale(&mut self) {
        if self.status_job.is_none() && self.last_status.is_none_or(|t| t.elapsed() >= REFRESH_INTERVAL) {
            self.refresh();
        }
    }

    /// Re-read the status now (and then the selected file's diff).
    pub fn refresh(&mut self) {
        self.status_gen += 1;
        let generation = self.status_gen;
        let dir = self.dir.clone();
        let ctx = self.ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.status_job = Some(rx);
        std::thread::spawn(move || {
            let status = git::load_status(&dir);
            let _ = tx.send((generation, status));
            ctx.request_repaint();
        });
    }

    pub fn loading(&self) -> bool {
        self.status_job.is_some() || self.diff_job.is_some()
    }

    /// Load the selected file's diff in the background.
    fn load_diff(&mut self) {
        let (Some(sel), Some(Status::Ready(snap))) = (self.selected.clone(), self.status.as_deref()) else {
            self.diff = None;
            return;
        };
        let Some(file) = find(snap, &sel).cloned() else { return };
        self.diff_gen += 1;
        let generation = self.diff_gen;
        let root = snap.root.clone();
        let ctx = self.ctx.clone();
        let (tx, rx) = mpsc::channel();
        self.diff_job = Some(rx);
        std::thread::spawn(move || {
            let diff = git::load_diff(&root, &file);
            let _ = tx.send((generation, sel, diff));
            ctx.request_repaint();
        });
    }

    /// Pick up finished background work.
    fn poll(&mut self) {
        if let Some(rx) = &self.status_job
            && let Ok((generation, status)) = rx.try_recv()
        {
            self.status_job = None;
            if generation == self.status_gen {
                self.last_status = Some(Instant::now());
                self.set_status(status);
            }
        }
        if let Some(rx) = &self.diff_job
            && let Ok((generation, sel, diff)) = rx.try_recv()
        {
            self.diff_job = None;
            if generation == self.diff_gen && Some(&sel) == self.selected.as_ref() {
                let same_file = self.diff.as_ref().is_some_and(|(s, _)| *s == sel);
                if !same_file {
                    self.diff_scroll.set(0.0);
                    self.diff_scroll_x = 0.0;
                    self.view.fade_in(&self.ctx);
                }
                let unchanged = self.diff.as_ref().is_some_and(|(s, d)| *s == sel && **d == diff);
                if !unchanged {
                    self.diff = Some((sel, Arc::new(diff)));
                }
            }
        }
    }

    fn set_status(&mut self, status: Status) {
        let new_repo = match (self.status.as_deref(), &status) {
            (Some(Status::Ready(a)), Status::Ready(b)) => a.root != b.root,
            _ => true,
        };
        self.status = Some(Arc::new(status));
        if new_repo {
            self.selected = None;
            self.diff = None;
            self.list_scroll = 0.0;
            self.show_detail = false;
        }
        let files = self.visible_files();
        if !self.selected.as_ref().is_some_and(|s| files.contains(s)) {
            self.selected = files.first().cloned();
            self.reveal = true;
        }
        // The file may have changed too: its diff is re-read with every status.
        self.load_diff();
    }

    /// The snapshot, if the folder is a repository.
    fn snapshot(&self) -> Option<&git::Snapshot> {
        match self.status.as_deref() {
            Some(Status::Ready(s)) => Some(s),
            _ => None,
        }
    }

    /// Sections the filter shows, in order: `(staged, files)`.
    fn sections(&self) -> Vec<(bool, &[FileChange])> {
        let Some(s) = self.snapshot() else { return Vec::new() };
        let mut out = Vec::new();
        if self.filter != Filter::Unstaged && !s.staged.is_empty() {
            out.push((true, s.staged.as_slice()));
        }
        if self.filter != Filter::Staged && !s.unstaged.is_empty() {
            out.push((false, s.unstaged.as_slice()));
        }
        out
    }

    /// The files the keyboard can step through: those in open sections, in list order.
    fn visible_files(&self) -> Vec<Sel> {
        self.sections()
            .into_iter()
            .filter(|(staged, _)| !self.collapsed[if *staged { 0 } else { 1 }])
            .flat_map(|(_, files)| files.iter().map(Sel::of))
            .collect()
    }

    fn select(&mut self, sel: Sel) {
        if self.selected.as_ref() != Some(&sel) {
            self.selected = Some(sel);
            self.reveal = true;
            self.load_diff();
        }
    }

    fn step_selection(&mut self, delta: isize) {
        let files = self.visible_files();
        if files.is_empty() {
            return;
        }
        let at = self.selected.as_ref().and_then(|s| files.iter().position(|f| f == s));
        let next = match at {
            Some(i) => (i as isize + delta).clamp(0, files.len() as isize - 1) as usize,
            None => 0,
        };
        self.select(files[next].clone());
    }

    fn set_filter(&mut self, filter: Filter) {
        if self.filter == filter {
            return;
        }
        self.filter = filter;
        self.list_scroll = 0.0;
        let files = self.visible_files();
        if !self.selected.as_ref().is_some_and(|s| files.contains(s)) {
            self.selected = None;
            self.diff = None;
            if let Some(first) = files.first().cloned() {
                self.select(first);
            }
        }
    }

    fn toggle_section(&mut self, staged: bool) {
        let i = if staged { 0 } else { 1 };
        self.collapsed[i] = !self.collapsed[i];
    }

    /// The selected file's full path.
    fn selected_path(&self) -> Option<PathBuf> {
        let (snap, sel) = (self.snapshot()?, self.selected.as_ref()?);
        Some(snap.root.join(Path::new(&sel.path)))
    }

    /// The selected file, for the clipboard (its repository-relative path).
    pub fn copy_text(&self) -> Option<String> {
        self.selected.as_ref().map(|s| s.path.clone())
    }

    fn reveal_selected(&mut self) {
        if let Some(path) = self.selected_path().filter(|p| p.exists()) {
            self.requests.push(Request::Reveal(path));
        }
    }

    /// Scroll the diff to the next (or previous) hunk.
    fn jump_hunk(&mut self, forward: bool) {
        let at = self.diff_scroll.target;
        let tops = &self.view.hunk_tops;
        let next = if forward { tops.iter().copied().find(|&t| t > at + 1.0) } else { tops.iter().rev().copied().find(|&t| t < at - 1.0) };
        if let Some(t) = next.or(if forward { None } else { Some(0.0) }) {
            self.diff_scroll.target = t;
            self.focus = Column::Diff;
        }
    }

    /// A key for this pane; returns whether it was used.
    pub fn key(&mut self, key: Key, mods: Modifiers) -> bool {
        if mods.ctrl || mods.alt || mods.mac_cmd || mods.command {
            return false;
        }
        let line = self.view.line_height.max(1.0);
        let page = (self.view.diff_view_height - line * 2.0).max(line);
        let narrow = self.view.narrow;
        match key {
            Key::ArrowDown | Key::J if self.focus == Column::List => self.step_selection(1),
            Key::ArrowUp | Key::K if self.focus == Column::List => self.step_selection(-1),
            Key::ArrowDown | Key::J => self.diff_scroll.target += line * 3.0,
            Key::ArrowUp | Key::K => self.diff_scroll.target -= line * 3.0,
            Key::PageDown | Key::Space => self.diff_scroll.target += page,
            Key::PageUp => self.diff_scroll.target -= page,
            Key::Home => self.diff_scroll.target = 0.0,
            Key::End => self.diff_scroll.target = f32::MAX,
            Key::G if mods.shift => self.diff_scroll.target = f32::MAX,
            Key::G => self.diff_scroll.target = 0.0,
            Key::N if mods.shift => self.jump_hunk(false),
            Key::N => self.jump_hunk(true),
            Key::P => self.jump_hunk(false),
            Key::R => self.refresh(),
            Key::Num1 => self.set_filter(Filter::All),
            Key::Num2 => self.set_filter(Filter::Unstaged),
            Key::Num3 => self.set_filter(Filter::Staged),
            Key::Tab => {
                self.focus = if self.focus == Column::List { Column::Diff } else { Column::List };
                if narrow {
                    self.show_detail = self.focus == Column::Diff;
                }
            }
            Key::ArrowRight | Key::L => {
                if self.focus == Column::Diff {
                    self.diff_scroll_x += 40.0;
                }
                self.focus = Column::Diff;
                self.show_detail = true;
            }
            Key::ArrowLeft | Key::H => {
                if self.focus == Column::Diff && self.diff_scroll_x > 0.0 {
                    self.diff_scroll_x = (self.diff_scroll_x - 40.0).max(0.0);
                } else {
                    self.focus = Column::List;
                    self.show_detail = false;
                }
            }
            Key::Enter if narrow && !self.show_detail => {
                self.show_detail = true;
                self.focus = Column::Diff;
            }
            Key::Escape if narrow && self.show_detail => {
                self.show_detail = false;
                self.focus = Column::List;
            }
            Key::Enter | Key::O => self.reveal_selected(),
            _ => return false,
        }
        true
    }
}

/// `sel`'s entry in the snapshot.
fn find<'a>(snap: &'a git::Snapshot, sel: &Sel) -> Option<&'a FileChange> {
    let list = if sel.staged { &snap.staged } else { &snap.unstaged };
    list.iter().find(|f| f.path == sel.path)
}
