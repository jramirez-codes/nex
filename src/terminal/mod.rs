//! One terminal "surface": a PTY + child process, the alacritty VT state machine, and the
//! bookkeeping (title, cwd, foreground process) shown in pane headers and tabs.

pub mod input;
pub mod osc7;
pub mod search;
pub mod snapshot;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle as AnsiCursorStyle, Rgb};

use crate::platform;
use crate::config::{Config, CursorStyle};
use crate::render::{self, FrameKey};
use crate::terminal::snapshot::Snapshot;

/// Forwards terminal events from the PTY thread to the UI thread and wakes egui up.
#[derive(Clone)]
pub struct EventProxy {
    tx: Sender<Event>,
    /// Set once the window exists (the first shell is started before it; see
    /// [`Terminal::spawn_early`]). Until then events just queue up for the first frame.
    ctx: Arc<OnceLock<egui::Context>>,
    /// Whether the pane is on screen. New output in a hidden pane (a background tab) doesn't
    /// repaint the window; it's picked up whenever the next frame runs anyway.
    visible: Arc<AtomicBool>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        let wakeup = matches!(event, Event::Wakeup);
        let _ = self.tx.send(event);
        // Everything but plain output (titles, bells, replies the program waits on, exit)
        // is handled promptly even when hidden.
        if (!wakeup || self.visible.load(Ordering::Relaxed))
            && let Some(ctx) = self.ctx.get() {
                ctx.request_repaint();
            }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct GridSize {
    pub cols: usize,
    pub lines: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.lines
    }
    fn screen_lines(&self) -> usize {
        self.lines
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// How long a restored command waits for its shell to be ready before it's typed anyway.
const PENDING_COMMAND_TIMEOUT: Duration = Duration::from_secs(5);

/// One argument as a POSIX (and fish) shell word: bare if plain, else single-quoted. Unlike
/// `platform::shell_quote`, `~` is quoted too: in a saved argv it was already literal.
fn quote_arg(a: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./-+:,@%=".contains(c);
    if !a.is_empty() && a.chars().all(safe) { a.to_string() } else { format!("'{}'", a.replace('\'', r"'\''")) }
}

pub struct Terminal {
    pub term: Arc<FairMutex<Term<EventProxy>>>,
    notifier: Notifier,
    events: Receiver<Event>,
    pid: u32,
    size: GridSize,
    cell_px: (u16, u16),

    /// Title set by the program via OSC 0/2.
    pub osc_title: Option<String>,
    /// Foreground process at the time the title was set; the title only applies while that
    /// process is still in the foreground (so a shell's prompt title doesn't label `node`).
    osc_title_owner: String,
    /// Name of the foreground process (e.g. `fish`, `node`, `nvim`).
    pub process: String,
    /// The foreground process's command line (to reconnect to an `ssh` session's host).
    process_argv: Vec<String>,
    /// The foreground job's pid while the shell is running something (see `ProcInfo::job`).
    job: Option<u32>,
    /// A restored command waiting to be typed into the shell once it is ready, and when it
    /// was queued (see [`Terminal::queue_command`]).
    pending_command: Option<(String, Instant)>,
    pub cwd: Option<PathBuf>,
    /// The directory the shell last reported with OSC 7 (see [`osc7`]).
    osc7: osc7::Slot,
    /// Raised when the program sends a notification OSC (see [`osc7::Alert`]).
    alert: osc7::Alert,
    /// The `nex-host` agent this pane's ssh session is showing, if it said (see [`osc7`]).
    agent_tag: osc7::AgentSlot,
    pub exited: bool,
    pub bell_at: Option<Instant>,
    /// The program rang the bell or sent a notification since the user last looked at this
    /// pane (shown as a dot on its tab).
    pub attention: bool,
    /// Last time the user typed into this pane (keeps the cursor solid while typing).
    pub last_input: Instant,
    /// Fractional scroll accumulated from smooth (pixel) wheel deltas.
    pub scroll_accum: f32,
    /// Bumped whenever the PTY produced output (used to refresh search results).
    pub generation: u64,
    /// When the PTY last produced output (the Agents pane's "working" fallback).
    pub last_output: Instant,
    visible: Arc<AtomicBool>,
    /// The window's egui context, shared with the [`EventProxy`].
    repaint: Arc<OnceLock<egui::Context>>,
    /// The last copy of the screen taken for painting; its buffers are reused.
    snapshot: Option<Snapshot>,
    /// The shapes last drawn for this pane and what they were built from.
    frame: Option<(FrameKey, Vec<egui::Shape>)>,
}

impl Terminal {
    pub fn spawn(
        id: u64,
        cfg: &Config,
        command: Option<Vec<String>>,
        cwd: Option<PathBuf>,
        size: GridSize,
        cell_px: (u16, u16),
        ctx: &egui::Context,
    ) -> std::io::Result<Self> {
        let t = Self::spawn_detached(id, cfg, command, cwd, size, cell_px)?;
        t.attach(ctx);
        Ok(t)
    }

    /// Start the first shell before the window exists, so the shell's own startup (rc files,
    /// prompt) runs while the window, GPU context and fonts are set up. The size is a guess
    /// (the first frame resizes it to fit); [`Terminal::attach`] connects it to the window.
    pub fn spawn_early(cfg: &Config, command: Option<Vec<String>>, cwd: Option<PathBuf>) -> std::io::Result<Self> {
        // Roughly the default window with the default font: JetBrains Mono's advance is about
        // 0.6 em and egui's row height about 1.32 em. Only a starting point.
        let (w, h) = (cfg.font_size * 0.6, cfg.font_size * 1.32 * (1.0 + cfg.cell_height_adjust));
        let size = GridSize { cols: ((1060.0 / w) as usize).max(2), lines: ((600.0 / h) as usize).max(1) };
        Self::spawn_detached(1, cfg, command, cwd, size, (w as u16, h as u16))
    }

    /// Hand the terminal the window's egui context, so its output repaints the window.
    pub fn attach(&self, ctx: &egui::Context) {
        if self.repaint.set(ctx.clone()).is_ok() {
            // Output may already be waiting from before the window existed.
            ctx.request_repaint();
        }
    }

    fn spawn_detached(
        id: u64,
        cfg: &Config,
        command: Option<Vec<String>>,
        cwd: Option<PathBuf>,
        size: GridSize,
        cell_px: (u16, u16),
    ) -> std::io::Result<Self> {
        // `-e` (a program and its arguments), else `command` from the config, else the shell.
        let command = command.or_else(|| {
            cfg.command.as_ref().map(|cmd| cmd.split_whitespace().map(str::to_string).collect())
        });
        let shell = command.filter(|p| !p.is_empty()).map(|mut parts| {
            let program = parts.remove(0);
            tty::Shell::new(program, parts)
        });
        let mut env = HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("TERM_PROGRAM".to_string(), "nex".to_string()),
            ("TERM_PROGRAM_VERSION".to_string(), env!("CARGO_PKG_VERSION").to_string()),
            // What ssh passes on (as LC_*, which most servers accept), so a program on the
            // other end can tell it's in nex: nex-host's shim does (see nex-host/src/shim.rs).
            ("LC_TERMINAL".to_string(), "nex".to_string()),
            ("LC_TERMINAL_VERSION".to_string(), env!("CARGO_PKG_VERSION").to_string()),
        ]);
        env.extend(platform::shell_env());
        let options = platform::pty_options(shell, cwd.clone().filter(|p| p.is_dir()), env);

        let window_size = window_size(size, cell_px);
        let pty = tty::new(&options, window_size, id)?;
        let pid = platform::pty_pid(&pty);
        let osc7 = osc7::Slot::default();
        let repaint = Arc::new(OnceLock::new());
        let alert = osc7::Alert::new(repaint.clone());
        let agent_tag = osc7::AgentSlot::default();
        #[cfg(unix)]
        let pty = osc7::ScanPty::new(pty, osc7.clone(), alert.clone(), agent_tag.clone())?;

        let (tx, events) = mpsc::channel();
        let visible = Arc::new(AtomicBool::new(true));
        let proxy = EventProxy { tx, ctx: repaint.clone(), visible: visible.clone() };
        let term = Arc::new(FairMutex::new(Term::new(term_config(cfg), &size, proxy.clone())));

        let event_loop = EventLoop::new(term.clone(), proxy, pty, true, false)?;
        let notifier = Notifier(event_loop.channel());
        event_loop.spawn();

        let mut t = Self {
            term,
            notifier,
            events,
            pid,
            size,
            cell_px,
            osc_title: None,
            osc_title_owner: String::new(),
            process: String::new(),
            process_argv: Vec::new(),
            job: None,
            pending_command: None,
            cwd,
            osc7,
            alert,
            agent_tag,
            exited: false,
            bell_at: None,
            attention: false,
            last_input: Instant::now(),
            scroll_accum: 0.0,
            generation: 0,
            last_output: Instant::now(),
            visible,
            repaint,
            snapshot: None,
            frame: None,
        };
        t.refresh_process_info();
        Ok(t)
    }

