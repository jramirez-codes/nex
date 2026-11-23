//! The application: window chrome (title bar + tabs), split panes, input handling.
//! Each concern lives in its own submodule; this file holds the state and the frame loop:
//! `input/` (keys, mouse, zoom), `chrome/` (title bar, sidebar, window), `workspace/` (tabs and
//! panes), `animation/` (motion and tab switches), `ssh/` (remote hosts). `actions` is what
//! the user can ask for, shared by input and chrome.

mod actions;
mod animation;
mod chrome;
mod connect;
mod input;
mod reload;
pub mod restore;
mod ssh;
mod workspace;
#[cfg(test)]
mod tests;

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2, ViewportCommand};

use crate::config::{self, Config};
use crate::file_explorer::{self, FileExplorer};
use crate::agents::{self, AgentsPane};
use crate::git_diff::{self, GitDiff};
use crate::render::fonts;
use crate::render::glyphs::GlyphCache;
use crate::layout::PaneId;
use crate::render::CellMetrics;
use crate::terminal::search::{Link, LinkFinder, Search};
use crate::terminal::Terminal;
use crate::ui::glide::Glide;
use crate::ui::theme::{Theme, white_alpha};
use crate::ui::toast::{self, Toast};
use crate::ui::widgets::is_maximized;

use actions::Action;
use input::keymap::{Binding, keybindings};
use input::mouse::MouseState;
use workspace::tab::Tab;
use animation::tab_switch::TabSwitch;

