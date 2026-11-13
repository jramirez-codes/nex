//! The file-explorer pane shown in the Files tab: a directory listing (as a list or a grid of
//! tiles) you can navigate with the mouse, keyboard or a touch screen. Folders open in place;
//! files open with the desktop's default app.
//!
//! Files can be copied, cut and pasted (Ctrl+C / Ctrl+X / Ctrl+V, the right-click menu, or the
//! action bar that appears while items are selected), and dragged onto folders, onto another
//! explorer pane, or onto a terminal (which inserts their paths). Hold Ctrl while dropping to
//! copy instead of move. On touch screens a tap opens, a long press starts selecting, dragging
//! scrolls, and a long press followed by a drag moves the selection.

use std::collections::BTreeSet;
use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::{Duration, Instant};

use egui::{Align2, Color32, DragAndDrop, FontId, Id, Key, LayerId, Modifiers, Order, Painter, PointerButton, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::ui::{icons, paint};
use crate::layout::PaneId;
use crate::ui::theme::Theme;
use crate::search;
use crate::terminal::tilde;

/// How often an idle listing is re-read so files created elsewhere show up.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// Holding a touch this long (without moving) starts selecting.
const LONG_PRESS: f64 = 0.45;
const DOUBLE_CLICK: f64 = 0.4;
/// How long a finished operation's message stays in the action bar.
const STATUS_TIME: Duration = Duration::from_secs(4);
const ACCENT: Color32 = Color32::from_rgb(0x6c, 0xa8, 0xff);
const ERROR: Color32 = Color32::from_rgb(0xf2, 0x77, 0x7a);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum View {
    #[default]
    List,
    Grid,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
}

/// Sizes of the explorer's parts; touch screens get larger hit targets.
struct Sizes {
    bar: f32,
    row: f32,
    tile: Vec2,
    icon: f32,
    action_bar: f32,
    button: f32,
    font: f32,
}

fn sizes(touch: bool) -> Sizes {
    if touch {
        Sizes { bar: 40.0, row: 40.0, tile: Vec2::new(112.0, 104.0), icon: 44.0, action_bar: 52.0, button: 38.0, font: 14.0 }
    } else {
        Sizes { bar: 30.0, row: 24.0, tile: Vec2::new(96.0, 86.0), icon: 36.0, action_bar: 38.0, button: 26.0, font: 13.0 }
    }
}

/// Files picked up by Copy or Cut, waiting to be pasted. Shared by every explorer pane.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

static CLIPBOARD: Mutex<Option<Clip>> = Mutex::new(None);

pub fn clipboard() -> Option<Clip> {
    CLIPBOARD.lock().map(|c| c.clone()).unwrap_or(None)
}

fn set_clipboard(clip: Option<Clip>) {
    if let Ok(mut c) = CLIPBOARD.lock() {
        *c = clip;
    }
}

/// Bumped whenever a file operation finishes, so every explorer re-reads its listing.
static FS_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Drag-and-drop payload for files dragged out of a explorer.
pub struct ExplorerDrag {
    pub paths: Vec<PathBuf>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// File size in bytes (`None` for directories).
    pub size: Option<u64>,
}

#[derive(Clone, Copy, PartialEq)]
enum Gesture {
    /// Pressed, not yet moved: may become a click, a long press, a pan or a drag.
    Pending,
    /// A touch was held: the item is selected and a following move drags the selection.
    LongPressed,
    /// A touch drag scrolling the listing.
    Panning,
    /// Files are being dragged (the payload is in `egui::DragAndDrop`).
    Dragging,
    /// Nothing more to do for this press.
    Idle,
}

struct Press {
    item: Option<usize>,
    origin: Pos2,
    at: f64,
    touch: bool,
    gesture: Gesture,
}

/// Where items are laid out in the listing.
#[derive(Clone, Copy)]
struct Layout {
    list: Rect,
    cols: usize,
    item: Vec2,
}

impl Layout {
    fn item_rect(&self, i: usize, scroll: f32) -> Rect {
        let (row, col) = (i / self.cols, i % self.cols);
        let min = self.list.min + Vec2::new(col as f32 * self.item.x, row as f32 * self.item.y - scroll);
        Rect::from_min_size(min, self.item)
    }

    fn item_at(&self, pos: Pos2, scroll: f32, len: usize) -> Option<usize> {
        if !self.list.contains(pos) {
            return None;
        }
        let local = pos - self.list.min + Vec2::new(0.0, scroll);
        let col = (local.x / self.item.x).floor() as usize;
        let row = (local.y / self.item.y).floor() as usize;
        let i = row * self.cols + col;
        (col < self.cols && i < len).then_some(i)
    }

    fn rows(&self, len: usize) -> usize {
        len.div_ceil(self.cols)
    }
}

/// Things the right-click menu or the action bar asked for (applied after drawing).
#[derive(Clone, Copy, PartialEq)]
enum Command {
    Open(usize),
    Copy,
    Cut,
    Paste,
    PasteInto(usize),
    ClearClipboard,
    CopyPath,
    SelectMode,
    SelectAll,
    ClearSelection,
    ToggleView,
    ToggleHidden,
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
    /// Scroll the selection into view on the next paint (after keyboard moves).
    reveal: bool,
    /// Columns and items per page, from the last paint (for keyboard movement).
    cols: usize,
    page_items: usize,
    layout: Option<Layout>,
    last_refresh: Instant,
    seen_generation: u64,
    press: Option<Press>,
    /// Last known pointer position: a touch release also reports the pointer as gone, so the
    /// release frame needs this to know where a drop landed.
    last_pos: Option<Pos2>,
    last_click: Option<(usize, f64)>,
    jobs: Vec<Receiver<Result<String, String>>>,
    status: Option<(String, bool, Instant)>,
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
            reveal: false,
            cols: 1,
            page_items: 10,
            layout: None,
            last_refresh: Instant::now(),
            seen_generation: FS_GENERATION.load(Ordering::Relaxed),
            press: None,
            last_pos: None,
            last_click: None,
            jobs: Vec::new(),
            status: None,
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

    pub fn navigate(&mut self, dir: PathBuf) {
        self.dir = dir;
        self.entries.clear();
        self.marked.clear();
        self.selecting = false;
        self.selected = 0;
        self.scroll = 0.0;
        self.refresh();
        self.reveal = true;
    }

    /// Go to the parent folder, selecting the folder we came from.
    pub fn up(&mut self) {
        let Some(parent) = self.dir.parent().map(Path::to_path_buf) else { return };
        let from = self.dir.file_name().map(|n| n.to_string_lossy().to_string());
        self.navigate(parent);
        if let Some(i) = from.and_then(|name| self.entries.iter().position(|e| e.name == name)) {
            self.selected = i;
        }
    }

    /// Open a folder in place, or a file with the desktop's default app.
    pub fn activate(&mut self, index: usize) {
        let Some(entry) = self.entries.get(index) else { return };
        if entry.is_dir {
            let path = entry.path.clone();
            self.navigate(path);
        } else {
            search::open(&entry.path);
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

    /// Put the targets on the file clipboard (and their paths on the system clipboard as text).
    pub fn copy(&mut self, ctx: &egui::Context, cut: bool) {
        let paths = self.targets();
        if paths.is_empty() {
            return;
        }
        ctx.copy_text(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        let verb = if cut { "Cut" } else { "Copied" };
        self.set_status(format!("{verb} {} — paste in any folder", count(paths.len())), false);
        set_clipboard(Some(Clip { paths, cut }));
        self.clear_selection();
    }

    /// Paste the file clipboard into `dest` (default: the current folder). Without a file
    /// clipboard, `text` holding absolute paths (or file:// URIs) is pasted as a copy.
    pub fn paste(&mut self, ctx: &egui::Context, dest: Option<PathBuf>, text: Option<&str>) {
        let dest = dest.unwrap_or_else(|| self.dir.clone());
        match clipboard() {
            Some(clip) => {
                if clip.cut {
                    set_clipboard(None);
                }
                self.start_op(ctx, if clip.cut { Op::Move } else { Op::Copy }, clip.paths, dest);
            }
            None => {
                let paths = text.map(parse_paths).unwrap_or_default();
                if !paths.is_empty() {
                    self.start_op(ctx, Op::Copy, paths, dest);
                }
            }
        }
    }

    /// Files dropped from another application: copy them into the folder under `pos`
    /// (or the current folder).
    pub fn drop_external(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, pos: Option<Pos2>) {
        let dest = pos.and_then(|p| self.folder_at(p, &[])).unwrap_or_else(|| self.dir.clone());
        self.start_op(ctx, Op::Copy, paths, dest);
    }

    /// The folder entry under `pos`, unless it's one of the files being dragged.
    fn folder_at(&self, pos: Pos2, dragged: &[PathBuf]) -> Option<PathBuf> {
        let layout = self.layout?;
        let i = layout.item_at(pos, self.scroll, self.entries.len())?;
        let e = &self.entries[i];
        (e.is_dir && !dragged.contains(&e.path)).then(|| e.path.clone())
    }

    /// Copy or move `paths` into `dest` on a background thread.
    pub fn start_op(&mut self, ctx: &egui::Context, op: Op, paths: Vec<PathBuf>, dest: PathBuf) {
        if paths.is_empty() {
            return;
        }
        let verb = if op == Op::Copy { "Copying" } else { "Moving" };
        self.set_status(format!("{verb} {}…", count(paths.len())), false);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = run_op(op, &paths, &dest).map(|n| {
                let verb = if op == Op::Copy { "Copied" } else { "Moved" };
                if n == 0 { "Nothing to do — already here".to_string() } else { format!("{verb} {} to {}", count(n), tilde(&dest)) }
            });
            FS_GENERATION.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.jobs.push(rx);
    }

    fn set_status(&mut self, text: String, error: bool) {
        self.status = Some((text, error, Instant::now()));
    }

    fn poll_jobs(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.jobs.retain(|rx| match rx.try_recv() {
            Ok(r) => {
                finished.push(r);
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        for r in finished {
            match r {
                Ok(msg) => self.set_status(msg, false),
                Err(msg) => self.set_status(msg, true),
            }
        }
        let generation = FS_GENERATION.load(Ordering::Relaxed);
        if generation != self.seen_generation {
            self.seen_generation = generation;
            self.refresh();
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }

    /// Handle a key press while this pane is focused. Returns true if it was used.
    pub fn key(&mut self, key: Key, mods: Modifiers) -> bool {
        let cols = self.cols.max(1) as isize;
        let page = self.page_items.max(1) as isize;
        match key {
            Key::ArrowUp if mods.alt => self.up(),
            Key::ArrowUp => self.move_selection(-cols),
            Key::ArrowDown => self.move_selection(cols),
            Key::ArrowLeft if self.view == View::Grid => self.move_selection(-1),
            Key::ArrowRight if self.view == View::Grid => self.move_selection(1),
            Key::PageUp => self.move_selection(-page),
            Key::PageDown => self.move_selection(page),
            Key::Home => self.move_selection(isize::MIN / 2),
            Key::End => self.move_selection(isize::MAX / 2),
            Key::Enter => self.activate(self.selected),
            Key::Backspace => self.up(),
            Key::Space => self.toggle_mark(self.selected),
            Key::Escape => {
                if !self.marked.is_empty() || self.selecting {
                    self.clear_selection();
                } else {
                    set_clipboard(None);
                }
            }
            Key::A if mods.ctrl => self.select_all(),
            Key::G if mods.ctrl => self.toggle_view(),
            Key::H if mods.ctrl => self.toggle_hidden(),
            _ => return false,
        }
        true
    }

    /// Draw the path bar, listing and action bar in `rect`, handling clicks, touch gestures,
    /// scrolling and drops.
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: PaneId, c: &Theme) {
        let ctx = ui.ctx().clone();
        self.poll_jobs(&ctx);
        let s = sizes(self.touch);
        let (pos, pressed, released, down, delta, mods, touches, now) = ui.input(|i| {
            (
                i.pointer.interact_pos(),
                i.pointer.primary_pressed(),
                i.pointer.primary_released(),
                i.pointer.primary_down(),
                i.pointer.delta(),
                i.modifiers,
                i.any_touches(),
                i.time,
            )
        });
        let pos = if released { pos.or(self.last_pos) } else { pos };
        if pos.is_some() {
            self.last_pos = pos;
        }
        let drag = DragAndDrop::payload::<ExplorerDrag>(&ctx);
        let mut commands = Vec::new();

        // ---- Path bar: up button, current directory, select + view toggles.
        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), s.bar));
        let btn = s.bar - 8.0;
        let up_r = Rect::from_center_size(Pos2::new(bar.left() + btn / 2.0 + 1.0, bar.center().y), Vec2::splat(btn));
        let can_go_up = self.dir.parent().is_some();
        let parent_drop = drag.is_some() && can_go_up && pos.is_some_and(|p| up_r.contains(p));
        if icon_button(ui, painter, up_r, Id::new(("files-up", pane)), "Parent folder  (Backspace)", can_go_up, parent_drop, c, |p, r, col| {
            icons::icon_chevron_up(p, r, col)
        }) {
            self.up();
        }

        // View toggle: a two-segment control (list | grid).
        let seg = Rect::from_min_max(Pos2::new(bar.right() - btn * 2.0 - 4.0, up_r.top()), Pos2::new(bar.right(), up_r.bottom()));
        painter.rect_filled(seg, 7, Color32::from_black_alpha(50));
        for (k, view) in [View::List, View::Grid].into_iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(seg.left() + 2.0 + k as f32 * btn, seg.top() + 2.0), Vec2::new(btn, btn - 4.0));
            let on = self.view == view;
            if on {
                painter.rect_filled(r, 6, Color32::from_white_alpha(30));
            }
            let (tip, icon): (&str, fn(&Painter, Rect, Color32)) = match view {
                View::List => ("List view  (Ctrl+G)", icons::icon_list),
                View::Grid => ("Grid view  (Ctrl+G)", icons::icon_grid),
            };
            let resp = ui.interact(r, Id::new(("files-view", pane, k)), Sense::click());
            let col = if on || resp.hovered() { c.text_bright } else { c.text_dim };
            icon(painter, Rect::from_center_size(r.center(), Vec2::splat(12.0)), col);
            if resp.on_hover_text(tip).clicked() && !on {
                commands.push(Command::ToggleView);
            }
        }
        // Selection-mode toggle (mainly for touch screens; a long press also starts it).
        let sel_r = Rect::from_center_size(Pos2::new(seg.left() - 6.0 - btn / 2.0, bar.center().y), Vec2::splat(btn));
        let selecting = self.selecting;
        if icon_button(ui, painter, sel_r, Id::new(("files-select", pane)), "Select items", true, selecting, c, |p, r, col| {
            icons::icon_check(p, r.shrink(1.0), if selecting { ACCENT } else { col }, selecting)
        }) {
            commands.push(if selecting { Command::ClearSelection } else { Command::SelectMode });
        }

        let path_left = up_r.right() + 8.0;
        paint::faded_text(
            painter,
            Pos2::new(path_left, bar.center().y),
            &self.title(),
            FontId::proportional(s.font - 0.5),
            c.text_dim,
            sel_r.left() - path_left - 6.0,
            20.0,
        );
        painter.line_segment(
            [Pos2::new(rect.left(), bar.bottom()), Pos2::new(rect.right(), bar.bottom())],
            Stroke::new(1.0, Color32::from_white_alpha(14)),
        );

        // ---- Action bar (selection, clipboard, progress), at the bottom.
        let clip = clipboard();
        let status = self.status.as_ref().filter(|(_, _, at)| !self.jobs.is_empty() || at.elapsed() < STATUS_TIME).cloned();
        let show_actions = self.selecting || !self.marked.is_empty() || clip.is_some() || status.is_some();
        let mut list_bottom = rect.bottom();
        if show_actions {
            let ab = Rect::from_min_max(Pos2::new(rect.left(), rect.bottom() - s.action_bar), rect.max);
            list_bottom = ab.top() - 2.0;
            self.action_bar(ui, painter, ab, pane, c, &s, clip.as_ref(), status.as_ref(), &mut commands);
            if let Some((_, _, at)) = &status {
                ctx.request_repaint_after(STATUS_TIME.saturating_sub(at.elapsed()));
            }
        }

        // ---- Listing.
        let list = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom() + 4.0), Pos2::new(rect.right(), list_bottom));
        if !list.is_positive() {
            self.apply(&ctx, commands);
            return;
        }
        let layout = match self.view {
            View::List => Layout { list, cols: 1, item: Vec2::new(list.width(), s.row) },
            View::Grid => {
                let cols = ((list.width() / s.tile.x).floor() as usize).max(1);
                Layout { list, cols, item: Vec2::new(list.width() / cols as f32, s.tile.y) }
            }
        };
        self.layout = Some(layout);
        self.cols = layout.cols;
        self.page_items = ((list.height() / layout.item.y).floor() as usize).max(1) * layout.cols;
        let len = self.entries.len();

        let resp = ui.interact(list, Id::new(("files-list", pane)), Sense::click_and_drag());

        // Pointer gestures. A press starts here; the rest of it is tracked even outside the list.
        if pressed && pos.is_some_and(|p| list.contains(p)) {
            let p = pos.unwrap_or_default();
            self.touch = touches;
            self.press = Some(Press { item: layout.item_at(p, self.scroll, len), origin: p, at: now, touch: touches, gesture: Gesture::Pending });
        }
        if let (Some(press), Some(p)) = (self.press.as_mut(), pos) {
            let moved = (p - press.origin).length();
            let slop = if press.touch { 10.0 } else { 6.0 };
            match press.gesture {
                Gesture::Pending if down => {
                    if moved > slop {
                        if press.touch {
                            press.gesture = Gesture::Panning;
                            self.scroll -= p.y - press.origin.y;
                        } else if let Some(i) = press.item {
                            press.gesture = Gesture::Dragging;
                            self.start_drag(&ctx, i);
                        } else {
                            press.gesture = Gesture::Idle;
                        }
                    } else if press.touch && press.item.is_some() {
                        let left = LONG_PRESS - (now - press.at);
                        if left <= 0.0 {
                            press.gesture = Gesture::LongPressed;
                            let i = press.item.unwrap_or_default();
                            self.selecting = true;
                            self.selected = i;
                            if !self.is_marked(i) {
                                self.toggle_mark(i);
                            }
                        } else {
                            ctx.request_repaint_after(Duration::from_secs_f64(left));
                        }
                    }
                }
                Gesture::LongPressed if down && moved > slop => {
                    press.gesture = Gesture::Dragging;
                    let i = press.item.unwrap_or_default();
                    self.start_drag(&ctx, i);
                }
                Gesture::Panning if down => self.scroll -= delta.y,
                _ => {}
            }
        }
        if released {
            if let Some(press) = self.press.take() {
                if press.gesture == Gesture::Pending {
                    self.click(press.item, press.touch, mods, now);
                }
            }
        }
        if resp.secondary_clicked() {
            match pos.and_then(|p| layout.item_at(p, self.scroll, len)) {
                Some(i) if !self.is_marked(i) => {
                    self.marked.clear();
                    self.selected = i;
                }
                Some(_) => {}
                None => self.marked.clear(),
            }
        }

        // Wheel scrolling, then keep the keyboard cursor in view.
        if ui.rect_contains_pointer(list) {
            self.scroll -= ui.input(|i| i.raw_scroll_delta.y);
        }
        if std::mem::take(&mut self.reveal) && len > 0 {
            let top = (self.selected / layout.cols) as f32 * layout.item.y;
            if top < self.scroll {
                self.scroll = top;
            } else if top + layout.item.y > self.scroll + list.height() {
                self.scroll = top + layout.item.y - list.height();
            }
        }
        let content_h = layout.rows(len) as f32 * layout.item.y;
        self.scroll = self.scroll.clamp(0.0, (content_h - list.height()).max(0.0));

        // Drops of files dragged out of a explorer: onto a folder, onto the listing (this
        // folder), or onto the up button (the parent folder).
        let mut drop_target = None;
        if let (Some(drag), Some(p)) = (&drag, pos) {
            let target = if parent_drop {
                self.dir.parent().map(Path::to_path_buf)
            } else if list.contains(p) {
                Some(self.folder_at(p, &drag.paths).unwrap_or_else(|| self.dir.clone()))
            } else {
                None
            };
            let op = if mods.ctrl { Op::Copy } else { Op::Move };
            // Moving files into the folder they're already in does nothing; don't advertise it.
            let useful = target.as_ref().is_some_and(|t| op == Op::Copy || drag.paths.iter().any(|f| f.parent() != Some(t.as_path())));
            if useful {
                drop_target = target.clone();
                if released {
                    if let Some(drag) = DragAndDrop::take_payload::<ExplorerDrag>(&ctx) {
                        self.start_op(&ctx, op, drag.paths.clone(), target.unwrap_or_default());
                        self.clear_selection();
                    }
                }
            }
        }
        // Files dragged in from other applications.
        let os_hover = ui.input(|i| !i.raw.hovered_files.is_empty()) && ui.rect_contains_pointer(list);

        let highlight_list = drop_target.as_ref() == Some(&self.dir) || os_hover;
        if highlight_list {
            painter.rect_filled(list, 8, ACCENT.gamma_multiply(0.06));
            painter.rect_stroke(list.shrink(1.0), 8, Stroke::new(1.5, ACCENT.gamma_multiply(0.7)), StrokeKind::Inside);
        }

        // ---- Items.
        if let Some(err) = &self.error {
            painter.text(list.center_top() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, err, FontId::proportional(12.5), ERROR);
        } else if len == 0 {
            painter.text(list.center_top() + Vec2::new(0.0, 24.0), Align2::CENTER_CENTER, "Empty folder", FontId::proportional(12.5), c.text_dim);
        }
        let clip_p = painter.with_clip_rect(list);
        let hover = pos.filter(|_| !self.touch && drag.is_none() && ui.rect_contains_pointer(list)).and_then(|p| layout.item_at(p, self.scroll, len));
        let first_row = (self.scroll / layout.item.y).floor() as usize;
        let last_row = ((self.scroll + list.height()) / layout.item.y).ceil() as usize;
        let cut_paths: &[PathBuf] = clip.as_ref().filter(|c| c.cut).map(|c| c.paths.as_slice()).unwrap_or(&[]);
        for i in (first_row * layout.cols)..((last_row * layout.cols).min(len)) {
            let r = layout.item_rect(i, self.scroll);
            let e = &self.entries[i];
            let marked = self.marked.contains(&e.path);
            let cursor = self.marked.is_empty() && !self.selecting && !self.touch && i == self.selected;
            let is_drop = drop_target.as_ref() == Some(&e.path);
            let dimmed = cut_paths.contains(&e.path);
            let state = ItemState { marked, cursor, hovered: hover == Some(i), drop: is_drop, dimmed, selecting: self.selecting };
            match self.view {
                View::List => paint_row(&clip_p, r, e, &state, c, &s),
                View::Grid => paint_tile(&clip_p, r, e, &state, c, &s),
            }
        }

        // ---- Right-click menu.
        let menu_item = self.targets().len();
        let menu_folder = (self.marked.len() <= 1).then(|| self.entries.get(self.selected).filter(|e| e.is_dir).map(|_| self.selected)).flatten();
        let (selected, has_clip, view, hidden) = (self.selected, clip.is_some(), self.view, self.show_hidden);
        resp.context_menu(|ui| {
            ui.set_min_width(180.0);
            let mut item = |ui: &mut Ui, label: &str, cmd: Command| {
                if ui.button(label).clicked() {
                    commands.push(cmd);
                    ui.close();
                }
            };
            if menu_item > 0 {
                if self.marked.len() <= 1 {
                    item(ui, "Open", Command::Open(selected));
                }
                item(ui, "Copy", Command::Copy);
                item(ui, "Cut", Command::Cut);
                if let (true, Some(f)) = (has_clip, menu_folder) {
                    item(ui, "Paste into folder", Command::PasteInto(f));
                }
                item(ui, if menu_item > 1 { "Copy paths" } else { "Copy path" }, Command::CopyPath);
                ui.separator();
            }
            if has_clip {
                item(ui, "Paste here", Command::Paste);
            }
            item(ui, "Select…", Command::SelectMode);
            item(ui, "Select all", Command::SelectAll);
            ui.separator();
            item(ui, if view == View::List { "Grid view" } else { "List view" }, Command::ToggleView);
            item(ui, if hidden { "Hide hidden files" } else { "Show hidden files" }, Command::ToggleHidden);
        });

        self.apply(&ctx, commands);
    }

    /// The bottom bar: a status line plus selection and clipboard buttons.
    #[allow(clippy::too_many_arguments)]
    fn action_bar(
        &self,
        ui: &Ui,
        painter: &Painter,
        ab: Rect,
        pane: PaneId,
        c: &Theme,
        s: &Sizes,
        clip: Option<&Clip>,
        status: Option<&(String, bool, Instant)>,
        commands: &mut Vec<Command>,
    ) {
        painter.line_segment([ab.left_top(), ab.right_top()], Stroke::new(1.0, Color32::from_white_alpha(14)));
        let mut buttons: Vec<(&str, Command, bool)> = Vec::new();
        if self.selecting || !self.marked.is_empty() {
            buttons.push(("All", Command::SelectAll, false));
            if !self.marked.is_empty() {
                buttons.push(("Copy", Command::Copy, false));
                buttons.push(("Cut", Command::Cut, false));
            }
        }
        if let Some(clip) = clip {
            buttons.push((if clip.cut { "Move here" } else { "Paste here" }, Command::Paste, true));
            if !self.selecting && self.marked.is_empty() {
                buttons.push(("Cancel", Command::ClearClipboard, false));
            }
        }
        if self.selecting || !self.marked.is_empty() {
            buttons.push(("Done", Command::ClearSelection, false));
        }

        // Lay the buttons out from the right.
        let font = FontId::proportional(s.font - 0.5);
        let mut x = ab.right() - 4.0;
        for (k, (label, cmd, primary)) in buttons.iter().enumerate().rev() {
            let w = painter.layout_no_wrap(label.to_string(), font.clone(), c.text).size().x + if self.touch { 28.0 } else { 20.0 };
            let r = Rect::from_min_size(Pos2::new(x - w, ab.center().y - s.button / 2.0), Vec2::new(w, s.button));
            if r.left() < ab.left() + 4.0 {
                break;
            }
            x = r.left() - 6.0;
            let resp = ui.interact(r, Id::new(("files-action", pane, k)), Sense::click());
            let fill = match (*primary, resp.hovered()) {
                (true, true) => ACCENT,
                (true, false) => ACCENT.gamma_multiply(0.8),
                (false, true) => Color32::from_white_alpha(34),
                (false, false) => Color32::from_white_alpha(18),
            };
            painter.rect_filled(r, (s.button / 2.0) as u8, fill);
            let col = if *primary { Color32::WHITE } else { c.text_bright };
            painter.text(r.center(), Align2::CENTER_CENTER, label, font.clone(), col);
            if resp.clicked_by(PointerButton::Primary) {
                commands.push(*cmd);
            }
        }

        // Status / summary on the left.
        let (text, col) = match (status, clip) {
            (Some((msg, err, _)), _) => (msg.clone(), if *err { ERROR } else { c.text }),
            _ if !self.marked.is_empty() => (format!("{} selected", count(self.marked.len())), c.text),
            _ if self.selecting => ("Tap items to select".to_string(), c.text_dim),
            (None, Some(clip)) => (format!("{} to {}", count(clip.paths.len()), if clip.cut { "move" } else { "copy" }), c.text_dim),
            _ => (String::new(), c.text_dim),
        };
        let left = ab.left() + 8.0;
        paint::faded_text(painter, Pos2::new(left, ab.center().y), &text, font, col, x - left - 4.0, 18.0);
    }

    /// Start dragging item `i` (with the rest of the selection, if it's part of it).
    fn start_drag(&mut self, ctx: &egui::Context, i: usize) {
        if !self.is_marked(i) {
            self.marked.clear();
            self.selected = i;
        }
        let paths = self.targets();
        if !paths.is_empty() {
            DragAndDrop::set_payload(ctx, ExplorerDrag { paths });
        }
    }

    /// A press released without moving: select, toggle or open.
    fn click(&mut self, item: Option<usize>, touch: bool, mods: Modifiers, now: f64) {
        let Some(i) = item else {
            if !mods.ctrl && !mods.shift {
                self.marked.clear();
            }
            return;
        };
        if self.selecting {
            self.toggle_mark(i);
            if self.marked.is_empty() {
                self.selecting = false;
            }
        } else if touch {
            self.selected = i;
            self.activate(i);
        } else if mods.ctrl {
            if self.marked.is_empty() && i != self.selected {
                self.toggle_mark(self.selected);
            }
            self.toggle_mark(i);
        } else if mods.shift {
            let (a, b) = (self.selected.min(i), self.selected.max(i));
            self.marked = self.entries[a..=b].iter().map(|e| e.path.clone()).collect();
        } else {
            self.marked.clear();
            self.selected = i;
            match self.last_click {
                Some((last, at)) if last == i && now - at < DOUBLE_CLICK => {
                    self.last_click = None;
                    self.activate(i);
                }
                _ => self.last_click = Some((i, now)),
            }
        }
    }

    fn apply(&mut self, ctx: &egui::Context, commands: Vec<Command>) {
        for cmd in commands {
            match cmd {
                Command::Open(i) => self.activate(i),
                Command::Copy => self.copy(ctx, false),
                Command::Cut => self.copy(ctx, true),
                Command::Paste => self.paste(ctx, None, None),
                Command::PasteInto(i) => {
                    let dest = self.entries.get(i).map(|e| e.path.clone());
                    self.paste(ctx, dest, None);
                }
                Command::ClearClipboard => set_clipboard(None),
                Command::CopyPath => {
                    let text = self.targets().iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n");
                    ctx.copy_text(text);
                }
                Command::SelectMode => {
                    self.selecting = true;
                    if self.marked.is_empty() && self.targets().len() == 1 && !self.touch {
                        self.toggle_mark(self.selected);
                    }
                }
                Command::SelectAll => {
                    self.selecting = true;
                    self.select_all();
                }
                Command::ClearSelection => self.clear_selection(),
                Command::ToggleView => self.toggle_view(),
                Command::ToggleHidden => self.toggle_hidden(),
            }
        }
    }
}

