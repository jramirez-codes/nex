//! One agent the daemon holds: its program in a pty of its own, and an `alacritty_terminal`
//! screen fed with everything it prints, so a pane attaching later sees what it would have
//! seen. At most one pane is attached at a time; output goes to it as it comes.

use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::ExitStatusExt;
use std::path::PathBuf;
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

use nex_host::clis::{self, Running};
use nex_host::proto::{self, Session, Status};

use crate::daemon::{deliver, pty, redraw};
use crate::util::{Locked, now_ms};

/// Lines of scrollback kept for a pane that attaches later.
const HISTORY: usize = 10_000;
/// How long `kill` waits after SIGHUP before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(3);

/// What a pane's connection is sent.
pub enum Out {
    Bytes(Vec<u8>),
    /// A last line to show, then the connection closes.
    Close(String),
}

/// The pane attached to an agent.
struct Client {
    token: u64,
    tx: Sender<Out>,
}

/// Title and bell, as the screen reports them while it parses output.
#[derive(Default)]
struct Meta {
    title: Option<String>,
    bell: bool,
}

/// Hears what the screen wants while it parses: the title, the bell, and answers to the
/// program's questions (cursor position, device attributes). While a pane is attached its
/// own terminal answers those, so they're only sent when nobody is.
struct Listener {
    meta: Arc<Mutex<Meta>>,
    attached: Arc<AtomicBool>,
    pty: Arc<File>,
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(t) => self.meta.locked().title = Some(t),
            Event::ResetTitle => self.meta.locked().title = None,
            Event::Bell if !self.attached.load(Ordering::Relaxed) => self.meta.locked().bell = true,
            Event::PtyWrite(s) if !self.attached.load(Ordering::Relaxed) => {
                let _ = (&*self.pty).write_all(s.as_bytes());
            }
            _ => {}
        }
    }
}

struct Screen {
    term: Term<Listener>,
    parser: Processor<StdSyncHandler>,
    cols: u16,
    rows: u16,
}

/// Everything that changes, behind one lock so output and attaching don't interleave.
struct Inner {
    screen: Screen,
    child: Option<Child>,
    client: Option<Client>,
    exit: Option<i32>,
}

pub struct Agent {
    pub id: String,
    pub kind: String,
    pub name: String,
    pub cwd: String,
    pub argv: Vec<String>,
    pub pid: u32,
    pub started: u64,
    /// The pty's master side, for keys and resizes.
    pty: Arc<File>,
    meta: Arc<Mutex<Meta>>,
    attached: Arc<AtomicBool>,
    last_output: AtomicU64,
    last_input: AtomicU64,
    /// `kill` was asked for: forget the session once it ends.
    pub killed: AtomicBool,
    /// When it ended (unix ms), for exited sessions to expire.
    pub ended_at: AtomicU64,
    /// The agent that spawned it, and what for.
    pub parent: Option<String>,
    pub role: Option<String>,
    /// When it was last sent a message (unix ms).
    pub last_message: AtomicU64,
    /// Messages to type in, one after another (see [`Agent::type_in`]).
    typer: Sender<String>,
    inner: Mutex<Inner>,
}

/// What to start.
pub struct Spec {
    pub argv: Vec<String>,
    pub kind: Option<String>,
    pub name: Option<String>,
    pub cwd: PathBuf,
    pub cols: u16,
    pub rows: u16,
    /// Run with exactly this environment, rather than under the login shell.
    pub env: Option<Vec<(String, String)>>,
    pub parent: Option<String>,
    pub role: Option<String>,
}