const TITLEBAR_HEIGHT: f32 = 46.0;
const HEADER_HEIGHT: f32 = 34.0;
/// Pane headers with the sidebar tab bar, tall enough for touch-sized buttons.
const MOBILE_HEADER_HEIGHT: f32 = 44.0;
const PANE_GAP: f32 = 8.0;
const WINDOW_RADIUS: u8 = 14;
const WINDOW_BORDER: Color32 = white_alpha(22);
/// How often process names, working directories and file listings are refreshed.
const PROCESS_POLL_INTERVAL: Duration = Duration::from_millis(750);
/// Widest font atlas egui may make. Its default is the GPU's limit, 16384 px.
const MAX_ATLAS_SIDE: usize = 4096;
/// How long after startup or a font change freed memory is handed back to the OS.
const TRIM_HEAP_DELAY: Duration = Duration::from_millis(500);

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
    /// The zoomed pane of the shown tab and its font size: scaled up from its own size so it
    /// keeps its grid while filling the tab.
    zoom_font: Option<(PaneId, f32)>,
    /// Cell metrics for every font size in use this frame, keyed by `f32::to_bits`.
    metrics: HashMap<u32, CellMetrics>,
    mouse: MouseState,
    /// A finger on a pane that may become a sideways swipe to the next or previous tab.
    tab_swipe: Option<input::swipe::TabSwipe>,
    /// Pinch zoom of a terminal not yet applied (font sizes change in whole points).
    pinch_accum: f32,
    pending: Vec<Action>,
    /// Terminal (content) rects from the current frame, used for mouse hit-testing.
    term_rects: Vec<(PaneId, Rect)>,
    /// Card rects (including header), used for directional focus.
    card_rects: Vec<(PaneId, Rect)>,
    /// Where the layout put the focused pane's card this frame, and how it looked (the
    /// connect handoff lands there).
    focused_card: Option<animation::handoff::FocusedCard>,
    /// The connect screen handing over to the panes, while it plays (`animation/handoff.rs`).
    handoff: Option<animation::handoff::Handoff>,
    hostname: String,
    last_proc_poll: Instant,
    /// When to hand memory freed during startup (font loading) back to the OS; `None` once done.
    trim_heap_at: Option<Instant>,
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
    /// A pane being dragged by its header, if any (it can be carried to another tab).
    pane_drag: Option<workspace::pane_drag::PaneDrag>,
    /// The tab-switch transition (`tab-animation`): the old tab's panes giving way to the new.
    switch: TabSwitch,
    /// File-explorer panes (the Files tab). They share the pane id space and split tree with
    /// terminals.
    explorers: HashMap<PaneId, FileExplorer>,
    /// What explorers asked for while painting (a terminal in some folder), applied with
    /// the pending actions.
    explorer_requests: Vec<(PaneId, file_explorer::Request)>,
    /// Changes (git diff) panes, in the same pane id space and split tree.
    diffs: HashMap<PaneId, GitDiff>,
    /// What Changes panes asked for while painting (show a file in Files).
    diff_requests: Vec<(PaneId, git_diff::Request)>,
    /// Agents panes, in the same pane id space and split tree.
    agents: HashMap<PaneId, AgentsPane>,
    /// What Agents panes asked for while painting (jump to an agent's pane).
    agent_requests: Vec<(PaneId, agents::Request)>,
    /// Every agent running in a pane and what it's doing, as of the last process poll; what
    /// Agents panes and the title bar's Agents label show.
    agent_tracker: agents::model::Tracker,
    /// The home tab, outside `tabs`: the Files tab (the title-bar directory label is its
    /// tab), or the Agents tab when it holds an Agents pane (see `home_is_files`). Saved in
    /// the session as `explorer`.
    home_tab: Option<Tab>,
    /// Whether the home tab is showing (`active` keeps the last terminal tab).
    home_active: bool,
    /// The sticky pane: it follows the user from tab to tab, a leaf in every tab's split
    /// tree (each tab keeps its own place and size for it). See `workspace/sticky.rs`.
    sticky: Option<PaneId>,
    /// Its card gliding from where the last tab had it to where this one does.
    sticky_glide: animation::sticky_glide::StickyGlide,
    /// Last known pointer position (touch releases report the pointer as gone).
    last_pointer: Option<Pos2>,
    /// Flags passed on to the nex started by Ctrl+Shift+N (`--config`, `--app-id`).
    new_window_args: Vec<String>,
    /// `--explorer` (or a folder argument): show the Files tab once the first shell opens.
    open_explorer: bool,
    /// `--select`: the item to select in that Files tab.
    select_at_start: Option<std::path::PathBuf>,
    /// `-e` command, run by the first pane only; later panes start the shell.
    first_command: Option<Vec<String>>,
    /// The first shell, started before the window existed (see `Terminal::spawn_early`);
    /// the first pane opened takes it.
    early_terminal: Option<Terminal>,
    /// Drag and drop with other windows, where the platform needs nex to do it (Wayland).
    dnd: Option<crate::platform::Dnd>,
    /// The "Copied …" / "Pasted …" message currently on screen.
    toast: Option<Toast>,
    /// Typing goes through an on-screen keyboard (`platform::SOFT_KEYBOARD`; tests turn it on).
    soft_keyboard: bool,
    /// The floating button that shows or hides the on-screen keyboard.
    keyboard_button: chrome::keyboard_button::KeyboardButton,
    /// A toast to show on the next frame, from code without the egui context.
    notice: Option<String>,
    /// Uploads, the shared mic, watched agents and their dialogs (see `ssh/`).
    ssh: ssh::SshState,
    /// Restoring the last session and keeping the session file (see `restore.rs`).
    session: restore::SessionState,
    /// Where the tabs were drawn and how they are animating (see `chrome/titlebar.rs`).
    strip: chrome::titlebar::TabStrip,
    /// The host every pane works on, where this window is a thin client (Android,
    /// `--remote`), and the connect screen shown until a pane is open (see `connect.rs`).
    connect: connect::Connect,
    /// Shells already on that host that the next panes opened attach to, instead of starting
    /// new ones (the ones this device left open there).
    adopt: Vec<String>,
    /// The agent session the next remote pane attaches to (picked in the Agents pane).
    attach_next: Option<String>,
    /// The folder on the host the next pane opened there starts in, when it isn't the
    /// focused pane's (a Files pane's "Open Terminal Here").
    next_remote_cwd: Option<String>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, cfg: Config, args: crate::cli::Args, early_terminal: Option<Terminal>, session: Option<crate::session::Session>, session_lock: Option<crate::session::Lock>) -> Self {
        let ctx = &cc.egui_ctx;
        ctx.options_mut(|o| o.zoom_with_keyboard = false);
        ctx.set_visuals(egui::Visuals::dark());
        fonts::install(ctx, &cfg);
        crate::ui::glide::set_animations(ctx, cfg.animations);
        // A watched folder changed (a file moved away by another window, `mv` in a terminal):
        // every explorer re-reads its listing on the next frame.
        let repaint = ctx.clone();
        crate::platform::on_dir_change(move || {
            file_explorer::files_changed();
            repaint.request_repaint();
        });

        let hostname = crate::platform::hostname();

        // Read before the literal moves `args.command` out.
        let save_session = args.restores_session() && session_lock.is_some();
        let remote_window = !crate::platform::LOCAL_SHELLS || args.remote.is_some();
        let remote_arg = args.remote.clone();
        Self {
            cfg,
            config_mtime: config::modified(),
            panes: HashMap::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            pane_fonts: HashMap::new(),
            zoom_font: None,
            metrics: HashMap::new(),
            mouse: MouseState::default(),
            tab_swipe: None,
            pinch_accum: 1.0,
            pending: Vec::new(),
            term_rects: Vec::new(),
            card_rects: Vec::new(),
            focused_card: None,
            handoff: None,
            hostname,
            last_proc_poll: Instant::now(),
            trim_heap_at: Some(Instant::now() + TRIM_HEAP_DELAY),
            window_title: String::new(),
            bindings: keybindings(),
            content_area: Rect::NOTHING,
            started: false,
            search: None,
            glyphs: GlyphCache::default(),
            links: LinkFinder::new(),
            hover_link: None,
            pane_drag: None,
            switch: TabSwitch::default(),
            explorers: HashMap::new(),
            explorer_requests: Vec::new(),
            diffs: HashMap::new(),
            diff_requests: Vec::new(),
            agents: HashMap::new(),
            agent_requests: Vec::new(),
            agent_tracker: agents::model::Tracker::default(),
            home_tab: None,
            home_active: false,
            sticky: None,
            sticky_glide: Default::default(),
            last_pointer: None,
            new_window_args: args.for_new_window(),
            open_explorer: args.explorer,
            select_at_start: args.select.clone(),
            // An early terminal already runs the `-e` command.
            first_command: if early_terminal.is_some() { None } else { args.command },
            early_terminal,
            dnd: crate::platform::Dnd::new(cc),
            toast: None,
            soft_keyboard: crate::platform::SOFT_KEYBOARD,
            keyboard_button: Default::default(),
            notice: None,
            ssh: ssh::SshState {
                uploads: Vec::new(),
                sharing_prompt: None,
                sharing_declined: false,
                pending_pane: None,
                mics: ssh::mic::Mics::default(),
                remote_hosts: ssh::hosts::Hosts::default(),
                agent_prompt: None,
                agent_notes: Vec::new(),
            },
            session: restore::SessionState {
                restore_at_start: session,
                save_session,
                _session_lock: session_lock,
                had_tabs: false,
                closed_by_user: false,
                session_suspended: false,
                last_user_input: None,
                saved_session: None,
            },
            strip: chrome::titlebar::TabStrip {
                pill_glide: Glide::new(0.2),
                tab_rects: HashMap::new(),
                tab_ghosts: Vec::new(),
                files_rect: None,
                dog_wake: None,
                dog_was_active: false,
                dog_rect: None,
                tab_hits: Vec::new(),
                strip_rect: None,
                scroll: chrome::titlebar::TabScroll::new(crate::platform::WINDOW.scroll_tabs),
            },
            connect: connect::Connect::new(remote_window, remote_arg.as_deref()),
            adopt: Vec::new(),
            attach_next: None,
            next_remote_cwd: None,
        }
    }
}