/// Draw the floating label that follows the pointer while files are dragged.
pub fn paint_drag_ghost(ctx: &egui::Context, c: &Theme) {
    let Some(drag) = DragAndDrop::payload::<ExplorerDrag>(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let copy = ctx.input(|i| i.modifiers.ctrl);
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("file-drag-ghost")));
    let text = match drag.paths.as_slice() {
        [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        many => count(many.len()),
    };
    let galley = p.layout_no_wrap(text, FontId::proportional(13.0), c.text_bright);
    let size = Vec2::new(galley.size().x + 40.0, 28.0);
    let r = Rect::from_min_size(pos + Vec2::new(14.0, 10.0), size);
    p.rect_filled(r, 8, Color32::from_rgba_unmultiplied(30, 32, 40, 235));
    p.rect_stroke(r, 8, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Inside);
    let icon = Rect::from_center_size(Pos2::new(r.left() + 15.0, r.center().y), Vec2::new(13.0, 13.0));
    if drag.paths.len() == 1 && drag.paths[0].is_dir() {
        icons::icon_folder(&p, icon, ACCENT);
    } else {
        icons::icon_file(&p, icon, c.text);
    }
    p.galley(Pos2::new(r.left() + 28.0, r.center().y - galley.size().y / 2.0), galley, c.text_bright);
    if copy {
        let badge = Pos2::new(r.right(), r.top());
        p.circle_filled(badge, 8.0, ACCENT);
        icons::icon_plus(&p, Rect::from_center_size(badge, Vec2::splat(12.0)), Color32::WHITE);
    }
}