impl Agent {
    /// Start `spec` in a new pty, and a thread reading its output for as long as it runs.
    /// `on_end` runs on that thread once it has ended.
    pub fn spawn(id: String, spec: Spec, on_end: impl FnOnce(&Arc<Agent>) + Send + 'static) -> io::Result<Arc<Agent>> {
        // No command: the login shell itself.
        let base = match spec.argv.first() {
            Some(program) => program.rsplit('/').next().unwrap_or(program).to_string(),
            None => proto::SHELL_KIND.to_string(),
        };
        let kind = spec.kind.clone().unwrap_or_else(|| detect_kind(&spec.argv).unwrap_or(&base).to_string());
        let cwd = if spec.cwd.is_dir() { spec.cwd.clone() } else { PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())) };
        let name = spec
            .name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .unwrap_or_else(|| cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| kind.clone()));
        let (cols, rows) = (spec.cols.max(2), spec.rows.max(1));

        let (master, child) = pty::spawn_in_pty(&spec.argv, spec.env.as_deref(), &cwd, cols, rows, pty::agent_env(&id, &name))?;
        let pid = child.id();
        let pty = Arc::new(master);
        let meta = Arc::new(Mutex::new(Meta::default()));
        let attached = Arc::new(AtomicBool::new(false));
        let listener = Listener { meta: meta.clone(), attached: attached.clone(), pty: pty.clone() };
        let config = term::Config { scrolling_history: HISTORY, kitty_keyboard: true, ..Default::default() };
        let term = Term::new(config, &TermSize::new(cols as usize, rows as usize), listener);
        let now = now_ms();
        let (typer, typed) = mpsc::channel::<String>();
        let agent = Arc::new(Agent {
            id,
            kind,
            name,
            cwd: cwd.to_string_lossy().into_owned(),
            argv: spec.argv,
            pid,
            started: now,
            pty: pty.clone(),
            meta,
            attached,
            last_output: AtomicU64::new(now),
            last_input: AtomicU64::new(0),
            killed: AtomicBool::new(false),
            ended_at: AtomicU64::new(0),
            parent: spec.parent,
            role: spec.role,
            last_message: AtomicU64::new(0),
            typer,
            inner: Mutex::new(Inner { screen: Screen { term, parser: Processor::new(), cols, rows }, child: Some(child), client: None, exit: None }),
        });

        // Typing messages in: each one a paste, then Enter a moment later (straight after it,
        // TUIs such as Codex take the Enter as part of the paste), then a pause so the next
        // one is a turn of its own.
        let weak = Arc::downgrade(&agent);
        std::thread::Builder::new().name(format!("typer-{}", agent.id)).spawn(move || {
            for text in typed {
                let Some(agent) = weak.upgrade() else { break };
                if agent.exited() {
                    break;
                }
                let bracketed = agent.inner.locked().screen.term.mode().contains(term::TermMode::BRACKETED_PASTE);
                let _ = (&*agent.pty).write_all(&deliver::paste(&text, bracketed));
                drop(agent);
                std::thread::sleep(Duration::from_millis(200));
                let Some(agent) = weak.upgrade() else { break };
                let _ = (&*agent.pty).write_all(b"\r");
                drop(agent);
                std::thread::sleep(Duration::from_millis(300));
            }
        })?;

        let reader = agent.clone();
        let mut out = pty.try_clone()?;
        std::thread::Builder::new().name(format!("agent-{}", agent.id)).spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                match out.read(&mut buf) {
                    Ok(0) => break,
                    Ok(n) => reader.output(&buf[..n]),
                    Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                    // EIO: every process holding the pty's other side has closed it.
                    Err(_) => break,
                }
            }
            reader.end();
            on_end(&reader);
        })?;
        Ok(agent)
    }

    fn output(&self, bytes: &[u8]) {
        let mut inner = self.inner.locked();
        let Inner { screen, client, .. } = &mut *inner;
        screen.parser.advance(&mut screen.term, bytes);
        if let Some(c) = client
            && c.tx.send(Out::Bytes(bytes.to_vec())).is_err()
        {
            *client = None;
            self.attached.store(false, Ordering::Relaxed);
        }
        self.last_output.store(now_ms(), Ordering::Relaxed);
    }

    /// Its output has ended: wait for the program, and tell the attached pane.
    fn end(&self) {
        let mut inner = self.inner.locked();
        let code = inner.child.take().and_then(|mut c| c.wait().ok()).map(|s| s.code().unwrap_or_else(|| 128 + s.signal().unwrap_or(0)));
        let code = code.unwrap_or(-1);
        inner.exit = Some(code);
        if let Some(c) = inner.client.take() {
            let _ = c.tx.send(Out::Close(format!("[{} exited{}]", self.kind, if code == 0 { String::new() } else { format!(" with code {code}") })));
        }
        self.attached.store(false, Ordering::Relaxed);
        self.ended_at.store(now_ms(), Ordering::Relaxed);
    }

    pub fn exited(&self) -> bool {
        self.inner.locked().exit.is_some()
    }

    /// Show this agent on a pane's connection, `cols` by `rows`: the pane it was on (if any)
    /// is let go, `ack` (the reply to the pane's request) is sent, the screen is resized to
    /// fit and sent, then output flows. Returns the token that identifies this attachment,
    /// or `None` (with nothing sent) if the agent has ended.
    ///
    /// `ack` goes out under the same lock as the takeover, so once a pane has its reply it
    /// is the one attached: a pane attaching after it takes over from it, never the reverse.
    pub fn attach(&self, tx: &Sender<Out>, ack: Vec<u8>, token: u64, cols: u16, rows: u16) -> Option<u64> {
        let mut inner = self.inner.locked();
        if inner.exit.is_some() {
            return None;
        }
        if let Some(old) = inner.client.take() {
            let _ = old.tx.send(Out::Close("[attached in another pane]".into()));
        }
        let _ = tx.send(Out::Bytes(ack));
        self.resize_locked(&mut inner, cols, rows);
        let title = self.meta.locked().title.clone();
        let _ = tx.send(Out::Bytes(redraw::screen(&inner.screen.term, title.as_deref())));
        inner.client = Some(Client { token, tx: tx.clone() });
        self.attached.store(true, Ordering::Relaxed);
        self.meta.locked().bell = false;
        Some(token)
    }

    /// Let go of the pane attached with `token`, if it still is.
    pub fn detach(&self, token: u64) {
        let mut inner = self.inner.locked();
        if inner.client.as_ref().is_some_and(|c| c.token == token) {
            inner.client = None;
            self.attached.store(false, Ordering::Relaxed);
        }
    }

    /// Keys from the pane attached with `token` (a pane that has been taken over is ignored).
    pub fn input(&self, token: u64, bytes: &[u8]) {
        if !self.is_current(token) {
            return;
        }
        let _ = (&*self.pty).write_all(bytes);
        self.last_input.store(now_ms(), Ordering::Relaxed);
    }

    /// Type `text` in as a message, whoever is attached (it's queued behind any before it).
    /// It doesn't count as the user typing.
    pub fn type_in(&self, text: String) {
        let _ = self.typer.send(text);
        self.last_message.store(now_ms(), Ordering::Relaxed);
    }

    /// Its last `lines` lines (scrollback included) as plain text, wrapped lines joined and
    /// trailing blank lines left off.
    pub fn text(&self, lines: usize) -> String {
        let inner = self.inner.locked();
        let grid = inner.screen.term.grid();
        let (history, screen, cols) = (grid.history_size() as i32, grid.screen_lines() as i32, grid.columns());
        let mut out: Vec<String> = Vec::new();
        let mut joining = false;
        for line in -history..screen {
            let row = &grid[Line(line)];
            let mut s = String::new();
            for c in 0..cols {
                let cell = &row[Column(c)];
                if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    s.push(cell.c);
                }
            }
            let wraps = row[Column(cols - 1)].flags.contains(Flags::WRAPLINE);
            let s = if wraps { s } else { s.trim_end().to_string() };
            match out.last_mut() {
                Some(last) if joining => last.push_str(&s),
                _ => out.push(s),
            }
            joining = wraps;
        }
        while out.last().is_some_and(|l| l.trim().is_empty()) {
            out.pop();
        }
        let from = out.len().saturating_sub(lines);
        out[from..].join("\n")
    }

    pub fn resize(&self, token: u64, cols: u16, rows: u16) {
        let mut inner = self.inner.locked();
        if inner.client.as_ref().is_some_and(|c| c.token == token) {
            self.resize_locked(&mut inner, cols, rows);
        }
    }

    fn is_current(&self, token: u64) -> bool {
        self.inner.locked().client.as_ref().is_some_and(|c| c.token == token)
    }

    fn resize_locked(&self, inner: &mut Inner, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if (cols, rows) == (inner.screen.cols, inner.screen.rows) {
            return;
        }
        inner.screen.term.resize(TermSize::new(cols as usize, rows as usize));
        inner.screen.cols = cols;
        inner.screen.rows = rows;
        pty::set_size(self.pty.as_raw_fd(), cols, rows);
    }

    /// Hang up on its program (its whole process group), then kill it if it hasn't ended
    /// after a grace period.
    pub fn kill(self: &Arc<Self>) {
        self.killed.store(true, Ordering::Relaxed);
        let pgid = self.pid as i32;
        unsafe { libc::kill(-pgid, libc::SIGHUP) };
        let me = self.clone();
        std::thread::spawn(move || {
            std::thread::sleep(KILL_GRACE);
            if !me.exited() {
                unsafe { libc::kill(-pgid, libc::SIGKILL) };
            }
        });
    }

    /// The session as `list` and `watch` report it.
    pub fn session(&self, now: u64) -> Session {
        let exit = self.inner.locked().exit;
        let (title, bell) = {
            let m = self.meta.locked();
            (m.title.clone(), m.bell)
        };
        // A shell someone typed `claude` or `codex` into is that agent while it runs (the
        // shim skips shells nex-host already holds, so this is how those show up).
        let job = (self.kind == proto::SHELL_KIND && exit.is_none()).then(|| pty::foreground(&self.pty, self.pid)).flatten();
        let running = job.as_ref().and_then(|(_, argv)| detect_kind(argv)).map(str::to_string);
        let agent_pid = match &job {
            Some((pid, _)) if running.is_some() => *pid,
            _ => self.pid,
        };
        let kind = running.as_deref().unwrap_or(&self.kind);
        let cli = clis::by_kind(kind).filter(|_| exit.is_none());
        let home = cli.and_then(|c| c.home());
        let claude = cli.zip(home.as_deref()).and_then(|(c, h)| c.session(h, agent_pid, clis::proc_start(agent_pid).as_deref()));
        let me = Running { pid: agent_pid, cwd: &self.cwd, session: claude.as_ref() };
        let subagents = cli.zip(home.as_deref()).map(|(c, h)| c.subagents(h, &me, nex_host::subagents::now_ms())).unwrap_or_default();
        let mut s = Session {
            id: self.id.clone(),
            kind: self.kind.clone(),
            name: claude.as_ref().and_then(|c| c.name.clone()).unwrap_or_else(|| self.name.clone()),
            cwd: self.cwd.clone(),
            argv: self.argv.clone(),
            pid: self.pid,
            started: self.started,
            title,
            bell,
            attached: self.attached.load(Ordering::Relaxed),
            last_output: self.last_output.load(Ordering::Relaxed),
            last_input: self.last_input.load(Ordering::Relaxed),
            exit,
            claude,
            running,
            status: Status::Idle,
            parent: self.parent.clone(),
            role: self.role.clone(),
            last_message: self.last_message.load(Ordering::Relaxed),
            subagents,
        };
        s.status = s.compute_status(now);
        // Without a session file to say so (before it reaches its prompt, and in versions that
        // don't write one), a CLI asking the user something may show it only on its screen
        // (Claude Code: trusting the folder, a new MCP server, a tool's permission). Typing a
        // message in would answer it (Enter takes the default: "No, exit" when trusting the
        // folder).
        let marks = clis::by_kind(s.agent_kind()).map_or(&[][..], |c| c.dialog_marks());
        if s.status != Status::Exited && !marks.is_empty() && s.claude.is_none() && shows_any(&self.text(DIALOG_LINES), marks) {
            s.status = Status::NeedsYou;
        }
        s
    }
}