    /// Apply a reloaded config to this running terminal (scrollback and cursor). Colors may
    /// have changed too, so the next frame is drawn from scratch.
    pub fn apply_config(&mut self, cfg: &Config) {
        self.term.lock().set_options(term_config(cfg));
        self.frame = None;
    }

    pub fn size(&self) -> GridSize {
        self.size
    }

    /// Mark the pane as on or off screen (see [`EventProxy`]).
    pub fn set_visible(&self, visible: bool) {
        self.visible.store(visible, Ordering::Relaxed);
    }

    /// The shapes to draw this pane: the last frame's if `key` is unchanged, otherwise
    /// `build` run on a fresh copy of the screen (the lock is held only for the copy).
    pub fn frame_shapes(&mut self, key: FrameKey, build: impl FnOnce(&Snapshot) -> Vec<egui::Shape>) -> &[egui::Shape] {
        if self.frame.as_ref().is_none_or(|(k, _)| *k != key) {
            let previous = self.snapshot.take();
            let snap = self.snapshot.insert(Snapshot::capture(&self.term.lock(), previous));
            self.frame = Some((key, build(snap)));
        }
        self.frame.as_ref().map_or(&[], |(_, shapes)| shapes)
    }

    /// A copy of the last frame's shapes (to draw a pane hidden by zoom while it fades out).
    pub fn shapes(&self) -> Vec<egui::Shape> {
        self.frame.as_ref().map(|(_, shapes)| shapes.clone()).unwrap_or_default()
    }