impl eframe::App for App {
    /// Stop sharing the microphone, so hosts get their own `~/.asoundrc` back.
    #[cfg(not(target_os = "ios"))]
    fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
        self.stop_mics();
        self.save_session_on_exit();
    }

    /// The same on iOS, where eframe draws with wgpu (no glow context to pass).
    #[cfg(target_os = "ios")]
    fn on_exit(&mut self) {
        self.stop_mics();
        self.save_session_on_exit();
    }

    /// Files dragged in from another window, reported as winit does on X11.
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        // egui makes its font atlas as wide as the GPU allows (16384 px), most of it empty,
        // in memory and again on the GPU. A narrower atlas grows taller only as glyphs need it.
        raw_input.max_texture_side = raw_input.max_texture_side.map(|side| side.min(MAX_ATLAS_SIDE));
        if let Some(mut dnd) = self.dnd.take() {
            dnd.feed(ctx, raw_input, || self.paste_uploads_files());
            self.dnd = Some(dnd);
        }
        // Android: the window covers the whole screen; keep nex out from under the status bar
        // and navigation bar.
        if let (Some([l, t, r, b]), Some(screen)) = (crate::platform::content_rect_px(), raw_input.screen_rect) {
            let ppp = ctx.pixels_per_point();
            let free = Rect::from_min_max(Pos2::new(l / ppp, t / ppp), Pos2::new(r / ppp, b / ppp)).intersect(screen);
            if free.is_positive() {
                raw_input.screen_rect = Some(free);
            }
        }
        // The connect screen's text fields take what the on-screen keyboard typed or pasted (its
        // takebacks as Backspace); terminals take it in `handle_keyboard`.
        if self.connect.active && self.tabs.is_empty() {
            for typed in crate::platform::typed_text() {
                let text = match typed {
                    crate::platform::Typed::Paste(text) => text,
                    crate::platform::Typed::Enter => {
                        for pressed in [true, false] {
                            let modifiers = egui::Modifiers::NONE;
                            raw_input.events.push(egui::Event::Key { key: egui::Key::Enter, physical_key: None, pressed, repeat: false, modifiers });
                        }
                        continue;
                    }
                    crate::platform::Typed::Text { delete, text } => {
                        for pressed in std::iter::repeat_n([true, false], delete).flatten() {
                            let modifiers = egui::Modifiers::NONE;
                            raw_input.events.push(egui::Event::Key { key: egui::Key::Backspace, physical_key: None, pressed, repeat: false, modifiers });
                        }
                        text
                    }
                };
                if !text.is_empty() {
                    raw_input.events.push(egui::Event::Text(text));
                }
            }
        }
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.0, 0.0, 0.0, 0.0]
    }

    /// One frame: read PTY output, handle keys, draw, handle the mouse, then apply whatever
    /// the user asked for along the way.
    fn update(&mut self, ctx: &egui::Context, frame: &mut eframe::Frame) {
        self.update_metrics(ctx);
        self.update_content_area(ctx);
        if !self.started && !self.connect.active {
            // The first shell is spawned on the first frame, once the window size is known.
            self.started = true;
            let restored = self.session.restore_at_start.take().is_some_and(|s| self.restore_session(ctx, s));
            if !restored {
                let cwd = std::env::current_dir().ok();
                self.new_tab(ctx, cwd, None);
            }
            if self.open_explorer {
                // Opened as a file manager: the Files tab, at the first shell's folder.
                self.toggle_explorer();
                if let Some(item) = self.select_at_start.take() {
                    self.select_in_explorer(&item);
                }
            }
        }

        // A thin client (Android, `--remote`) with nothing open: connect, then open the panes.
        if self.connect.active && self.tabs.is_empty() {
            self.connect_screen(ctx);
            return;
        }

        if ctx.input(|i| i.events.iter().any(is_user_input)) {
            self.session.last_user_input = Some(Instant::now());
            self.session.session_suspended = false;
        }
        self.poll_ptys(ctx);
        self.poll_uploads(ctx);
        self.poll_mics(ctx);
        self.ssh.remote_hosts.set_ctx(ctx);
        self.poll_agent_jobs(ctx);
        if let Some(n) = self.notice.take() {
            self.toast = Some(Toast::new(ctx, n));
        }
        if self.close_if_empty(ctx) {
            return;
        }
        self.poll_background();
        self.trim_heap_when_due();
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
        self.follow_focus();
        if self.close_if_empty(ctx) {
            return;
        }
        self.remember_terminal_focus();

        self.clear_seen_attention(ctx);
        self.glyphs.begin_frame(ctx);
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| self.draw_window(ui));
        self.glyphs.end_frame(ctx);
        self.paint_handoff(ctx);

        self.handle_pinch(ctx);
        self.keyboard_button(ctx);
        self.handle_tab_swipe(ctx);
        self.handle_mouse(ctx);
        if self.soft_keyboard {
            self.ask_for_keyboard(ctx);
        }
        crate::platform::mirror_copies(ctx);
        crate::platform::sync_soft_keyboard(ctx);
        if !self.pending.is_empty() {
            self.drain_pending(ctx);
            ctx.request_repaint();
        }
        let buttons_height = (self.cfg.window_decorations && crate::platform::WINDOW.overlays_titlebar())
            .then(|| TITLEBAR_HEIGHT * ctx.zoom_factor());
        self.sync_window_title(ctx, frame, buttons_height);
        if let Some(height) = buttons_height {
            crate::platform::align_window_buttons(frame, height);
        }
    }
}

