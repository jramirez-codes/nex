//! The application: window chrome (title bar + tabs), split panes, input handling.
//! Each concern lives in its own submodule; this file holds the state and the frame loop.

mod actions;
mod keyboard;
mod metrics;
mod motion;
mod mouse;
mod panes;
mod reload;
mod sidebar;
mod tab;
mod titlebar;
mod window;
mod workspace;
mod zoom;
#[cfg(test)]
mod drop_test;

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2, ViewportCommand};

use crate::config::{self, Config};
use crate::file_explorer::{self, FileExplorer};
use crate::fonts;
use crate::glyphs::GlyphCache;
use crate::layout::PaneId;
use crate::render::CellMetrics;
use crate::search::{Link, LinkFinder, Search};
use crate::terminal::Terminal;
use crate::ui::glide::Glide;
use crate::ui::theme::{Theme, white_alpha};
use crate::ui::toast::{self, Toast};
use crate::ui::widgets::is_maximized;

use actions::{keybindings, Action, Binding};
use mouse::MouseState;
use tab::Tab;

const TITLEBAR_HEIGHT: f32 = 46.0;
const HEADER_HEIGHT: f32 = 34.0;
/// Pane headers in mobile mode, tall enough for touch-sized buttons.
const MOBILE_HEADER_HEIGHT: f32 = 44.0;
const PANE_GAP: f32 = 8.0;
const WINDOW_RADIUS: u8 = 14;
const WINDOW_BORDER: Color32 = white_alpha(22);
/// How often process names, working directories and file listings are refreshed.
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(750);

pub struct App {
    cfg: Config,
    /// Modification time of the config file when it was last read; a change triggers a reload.
    config_mtime: Option<SystemTime>,
    panes: HashMap<PaneId, Terminal>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: PaneId,
    /// Per-pane font size overrides (zoom is per pane); panes not listed use the config size.
    pane_fonts: HashMap<PaneId, f32>,
    /// Cell metrics for every font size in use this frame, keyed by `f32::to_bits`.
    metrics: HashMap<u32, CellMetrics>,
    mouse: MouseState,
    /// Pinch zoom of a terminal not yet applied (font sizes change in whole points).
    pinch_accum: f32,
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
    /// Where each character's bitmap is in egui's font atlas (shared by all panes).
    glyphs: GlyphCache,
    links: LinkFinder,
    /// Link under the pointer while Ctrl is held.
    hover_link: Option<(PaneId, Link)>,
    /// Animated position of the active-tab highlight, in tab slots.
    pill_glide: Glide,
    /// File-explorer panes (the Files tab). They share the pane id space and split tree with
    /// terminals.
    explorers: HashMap<PaneId, FileExplorer>,
    /// The Files tab. It lives outside `tabs`: the title-bar directory label is its tab.
    explorer_tab: Option<Tab>,
    /// Whether the Files tab is showing (`active` keeps the last terminal tab).
    explorer_active: bool,
    /// Last known pointer position (touch releases report the pointer as gone).
    last_pointer: Option<Pos2>,
    /// Flags passed on to the nex started by Ctrl+Shift+N (`--config`, `--app-id`).
    new_window_args: Vec<String>,
    /// `-e` command, run by the first pane only; later panes start the shell.
    first_command: Option<Vec<String>>,
    /// Drag and drop with other windows (Wayland only).
    dnd: Option<crate::wl_dnd::WlDnd>,
    /// The "Copied …" / "Pasted …" message currently on screen.
    toast: Option<Toast>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, args: crate::cli::Args) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        ctx.set_visuals(egui::Visuals::dark());
        fonts::install(ctx, &cfg);
        crate::ui::glide::set_animations(ctx, cfg.animations);

        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "localhost".into());

        Self {
            cfg,
            config_mtime: config::modified(),
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            pane_fonts: HashMap::new(),
            metrics: HashMap::new(),
            mouse: MouseState::default(),
            pinch_accum: 1.0,
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
            glyphs: GlyphCache::default(),
            links: LinkFinder::new(),
            hover_link: None,
            pill_glide: Glide::new(0.2),
            explorers: HashMap::new(),
            explorer_tab: None,
            explorer_active: false,
            last_pointer: None,
            new_window_args: args.for_new_window(),
            first_command: args.command,
            dnd: crate::wl_dnd::WlDnd::new(cc),
            toast: None,
        }
    }
}

impl eframe::App for App {
    /// Files dragged in from another window, reported as winit does on X11.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        if let Some(dnd) = &mut self.dnd {
            dnd.feed(ctx, raw_input);
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// One frame: read PTY output, handle keys, draw, handle the mouse, then apply whatever
    /// the user asked for along the way.
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.update_metrics(ctx);
        self.update_content_area(ctx);
        if !self.started {
            // The first shell is spawned on the first frame, once the window size is known.
            self.started = true;
            let cwd = std::env::current_dir().ok();
            self.new_tab(ctx, cwd);
        }

        self.poll_ptys(ctx);
        if self.close_if_empty(ctx) {
            return;
        }
        self.poll_background();
        ctx.request_repaint_after(Duration::from_secs(1));

