//! The application: window chrome (title bar + tabs), split panes, input handling.
//! Each concern lives in its own submodule; this file holds the state and the frame loop:
//! `input/` (keys, mouse, zoom), `chrome/` (title bar, sidebar, window), `workspace/` (tabs and
//! panes), `animation/` (motion and tab switches), `ssh/` (remote hosts). `actions` is what
//! the user can ask for, shared by input and chrome; `launch` is what the command line hands
//! the first panes. `App`'s state is grouped the same way (`InputState`, `Chrome`,
//! `LaunchState`, `Timers`).

mod actions;
mod animation;
mod chrome;
mod connect;
mod input;
mod launch;
mod reload;
pub mod restore;
mod ssh;
#[cfg(test)]
mod tests;
mod workspace;

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime};

use egui::{Color32, Pos2, Rect, Stroke, StrokeKind, Vec2, ViewportCommand};

use crate::agents;
use crate::config::{self, Config};
use crate::file_explorer;
use crate::git_diff;
use crate::layout::PaneId;
use crate::render::CellMetrics;
use crate::render::fonts;
use crate::render::glyphs::GlyphCache;
use crate::terminal::Terminal;
use crate::ui::theme::{Theme, white_alpha};
use crate::ui::toast::{self, Toast};
use crate::ui::widgets::is_maximized;

use actions::Action;
use animation::tab_switch::TabSwitch;
use input::keymap::{Binding, keybindings};
use workspace::panes::{Pane, PaneRequest, Panes};
use workspace::tab::Tab;

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
/// A new shell opened in place of an ended last tab (`App::replace_ended_tab`) that ends
/// within this long isn't replaced again.
const REPLACE_WINDOW: Duration = Duration::from_secs(5);

pub struct App {
    cfg: Config,
    /// Every pane (terminal, file explorer, Changes, Agents), in one id space and split tree
    /// (see `workspace/panes.rs`).
    panes: Panes,
    /// What panes asked for while painting, applied with the pending actions.
    pane_requests: Vec<(PaneId, PaneRequest)>,
    tabs: Vec<Tab>,
    active: usize,
    next_id: PaneId,
    /// Per-pane font size overrides (zoom is per pane); panes not listed use the config size.
    pane_fonts: HashMap<PaneId, f32>,
    /// Cell metrics for every font size in use this frame, keyed by `f32::to_bits`.
    metrics: HashMap<u32, CellMetrics>,
    pending: Vec<Action>,
    /// Pointer, touch, zoom and search state (see `input/`).
    input: input::InputState,
    /// The tab strip, keyboard button, toasts and window title (see `chrome/`).
    chrome: chrome::Chrome,
    /// What the command line and connect screen hand to the first panes (see `launch.rs`).
    launch: launch::LaunchState,
    timers: Timers,
    /// Terminal (content) rects from the current frame, used for mouse hit-testing.
    term_rects: Vec<(PaneId, Rect)>,
    /// Card rects (including header), used for directional focus.
    card_rects: Vec<(PaneId, Rect)>,
    /// The lists drawing the panes fills each frame, kept for their allocations.
    draw_bufs: workspace::draw::DrawBuffers,
    /// Where the layout put the focused pane's card this frame, and how it looked (the
    /// connect handoff lands there).
    focused_card: Option<animation::handoff::FocusedCard>,
    /// The connect screen handing over to the panes, while it plays (`animation/handoff.rs`).
    handoff: Option<animation::handoff::Handoff>,
    hostname: String,
    bindings: Vec<Binding>,
    /// Area available for panes in the current window (used to size new terminals).
    content_area: Rect,
    /// Where each character's bitmap is in egui's font atlas (shared by all panes).
    glyphs: GlyphCache,
    /// The tab-switch transition (`tab-animation`): the old tab's panes giving way to the new.
    switch: TabSwitch,
    /// Every agent running in a pane and what it's doing, as of the last process poll; what
    /// Agents panes and the title bar's Agents label show.
    agent_tracker: agents::model::Tracker,
    /// Which agent changes get a system notification (`system-notifications`).
    agent_notifier: agents::notify::Notifier,
    /// The home tab, outside `tabs`: the Files tab (the title-bar directory label is its
    /// tab), or the Agents tab when it holds an Agents pane (see `home_is_files`). Saved in
    /// the session as `explorer`.
    home_tab: Option<Tab>,
    /// Whether the home tab is showing (`active` keeps the last terminal tab).
    home_active: bool,
    /// The tabs shown before this one, for Back (see `workspace/history.rs`).
    history: workspace::history::TabHistory,
    /// The system Back key goes back a tab (`platform::SYSTEM_BACK`; tests turn it on).
    system_back: bool,
    /// The sticky pane: it follows the user from tab to tab, a leaf in every tab's split
    /// tree (each tab keeps its own place and size for it). See `workspace/sticky.rs`.
    sticky: Option<PaneId>,
    /// Its card gliding from where the last tab had it to where this one does.
    sticky_glide: animation::sticky_glide::StickyGlide,
    /// Agent panes' header titles rolling over when they change (`animation/title_roll.rs`).
    title_rolls: animation::title_roll::TitleRolls,
    /// A header's long title scrolling sideways while it's held (`animation/title_scroll.rs`).
    title_scroll: animation::title_scroll::TitleScroll,
    /// Drag and drop with other windows, where the platform needs nex to do it (Wayland).
    dnd: Option<crate::platform::Dnd>,
    /// Uploads, the shared mic, watched agents and their dialogs (see `ssh/`).
    ssh: ssh::SshState,
    /// Restoring the last session and keeping the session file (see `restore.rs`).
    session: restore::SessionState,
    /// The host every pane works on, where this window is a thin client (Android,
    /// `--remote`), and the connect screen shown until a pane is open (see `connect.rs`).
    connect: connect::Connect,
    /// The pane showing each sub-agent opened from the Agents pane, by its card's key (see
    /// `workspace/agents.rs`).
    sub_views: std::collections::HashMap<crate::layout::PaneId, crate::layout::PaneId>,
    /// The folder on the host the next pane opened there starts in, when it isn't the
    /// focused pane's (a Files pane's "Open Terminal Here").
    next_remote_cwd: Option<String>,
}