    /// Take the last frame's shapes (to keep drawing a closed pane while it fades out).
    pub fn take_shapes(&mut self) -> Vec<egui::Shape> {
        self.frame.take().map(|(_, shapes)| shapes).unwrap_or_default()
    }

    /// Drain events produced by the PTY thread since the last frame.
    pub fn process_events(&mut self, ctx: &egui::Context, cfg: &Config) {
        while let Ok(event) = self.events.try_recv() {
            if matches!(event, Event::Wakeup) {
                self.generation += 1;
                self.last_output = Instant::now();
            }
            match event {
                Event::Title(title) => {
                    self.refresh_process_info();
                    self.osc_title_owner = self.process.clone();
                    self.osc_title = Some(title);
                }
                Event::ResetTitle => self.osc_title = None,
                Event::ClipboardStore(_, text) => ctx.copy_text(text),
                Event::ClipboardLoad(..) => {} // Reading the clipboard via OSC 52 is not allowed.
                Event::ColorRequest(index, format) => {
                    let color = {
                        let term = self.term.lock();
                        term.colors()[index]
                    };
                    let rgb = color.unwrap_or_else(|| {
                        let c = render::default_indexed_color(index, cfg);
                        Rgb { r: c.r(), g: c.g(), b: c.b() }
                    });
                    self.write(format(rgb).into_bytes());
                }
                Event::PtyWrite(text) => self.write(text.into_bytes()),
                Event::TextAreaSizeRequest(format) => {
                    let text = format(window_size(self.size, self.cell_px));
                    self.write(text.into_bytes());
                }
                Event::Bell => {
                    self.bell_at = Some(Instant::now());
                    self.attention = true;
                }
                Event::ChildExit(_) | Event::Exit => self.exited = true,
                Event::MouseCursorDirty | Event::CursorBlinkingChange | Event::Wakeup => {}
            }
        }
        if self.alert.take() {
            self.attention = true;
        }
        self.flush_pending_command();
    }

    /// Send raw bytes to the child process.
    pub fn write(&self, bytes: impl Into<std::borrow::Cow<'static, [u8]>>) {
        self.notifier.notify(bytes);
    }

    /// Send user input: snaps the viewport back to the bottom and clears the selection.
    pub fn input(&mut self, bytes: Vec<u8>) {
        {
            let mut term = self.term.lock();
            term.scroll_display(Scroll::Bottom);
            term.selection = None;
        }
        self.last_input = Instant::now();
        self.write(bytes);
    }

    pub fn paste(&mut self, text: &str) {
        let bracketed = self
            .term
            .lock()
            .mode()
            .contains(term::TermMode::BRACKETED_PASTE);
        let text = text.replace("\r\n", "\n").replace('\n', "\r");
        let bytes = if bracketed {
            // Strip any embedded end-of-paste markers so pasted text can't escape the bracket.
            let text = text.replace("\x1b[201~", "");
            format!("\x1b[200~{text}\x1b[201~").into_bytes()
        } else {
            text.into_bytes()
        };
        self.input(bytes);
    }

