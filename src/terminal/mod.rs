//! One terminal "surface": a PTY + child process, the alacritty VT state machine, and the
//! bookkeeping (title, cwd, foreground process) shown in pane headers and tabs.

pub mod input;
pub mod search;
pub mod snapshot;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, Notify, OnResize, WindowSize};
use alacritty_terminal::event_loop::{EventLoop, Msg, Notifier};
use alacritty_terminal::grid::{Dimensions, Scroll};
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::tty;
use alacritty_terminal::vte::ansi::{CursorShape, CursorStyle as AnsiCursorStyle, Rgb};

use crate::config::{Config, CursorStyle};
use crate::render::{self, FrameKey};
use crate::terminal::snapshot::Snapshot;

/// Forwards terminal events from the PTY thread to the UI thread and wakes egui up.
#[derive(Clone)]
pub struct EventProxy {
    tx: Sender<Event>,
    ctx: egui::Context,
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
        if !wakeup || self.visible.load(Ordering::Relaxed) {
            self.ctx.request_repaint();
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
    pub cwd: Option<PathBuf>,
    pub exited: bool,
    pub bell_at: Option<Instant>,
    /// Last time the user typed into this pane (keeps the cursor solid while typing).
    pub last_input: Instant,
    /// Fractional scroll accumulated from smooth (pixel) wheel deltas.
    pub scroll_accum: f32,
    /// Bumped whenever the PTY produced output (used to refresh search results).
    pub generation: u64,
    visible: Arc<AtomicBool>,
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
        // `-e` (a program and its arguments), else `command` from the config, else the shell.
        let command = command.or_else(|| {
            cfg.command.as_ref().map(|cmd| cmd.split_whitespace().map(str::to_string).collect())
        });
        let shell = command.filter(|p| !p.is_empty()).map(|mut parts| {
            let program = parts.remove(0);
            tty::Shell::new(program, parts)
        });
        let env = HashMap::from([
            ("TERM".to_string(), "xterm-256color".to_string()),
            ("COLORTERM".to_string(), "truecolor".to_string()),
            ("TERM_PROGRAM".to_string(), "nex".to_string()),
            ("TERM_PROGRAM_VERSION".to_string(), env!("CARGO_PKG_VERSION").to_string()),
        ]);
        let options = tty::Options {
            shell,
            working_directory: cwd.clone().filter(|p| p.is_dir()),
            drain_on_exit: true,
            env,
        };

        let window_size = window_size(size, cell_px);
        let pty = tty::new(&options, window_size, id)?;
        let pid = pty.child().id();

        let (tx, events) = mpsc::channel();
        let visible = Arc::new(AtomicBool::new(true));
        let proxy = EventProxy { tx, ctx: ctx.clone(), visible: visible.clone() };
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
            cwd,
            exited: false,
            bell_at: None,
            last_input: Instant::now(),
            scroll_accum: 0.0,
            generation: 0,
            visible,
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

    /// Take the last frame's shapes (to keep drawing a closed pane while it fades out).
    pub fn take_shapes(&mut self) -> Vec<egui::Shape> {
        self.frame.take().map(|(_, shapes)| shapes).unwrap_or_default()
    }

    /// Drain events produced by the PTY thread since the last frame.
    pub fn process_events(&mut self, ctx: &egui::Context, cfg: &Config) {
        while let Ok(event) = self.events.try_recv() {
            if matches!(event, Event::Wakeup) {
                self.generation += 1;
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
                Event::Bell => self.bell_at = Some(Instant::now()),
                Event::ChildExit(_) | Event::Exit => self.exited = true,
                Event::MouseCursorDirty | Event::CursorBlinkingChange | Event::Wakeup => {}
            }
        }
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

    /// Update `process` and `cwd` from /proc: we look at the *foreground* process group of
    /// the shell's controlling terminal, so running `node` shows "Node" rather than "bash".
    pub fn refresh_process_info(&mut self) {
        let shell_cwd = std::fs::read_link(format!("/proc/{}/cwd", self.pid)).ok();
        let fg_pid = std::fs::read_to_string(format!("/proc/{}/stat", self.pid))
            .ok()
            .and_then(|stat| {
                // Fields after the `(comm)` part: state ppid pgrp session tty_nr tpgid ...
                let rest = &stat[stat.rfind(')')? + 1..];
                rest.split_whitespace().nth(5)?.parse::<i32>().ok()
            })
            .filter(|&p| p > 0)
            .map(|p| p as u32)
            .unwrap_or(self.pid);

        let comm = std::fs::read_to_string(format!("/proc/{fg_pid}/comm"))
            .or_else(|_| std::fs::read_to_string(format!("/proc/{}/comm", self.pid)))
            .map(|s| s.trim().to_string())
            .unwrap_or_default();
        let fg_cwd = std::fs::read_link(format!("/proc/{fg_pid}/cwd")).ok();

        if !comm.is_empty() {
            self.process = comm;
        }
        if let Some(cwd) = fg_cwd.or(shell_cwd) {
            self.cwd = Some(cwd);
        }
    }

    /// The label shown in pane headers, e.g. "Node ~/Sites/rex-snake".
    pub fn title(&self) -> String {
        if let Some(t) = self.osc_title.as_deref().filter(|t| !t.trim().is_empty()) {
            if self.osc_title_owner == self.process {
                return t.to_string();
            }
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
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        if let Ok(rest) = path.strip_prefix(&home) {
            return if rest.as_os_str().is_empty() {
                "~".into()
            } else {
                format!("~/{}", rest.display())
            };
        }
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