struct ItemState {
    marked: bool,
    cursor: bool,
    hovered: bool,
    drop: bool,
    dimmed: bool,
    selecting: bool,
}

fn item_background(p: &Painter, r: Rect, st: &ItemState) {
    if st.drop {
        p.rect_filled(r, 8, ACCENT.gamma_multiply(0.22));
        p.rect_stroke(r, 8, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    } else if st.marked {
        p.rect_filled(r, 8, ACCENT.gamma_multiply(0.25));
    } else if st.cursor {
        p.rect_filled(r, 8, Color32::from_white_alpha(26));
    } else if st.hovered {
        p.rect_filled(r, 8, Color32::from_white_alpha(10));
    }
}

fn paint_row(p: &Painter, row: Rect, e: &Entry, st: &ItemState, c: &Theme, s: &Sizes) {
    item_background(p, row.shrink2(Vec2::new(0.0, 1.0)), st);
    let mut left = row.left() + 8.0;
    if st.selecting {
        let r = Rect::from_center_size(Pos2::new(left + 8.0, row.center().y), Vec2::splat(16.0));
        icons::icon_check(p, r, if st.marked { ACCENT } else { c.text_dim }, st.marked);
        left += 22.0;
    }
    let fade = if st.dimmed { 0.45 } else { 1.0 };
    let glyph_h = (s.row * 0.55).min(18.0);
    let glyph = Rect::from_center_size(Pos2::new(left + 8.0, row.center().y), Vec2::new(glyph_h * 1.15, glyph_h));
    if e.is_dir {
        icons::icon_folder(p, glyph, ACCENT.gamma_multiply(fade));
    } else {
        icons::icon_file(p, glyph, c.text_dim.gamma_multiply(fade));
    }
    let size = e.size.map(human_size).unwrap_or_default();
    let size_galley = p.layout_no_wrap(size, FontId::proportional(s.font - 1.5), c.text_dim);
    let size_x = row.right() - 10.0 - size_galley.size().x;
    let name_left = glyph.right() + 10.0;
    let name_color = if st.marked || st.cursor { c.text_bright } else { c.text };
    paint::faded_text(p, Pos2::new(name_left, row.center().y), &e.name, FontId::proportional(s.font), name_color.gamma_multiply(fade), size_x - 12.0 - name_left, 20.0);
    p.galley(Pos2::new(size_x, row.center().y - size_galley.size().y / 2.0), size_galley, c.text_dim);
}

fn paint_tile(p: &Painter, cell: Rect, e: &Entry, st: &ItemState, c: &Theme, s: &Sizes) {
    let tile = cell.shrink(4.0);
    item_background(p, tile, st);
    let fade = if st.dimmed { 0.45 } else { 1.0 };
    let icon_center = Pos2::new(tile.center().x, tile.top() + 8.0 + s.icon / 2.0);
    if e.is_dir {
        let r = Rect::from_center_size(icon_center, Vec2::new(s.icon * 1.12, s.icon * 0.86));
        icons::icon_folder(p, r, ACCENT.gamma_multiply(fade));
    } else {
        let r = Rect::from_center_size(icon_center, Vec2::new(s.icon, s.icon));
        icons::icon_file(p, r, c.text_dim.gamma_multiply(fade));
        // The extension, printed on the page.
        let ext = e.path.extension().map(|x| x.to_string_lossy().to_uppercase()).filter(|x| x.chars().count() <= 4);
        if let Some(ext) = ext {
            p.text(icon_center + Vec2::new(0.0, s.icon * 0.18), Align2::CENTER_CENTER, ext, paint::bold(s.icon * 0.22), c.text_dim.gamma_multiply(fade));
        }
    }
    if st.selecting {
        let r = Rect::from_center_size(tile.left_top() + Vec2::splat(13.0), Vec2::splat(16.0));
        icons::icon_check(p, r, if st.marked { ACCENT } else { c.text_dim }, st.marked);
    }
    // Name, centered under the icon (faded at the edge when too long).
    let font = FontId::proportional(s.font - 1.0);
    let name_color = (if st.marked || st.cursor { c.text_bright } else { c.text }).gamma_multiply(fade);
    let y = tile.bottom() - 6.0 - s.font * 0.6;
    let max_w = tile.width() - 10.0;
    let w = p.layout_no_wrap(e.name.clone(), font.clone(), name_color).size().x;
    let left = if w <= max_w { tile.center().x - w / 2.0 } else { tile.left() + 5.0 };
    paint::faded_text(p, Pos2::new(left, y), &e.name, font, name_color, max_w, 16.0);
}

/// A small square icon button in the path bar. `active` draws it highlighted (also used to
/// show it as a drop target).
#[allow(clippy::too_many_arguments)]
fn icon_button(
    ui: &Ui,
    painter: &Painter,
    r: Rect,
    id: Id,
    tip: &str,
    enabled: bool,
    active: bool,
    c: &Theme,
    draw: impl Fn(&Painter, Rect, Color32),
) -> bool {
    let resp = ui.interact(r, id, Sense::click());
    if active {
        painter.rect_filled(r, 6, ACCENT.gamma_multiply(0.25));
    } else if enabled && resp.hovered() {
        painter.rect_filled(r, 6, Color32::from_white_alpha(16));
    }
    let col = match (enabled, resp.hovered() || active) {
        (false, _) => c.text_dim.gamma_multiply(0.4),
        (true, true) => c.text_bright,
        (true, false) => c.text_dim,
    };
    draw(painter, Rect::from_center_size(r.center(), Vec2::splat(r.width().min(26.0) - 10.0)), col);
    enabled && resp.on_hover_text(tip).clicked_by(PointerButton::Primary)
}

fn count(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

// ---------------------------------------------------------------- file operations

/// Copy or move `sources` into the folder `dest`. Name clashes get a " (2)" style suffix
/// instead of overwriting anything. Returns how many items were copied or moved.
pub fn run_op(op: Op, sources: &[PathBuf], dest: &Path) -> Result<usize, String> {
    let dest_real = dest.canonicalize().map_err(|e| format!("{}: {e}", tilde(dest)))?;
    let mut done = 0;
    for src in sources {
        let Some(name) = src.file_name() else { continue };
        let fail = |e: std::io::Error| format!("{}: {e}", name.to_string_lossy());
        let src_meta = std::fs::symlink_metadata(src).map_err(fail)?;
        let src_real = src.canonicalize().map_err(fail)?;
        if src_meta.is_dir() && dest_real.starts_with(&src_real) {
            return Err(format!("Can't put “{}” inside itself", name.to_string_lossy()));
        }
        let same_folder = src.parent().and_then(|p| p.canonicalize().ok()).as_deref() == Some(dest_real.as_path());
        if op == Op::Move && same_folder {
            continue;
        }
        let target = unique_target(dest, name);
        match op {
            Op::Copy => copy_recursive(src, &target),
            Op::Move => move_path(src, &target),
        }
        .map_err(fail)?;
        done += 1;
    }
    Ok(done)
}

/// `dest/name`, or `dest/name (2).ext`, `(3)`, … if that's taken.
fn unique_target(dest: &Path, name: &OsStr) -> PathBuf {
    let first = dest.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let name = Path::new(name);
    let stem = name.file_stem().unwrap_or(name.as_os_str()).to_string_lossy();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| dest.join(format!("{stem} ({n}){ext}")))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .unwrap_or(first)
}

fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        std::os::unix::fs::symlink(std::fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dst, meta.permissions())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

fn move_path(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        // Across filesystems a rename isn't possible: copy, then remove the original.
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_recursive(src, dst)?;
            let meta = std::fs::symlink_metadata(src)?;
            if meta.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) }
        }
        Err(e) => Err(e),
    }
}