/// When the frame loop's periodic work is next due.
struct Timers {
    /// When process names, working directories and file listings were last refreshed.
    last_proc_poll: Instant,
    /// When to hand memory freed during startup (font loading) back to the OS; `None` once done.
    trim_heap_at: Option<Instant>,
    /// Modification time of the config file when it was last read; a change triggers a reload.
    config_mtime: Option<SystemTime>,
}

impl App {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        cfg: Config,
        args: crate::cli::Args,
        early_terminal: Option<Terminal>,
        session: Option<crate::session::Session>,
        session_lock: Option<crate::session::Lock>,
    ) -> Self {
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
        let launch = launch::LaunchState::new(args, early_terminal);
        Self {
            cfg,
            panes: Panes::default(),
            pane_requests: Vec::new(),
            tabs: Vec::new(),
            active: 0,
            next_id: 1,
            pane_fonts: HashMap::new(),
            metrics: HashMap::new(),
            pending: Vec::new(),
            input: input::InputState::default(),
            chrome: chrome::Chrome::default(),
            launch,
            timers: Timers { last_proc_poll: Instant::now(), trim_heap_at: Some(Instant::now() + TRIM_HEAP_DELAY), config_mtime: config::modified() },
            term_rects: Vec::new(),
            card_rects: Vec::new(),
            draw_bufs: Default::default(),
            focused_card: None,
            handoff: None,
            hostname,
            bindings: keybindings(),
            content_area: Rect::NOTHING,
            glyphs: GlyphCache::default(),
            switch: TabSwitch::default(),
            agent_tracker: agents::model::Tracker::default(),
            agent_notifier: agents::notify::Notifier::default(),
            home_tab: None,
            home_active: false,
            history: Default::default(),
            system_back: crate::platform::SYSTEM_BACK,
            sticky: None,
            sticky_glide: Default::default(),
            title_rolls: Default::default(),
            title_scroll: Default::default(),
            dnd: crate::platform::Dnd::new(cc),
            ssh: ssh::SshState {
                uploads: Default::default(),
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
            connect: connect::Connect::new(remote_window, remote_arg.as_deref()),
            sub_views: std::collections::HashMap::new(),
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

    /// Files dragged in from another window, reported as winit does on X11, and the system
    /// Back key (see `workspace/history.rs`).
    fn raw_input_hook(&mut self, ctx: &egui::Context, raw_input: &mut egui::RawInput) {
        self.system_back(ctx, raw_input);
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
                            raw_input.events.push(egui::Event::Key {
                                key: egui::Key::Backspace,
                                physical_key: None,
                                pressed,
                                repeat: false,
                                modifiers,
                            });
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
        if !self.launch.started && !self.connect.active {
            // The first shell is spawned on the first frame, once the window size is known.
            self.launch.started = true;
            let restored = self.session.restore_at_start.take().is_some_and(|s| self.restore_session(ctx, s));
            if !restored {
                let cwd = std::env::current_dir().ok();
                self.new_tab(ctx, cwd, None);
            }
            if self.launch.open_explorer {
                // Opened as a file manager: the Files tab, at the first shell's folder.
                self.toggle_explorer(ctx);
                if let Some(item) = self.launch.select_at_start.take() {
                    self.select_in_explorer(&item);
                }
            }
        }

        // A thin client (Android, `--remote`) with nothing open: connect, then open the panes.
        if self.connect.active && self.tabs.is_empty() {
            self.connect_screen(ctx);
            // Just connected: the workspace draws in this same frame, under the handoff, so no
            // empty frame comes between the connect screen's last and the workspace's first.
            if self.tabs.is_empty() {
                return;
            }
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
        if let Some(n) = self.chrome.notice.take() {
            self.chrome.toast = Some(Toast::new(ctx, n));
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
        if self.chrome.soft_keyboard {
            self.ask_for_keyboard(ctx);
        }
        crate::platform::mirror_copies(ctx);
        crate::platform::sync_soft_keyboard(ctx);
        if !self.pending.is_empty() {
            self.drain_pending(ctx);
            ctx.request_repaint();
        }
        self.note_place();
        let buttons_height =
            (self.cfg.window_decorations && crate::platform::WINDOW.overlays_titlebar()).then(|| TITLEBAR_HEIGHT * ctx.zoom_factor());
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
            self.launch.adopt = shells;
            // Oldest first, as they were opened.
            self.launch.adopt.reverse();
            for _ in 0..tabs {
                self.new_tab(ctx, None, None);
            }
            if crate::ui::glide::animations(ctx)
                && let Some(drawn) = self.connect.last_card()
            {
                let start = ctx.input(|i| i.time);
                let drawn = drawn.clone();
                self.handoff = Some(animation::handoff::Handoff { start, drawn, sidebar: self.cfg.sidebar(), painted: 0 });
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
        if self.chrome.keyboard_button.down(target) {
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
        for (id, t) in self.panes.terminals_mut() {
            let had = t.attention;
            t.process_events(ctx, &self.cfg);
            alerted |= t.attention && !had;
            if t.exited {
                exited.push(id);
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
        if let Some(t) = seen.and_then(|id| self.panes.terminal_mut(id)) {
            t.attention = false;
        }
        self.agent_tracker.mark_seen(seen, std::time::Instant::now());
        // An agent finished or wants input while nex is in the background: flag it in the
        // dock/taskbar, as a bell does.
        if self.agent_tracker.take_alerted() && !window_focused {
            ctx.send_viewport_cmd(ViewportCommand::RequestUserAttention(egui::UserAttentionType::Informational));
        }
        self.send_agent_notifications(window_focused, seen);
    }

    /// With `system-notifications` on, a system notification for each agent that started
    /// needing you and each pipeline run that finished, unless it's on screen. Off, the
    /// notifier starts over, so turning it on doesn't send what was already so.
    fn send_agent_notifications(&mut self, focused: bool, pane: Option<PaneId>) {
        if !self.cfg.system_notifications {
            self.agent_notifier = agents::notify::Notifier::default();
            return;
        }
        let looking = agents::notify::Looking { focused, agents_tab: self.home_active && !self.home_is_files(), pane };
        let list = self.agent_tracker.agents();
        for note in self.agent_notifier.update(list, list.generation(), looking) {
            crate::platform::notify(&note.title, &note.body);
        }
    }

    /// Close the window once no tabs are left. With only the Files tab left (or an Agents tab
    /// with a terminal or explorer split into it), show it. Returns `true` if the window is
    /// closing.
    fn close_if_empty(&mut self, ctx: &egui::Context) -> bool {
        if self.tabs.is_empty() && !self.home_keeps_window() {
            if self.replace_ended_tab(ctx) {
                return false;
            }
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

    /// A thin client (Android, `--remote`) whose last tab ended on the host without the user
    /// closing it: its agent was stopped from another connection while the phone slept, or
    /// its shell was killed. Rather than close the window (which an Android app can't open
    /// again without being restarted), open a new shell on the host in its place. Should that
    /// one end right away too, go back to the connect screen instead of trying again and
    /// again. Returns `true` if the window stays open.
    fn replace_ended_tab(&mut self, ctx: &egui::Context) -> bool {
        let Some(host) = self.connect.host.clone().filter(|_| self.connect.active && !self.session.closed_by_user) else { return false };
        let now = Instant::now();
        let again = self.connect.replaced_at.is_some_and(|at| now.duration_since(at) < REPLACE_WINDOW);
        self.connect.replaced_at = Some(now);
        if !again {
            self.new_tab(ctx, None, None);
        }
        if self.tabs.is_empty() {
            self.connect.say(format!("The shell on {} ended right away. Connect to try again.", host.server.host));
        }
        true
    }

    /// Once startup (or a font change) is over, give the memory it freed back to the OS.
    fn trim_heap_when_due(&mut self) {
        if self.timers.trim_heap_at.is_some_and(|at| Instant::now() >= at) {
            self.timers.trim_heap_at = None;
            crate::platform::trim_heap();
        }
    }

    /// Every so often: refresh process names / cwds, re-read stale file listings, and reload
    /// the config if its file changed.
    fn poll_background(&mut self) {
        if self.timers.last_proc_poll.elapsed() <= PROCESS_POLL_INTERVAL {
            return;
        }
        self.timers.last_proc_poll = Instant::now();
        if config::modified() != self.timers.config_mtime {
            self.pending.push(Action::ReloadConfig);
        }
        for (_, t) in self.panes.terminals_mut() {
            t.refresh_process_info();
        }
        // Only Files and Changes panes on screen keep asking.
        let shown = self.cur_tab().map(|t| t.root.panes()).unwrap_or_default();
        for id in shown {
            match self.panes.get_mut(id) {
                Some(Pane::Explorer(b)) => b.refresh_if_stale(),
                Some(Pane::Diff(d)) => d.refresh_if_stale(),
                Some(Pane::Terminal(_) | Pane::Agents(_)) | None => {}
            }
        }
        self.track_agents();
        self.autosave_session();
    }

    fn drain_pending(&mut self, ctx: &egui::Context) {
        for action in std::mem::take(&mut self.pending) {
            self.apply(ctx, action);
        }
        for (pane, request) in std::mem::take(&mut self.pane_requests) {
            match request {
                PaneRequest::Explorer(file_explorer::Request::OpenTerminal(dir)) => self.open_terminal_in(ctx, pane, dir),
                PaneRequest::Diff(git_diff::Request::Reveal(path)) => self.reveal_from_diff(ctx, pane, &path),
                PaneRequest::Explorer(file_explorer::Request::NeedsLogin(host)) | PaneRequest::Diff(git_diff::Request::NeedsLogin(host)) => {
                    self.remote_pane_login_failed(ctx, &host)
                }
                PaneRequest::Agents(agents::Request::Focus(target)) => self.focus_agent(ctx, target),
                PaneRequest::Agents(agents::Request::Dismiss(target)) => self.dismiss_agent(target),
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
        if self.chrome.toast.as_ref().is_some_and(|t| !t.paint(&ctx, area, &theme)) {
            self.chrome.toast = None;
        }
    }

    /// Confirm a terminal copy or paste with a toast (unless turned off in the config).
    fn clipboard_toast(&mut self, ctx: &egui::Context, verb: &str, text: &str) {
        if self.cfg.clipboard_toasts && !text.is_empty() {
            self.chrome.toast = Some(Toast::new(ctx, toast::describe(verb, text)));
        }
    }

    /// The window title follows the focused pane.
    /// `buttons_height` is where the OS's own window buttons sit, when it draws them over
    /// nex's title bar (see `platform::set_window_title`).
    fn sync_window_title(&mut self, ctx: &egui::Context, frame: &eframe::Frame, buttons_height: Option<f32>) {
        let Some(title) = self.focused_pane().map(|id| self.pane_label(id).0).filter(|t| !t.is_empty()) else { return };
        if title != self.chrome.window_title {
            crate::platform::set_window_title(ctx, frame, &format!("{title} — nex"), buttons_height);
            self.chrome.window_title = title.into_owned();
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
