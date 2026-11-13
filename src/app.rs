//! Window chrome (title bar + tabs), split panes, keybindings and mouse handling.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use alacritty_terminal::grid::Scroll;
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use egui::{
    Color32, CursorIcon, Event, FontFamily, FontId, Id, Key,
    Painter, PointerButton, Pos2, Rect, ResizeDirection, Sense, Stroke, StrokeKind, Ui,
    Vec2, ViewportCommand,
};

use crate::chrome::{self, bold};
use crate::config::Config;
use crate::files::{self, FileBrowser};
use crate::fonts;
use crate::input::{self, MouseAction, MouseButton};
use crate::layout::{self, Axis, Direction, Node, PaneId};
use crate::render::{self, CellMetrics, PaintOptions, FONT_REGULAR};
use crate::search::{self, Link, LinkFinder, Search};
use crate::terminal::{GridSize, Terminal};

const TITLEBAR_HEIGHT: f32 = 46.0;
const HEADER_HEIGHT: f32 = 34.0;
const PANE_GAP: f32 = 8.0;
const PANE_RADIUS: u8 = 12;
const WINDOW_RADIUS: u8 = 14;
const RESIZE_BORDER: f32 = 6.0;
const BLINK_INTERVAL: Duration = Duration::from_millis(600);

struct Tab {
    /// Stable identity (survives reordering) for egui widget ids.
    uid: u64,
    root: Node,
    focused: PaneId,
    zoomed: bool,
    /// Pane-icon deck in the tab header: rotation (in cards) of the stack, plus the focus and
    /// pane count it was last drawn for, to tell focus moves (animate) from splits (snap).
    deck: Glide,
    deck_focus: PaneId,
    deck_len: usize,
}

impl Tab {
    fn new(uid: u64, pane: PaneId) -> Self {
        Self { uid, root: Node::Leaf(pane), focused: pane, zoomed: false, deck: Glide::new(0.28), deck_focus: pane, deck_len: 1 }
    }

    /// `(process, depth)` for every pane's card in the header deck, 0 being the top.
    /// Focus moves rotate the deck the short way round, flipping through the cards between.
    fn deck_cards(&mut self, ctx: &egui::Context, process_of: impl Fn(PaneId) -> String) -> Vec<(String, f32)> {
        let ids = self.root.panes();
        let n = ids.len();
        let Some(idx) = ids.iter().position(|&p| p == self.focused) else { return Vec::new() };
        let prev = ids.iter().position(|&p| p == self.deck_focus);
        match prev {
            Some(prev) if n == self.deck_len => {
                if prev != idx {
                    // Shortest signed step in (-n/2, n/2].
                    let mut k = (idx as i64 - prev as i64).rem_euclid(n as i64);
                    if k > n as i64 / 2 {
                        k -= n as i64;
                    }
                    let target = self.deck.to + k as f32;
                    self.deck.value(ctx, target);
                }
            }
            _ => self.deck.snap(ctx, idx as f32),
        }
        self.deck_focus = self.focused;
        self.deck_len = n;
        let target = self.deck.to;
        let d = self.deck.value(ctx, target);
        ids.iter()
            .enumerate()
            .map(|(j, id)| {
                (process_of(*id), (j as f32 - d).rem_euclid(n as f32))
            })
            .collect()
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Action {
    NewTab,
    NewWindow,
    ClosePane(PaneId),
    CloseTab(usize),
    Split(Axis),
    SplitPane(PaneId, Axis),
    Focus(Direction),
    FocusPane(PaneId),
    ResizeSplit(Direction),
    ToggleZoom,
    ToggleZoomPane(PaneId),
    Equalize,
    NextTab,
    PrevTab,
    GotoTab(usize),
    Copy,
    FontBigger,
    FontSmaller,
    FontReset,
    ReloadConfig,
    Quit,
    ScrollPageUp,
    ScrollPageDown,
    ScrollTop,
    ScrollBottom,
    ClearScreen,
    ToggleSearch,
    /// Open the Files tab, switch to it, or (from it) back to the previous tab.
    ToggleFiles,
    CloseFiles,
}

/// Keyboard shortcuts, following Ghostty's Linux defaults where they exist.
/// Each entry is (ctrl, shift, alt, key, action).
fn keybindings() -> Vec<(bool, bool, bool, Key, Action)> {
    use Action::*;
    let mut b = vec![
        (true, true, false, Key::T, NewTab),
        (true, true, false, Key::N, NewWindow),
        (true, true, false, Key::W, ClosePane(0)),
        (true, true, false, Key::Q, Quit),
        (true, true, false, Key::O, Split(Axis::Horizontal)),
        (true, true, false, Key::E, Split(Axis::Vertical)),
        (true, true, false, Key::Enter, ToggleZoom),
        (true, false, true, Key::ArrowLeft, Focus(Direction::Left)),
        (true, false, true, Key::ArrowRight, Focus(Direction::Right)),
        (true, false, true, Key::ArrowUp, Focus(Direction::Up)),
        (true, false, true, Key::ArrowDown, Focus(Direction::Down)),
        (true, true, true, Key::ArrowLeft, ResizeSplit(Direction::Left)),
        (true, true, true, Key::ArrowRight, ResizeSplit(Direction::Right)),
        (true, true, true, Key::ArrowUp, ResizeSplit(Direction::Up)),
        (true, true, true, Key::ArrowDown, ResizeSplit(Direction::Down)),
        (true, true, true, Key::Equals, Equalize),
        (true, true, true, Key::Plus, Equalize),
        (true, false, false, Key::Tab, NextTab),
        (true, true, false, Key::Tab, PrevTab),
        (true, false, false, Key::PageDown, NextTab),
        (true, false, false, Key::PageUp, PrevTab),
        (true, true, false, Key::ArrowRight, NextTab),
        (true, true, false, Key::ArrowLeft, PrevTab),
        (true, false, false, Key::Equals, FontBigger),
        (true, false, false, Key::Plus, FontBigger),
        (true, true, false, Key::Plus, FontBigger),
        (true, true, false, Key::Equals, FontBigger),
        (true, false, false, Key::Minus, FontSmaller),
        (true, false, false, Key::Num0, FontReset),
        (true, true, false, Key::Comma, ReloadConfig),
        (true, true, false, Key::K, ClearScreen),
        (true, true, false, Key::F, ToggleSearch),
        (false, true, false, Key::PageUp, ScrollPageUp),
        (false, true, false, Key::PageDown, ScrollPageDown),
        (false, true, false, Key::Home, ScrollTop),
        (false, true, false, Key::End, ScrollBottom),
    ];
    let digits = [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9];
    for (i, k) in digits.into_iter().enumerate() {
        b.push((false, false, true, k, GotoTab(i)));
    }
    b
}

#[derive(Default)]
struct MouseState {
    /// Pane receiving drag/release events, with its terminal rect.
    capture: Option<(PaneId, Rect)>,
    /// Button held while reporting to the program, or `None` if we're selecting text.
    reporting: Option<MouseButton>,
    last_cell: (usize, usize),
    moved: bool,
    last_click: Option<(Instant, Pos2, u8)>,
    click_count: u8,
}

pub struct App {
    cfg: Config,
    panes: HashMap<PaneId, Terminal>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: PaneId,
    /// Per-pane font size overrides (zoom is per pane); panes not listed use the config size.
    pane_fonts: HashMap<PaneId, f32>,
    /// Cell metrics for every font size in use this frame, keyed by `f32::to_bits`.
    metrics: HashMap<u32, CellMetrics>,
    mouse: MouseState,
    pending: Vec<Action>,
    /// Terminal (content) rects from the current frame, used for mouse hit-testing.
    term_rects: Vec<(PaneId, Rect)>,
    /// Card rects (including header), used for directional focus.
    card_rects: Vec<(PaneId, Rect)>,
    hostname: String,
    last_proc_poll: Instant,
    window_title: String,
    bindings: Vec<(bool, bool, bool, Key, Action)>,
    /// Area available for panes in the current window (used to size new terminals).
    content_area: Rect,
    started: bool,
    search: Option<Search>,
    links: LinkFinder,
    /// Link under the pointer while Ctrl is held.
    hover_link: Option<(PaneId, Link)>,
    /// Animated position of the active-tab highlight, in tab slots.
    pill_glide: Glide,
    /// File-browser panes (the Files tab). They share the pane id space and split tree with
    /// terminals.
    browsers: HashMap<PaneId, FileBrowser>,
    /// The Files tab. It lives outside `tabs`: the title-bar directory label is its tab.
    files: Option<Tab>,
    /// Whether the Files tab is showing (`active` keeps the last terminal tab).
    files_active: bool,
    /// Last known pointer position (touch releases report the pointer as gone).
    last_pointer: Option<Pos2>,
}

/// A value that glides toward its target with an ease-out curve (the active-tab pill, the
/// pane-icon decks).
struct Glide {
    from: f32,
    to: f32,
    start: f64,
    init: bool,
    duration: f64,
}

impl Glide {
    fn new(duration: f64) -> Self {
        Self { from: 0.0, to: 0.0, start: 0.0, init: false, duration }
    }

    /// Jump straight to `v` without animating.
    fn snap(&mut self, ctx: &egui::Context, v: f32) {
        let now = ctx.input(|i| i.time);
        *self = Self { from: v, to: v, start: now, init: true, duration: self.duration };
    }

    /// Current position; retargets (from wherever the value is now) when `target` changes.
    fn value(&mut self, ctx: &egui::Context, target: f32) -> f32 {
        let now = ctx.input(|i| i.time);
        if !self.init {
            self.snap(ctx, target);
        } else if self.to != target {
            self.from = self.current(now);
            self.to = target;
            self.start = now;
        }
        let v = self.current(now);
        if v != self.to {
            ctx.request_repaint();
        }
        v
    }

    fn current(&self, now: f64) -> f32 {
        let t = ((now - self.start) / self.duration).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            return self.to;
        }
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        ctx.set_visuals(egui::Visuals::dark());
        fonts::install(ctx, &cfg);

        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "localhost".into());

        Self {
            cfg,
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            pane_fonts: HashMap::new(),
            metrics: HashMap::new(),
            mouse: MouseState::default(),
            pending: Vec::new(),
            term_rects: Vec::new(),
            card_rects: Vec::new(),
            hostname,
            last_proc_poll: Instant::now(),
            window_title: String::new(),
            bindings: keybindings(),
            content_area: Rect::NOTHING,
            started: false,
            search: None,
            links: LinkFinder::new(),
            hover_link: None,
            pill_glide: Glide::new(0.2),
            browsers: HashMap::new(),
            files: None,
            files_active: false,
            last_pointer: None,
        }
    }