    pub fn resize(&mut self, size: GridSize, cell_px: (u16, u16)) {
        if size == self.size && cell_px == self.cell_px {
            return;
        }
        self.size = size;
        self.cell_px = cell_px;
        self.notifier.on_resize(window_size(size, cell_px));
        self.term.lock().resize(size);
    }

    pub fn scroll(&mut self, lines: i32) {
        self.term.lock().scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to(&mut self, scroll: Scroll) {
        self.term.lock().scroll_display(scroll);
    }

    pub fn selection_text(&self) -> Option<String> {
        self.term
            .lock()
            .selection_to_string()
            .filter(|s| !s.is_empty())
    }

    /// Update `process` and `cwd` from the shell's foreground process, so running `node`
    /// shows "node" rather than "bash".
    pub fn refresh_process_info(&mut self) {
        let info = platform::proc_info(self.pid);
        if let Some(name) = info.name {
            self.process = name;
            self.process_argv = info.argv;
        }
        self.job = info.job;
        if let Some(cwd) = info.cwd {
            self.cwd = Some(cwd);
        }
    }

    /// The foreground job's pid and command line, as of the last process poll (`None` at the
    /// prompt).
    pub fn foreground_job(&self) -> Option<(u32, &[String])> {
        Some((self.job?, &self.process_argv))
    }

    /// What the shell is running, as of the last process poll, to run again when the session
    /// is restored: the foreground job's command line, or `None` at the prompt. Claude Code
    /// is rewritten to resume the same conversation, and an `ssh` showing an agent `nex-host`
    /// keeps (typed in the pane, or a tab the Agents pane opened, which runs `ssh` itself
    /// with no shell) to attach to it again.
    pub fn running_command(&self) -> Option<Vec<String>> {
        if self.process == "ssh"
            && let Some(cmd) = reattach_command(&self.process_argv, self.agent_session())
        {
            return Some(cmd);
        }
        let job = self.job?;
        let argv = &self.process_argv;
        if argv.first().is_none_or(String::is_empty) {
            return None;
        }
        if crate::session::claude::is_claude(argv) {
            return Some(crate::session::claude::resume_argv(argv, crate::session::claude::session_id(job).as_deref()));
        }
        Some(argv.clone())
    }

    /// Type `argv` into the shell once it is ready: it has drawn something (its prompt) and
    /// nothing else holds the terminal (rc files can run programs that read input). Typed
    /// rather than run in its place, so it lands in history and quitting it leaves the shell.
    pub fn queue_command(&mut self, argv: &[String]) {
        if argv.is_empty() {
            return;
        }
        let line = argv.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
        self.pending_command = Some((line, Instant::now()));
    }

    pub fn has_pending_command(&self) -> bool {
        self.pending_command.is_some()
    }

    /// Send the queued command if the shell is ready, or has had long enough.
    fn flush_pending_command(&mut self) {
        let Some((_, queued)) = &self.pending_command else { return };
        let overdue = queued.elapsed() >= PENDING_COMMAND_TIMEOUT;
        if !overdue {
            if self.generation == 0 {
                return;
            }
            self.refresh_process_info();
            if self.job.is_some() {
                return;
            }
        }
        if let Some((line, _)) = self.pending_command.take()
            && !self.exited
        {
            self.write(format!("{line}\r").into_bytes());
        }
    }

    /// The host this pane's foreground `ssh` is connected to, if that is what it's running.
    /// Refreshes the process info first, so an `ssh` started a moment ago counts.
    pub fn ssh_target(&mut self) -> Option<crate::remote::SshTarget> {
        self.refresh_process_info();
        if self.process != "ssh" {
            return None;
        }
        crate::remote::SshTarget::from_argv(&self.process_argv)
    }

    /// Like [`ssh_target`](Self::ssh_target), but from the last process poll, without asking
    /// the OS again: cheap enough to call every frame.
    pub fn last_ssh_target(&self) -> Option<crate::remote::SshTarget> {
        if self.process != "ssh" {
            return None;
        }
        crate::remote::SshTarget::from_argv(&self.process_argv)
    }

    /// The directory this pane's `ssh` session is in on the remote host, as far as nex can
    /// tell: what the remote shell last reported with OSC 7, else the directory in a
    /// `user@host: dir` title. `None` when the remote side says neither.
    pub fn remote_cwd(&self) -> Option<String> {
        let report = self.osc7.lock().ok().and_then(|r| r.clone());
        if let Some(r) = report.filter(|r| !is_local_host(&r.host)) {
            return Some(r.path);
        }
        if self.osc_title_owner != "ssh" {
            return None;
        }
        self.osc_title.as_deref().and_then(osc7::title_dir).map(str::to_string)
    }

    /// The `nex-host` session this pane shows (it's running `ssh` to a host where
    /// `nex-host attach` said so), from the last process poll.
    pub fn agent_session(&self) -> Option<String> {
        if self.process != "ssh" {
            return None;
        }
        self.agent_tag.lock().ok().and_then(|t| t.clone())
    }

    /// The label shown in pane headers, e.g. "Node ~/Sites/rex-snake".
    pub fn title(&self) -> String {
        if let Some(t) = self.osc_title.as_deref().filter(|t| !t.trim().is_empty())
            && self.osc_title_owner == self.process {
                return t.to_string();
            }
        let mut name = self.process.clone();
        if let Some(first) = name.get_mut(0..1) {
            first.make_ascii_uppercase();
        }
        match &self.cwd {
            Some(cwd) => format!("{name} {}", tilde(cwd)),
            None => name,
        }
    }
}

impl Drop for Terminal {
    fn drop(&mut self) {
        // Stops the PTY thread; dropping the PTY sends SIGHUP to the child.
        let _ = self.notifier.0.send(Msg::Shutdown);
    }
}

fn window_size(size: GridSize, cell_px: (u16, u16)) -> WindowSize {
    WindowSize {
        num_lines: size.lines as u16,
        num_cols: size.cols as u16,
        cell_width: cell_px.0,
        cell_height: cell_px.1,
    }
}

pub fn tilde(path: &std::path::Path) -> String {
    if let Some(home) = platform::home_dir()
        && let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            };
        }
    path.display().to_string()
}