/// Absolute paths (or file:// URIs), one per line, that exist.
pub fn parse_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter_map(|l| {
            let p = l.strip_prefix("file://").map(percent_decode).unwrap_or_else(|| l.to_string());
            let p = PathBuf::from(p);
            (p.is_absolute() && std::fs::symlink_metadata(&p).is_ok()).then_some(p)
        })
        .collect()
}

fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Quote a path for a POSIX shell (only when needed).
pub fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./-+:,@%=~".contains(c);
    if !s.is_empty() && s.chars().all(safe) { s.into_owned() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// List `dir`: folders first, then files, each sorted case-insensitively by name.
fn read_entries(dir: &Path, show_hidden: bool) -> std::io::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|de| {
            let name = de.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                return None;
            }
            let path = de.path();
            // Follow symlinks so a link to a folder behaves like a folder.
            let meta = std::fs::metadata(&path).or_else(|_| de.metadata()).ok();
            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            let size = meta.filter(|m| !m.is_dir()).map(|m| m.len());
            Some(Entry { name, path, is_dir, size })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else if v < 10.0 { format!("{v:.1} {}", UNITS[unit]) } else { format!("{v:.0} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
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
    fn unreadable_dir_reports_error() {
        let b = FileExplorer::new(PathBuf::from("/definitely/not/a/real/dir"));
        assert!(b.error.is_some());
        assert!(b.entries.is_empty());
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 K");
        assert_eq!(human_size(50 * 1024 * 1024), "50 M");
    }

    #[test]
    fn copy_renames_on_clash_and_recurses() {
        let dir = fixture("copy");
        std::fs::write(dir.join("beta/inner.txt"), "in").unwrap();
        // Copying into the same folder makes a numbered duplicate.
        assert_eq!(run_op(Op::Copy, &[dir.join("Apple.md")], &dir), Ok(1));
        assert_eq!(std::fs::read_to_string(dir.join("Apple (2).md")).unwrap(), "apple");
        // Folders are copied with their contents.
        assert_eq!(run_op(Op::Copy, &[dir.join("beta")], &dir.join("Alpha")), Ok(1));
        assert_eq!(std::fs::read_to_string(dir.join("Alpha/beta/inner.txt")).unwrap(), "in");
        assert!(dir.join("beta/inner.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn move_skips_same_folder_and_refuses_self_nesting() {
        let dir = fixture("move");
        assert_eq!(run_op(Op::Move, &[dir.join("zeta.txt")], &dir), Ok(0));
        assert_eq!(run_op(Op::Move, &[dir.join("zeta.txt"), dir.join("Apple.md")], &dir.join("beta")), Ok(2));
        assert!(dir.join("beta/zeta.txt").exists() && !dir.join("zeta.txt").exists());
        std::fs::create_dir(dir.join("beta/sub")).unwrap();
        assert!(run_op(Op::Move, &[dir.join("beta")], &dir.join("beta/sub")).is_err());
        assert!(dir.join("beta").exists());
        std::fs::remove_dir_all(dir).unwrap();
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

    #[test]
    fn paths_and_quoting() {
        let dir = fixture("parse");
        let uri = format!("file://{}", dir.join("Apple.md").display()).replace(' ', "%20");
        assert_eq!(parse_paths(&format!("{uri}\nrelative/path\n/definitely/missing")), [dir.join("Apple.md")]);
        assert_eq!(percent_decode("/a%20b/c%2"), "/a b/c%2");
        assert_eq!(shell_quote(Path::new("/tmp/plain-file.txt")), "/tmp/plain-file.txt");
        assert_eq!(shell_quote(Path::new("/tmp/it's here")), r"'/tmp/it'\''s here'");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