    // ---------------------------------------------------------------- panes & tabs

    /// The terminal content rect inside a pane card (below the header, inside the padding).
    fn term_rect_for(&self, card: Rect, ppp: f32) -> Rect {
        let pad = self.cfg.window_padding;
        let top = if self.cfg.pane_headers { card.top() + HEADER_HEIGHT } else { card.top() + pad * 0.5 };
        let r = Rect::from_min_max(
            Pos2::new(card.left() + pad, top + 2.0),
            Pos2::new(card.right() - pad, card.bottom() - pad * 0.6),
        );
        // Snap the origin to the pixel grid so glyphs stay crisp.
        Rect::from_min_size(Pos2::new((r.left() * ppp).round() / ppp, (r.top() * ppp).round() / ppp), r.size())
    }

    fn grid_for(&self, card: Rect, ppp: f32, m: CellMetrics) -> GridSize {
        if !card.is_positive() {
            return GridSize { cols: 80, lines: 24 };
        }
        let r = self.term_rect_for(card, ppp);
        GridSize {
            cols: ((r.width() / m.width).floor() as usize).max(2),
            lines: ((r.height() / m.height).floor() as usize).max(1),
        }
    }

    /// Spawn a shell sized for `card`, so its first output is already laid out correctly.
    /// `font_size` is the new pane's own zoom level (`None` = config default).
    fn spawn_pane(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>, card: Rect, font_size: Option<f32>) -> Option<PaneId> {
        let id = self.next_id;
        self.next_id += 1;
        let m = self.metrics_for(ctx, font_size.unwrap_or(self.cfg.font_size));
        let size = self.grid_for(card, ctx.pixels_per_point(), m);
        let cell_px = cell_px(ctx, m);
        match Terminal::spawn(id, &self.cfg, cwd, size, cell_px, ctx) {
            Ok(t) => {
                self.panes.insert(id, t);
                if let Some(fs) = font_size {
                    self.pane_fonts.insert(id, fs);
                }
                Some(id)
            }
            Err(e) => {
                eprintln!("nex: failed to spawn shell: {e}");
                None
            }
        }
    }

    fn focused_pane(&self) -> Option<PaneId> {
        self.cur_tab().map(|t| t.focused)
    }

    /// The tab being shown: the Files tab or the active terminal tab.
    fn cur_tab(&self) -> Option<&Tab> {
        if self.files_active { self.files.as_ref() } else { self.tabs.get(self.active) }
    }

    fn cur_tab_mut(&mut self) -> Option<&mut Tab> {
        if self.files_active { self.files.as_mut() } else { self.tabs.get_mut(self.active) }
    }

    /// The tab (Files or terminal) containing `pane`.
    fn tab_with(&mut self, pane: PaneId) -> Option<&mut Tab> {
        self.files.iter_mut().chain(self.tabs.iter_mut()).find(|t| t.root.panes().contains(&pane))
    }

    fn show_tab(&mut self, index: usize) {
        self.active = index;
        self.files_active = false;
    }

    fn focused_cwd(&self) -> Option<PathBuf> {
        self.focused_pane().and_then(|id| self.pane_cwd(id))
    }

    /// Working directory of a pane: the shell's cwd, or the folder a browser is showing.
    fn pane_cwd(&self, id: PaneId) -> Option<PathBuf> {
        match self.browsers.get(&id) {
            Some(b) => Some(b.dir.clone()),
            None => self.panes.get(&id).and_then(|t| t.cwd.clone()),
        }
    }

    /// `(title, process)` for a pane's tab and header.
    fn pane_label(&self, id: PaneId) -> (String, String) {
        if let Some(b) = self.browsers.get(&id) {
            return (b.title(), chrome::FILES_PROCESS.into());
        }
        self.panes.get(&id).map(|t| (t.title(), t.process.clone())).unwrap_or_default()
    }

    /// Show the Files tab (opening it at the focused directory the first time), or hide it
    /// again, back to the terminal tab that was showing.
    fn toggle_files(&mut self) {
        if self.files_active {
            if !self.tabs.is_empty() {
                self.files_active = false;
            }
            return;
        }
        if self.files.is_none() {
            let dir = self
                .focused_cwd()
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/"));
            let id = self.next_id;
            self.next_id += 1;
            self.browsers.insert(id, FileBrowser::new(dir));
            self.files = Some(Tab::new(id, id));
        }
        self.files_active = true;
    }

    fn close_files(&mut self) {
        if let Some(tab) = self.files.take() {
            for id in tab.root.panes() {
                self.panes.remove(&id);
                self.pane_fonts.remove(&id);
                self.browsers.remove(&id);
            }
        }
        self.files_active = false;
    }

    fn new_tab(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>) {
        if let Some(id) = self.spawn_pane(ctx, cwd, self.content_area, None) {
            self.tabs.push(Tab::new(id, id));
            self.show_tab(self.tabs.len() - 1);
        }
    }