/// The emulator settings that come from the user's config.
fn term_config(cfg: &Config) -> term::Config {
    let shape = match cfg.cursor_style {
        CursorStyle::Block => CursorShape::Block,
        CursorStyle::Bar => CursorShape::Beam,
        CursorStyle::Underline => CursorShape::Underline,
    };
    term::Config {
        scrolling_history: cfg.scrollback_limit,
        default_cursor_style: AnsiCursorStyle { shape, blinking: cfg.cursor_blink },
        ..Default::default()
    }
}

/// Whether an OSC 7 host names this machine (an empty host means the shell left it out,
/// which only a local shell would do).
fn is_local_host(host: &str) -> bool {
    let short = |h: &str| h.split('.').next().unwrap_or_default().to_ascii_lowercase();
    host.is_empty() || host == "localhost" || short(host) == short(&platform::hostname())
}

/// The command line that attaches to the `nex-host` agent an `ssh` running `argv` shows
/// again: the one `nex-host attach` reported (`tagged`), else the one its command line names.
fn reattach_command(argv: &[String], tagged: Option<String>) -> Option<Vec<String>> {
    let target = crate::remote::SshTarget::from_argv(argv)?;
    let session = tagged.or_else(|| crate::remote::SshTarget::attach_id(argv))?;
    Some(target.normalized().attach_command(&session))
}

#[cfg(test)]
mod tests {
    use super::{quote_arg, reattach_command};

    #[test]
    fn an_ssh_showing_an_agent_reattaches_to_it() {
        let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let attach = argv(&["ssh", "-t", "-o", "RemoteCommand=none", "--", "me@devbox", "~/.nex/bin/nex-host attach 3fa91c0e"]);
        // A tab the Agents pane opened: its own command line names the agent.
        assert_eq!(reattach_command(&attach, None), Some(attach.clone()));
        // `claude` typed in a plain ssh pane, started through the shim.
        let plain = argv(&["ssh", "-p", "2222", "me@devbox"]);
        let expect = argv(&["ssh", "-t", "-o", "RemoteCommand=none", "-p", "2222", "--", "me@devbox", "~/.nex/bin/nex-host attach 3fa91c0e"]);
        assert_eq!(reattach_command(&plain, Some("3fa91c0e".into())), Some(expect));
        // A plain ssh session with no agent is left to run as it is.
        assert_eq!(reattach_command(&plain, None), None);
    }

    #[test]
    fn quotes_only_what_needs_it() {
        let line = |argv: &[&str]| argv.iter().map(|a| quote_arg(a)).collect::<Vec<_>>().join(" ");
        assert_eq!(line(&["npm", "run", "dev"]), "npm run dev");
        assert_eq!(line(&["python3", "-m", "http.server", "--bind=127.0.0.1"]), "python3 -m http.server --bind=127.0.0.1");
        assert_eq!(line(&["echo", "a b", "it's", ""]), r"echo 'a b' 'it'\''s' ''");
        assert_eq!(line(&["ls", "~", "$HOME", "*.rs"]), "ls '~' '$HOME' '*.rs'");
    }
}