impl App {
    /// Draw the connect screen; once connected, open a tab for each shell this device left on
    /// the host, or one new shell.
    fn connect_screen(&mut self, ctx: &egui::Context) {
        if let Some(shells) = self.connect.poll() {
            let tabs = shells.len().max(1);
            self.adopt = shells;
            // Oldest first, as they were opened.
            self.adopt.reverse();
            for _ in 0..tabs {
                self.new_tab(ctx, None, None);
            }
            if crate::ui::glide::animations(ctx) && let Some(drawn) = self.connect.last_card() {
                let start = ctx.input(|i| i.time);
                let drawn = drawn.clone();
                self.handoff = Some(animation::handoff::Handoff { start, drawn, sidebar: self.cfg.sidebar() });
            }
            if !self.tabs.is_empty() {
                self.show_tab(0);
                ctx.request_repaint();
                return;
            }
        }
        let theme = Theme::from_config(&self.cfg);
        self.connect.show(ctx, &theme, self.cfg.device_key_days, self.cfg.sidebar());
        crate::platform::mirror_copies(ctx);
        crate::platform::sync_soft_keyboard(ctx);
    }

    /// Keep the on-screen keyboard up while a terminal (or an Agents pane, once the keyboard
    /// button brought it up there) has focus: egui shows it for as long as a frame says where
    /// text goes (here, the pane's last line).
    fn ask_for_keyboard(&self, ctx: &egui::Context) {
        let Some(target) = self.keyboard_target(ctx) else { return };
        // Put away with the keyboard button: it stays down until the button brings it back.
        if self.keyboard_button.down(target) {
            return;
        }
        let Some(id) = self.focused_pane() else { return };
        let rects = match target {
            chrome::keyboard_button::Target::Terminal => &self.term_rects,
            chrome::keyboard_button::Target::Agents => &self.card_rects,
        };
        let Some(&(_, rect)) = rects.iter().find(|(p, _)| *p == id) else { return };
        let cursor_rect = Rect::from_min_size(rect.left_bottom() - Vec2::new(0.0, 2.0), Vec2::new(1.0, 2.0));
        ctx.output_mut(|o| o.ime = Some(egui::output::IMEOutput { rect, cursor_rect }));
    }