    fn split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let cwd = self.pane_cwd(target);
        let card = self
            .card_rects
            .iter()
            .find(|(id, _)| *id == target)
            .map(|&(_, r)| match axis {
                Axis::Horizontal => Rect::from_min_max(Pos2::new(r.center().x + PANE_GAP / 2.0, r.top()), r.max),
                Axis::Vertical => Rect::from_min_max(Pos2::new(r.left(), r.center().y + PANE_GAP / 2.0), r.max),
            })
            .unwrap_or(Rect::NOTHING);
        // The new pane starts at the zoom level of the pane it was split from.
        let font_size = self.pane_fonts.get(&target).copied();
        if let Some(new) = self.spawn_pane(ctx, cwd, card, font_size) {
            if let Some(tab) = self.tab_with(target) {
                tab.root.split(target, axis, new);
                tab.focused = new;
                tab.zoomed = false;
            }
        }
    }

    fn close_pane(&mut self, id: PaneId) {
        self.panes.remove(&id);
        self.pane_fonts.remove(&id);
        self.browsers.remove(&id);
        if let Some(tab) = self.files.as_mut().filter(|t| t.root.panes().contains(&id)) {
            if tab.root.remove(id) {
                if tab.focused == id {
                    tab.focused = tab.root.panes()[0];
                }
                tab.zoomed = false;
            } else {
                self.close_files();
            }
            return;
        }
        let Some(ti) = self.tabs.iter().position(|t| t.root.panes().contains(&id)) else { return };
        let tab = &mut self.tabs[ti];
        if tab.root.remove(id) {
            if tab.focused == id {
                tab.focused = tab.root.panes()[0];
            }
            tab.zoomed = false;
        } else {
            self.close_tab(ti);
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        for id in tab.root.panes() {
            self.panes.remove(&id);
            self.pane_fonts.remove(&id);
            self.browsers.remove(&id);
        }
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }

    /// Font size of one pane (its own zoom level, or the config default).
    fn pane_font(&self, id: PaneId) -> f32 {
        self.pane_fonts.get(&id).copied().unwrap_or(self.cfg.font_size)
    }

    /// Zoom only pane `id`; `None` resets it to the config size.
    fn set_pane_font(&mut self, ctx: &egui::Context, id: PaneId, size: Option<f32>) {
        match size.map(|s| s.clamp(6.0, 72.0)) {
            Some(s) if s != self.cfg.font_size => {
                self.pane_fonts.insert(id, s);
            }
            _ => {
                self.pane_fonts.remove(&id);
            }
        }
        self.update_metrics(ctx);
    }

    fn compute_metrics(&self, ctx: &egui::Context, font_size: f32) -> CellMetrics {
        let fid = FontId::new(font_size, FontFamily::Name(FONT_REGULAR.into()));
        let (w, h) = ctx.fonts_mut(|f| (f.glyph_width(&fid, 'M'), f.row_height(&fid)));
        let ppp = ctx.pixels_per_point();
        let height = (h * (1.0 + self.cfg.cell_height_adjust) * ppp).round() / ppp;
        let width = ((w * ppp).round().max(1.0)) / ppp;
        CellMetrics { width, advance: w, height, baseline_offset: (((height - h) / 2.0) * ppp).round() / ppp, font_size, ppp }
    }

    /// Metrics for `font_size`, from this frame's cache when available.
    fn metrics_for(&self, ctx: &egui::Context, font_size: f32) -> CellMetrics {
        match self.metrics.get(&font_size.to_bits()) {
            Some(m) => *m,
            None => self.compute_metrics(ctx, font_size),
        }
    }

    /// Cell metrics of pane `id` (filled by `update_metrics` each frame).
    fn pane_metrics(&self, id: PaneId) -> CellMetrics {
        let size = self.pane_font(id);
        self.metrics
            .get(&size.to_bits())
            .or_else(|| self.metrics.get(&self.cfg.font_size.to_bits()))
            .copied()
            .unwrap_or(CellMetrics { width: 8.0, advance: 8.0, height: 17.0, baseline_offset: 0.0, font_size: size, ppp: 1.0 })
    }

    /// Recompute metrics for the default size and every per-pane size in use.
    fn update_metrics(&mut self, ctx: &egui::Context) {
        let mut sizes: Vec<f32> = self.pane_fonts.values().copied().collect();
        sizes.push(self.cfg.font_size);
        self.metrics.clear();
        for s in sizes {
            if !self.metrics.contains_key(&s.to_bits()) {
                let m = self.compute_metrics(ctx, s);
                self.metrics.insert(s.to_bits(), m);
            }
        }
    }

    // ---------------------------------------------------------------- actions

    fn apply(&mut self, ctx: &egui::Context, action: Action) {
        let focused = self.focused_pane();
        match action {
            Action::NewTab => {
                let cwd = self.focused_cwd();
                self.new_tab(ctx, cwd);
            }
            Action::NewWindow => {
                if let Ok(exe) = std::env::current_exe() {
                    let mut cmd = std::process::Command::new(exe);
                    if let Some(cwd) = self.focused_cwd() {
                        cmd.current_dir(cwd);
                    }
                    let _ = cmd.spawn();
                }
            }
            Action::ClosePane(0) => {
                if let Some(id) = focused {
                    self.close_pane(id)
                }
            }
            Action::ClosePane(id) => self.close_pane(id),
            Action::CloseTab(i) => self.close_tab(i),
            Action::Split(axis) => {
                if let Some(id) = focused {
                    self.split(ctx, id, axis)
                }
            }
            Action::SplitPane(id, axis) => self.split(ctx, id, axis),
            Action::Focus(dir) => {
                if let Some(id) = focused {
                    if let Some(next) = layout::neighbor(&self.card_rects, id, dir) {
                        if let Some(tab) = self.cur_tab_mut() {
                            tab.focused = next;
                        }
                    }
                }
            }
            Action::FocusPane(id) => {
                if let Some(tab) = self.tab_with(id) {
                    tab.focused = id;
                }
            }
            Action::ResizeSplit(dir) => {
                if let (Some(id), Some(tab)) = (focused, self.cur_tab_mut()) {
                    tab.root.resize(id, dir, 0.05);
                }
            }
            Action::ToggleZoom => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.zoomed = !tab.zoomed && tab.root.panes().len() > 1;
                }
            }
            Action::ToggleZoomPane(id) => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.focused = id;
                    tab.zoomed = !tab.zoomed && tab.root.panes().len() > 1;
                }
            }
            Action::Equalize => {
                if let Some(tab) = self.cur_tab_mut() {
                    tab.root.equalize();
                }
            }
            // The Files tab, when open, sits before the first terminal tab in the cycle.
            Action::NextTab => {
                let n = self.tabs.len();
                if self.files_active {
                    if n > 0 {
                        self.show_tab(0);
                    }
                } else if self.active + 1 == n && self.files.is_some() {
                    self.files_active = true;
                } else if n > 0 {
                    self.show_tab((self.active + 1) % n);
                }
            }
            Action::PrevTab => {
                let n = self.tabs.len();
                if self.files_active {
                    if n > 0 {
                        self.show_tab(n - 1);
                    }
                } else if self.active == 0 && self.files.is_some() {
                    self.files_active = true;
                } else if n > 0 {
                    self.show_tab((self.active + n - 1) % n);
                }
            }
            Action::GotoTab(i) => {
                // Like Ghostty, alt+9 is "last tab".
                if i == 8 && !self.tabs.is_empty() {
                    self.show_tab(self.tabs.len() - 1);
                } else if i < self.tabs.len() {
                    self.show_tab(i);
                }
            }
            Action::Copy => {
                if let Some(text) = focused.and_then(|id| self.panes.get(&id)).and_then(|t| t.selection_text()) {
                    ctx.copy_text(text);
                }
            }
            Action::FontBigger | Action::FontSmaller | Action::FontReset => {
                if let Some(id) = focused {
                    let size = self.pane_font(id);
                    let size = match action {
                        Action::FontBigger => Some(size + 1.0),
                        Action::FontSmaller => Some(size - 1.0),
                        _ => None,
                    };
                    self.set_pane_font(ctx, id, size);
                }
            }
            Action::ReloadConfig => {
                self.cfg = Config::load();
                fonts::install(ctx, &self.cfg);
                self.pane_fonts.clear();
                self.update_metrics(ctx);
            }
            Action::Quit => ctx.send_viewport_cmd(ViewportCommand::Close),
            Action::ScrollPageUp | Action::ScrollPageDown | Action::ScrollTop | Action::ScrollBottom => {
                if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                    t.scroll_to(match action {
                        Action::ScrollPageUp => Scroll::PageUp,
                        Action::ScrollPageDown => Scroll::PageDown,
                        Action::ScrollTop => Scroll::Top,
                        _ => Scroll::Bottom,
                    });
                }
            }
            Action::ToggleFiles => self.toggle_files(),
            Action::CloseFiles => self.close_files(),
            Action::ToggleSearch => {
                // Nothing to search in a file browser.
                let focused = focused.filter(|id| !self.browsers.contains_key(id));
                self.search = match (self.search.take(), focused) {
                    // Pressing it again in the same pane closes search; in another pane, moves it.
                    (Some(s), Some(id)) if s.pane != id => Some(Search::new(id)),
                    (Some(_), _) => None,
                    (None, Some(id)) => Some(Search::new(id)),
                    (None, None) => None,
                };
            }
            Action::ClearScreen => {
                // Ask the shell to redraw on a clean screen: clear + form feed (Ctrl+L).
                if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                    t.input(b"\x0c".to_vec());
                }
            }
        }
    }

    // ---------------------------------------------------------------- keyboard

    fn handle_keyboard(&mut self, ctx: &egui::Context) {
        let (events, mods) = ctx.input(|i| (i.events.clone(), i.modifiers));
        let focused = self.focused_pane();
        for event in events {
            match event {
                Event::Key { key, pressed: true, modifiers, .. } => {
                    let binding = self.bindings.iter().find(|(c, s, a, k, _)| {
                        *k == key && *c == modifiers.ctrl && *s == modifiers.shift && *a == modifiers.alt
                    });
                    if let Some(&(.., action)) = binding {
                        self.pending.push(action);
                        continue;
                    }
                    if let Some(b) = focused.and_then(|id| self.browsers.get_mut(&id)) {
                        b.key(key, modifiers);
                        continue;
                    }
                    if let Some(s) = self.search.as_mut() {
                        match key {
                            Key::Escape => self.search = None,
                            Key::Backspace => {
                                s.query.pop();
                            }
                            Key::Enter | Key::F3 => {
                                // Enter = older match (up), Shift+Enter = newer.
                                if let Some(p) = s.step(!modifiers.shift) {
                                    if let Some(t) = self.panes.get(&s.pane) {
                                        t.term.lock().scroll_to_point(p);
                                    }
                                }
                            }
                            Key::ArrowUp | Key::ArrowDown => {
                                if let Some(p) = s.step(key == Key::ArrowUp) {
                                    if let Some(t) = self.panes.get(&s.pane) {
                                        t.term.lock().scroll_to_point(p);
                                    }
                                }
                            }
                            _ => {}
                        }
                        continue;
                    }
                    let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) else { continue };
                    let mode = *t.term.lock().mode();
                    if let Some(bytes) = input::encode_key(key, modifiers, mode) {
                        t.input(bytes);
                    }
                }
                Event::Text(text) => {
                    if mods.ctrl || focused.is_some_and(|id| self.browsers.contains_key(&id)) {
                        continue;
                    }
                    if let Some(s) = self.search.as_mut() {
                        s.query.push_str(&text);
                        continue;
                    }
                    let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) else { continue };
                    let mut bytes = Vec::with_capacity(text.len() + 1);
                    if mods.alt {
                        bytes.push(0x1b);
                    }
                    bytes.extend_from_slice(text.as_bytes());
                    t.input(bytes);
                }
                Event::Ime(egui::ImeEvent::Commit(text)) => {
                    if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                        t.input(text.into_bytes());
                    }
                }
                // egui-winit turns Ctrl+C / Ctrl+X / Ctrl+V into clipboard events. In a terminal
                // only the Ctrl+Shift variants are clipboard operations; plain Ctrl sends the
                // control character to the program (Ctrl+C → SIGINT).
                // In a file browser these copy / cut / paste files.
                Event::Copy | Event::Cut | Event::Paste(_) if focused.is_some_and(|id| self.browsers.contains_key(&id)) => {
                    let Some(b) = focused.and_then(|id| self.browsers.get_mut(&id)) else { continue };
                    match event {
                        Event::Paste(text) => b.paste(ctx, None, Some(&text)),
                        e => b.copy(ctx, e == Event::Cut),
                    }
                }
                Event::Copy => {
                    if mods.shift || !mods.ctrl {
                        self.pending.push(Action::Copy);
                    } else if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                        t.input(vec![0x03]);
                    }
                }
                Event::Cut => {
                    if let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) {
                        t.input(vec![0x18]);
                    }
                }
                Event::Paste(text) => {
                    if let Some(s) = self.search.as_mut() {
                        s.query.push_str(text.lines().next().unwrap_or_default());
                        continue;
                    }
                    let Some(t) = focused.and_then(|id| self.panes.get_mut(&id)) else { continue };
                    if mods.shift {
                        t.paste(&text);
                    } else {
                        t.input(vec![0x16]);
                    }
                }
                _ => {}
            }
        }
    }

    // ---------------------------------------------------------------- mouse

    fn handle_mouse(&mut self, ctx: &egui::Context) {
        let (events, hover) = ctx.input(|i| (i.events.clone(), i.pointer.hover_pos()));
        // Each pane has its own zoom level, so cell geometry is looked up per pane.
        let pane_m: HashMap<PaneId, CellMetrics> = self.term_rects.iter().map(|&(id, _)| (id, self.pane_metrics(id))).collect();
        let fallback = self.pane_metrics(0);
        let m = |id: PaneId| pane_m.get(&id).copied().unwrap_or(fallback);

        for event in events {
            match event {
                Event::PointerButton { pos, button, pressed: true, modifiers } => {
                    let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) else {
                        continue;
                    };
                    if modifiers.ctrl && button == PointerButton::Primary {
                        if let Some((link_pane, link)) = &self.hover_link {
                            if *link_pane == id {
                                search::open(&link.uri);
                                continue;
                            }
                        }
                    }
                    self.pending.push(Action::FocusPane(id));
                    let Some(t) = self.panes.get_mut(&id) else { continue };
                    let mode = *t.term.lock().mode();
                    let (point, side, col, line) = render::point_at(pos, rect, m(id), t);
                    self.mouse.capture = Some((id, rect));
                    self.mouse.last_cell = (col, line);
                    self.mouse.moved = false;

                    let btn = match button {
                        PointerButton::Primary => MouseButton::Left,
                        PointerButton::Middle => MouseButton::Middle,
                        PointerButton::Secondary => MouseButton::Right,
                        _ => continue,
                    };
                    if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
                        self.mouse.reporting = Some(btn);
                        if let Some(b) = input::encode_mouse(MouseAction::Press, btn, col, line, modifiers, mode) {
                            t.write(b);
                        }
                        continue;
                    }
                    self.mouse.reporting = None;
                    if btn != MouseButton::Left {
                        self.mouse.capture = None;
                        continue;
                    }
                    // Multi-click detection: double = word, triple = line.
                    let now = Instant::now();
                    let count = match self.mouse.last_click {
                        Some((at, p, n)) if now - at < Duration::from_millis(400) && p.distance(pos) < 6.0 => n % 3 + 1,
                        _ => 1,
                    };
                    self.mouse.last_click = Some((now, pos, count));
                    self.mouse.click_count = count;
                    let ty = match count {
                        1 => SelectionType::Simple,
                        2 => SelectionType::Semantic,
                        _ => SelectionType::Lines,
                    };
                    let mut term = t.term.lock();
                    term.selection = Some(Selection::new(ty, point, side));
                    if modifiers.alt && count == 1 {
                        term.selection = Some(Selection::new(SelectionType::Block, point, side));
                    }
                }
                Event::PointerMoved(pos) => {
                    if let Some((id, rect)) = self.mouse.capture {
                        let Some(t) = self.panes.get_mut(&id) else { continue };
                        let (point, side, col, line) = render::point_at(pos, rect, m(id), t);
                        let cell_changed = (col, line) != self.mouse.last_cell;
                        self.mouse.last_cell = (col, line);
                        if let Some(btn) = self.mouse.reporting {
                            let mode = *t.term.lock().mode();
                            if cell_changed && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                                let mods = ctx.input(|i| i.modifiers);
                                if let Some(b) = input::encode_mouse(MouseAction::Motion(Some(btn)), btn, col, line, mods, mode) {
                                    t.write(b);
                                }
                            }
                        } else {
                            if cell_changed {
                                self.mouse.moved = true;
                            }
                            // Auto-scroll when dragging past the top/bottom edge.
                            if pos.y < rect.top() {
                                t.scroll(1);
                            } else if pos.y > rect.bottom() {
                                t.scroll(-1);
                            }
                            if let Some(sel) = t.term.lock().selection.as_mut() {
                                sel.update(point, side);
                            }
                        }
                    } else if let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) {
                        // Plain motion reporting (no button held), e.g. for hover effects in TUIs.
                        let Some(t) = self.panes.get_mut(&id) else { continue };
                        let mode = *t.term.lock().mode();
                        if mode.contains(TermMode::MOUSE_MOTION) {
                            let (_, _, col, line) = render::point_at(pos, rect, m(id), t);
                            if (col, line) != self.mouse.last_cell {
                                self.mouse.last_cell = (col, line);
                                let mods = ctx.input(|i| i.modifiers);
                                if let Some(b) = input::encode_mouse(MouseAction::Motion(None), MouseButton::Left, col, line, mods, mode) {
                                    t.write(b);
                                }
                            }
                        }
                    }
                }
                Event::PointerButton { pos, button, pressed: false, modifiers } => {
                    let Some((id, rect)) = self.mouse.capture.take() else { continue };
                    let Some(t) = self.panes.get_mut(&id) else { continue };
                    if let Some(btn) = self.mouse.reporting.take() {
                        let mode = *t.term.lock().mode();
                        let (_, _, col, line) = render::point_at(pos, rect, m(id), t);
                        if let Some(b) = input::encode_mouse(MouseAction::Release, btn, col, line, modifiers, mode) {
                            t.write(b);
                        }
                    } else if button == PointerButton::Primary {
                        if self.mouse.click_count == 1 && !self.mouse.moved {
                            t.term.lock().selection = None;
                        } else if self.cfg.copy_on_select {
                            if let Some(text) = t.selection_text() {
                                ctx.copy_text(text);
                            }
                        }
                    }
                }
                Event::MouseWheel { unit, delta, modifiers } => {
                    if modifiers.ctrl {
                        continue;
                    }
                    let Some(pos) = hover else { continue };
                    let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) else {
                        continue;
                    };
                    let Some(t) = self.panes.get_mut(&id) else { continue };
                    let lines_f = match unit {
                        egui::MouseWheelUnit::Point => delta.y / m(id).height,
                        egui::MouseWheelUnit::Line => delta.y * 3.0,
                        egui::MouseWheelUnit::Page => delta.y * t.size().lines as f32,
                    };
                    t.scroll_accum += lines_f;
                    let lines = t.scroll_accum.trunc() as i32;
                    t.scroll_accum -= lines as f32;
                    if lines == 0 {
                        continue;
                    }
                    let mode = *t.term.lock().mode();
                    if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
                        let (_, _, col, line) = render::point_at(pos, rect, m(id), t);
                        let btn = if lines > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
                        for _ in 0..lines.unsigned_abs() {
                            if let Some(b) = input::encode_mouse(MouseAction::Press, btn, col, line, modifiers, mode) {
                                t.write(b);
                            }
                        }
                    } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !modifiers.shift {
                        // Programs on the alternate screen (less, man) get arrow keys instead.
                        let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                            (true, true) => b"\x1bOA",
                            (true, false) => b"\x1b[A",
                            (false, true) => b"\x1bOB",
                            (false, false) => b"\x1b[B",
                        };
                        t.write(seq.repeat(lines.unsigned_abs() as usize));
                    } else {
                        t.scroll(lines);
                    }
                }
                _ => {}
            }
        }

        // Ctrl+hover: find a link under the pointer.
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        let previous = self.hover_link.as_ref().map(|(id, l)| (*id, l.range.clone()));
        self.hover_link = None;
        if let (true, Some(pos), None) = (ctrl, hover, self.mouse.capture) {
            if let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) {
                if let Some(t) = self.panes.get(&id) {
                    let (point, ..) = render::point_at(pos, rect, m(id), t);
                    let term = t.term.lock();
                    self.hover_link = self.links.link_at(&term, point).map(|l| (id, l));
                }
            }
        }
        if self.hover_link.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if previous != self.hover_link.as_ref().map(|(id, l)| (*id, l.range.clone())) {
            ctx.request_repaint();
        }

        // Pointer shape over terminals.
        if let (Some(pos), None) = (hover, &self.hover_link) {
            if let Some((id, _)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) {
                let mouse_mode = self
                    .panes
                    .get(id)
                    .is_some_and(|t| t.term.lock().mode().intersects(TermMode::MOUSE_MODE));
                if !mouse_mode {
                    ctx.set_cursor_icon(CursorIcon::Text);
                }
            }
        }
    }

    // ---------------------------------------------------------------- drawing

    fn colors(&self) -> Theme {
        let bg = self.cfg.background;
        let alpha = self.cfg.background_opacity;
        Theme {
            window: darken(bg, 0.78).gamma_multiply(alpha.max(0.6)),
            card: lighten(bg, 0.03).gamma_multiply(alpha),
            text: self.cfg.foreground,
            text_bright: Color32::from_rgb(0xf5, 0xf2, 0xf6),
            text_dim: self.cfg.foreground.gamma_multiply(0.6),
        }
    }

    fn titlebar(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();

        // Background drag area (registered first so tabs & buttons placed later win hit tests).
        let drag = ui.interact(rect, Id::new("titlebar"), Sense::click_and_drag());
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        let cy = rect.center().y;
        let mut x = rect.left() + 18.0;

        // Window controls (close / minimize / maximize), macOS style.
        let lights = [
            (Color32::from_rgb(0xff, 0x5f, 0x57), "close"),
            (Color32::from_rgb(0xfe, 0xbc, 0x2e), "min"),
            (Color32::from_rgb(0x28, 0xc8, 0x40), "max"),
        ];
        let group = Rect::from_min_size(Pos2::new(x - 8.0, cy - 10.0), Vec2::new(60.0, 20.0));
        let group_hovered = ui.rect_contains_pointer(group);
        for (color, what) in lights {
            let center = Pos2::new(x + 6.0, cy);
            let r = Rect::from_center_size(center, Vec2::splat(14.0));
            let resp = ui.interact(r, Id::new(("light", what)), Sense::click());
            painter.circle_filled(center, 6.0, color);
            if group_hovered {
                let s = Stroke::new(1.2, Color32::from_black_alpha(150));
                match what {
                    "close" => {
                        painter.line_segment([center + Vec2::new(-2.5, -2.5), center + Vec2::new(2.5, 2.5)], s);
                        painter.line_segment([center + Vec2::new(-2.5, 2.5), center + Vec2::new(2.5, -2.5)], s);
                    }
                    "min" => {
                        painter.line_segment([center + Vec2::new(-3.0, 0.0), center + Vec2::new(3.0, 0.0)], s);
                    }
                    _ => {
                        painter.line_segment([center + Vec2::new(-3.0, 0.0), center + Vec2::new(3.0, 0.0)], s);
                        painter.line_segment([center + Vec2::new(0.0, -3.0), center + Vec2::new(0.0, 3.0)], s);
                    }
                }
            }
            if resp.clicked_by(PointerButton::Primary) {
                match what {
                    "close" => ctx.send_viewport_cmd(ViewportCommand::Close),
                    "min" => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                    _ => {
                        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    }
                }
            }
            x += 20.0;
        }
        x += 18.0;

        // Workspace label: current directory name + hostname. Clicking it toggles the Files tab.
        let project = self
            .focused_cwd()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".into());
        // The label *is* the Files tab: it gets the glass pill while Files is showing.
        let label_w = 96.0;
        let label_r = Rect::from_min_max(Pos2::new(x - 7.0, cy - 15.0), Pos2::new(x + 26.0 + label_w + 4.0, cy + 15.0));
        let label = ui.interact(label_r, Id::new("workspace-label"), Sense::click());
        let files_active = self.files_active;
        if files_active {
            chrome::glass_pill(&painter, label_r, 8);
        } else if label.hovered() {
            painter.rect_filled(label_r, 8, Color32::from_white_alpha(14));
        }
        let label_tip = if files_active { "Back to terminal  (middle-click closes)" } else { "Files" };
        let label = label.on_hover_text(label_tip);
        if label.clicked_by(PointerButton::Middle) {
            self.pending.push(Action::CloseFiles);
        } else if label.clicked_by(PointerButton::Primary) {
            self.pending.push(Action::ToggleFiles);
        }
        let glyph_color = if files_active { theme.text_bright } else { theme.text_dim };
        chrome::workspace_glyph(&painter, Rect::from_center_size(Pos2::new(x + 8.0, cy), Vec2::new(16.0, 13.0)), glyph_color);
        x += 26.0;
        let project_color = if files_active { theme.text_bright } else { theme.text };
        chrome::faded_text(&painter, Pos2::new(x, cy - 7.0), &project, bold(13.0), project_color, label_w, 16.0);
        chrome::faded_text(&painter, Pos2::new(x, cy + 8.0), &self.hostname, FontId::proportional(11.0), theme.text_dim, label_w, 16.0);
        x += label_w + 10.0;

        // Tabs live in a recessed, segmented-control style track.
        let plus_size = 28.0;
        let tabs_right = rect.right() - plus_size - 22.0;
        let pad = 3.0;
        let tab_h = 30.0;
        let n = self.tabs.len().max(1);
        let avail = (tabs_right - x - 2.0 * pad).max(80.0);
        let tab_w = (avail / n as f32).clamp(80.0, 232.0);
        let visible = ((avail / tab_w).floor() as usize).clamp(1, n);
        let track = Rect::from_min_size(
            Pos2::new(x, cy - tab_h / 2.0 - pad),
            Vec2::new(tab_w * visible as f32 + 2.0 * pad, tab_h + 2.0 * pad),
        );
        chrome::tab_track(&painter, track);
        let x0 = track.left() + pad;
        let shown = visible.min(self.tabs.len());

        // The active pill is painted once, beneath the tabs, and glides between slots.
        let pill = (!self.files_active && self.active < shown).then(|| self.pill_glide.value(&ctx, self.active as f32));
        if let Some(slot) = pill {
            let r = Rect::from_min_size(Pos2::new(x0 + slot * tab_w, track.top() + pad), Vec2::new(tab_w, tab_h));
            chrome::glass_pill(&painter, r, 8);
        }

        for i in 0..shown {
            let r = Rect::from_min_size(Pos2::new(x0 + i as f32 * tab_w, track.top() + pad), Vec2::new(tab_w, tab_h));
            let uid = self.tabs[i].uid;
            let resp = ui.interact(r, Id::new(("tab", uid)), Sense::click_and_drag());
            if resp.dragged_by(PointerButton::Primary) {
                // Drag to reorder: move to the slot under the pointer.
                if let Some(pos) = resp.interact_pointer_pos() {
                    let target = (((pos.x - x0) / tab_w).floor().max(0.0) as usize).min(shown - 1);
                    if target != i {
                        let tab = self.tabs.remove(i);
                        self.tabs.insert(target, tab);
                        self.show_tab(target);
                        ctx.request_repaint();
                        break;
                    }
                }
            }
            if resp.drag_started_by(PointerButton::Primary) {
                self.show_tab(i);
            }
            let active = i == self.active && !self.files_active;
            let hovered = resp.hovered();
            if !active && hovered {
                painter.rect_filled(r.shrink(1.0), 8, Color32::from_white_alpha(10));
            }

            // Hairline separators between neighbours, hidden wherever the pill covers them.
            let boundary = (i + 1) as f32;
            let covered = pill.is_some_and(|p| boundary > p - 0.05 && boundary < p + 1.05);
            if i + 1 < shown && !covered {
                let sx = r.right();
                painter.line_segment(
                    [Pos2::new(sx, r.center().y - 8.0), Pos2::new(sx, r.center().y + 8.0)],
                    Stroke::new(1.0, Color32::from_white_alpha(24)),
                );
            }

            let (title, process) = self.pane_label(self.tabs[i].focused);
            let (panes, browsers) = (&self.panes, &self.browsers);
            let tab = &mut self.tabs[i];
            let icon = Rect::from_center_size(Pos2::new(r.left() + 19.0, r.center().y), Vec2::splat(20.0));
            let cards = tab.deck_cards(&ctx, |id| {
                if browsers.contains_key(&id) {
                    chrome::FILES_PROCESS.into()
                } else {
                    panes.get(&id).map(|t| t.process.clone()).unwrap_or_default()
                }
            });
            let mut text_left = icon.right() + 9.0;
            if cards.len() > 1 {
                // One card per pane, focused on top; up to 3 peek out behind it.
                let cards: Vec<(&str, f32)> = cards.iter().map(|(p, d)| (p.as_str(), *d)).collect();
                chrome::icon_stack(&painter, icon, &cards);
                let behind = (cards.len() as f32 - 1.0).min(chrome::STACK_DEPTH - 1.0);
                text_left += chrome::STACK_STEP.x * behind;
            } else {
                chrome::app_tile(&painter, icon, &process);
            }

            let close_r = Rect::from_center_size(Pos2::new(r.right() - 15.0, r.center().y), Vec2::splat(18.0));
            let text_right = if hovered { close_r.left() - 2.0 } else { r.right() - 10.0 };
            chrome::faded_text(
                &painter,
                Pos2::new(text_left, r.center().y),
                &title,
                FontId::proportional(13.5),
                if active { theme.text_bright } else { theme.text_dim },
                text_right - text_left,
                26.0,
            );

            // Close button only on hover, like Safari/Finder tabs.
            let mut close_clicked = false;
            if hovered || ui.rect_contains_pointer(close_r) {
                let cresp = ui.interact(close_r, Id::new(("tab-close", uid)), Sense::click());
                if cresp.hovered() {
                    painter.circle_filled(close_r.center(), 9.0, Color32::from_white_alpha(28));
                }
                chrome::icon_close(&painter, close_r.shrink(3.0), if cresp.hovered() { theme.text_bright } else { theme.text_dim });
                close_clicked = cresp.clicked_by(PointerButton::Primary);
            }
            if close_clicked || resp.clicked_by(PointerButton::Middle) {
                self.pending.push(Action::CloseTab(i));
            } else if resp.clicked_by(PointerButton::Primary) {
                self.show_tab(i);
            }
        }

        // New tab button.
        let plus = Rect::from_center_size(Pos2::new(rect.right() - 14.0 - plus_size / 2.0, cy), Vec2::splat(plus_size));
        let presp = ui.interact(plus, Id::new("new-tab"), Sense::click());
        if presp.hovered() {
            painter.rect_filled(plus, 8, Color32::from_white_alpha(14));
        }
        chrome::icon_plus(&painter, plus, if presp.hovered() { theme.text_bright } else { theme.text });
        if presp.clicked_by(PointerButton::Primary) {
            self.pending.push(Action::NewTab);
        }
    }

    fn panes_area(&mut self, ui: &mut Ui, area: Rect, theme: &Theme) {
        self.term_rects.clear();
        self.card_rects.clear();
        let Some(tab) = self.cur_tab() else { return };
        let tab_uid = tab.uid;

        let mut rects = Vec::new();
        let mut dividers = Vec::new();
        if tab.zoomed {
            rects.push((tab.focused, area));
        } else {
            tab.root.layout(area, PANE_GAP, &mut rects, &mut dividers);
        }
        let focused = tab.focused;
        let multi = rects.len() > 1;

        // Divider dragging.
        for div in &dividers {
            let id = Id::new(("divider", tab_uid, &div.path));
            let resp = ui.interact(div.rect, id, Sense::drag());
            if resp.hovered() || resp.dragged() {
                ui.ctx().set_cursor_icon(match div.axis {
                    Axis::Horizontal => CursorIcon::ResizeHorizontal,
                    Axis::Vertical => CursorIcon::ResizeVertical,
                });
            }
            if resp.dragged() {
                if let Some(pos) = resp.interact_pointer_pos() {
                    let ratio = match div.axis {
                        Axis::Horizontal => (pos.x - div.parent.left()) / div.parent.width(),
                        Axis::Vertical => (pos.y - div.parent.top()) / div.parent.height(),
                    };
                    if let Some(r) = self.cur_tab_mut().and_then(|t| t.root.ratio_mut(&div.path)) {
                        *r = ratio.clamp(0.08, 0.92);
                    }
                }
            }
        }

        let ctx = ui.ctx().clone();
        let ppp = ctx.pixels_per_point();
        let painter = ui.painter().clone();
        let blink_on = {
            let ms = ctx.input(|i| i.time * 1000.0) as u128;
            (ms / BLINK_INTERVAL.as_millis()) % 2 == 0
        };

        for (id, card) in rects {
            let is_focused = id == focused;
            if self.browsers.contains_key(&id) {
                self.browser_pane(ui, &painter, id, card, is_focused, multi, theme);
                self.card_rects.push((id, card));
                continue;
            }
            let Some(t) = self.panes.get(&id) else { continue };

            // Card.
            let bell = t.bell_at.is_some_and(|at| at.elapsed() < Duration::from_millis(200));
            let accent = bell.then_some(self.cfg.palette[3]);
            chrome::card(&painter, card, PANE_RADIUS, theme.card, is_focused && multi, accent);

            // Header.
            let pad = self.cfg.window_padding;
            let term_rect = self.term_rect_for(card, ppp);
            let content_top = if self.cfg.pane_headers {
                let header = Rect::from_min_size(card.min, Vec2::new(card.width(), HEADER_HEIGHT));
                let title = t.title();
                let process = t.process.clone();
                let zoomed = self.cur_tab().is_some_and(|t| t.zoomed);
                let show_buttons = is_focused || ui.rect_contains_pointer(card);
                self.pane_header(ui, &painter, header, id, &title, &process, is_focused, show_buttons, zoomed, theme);
                header.bottom()
            } else {
                card.top() + pad * 0.5
            };

            let m = self.pane_metrics(id);
            let cell_px = cell_px(&ctx, m);
            let cols = ((term_rect.width() / m.width).floor() as usize).max(2);
            let lines = ((term_rect.height() / m.height).floor() as usize).max(1);

            let t = self.panes.get_mut(&id).unwrap();
            t.resize(GridSize { cols, lines }, cell_px);
            let blinking = t.term.lock().cursor_style().blinking;
            let typing = t.last_input.elapsed() < BLINK_INTERVAL;
            let cursor_visible = !is_focused || !blinking || typing || blink_on;

            // Keep search results in sync with the terminal contents.
            let search = self.search.as_mut().filter(|s| s.pane == id);
            let search = search.map(|s| {
                let term = t.term.lock();
                let jump = s.refresh(&term, t.generation);
                drop(term);
                if let Some(p) = jump {
                    t.term.lock().scroll_to_point(p);
                }
                &*s
            });
            let link = self.hover_link.as_ref().filter(|(pid, _)| *pid == id).map(|(_, l)| &l.range);

            let clip = painter.with_clip_rect(card.shrink(1.0));
            render::paint_terminal(
                &clip,
                term_rect,
                t,
                &self.cfg,
                m,
                PaintOptions {
                    focused: is_focused && ctx.input(|i| i.focused),
                    cursor_visible,
                    bg_alpha: self.cfg.background_opacity,
                    matches: search.map(|s| s.matches.as_slice()).unwrap_or_default(),
                    current_match: search.and_then(|s| s.current()),
                    link,
                },
            );
            if let Some(s) = search {
                search_bar(&painter, card, content_top, s, theme, blink_on);
            }

            // Dim unfocused splits.
            if multi && !is_focused && self.cfg.unfocused_split_opacity < 1.0 {
                let a = 1.0 - self.cfg.unfocused_split_opacity;
                painter.rect_filled(card, PANE_RADIUS, darken(self.cfg.background, 0.5).gamma_multiply(a));
            }

            // Scrollback indicator.
            let offset = t.term.lock().grid().display_offset();
            if offset > 0 {
                let label = format!("↑ {offset}");
                let galley = painter.layout_no_wrap(label, FontId::proportional(11.0), theme.text);
                let r = Rect::from_min_size(
                    Pos2::new(card.right() - galley.size().x - 22.0, content_top + 6.0),
                    galley.size() + Vec2::new(12.0, 6.0),
                );
                painter.rect_filled(r, 6, Color32::from_black_alpha(140));
                painter.galley(r.min + Vec2::new(6.0, 3.0), galley, theme.text);
            }

            self.term_rects.push((id, term_rect));
            self.card_rects.push((id, card));
        }

        // Keep blinking cursors blinking.
        let ms = ctx.input(|i| i.time * 1000.0);
        let next = BLINK_INTERVAL.as_millis() as f64 - ms % BLINK_INTERVAL.as_millis() as f64;
        ctx.request_repaint_after(Duration::from_millis(next as u64 + 1));
    }

    /// Draw a file-browser pane: card, header (with the usual split/zoom/close buttons) and
    /// the listing. It isn't added to `term_rects`, so terminal mouse handling ignores it.
    #[allow(clippy::too_many_arguments)]
    fn browser_pane(&mut self, ui: &mut Ui, painter: &Painter, id: PaneId, card: Rect, is_focused: bool, multi: bool, theme: &Theme) {
        chrome::card(painter, card, PANE_RADIUS, theme.card, is_focused && multi, None);
        if self.cfg.pane_headers {
            let header = Rect::from_min_size(card.min, Vec2::new(card.width(), HEADER_HEIGHT));
            let (title, process) = self.pane_label(id);
            let zoomed = self.cur_tab().is_some_and(|t| t.zoomed);
            let show_buttons = is_focused || ui.rect_contains_pointer(card);
            self.pane_header(ui, painter, header, id, &title, &process, is_focused, show_buttons, zoomed, theme);
        }
        let content = self.term_rect_for(card, ui.ctx().pixels_per_point());
        if ui.rect_contains_pointer(content) && ui.input(|i| i.pointer.primary_pressed()) {
            self.pending.push(Action::FocusPane(id));
        }
        let colors = files::Colors { text: theme.text, text_dim: theme.text_dim, text_bright: theme.text_bright };
        if let Some(b) = self.browsers.get_mut(&id) {
            b.paint(ui, &painter.with_clip_rect(card.shrink(1.0)), content, id, &colors);
        }
        if multi && !is_focused && self.cfg.unfocused_split_opacity < 1.0 {
            let a = 1.0 - self.cfg.unfocused_split_opacity;
            painter.rect_filled(card, PANE_RADIUS, darken(self.cfg.background, 0.5).gamma_multiply(a));
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn pane_header(
        &mut self,
        ui: &mut Ui,
        painter: &Painter,
        header: Rect,
        id: PaneId,
        title: &str,
        process: &str,
        focused: bool,
        show_buttons: bool,
        zoomed: bool,
        theme: &Theme,
    ) {
        let cy = header.center().y + 1.0;
        let title_color = if focused { theme.text_bright } else { theme.text.gamma_multiply(0.85) };
        let glyph = Rect::from_center_size(Pos2::new(header.left() + 20.0, cy), Vec2::splat(13.0));
        chrome::header_glyph(painter, glyph, process, title_color);

        // Buttons (right to left): close, zoom, split down, split right — shown on the
        // focused or hovered card only, keeping the other headers clean.
        let mut bx = header.right() - 20.0;
        if show_buttons {
            let mut button = |name: &str, draw: &dyn Fn(&Painter, Rect, Color32)| -> bool {
                let r = Rect::from_center_size(Pos2::new(bx, cy), Vec2::splat(22.0));
                bx -= 24.0;
                let resp = ui.interact(r, Id::new(("pane-btn", id, name)), Sense::click());
                if resp.hovered() {
                    painter.rect_filled(r, 6, Color32::from_white_alpha(16));
                }
                let c = if resp.hovered() { theme.text_bright } else { theme.text_dim };
                draw(painter, r.shrink(5.5), c);
                resp.on_hover_text(name).clicked_by(PointerButton::Primary)
            };
            if button("Close  (Ctrl+Shift+W)", &|p, r, c| chrome::icon_close(p, r, c)) {
                self.pending.push(Action::ClosePane(id));
            }
            if button("Zoom  (Ctrl+Shift+Enter)", &|p, r, c| chrome::icon_zoom(p, r, c, zoomed)) {
                self.pending.push(Action::ToggleZoomPane(id));
            }
            if button("Split down  (Ctrl+Shift+E)", &|p, r, c| chrome::icon_split(p, r, c, Axis::Vertical)) {
                self.pending.push(Action::SplitPane(id, Axis::Vertical));
            }
            if button("Split right  (Ctrl+Shift+O)", &|p, r, c| chrome::icon_split(p, r, c, Axis::Horizontal)) {
                self.pending.push(Action::SplitPane(id, Axis::Horizontal));
            }
        }

        let text_left = glyph.right() + 8.0;
        chrome::faded_text(painter, Pos2::new(text_left, cy), title, bold(13.5), title_color, bx - 4.0 - text_left, 24.0);

        // Clicking the header focuses the pane.
        if ui.rect_contains_pointer(header) && ui.input(|i| i.pointer.primary_pressed()) {
            self.pending.push(Action::FocusPane(id));
        }
    }

    /// Files dropped onto a pane: dragged out of a browser onto a terminal (inserts their
    /// paths), or dropped in from another application (into a browser: copied there; onto a
    /// terminal: paths inserted).
    fn file_drops(&mut self, ctx: &egui::Context) {
        let (pos, released, dropped) = ctx.input(|i| {
            let dropped: Vec<PathBuf> = i.raw.dropped_files.iter().filter_map(|f| f.path.clone()).collect();
            (i.pointer.interact_pos(), i.pointer.any_released(), dropped)
        });
        let pos = pos.or(self.last_pointer);
        self.last_pointer = pos;
        let terminal_at = |p: Pos2| self.term_rects.iter().find(|(_, r)| r.contains(p)).map(|&(id, _)| id);

        // Browser → terminal. (Browser → browser drops are handled by the browsers.)
        if released && egui::DragAndDrop::has_payload_of_type::<files::FileDrag>(ctx) {
            if let Some(id) = pos.and_then(terminal_at) {
                if let Some(drag) = egui::DragAndDrop::take_payload::<files::FileDrag>(ctx) {
                    self.insert_paths(id, &drag.paths);
                    self.pending.push(Action::FocusPane(id));
                }
            }
        }

        if dropped.is_empty() {
            return;
        }
        // The pane under the pointer, else the focused one.
        let target = pos
            .and_then(|p| self.card_rects.iter().find(|(_, r)| r.contains(p)).map(|&(id, _)| id))
            .or_else(|| self.focused_pane());
        let Some(id) = target else { return };
        if let Some(b) = self.browsers.get_mut(&id) {
            b.drop_external(ctx, dropped, pos);
        } else {
            self.insert_paths(id, &dropped);
        }
    }

    /// Type shell-quoted paths into a terminal, as if pasted.
    fn insert_paths(&mut self, id: PaneId, paths: &[PathBuf]) {
        if let Some(t) = self.panes.get_mut(&id) {
            let text = paths.iter().map(|p| files::shell_quote(p)).collect::<Vec<_>>().join(" ");
            t.paste(&format!("{text} "));
        }
    }

    /// Frameless windows need their own resize handles at the edges.
    fn window_resize_edges(&self, ctx: &egui::Context, rect: Rect) {
        if self.cfg.window_decorations || ctx.input(|i| i.viewport().maximized.unwrap_or(false)) {
            return;
        }
        let Some(pos) = ctx.input(|i| i.pointer.hover_pos()) else { return };
        let b = RESIZE_BORDER;
        let (l, r) = (pos.x < rect.left() + b, pos.x > rect.right() - b);
        let (t, bo) = (pos.y < rect.top() + b, pos.y > rect.bottom() - b);
        let dir = match (l, r, t, bo) {
            (true, _, true, _) => ResizeDirection::NorthWest,
            (_, true, true, _) => ResizeDirection::NorthEast,
            (true, _, _, true) => ResizeDirection::SouthWest,
            (_, true, _, true) => ResizeDirection::SouthEast,
            (true, ..) => ResizeDirection::West,
            (_, true, ..) => ResizeDirection::East,
            (_, _, true, _) => ResizeDirection::North,
            (_, _, _, true) => ResizeDirection::South,
            _ => return,
        };
        ctx.set_cursor_icon(match dir {
            ResizeDirection::North | ResizeDirection::South => CursorIcon::ResizeVertical,
            ResizeDirection::East | ResizeDirection::West => CursorIcon::ResizeHorizontal,
            ResizeDirection::NorthWest | ResizeDirection::SouthEast => CursorIcon::ResizeNwSe,
            ResizeDirection::NorthEast | ResizeDirection::SouthWest => CursorIcon::ResizeNeSw,
        });
        if ctx.input(|i| i.pointer.primary_pressed()) {
            ctx.send_viewport_cmd(ViewportCommand::BeginResize(dir));
        }
    }
}

/// Cell size in physical pixels (reported to the pty for image protocols).
fn cell_px(ctx: &egui::Context, m: CellMetrics) -> (u16, u16) {
    let ppp = ctx.pixels_per_point();
    ((m.width * ppp) as u16, (m.height * ppp) as u16)
}

impl eframe::App for App {
    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_metrics(ctx);
        let full = ctx.content_rect();
        self.content_area = Rect::from_min_max(
            Pos2::new(full.left() + PANE_GAP, full.top() + TITLEBAR_HEIGHT),
            Pos2::new(full.right() - PANE_GAP, full.bottom() - PANE_GAP),
        );
        if !self.started {
            // The first shell is spawned on the first frame, once the window size is known.
            self.started = true;
            let cwd = std::env::current_dir().ok();
            self.new_tab(ctx, cwd);
        }

        // PTY events; panes whose shell exited are closed (like Ghostty).
        let mut exited = Vec::new();
        for (id, t) in &mut self.panes {
            t.process_events(ctx, &self.cfg);
            if t.exited {
                exited.push(*id);
            }
        }
        for id in exited {
            self.close_pane(id);
        }
        if self.tabs.is_empty() && self.files.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if self.tabs.is_empty() {
            self.files_active = true;
        }

        if self.last_proc_poll.elapsed() > Duration::from_millis(750) {
            self.last_proc_poll = Instant::now();
            for t in self.panes.values_mut() {
                t.refresh_process_info();
            }
            for b in self.browsers.values_mut() {
                b.refresh_if_stale();
            }
        }
        ctx.request_repaint_after(Duration::from_secs(1));

        // We draw everything ourselves; never let egui give keyboard focus to a widget
        // (otherwise Tab/Space could "click" pane buttons).
        ctx.memory_mut(|mem| {
            if let Some(id) = mem.focused() {
                mem.surrender_focus(id);
            }
        });

        self.handle_keyboard(ctx);
        for action in std::mem::take(&mut self.pending) {
            self.apply(ctx, action);
        }
        if self.tabs.is_empty() && self.files.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return;
        }
        if self.tabs.is_empty() {
            self.files_active = true;
        }

        let theme = self.colors();
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let full = ui.max_rect();
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            let radius = if maximized || self.cfg.window_decorations { 0 } else { WINDOW_RADIUS };
            ui.painter().rect_filled(full, radius, theme.window);
            if radius > 0 {
                ui.painter().rect_stroke(full, radius, Stroke::new(1.0, Color32::from_white_alpha(22)), StrokeKind::Inside);
            }

            let titlebar = Rect::from_min_size(full.min, Vec2::new(full.width(), TITLEBAR_HEIGHT));
            self.titlebar(ui, titlebar, &theme);

            let area = self.content_area;
            self.panes_area(ui, area, &theme);
            self.file_drops(ctx);
            files::paint_drag_ghost(ctx, &files::Colors { text: theme.text, text_dim: theme.text_dim, text_bright: theme.text_bright });
            self.window_resize_edges(ctx, full);
        });

        self.handle_mouse(ctx);
        if !self.pending.is_empty() {
            for action in std::mem::take(&mut self.pending) {
                self.apply(ctx, action);
            }
            ctx.request_repaint();
        }

        // Window title follows the focused pane.
        if let Some(title) = self.focused_pane().map(|id| self.pane_label(id).0).filter(|t| !t.is_empty()) {
            if title != self.window_title {
                ctx.send_viewport_cmd(ViewportCommand::Title(format!("{title} — nex")));
                self.window_title = title;
            }
        }
    }
}