        // We draw everything ourselves; never let egui give keyboard focus to a widget
        // (otherwise Tab/Space could "click" pane buttons).
        ctx.memory_mut(|mem| {
            if let Some(id) = mem.focused() {
                mem.surrender_focus(id);
            }
        });

        self.handle_keyboard(ctx);
        self.drain_pending(ctx);
        if self.close_if_empty(ctx) {
            return;
        }

        self.glyphs.begin_frame(ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| self.draw_window(ui));
        self.glyphs.end_frame(ctx);

        self.handle_pinch(ctx);
        self.handle_mouse(ctx);
        if !self.pending.is_empty() {
            self.drain_pending(ctx);
            ctx.request_repaint();
        }
        self.sync_window_title(ctx);
    }
}

impl App {
    /// The area below the title bar (or right of the mobile sidebar) where panes go.
    fn update_content_area(&mut self, ctx: &egui::Context) {
        let full = ctx.content_rect();
        let min = if self.cfg.mobile {
            Pos2::new(full.left() + sidebar::SIDEBAR_WIDTH, full.top() + PANE_GAP)
        } else {
            Pos2::new(full.left() + PANE_GAP, full.top() + TITLEBAR_HEIGHT)
        };
        self.content_area = Rect::from_min_max(min, Pos2::new(full.right() - PANE_GAP, full.bottom() - PANE_GAP));
    }

    /// Height of a pane's header (taller in mobile mode).
    fn header_height(&self) -> f32 {
        if self.cfg.mobile { MOBILE_HEADER_HEIGHT } else { HEADER_HEIGHT }
    }

    /// Read PTY events; panes whose shell exited are closed (like Ghostty).
    fn poll_ptys(&mut self, ctx: &egui::Context) {
        let mut exited = Vec::new();
        for (id, t) in &mut self.panes {
            t.process_events(ctx, &self.cfg);
            if t.exited {
                exited.push(*id);
            }
        }
        for id in exited {
            self.close_pane(ctx, id);
        }
    }

    /// Close the window once no tabs are left. With only the Files tab left, show it.
    /// Returns `true` if the window is closing.
    fn close_if_empty(&mut self, ctx: &egui::Context) -> bool {
        if self.tabs.is_empty() && self.explorer_tab.is_none() {
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return true;
        }
        if self.tabs.is_empty() {
            self.explorer_active = true;
        }
        false
    }

    /// Every so often: refresh process names / cwds, re-read stale file listings, and reload
    /// the config if its file changed.
    fn poll_background(&mut self) {
        if self.last_proc_poll.elapsed() <= PROCESS_POLL_INTERVAL {
            return;
        }
        self.last_proc_poll = Instant::now();
        if config::modified() != self.config_mtime {
            self.pending.push(Action::ReloadConfig);
        }
        for t in self.panes.values_mut() {
            t.refresh_process_info();
        }
        for b in self.explorers.values_mut() {
            b.refresh_if_stale();
        }
    }

    fn drain_pending(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.pending) {
            self.apply(ctx, action);
        }
    }

    /// Window background, title bar, panes, and anything dragged over them.
    fn draw_window(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        let theme = Theme::from_config(&self.cfg);
        let full = ui.max_rect();
        let radius = if is_maximized(&ctx) || self.cfg.window_decorations { 0 } else { WINDOW_RADIUS };
        ui.painter().rect_filled(full, radius, theme.window);
        if radius > 0 {
            ui.painter().rect_stroke(full, radius, Stroke::new(1.0, WINDOW_BORDER), StrokeKind::Inside);
        }

        if self.cfg.mobile {
            let sidebar = Rect::from_min_size(full.min, Vec2::new(sidebar::SIDEBAR_WIDTH, full.height()));
            self.sidebar(ui, sidebar, &theme);
        } else {
            let titlebar = Rect::from_min_size(full.min, Vec2::new(full.width(), TITLEBAR_HEIGHT));
            self.titlebar(ui, titlebar, &theme);
        }
        let area = self.content_area;
        self.panes_area(ui, area, &theme);
        self.file_drops(&ctx);
        file_explorer::paint_drag_ghost(&ctx, &theme);
        self.window_resize_edges(&ctx, full);
        if self.toast.as_ref().is_some_and(|t| !t.paint(&ctx, area, &theme)) {
            self.toast = None;
        }
    }

    /// Confirm a terminal copy or paste with a toast (unless turned off in the config).
    fn clipboard_toast(&mut self, ctx: &egui::Context, verb: &str, text: &str) {
        if self.cfg.clipboard_toasts && !text.is_empty() {
            self.toast = Some(Toast::new(ctx, toast::describe(verb, text)));
        }
    }

    /// The window title follows the focused pane.
    fn sync_window_title(&mut self, ctx: &egui::Context) {
        let Some(title) = self.focused_pane().map(|id| self.pane_label(id).0).filter(|t| !t.is_empty()) else { return };
        if title != self.window_title {
            ctx.send_viewport_cmd(ViewportCommand::Title(format!("{title} — nex")));
            self.window_title = title;
        }
    }
}