/// How many lines at the bottom of the screen a start-up dialog fits in.
const DIALOG_LINES: usize = 40;

/// Whether `screen` shows any of a CLI's [`dialog_marks`](nex_host::clis::AgentCli::dialog_marks).
fn shows_any(screen: &str, marks: &[&str]) -> bool {
    marks.iter().any(|m| screen.contains(m))
}

/// The agent CLI's kind (`claude`, `codex`), when `argv` runs one (directly or under node /
/// bun). See [`clis::detect`].
pub fn detect_kind(argv: &[String]) -> Option<&'static str> {
    clis::detect(argv).map(|c| c.kind())
}

/// `argv` as one word for a shell: `'it'\''s'`.
pub fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn sees_claude_start_up_dialogs() {
        let dialog = |screen: &str| shows_any(screen, clis::CLAUDE.dialog_marks());
        let trust = " Quick safety check: Is this a project you created or one you trust?\n\n ❯ No, exit\n   Yes, I trust this folder\n\n Enter to confirm · Esc to cancel";
        assert!(dialog(trust));
        assert!(dialog(" New MCP server found in .mcp.json: db\n ❯ Use this server\n Enter to confirm · Esc to cancel"));
        assert!(dialog(" Bash command\n Do you want to proceed?\n ❯ 1. Yes\n   3. No\n\n Esc to cancel · Tab to amend"));
        assert!(!dialog("✻ Reading files… (esc to interrupt)"));
        assert!(!dialog("╭──────╮\n│ > Try \"fix the tests\" │\n╰──────╯\n  ? for shortcuts"));
        assert!(!shows_any(trust, clis::CODEX.dialog_marks()), "Codex has none");
    }

    #[test]
    fn knows_claude_and_codex() {
        assert_eq!(detect_kind(&v(&["claude", "--model", "opus"])), Some("claude"));
        assert_eq!(detect_kind(&v(&["/home/u/.local/bin/claude"])), Some("claude"));
        assert_eq!(detect_kind(&v(&["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])), Some("claude"));
        assert_eq!(detect_kind(&v(&["codex"])), Some("codex"));
        assert_eq!(detect_kind(&v(&["bun", "/x/@openai/codex/bin/codex.js"])), Some("codex"));
        assert_eq!(detect_kind(&v(&["vim"])), None);
    }

    #[test]
    fn quotes_for_any_shell() {
        assert_eq!(quote("claude"), "claude");
        assert_eq!(quote("--model=opus"), "--model=opus");
        assert_eq!(quote("fix it's tests"), r"'fix it'\''s tests'");
        assert_eq!(quote(""), "''");
    }
}