/// Ghostty-style floating search field in the top-right of the pane.
fn search_bar(painter: &Painter, card: Rect, top: f32, s: &Search, theme: &Theme, caret_on: bool) {
    let w = 300.0_f32.min(card.width() - 24.0);
    let r = Rect::from_min_size(Pos2::new(card.right() - w - 12.0, top + 4.0), Vec2::new(w, 30.0));
    painter.rect_filled(r.translate(Vec2::new(0.0, 2.0)), 8, Color32::from_black_alpha(80));
    painter.rect_filled(r, 8, Color32::from_rgb(0x2a, 0x27, 0x30));
    painter.rect_stroke(r, 8, Stroke::new(1.0, Color32::from_white_alpha(40)), StrokeKind::Inside);

    // Magnifier icon.
    let c = Pos2::new(r.left() + 16.0, r.center().y - 1.0);
    painter.circle_stroke(c, 4.5, Stroke::new(1.4, theme.text_dim));
    painter.line_segment([c + Vec2::new(3.2, 3.2), c + Vec2::new(6.5, 6.5)], Stroke::new(1.4, theme.text_dim));

    let count = match (s.query.is_empty(), s.index) {
        (true, _) => String::new(),
        (false, Some(i)) => format!("{}/{}", i + 1, s.matches.len()),
        (false, None) => "0/0".into(),
    };
    let font = FontId::proportional(12.5);
    let count_galley = painter.layout_no_wrap(count, font.clone(), theme.text_dim);
    let count_x = r.right() - 10.0 - count_galley.size().x;
    painter.galley(Pos2::new(count_x, r.center().y - count_galley.size().y / 2.0), count_galley, theme.text_dim);

    let text_rect = Rect::from_min_max(Pos2::new(r.left() + 30.0, r.top()), Pos2::new(count_x - 8.0, r.bottom()));
    let clip = painter.with_clip_rect(text_rect);
    let (text, color) = if s.query.is_empty() {
        ("Search…".to_string(), theme.text_dim.gamma_multiply(0.7))
    } else {
        (s.query.clone(), if s.matches.is_empty() { Color32::from_rgb(0xf2, 0x77, 0x7a) } else { theme.text })
    };
    let galley = painter.layout_no_wrap(text, font, color);
    // Keep the end of long queries visible.
    let tx = text_rect.left().min(text_rect.right() - galley.size().x - 3.0);
    let caret_x = if s.query.is_empty() { text_rect.left() } else { tx + galley.size().x + 1.0 };
    clip.galley(Pos2::new(tx, r.center().y - galley.size().y / 2.0), galley, color);
    if caret_on {
        clip.line_segment(
            [Pos2::new(caret_x, r.center().y - 7.0), Pos2::new(caret_x, r.center().y + 7.0)],
            Stroke::new(1.2, theme.text),
        );
    }
}

struct Theme {
    window: Color32,
    card: Color32,
    text: Color32,
    text_bright: Color32,
    text_dim: Color32,
}

fn lighten(c: Color32, f: f32) -> Color32 {
    let l = |v: u8| (v as f32 + (255.0 - v as f32) * f) as u8;
    Color32::from_rgba_unmultiplied(l(c.r()), l(c.g()), l(c.b()), c.a())
}

fn darken(c: Color32, f: f32) -> Color32 {
    Color32::from_rgba_unmultiplied((c.r() as f32 * f) as u8, (c.g() as f32 * f) as u8, (c.b() as f32 * f) as u8, c.a())
}
