//! The application: window chrome (title bar + tabs), split panes, input handling.
//! Each concern lives in its own submodule; this file holds the state and the frame loop.

mod actions;
mod keyboard;
mod metrics;
mod mouse;
mod panes;
mod tab;
mod titlebar;
mod window;
mod workspace;

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::{
    Color32, Pos2, Rect, Stroke, StrokeKind,
    Vec2, ViewportCommand,
};

use crate::config::Config;
use crate::files::{self, FileBrowser};
use crate::fonts;
use crate::layout::PaneId;
use crate::render::CellMetrics;
use crate::search::{Link, LinkFinder, Search};
use crate::terminal::Terminal;

use actions::{keybindings, Action, Binding};
use mouse::MouseState;
use tab::Tab;
use crate::glide::Glide;
use crate::theme::Theme;

const TITLEBAR_HEIGHT: f32 = 46.0;
const HEADER_HEIGHT: f32 = 34.0;
const PANE_GAP: f32 = 8.0;
const WINDOW_RADIUS: u8 = 14;

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
    bindings: Vec<Binding>,
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

        let theme = Theme::from_config(&self.cfg);
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