    /// The area below the title bar (or right of the sidebar) where panes go.
    fn update_content_area(&mut self, ctx: &egui::Context) {
        let full = ctx.content_rect();
        let min = if self.cfg.sidebar() {
            Pos2::new(full.left() + chrome::sidebar::SIDEBAR_WIDTH, full.top() + PANE_GAP)
        } else {
            Pos2::new(full.left() + PANE_GAP, full.top() + TITLEBAR_HEIGHT)
        };
        self.content_area = Rect::from_min_max(min, Pos2::new(full.right() - PANE_GAP, full.bottom() - PANE_GAP));
    }

    /// Height of a pane's header (taller with the sidebar tab bar).
    fn header_height(&self) -> f32 {
        header_height(self.cfg.sidebar())
    }

    /// Read PTY events; panes whose shell exited are closed (like Ghostty).
    fn poll_ptys(&mut self, ctx: &egui::Context) {
        let mut exited = Vec::new();
        let mut alerted = false;
        for (id, t) in &mut self.panes {
            let had = t.attention;
            t.process_events(ctx, &self.cfg);
            alerted |= t.attention && !had;
            if t.exited {
                exited.push(*id);
            }
            if t.has_pending_command() {
                // Hidden panes may not repaint on output: keep checking until it's typed.
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
            }
        }
        // A program wants input while nex is in the background: flag it in the dock/taskbar.
        if alerted && !ctx.input(|i| i.focused) {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        for id in &exited {
            if self.exit_by_user(*id) {
                self.session.closed_by_user = true;
            } else {
                self.session.closed_by_user = false;
                self.session.session_suspended = true;
            }
        }
        for id in exited {
            self.close_pane(ctx, id);
        }
    }

    /// Looking at a pane (the shown tab's focused pane, window focused) answers its bell, and
    /// an agent on it that was done or needed you. Run after this frame's tab switches and
    /// before drawing, so the shown tab never flashes a dot.
    fn clear_seen_attention(&mut self, ctx: &egui::Context) {
        let window_focused = ctx.input(|i| i.focused);
        let seen = window_focused.then(|| self.cur_tab().map(|t| t.focused)).flatten();
        if let Some(t) = seen.and_then(|id| self.panes.get_mut(&id)) {
            t.attention = false;
        }
        self.agent_tracker.mark_seen(seen, std::time::Instant::now());
        // An agent finished or wants input while nex is in the background: flag it in the
        // dock/taskbar, as a bell does.
        if self.agent_tracker.take_alerted() && !window_focused {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
    }

    /// Close the window once no tabs are left. With only the Files tab left (or an Agents tab
    /// with a terminal or explorer split into it), show it. Returns `true` if the window is
    /// closing.
    fn close_if_empty(&mut self, ctx: &egui::Context) -> bool {
        if self.tabs.is_empty() && !self.home_keeps_window() {
            if self.forgets_session_when_empty() {
                self.forget_session();
            }
            ctx.send_viewport_cmd(ViewportCommand::Close);
            return true;
        }
        self.session.had_tabs = true;
        if self.tabs.is_empty() {
            self.home_active = true;
        }
        false
    }

    /// Every so often: refresh process names / cwds, re-read stale file listings, and reload
    /// the config if its file changed.
    /// Once startup (or a font change) is over, give the memory it freed back to the OS.
    fn trim_heap_when_due(&mut self) {
        if self.trim_heap_at.is_some_and(|at| Instant::now() >= at) {
            self.trim_heap_at = None;
            crate::platform::trim_heap();
        }
    }

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
        // Only Changes panes (and Files panes on ssh hosts) on screen keep asking.
        let shown = self.cur_tab().map(|t| t.root.panes()).unwrap_or_default();
        for (id, b) in self.explorers.iter_mut() {
            if b.transport().is_none() || shown.contains(id) {
                b.refresh_if_stale();
            }
        }
        for id in shown {
            if let Some(d) = self.diffs.get_mut(&id) {
                d.refresh_if_stale();
            }
        }
        self.track_agents();
        self.autosave_session();
    }

    fn drain_pending(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.pending) {
            self.apply(ctx, action);
        }
        for (pane, request) in std::mem::take(&mut self.explorer_requests) {
            match request {
                file_explorer::Request::OpenTerminal(dir) => self.open_terminal_in(ctx, pane, dir),
                file_explorer::Request::NeedsLogin(host) => self.remote_pane_login_failed(ctx, &host),
            }
        }
        for (pane, request) in std::mem::take(&mut self.diff_requests) {
            match request {
                git_diff::Request::Reveal(path) => self.reveal_from_diff(ctx, pane, &path),
                git_diff::Request::NeedsLogin(host) => self.remote_pane_login_failed(ctx, &host),
            }
        }
        for (_, request) in std::mem::take(&mut self.agent_requests) {
            match request {
                agents::Request::Focus(target) => self.focus_agent(ctx, target),
                agents::Request::Dismiss(target) => self.dismiss_agent(target),
            }
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
            ui.painter().rect_stroke(full, radius, Stroke::new(1.0_f32, WINDOW_BORDER), StrokeKind::Inside);
        }

        if self.cfg.sidebar() {
            let sidebar = Rect::from_min_size(full.min, Vec2::new(chrome::sidebar::SIDEBAR_WIDTH, full.height()));
            self.sidebar(ui, sidebar, &theme);
        } else {
            let titlebar = Rect::from_min_size(full.min, Vec2::new(full.width(), TITLEBAR_HEIGHT));
            self.titlebar(ui, titlebar, &theme);
        }
        let area = self.content_area;
        self.panes_area(ui, area, &theme);
        self.file_drops(&ctx, &theme);
        file_explorer::paint_drag_ghost(&ctx, &theme);
        self.window_resize_edges(&ctx, full);
        self.sharing_dialog(&ctx, &theme);
        self.agent_dialog(&ctx, &theme);
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
    /// `buttons_height` is where the OS's own window buttons sit, when it draws them over
    /// nex's title bar (see `platform::set_window_title`).
    fn sync_window_title(&mut self, ctx: &egui::Context, frame: &eframe::Frame, buttons_height: Option<f32>) {
        let Some(title) = self.focused_pane().map(|id| self.pane_label(id).0).filter(|t| !t.is_empty()) else { return };
        if title != self.window_title {
            crate::platform::set_window_title(ctx, frame, &format!("{title} — nex"), buttons_height);
            self.window_title = title.into_owned();
        }
    }
}

/// Height of a pane's header (taller with the sidebar tab bar), shared with the connect
/// screen's card.
pub(in crate::app) fn header_height(sidebar: bool) -> f32 {
    if sidebar { MOBILE_HEADER_HEIGHT } else { HEADER_HEIGHT }
}

/// A key press, click, touch or typed text: the user is at the window.
fn is_user_input(e: &egui::Event) -> bool {
    use egui::Event;
    matches!(
        e,
        Event::Key { pressed: true, .. }
            | Event::PointerButton { pressed: true, .. }
            | Event::Touch { phase: egui::TouchPhase::Start, .. }
            | Event::Text(_)
            | Event::Paste(_)
    )
}
