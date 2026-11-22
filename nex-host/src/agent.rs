//! One agent the daemon holds: its program in a pty of its own, and an `alacritty_terminal`
//! screen fed with everything it prints, so a pane attaching later sees what it would have
//! seen. At most one pane is attached at a time; output goes to it as it comes.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::PathBuf;
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::term::test::TermSize;
use alacritty_terminal::term::{self, Term};
use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

use nex_host::proto::{self, Session, Status};

use crate::redraw;

/// Lines of scrollback kept for a pane that attaches later.
const HISTORY: usize = 10_000;
/// How long `kill` waits after SIGHUP before SIGKILL.
const KILL_GRACE: Duration = Duration::from_secs(3);

pub fn now_ms() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64)
}

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
            Event::Title(t) => self.meta.lock().unwrap().title = Some(t),
            Event::ResetTitle => self.meta.lock().unwrap().title = None,
            Event::Bell if !self.attached.load(Ordering::Relaxed) => self.meta.lock().unwrap().bell = true,
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
}

impl Agent {
    /// Start `spec` in a new pty, and a thread reading its output for as long as it runs.
    /// `on_end` runs on that thread once it has ended.
    pub fn spawn(id: String, spec: Spec, on_end: impl FnOnce(&Arc<Agent>) + Send + 'static) -> io::Result<Arc<Agent>> {
        let program = spec.argv.first().ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "nothing to run"))?;
        let base = program.rsplit('/').next().unwrap_or(program).to_string();
        let kind = spec.kind.clone().unwrap_or_else(|| detect_kind(&spec.argv).unwrap_or(&base).to_string());
        let cwd = if spec.cwd.is_dir() { spec.cwd.clone() } else { PathBuf::from(std::env::var_os("HOME").unwrap_or_else(|| "/".into())) };
        let name = spec.name.clone().filter(|n| !n.trim().is_empty()).unwrap_or_else(|| {
            cwd.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| kind.clone())
        });
        let (cols, rows) = (spec.cols.max(2), spec.rows.max(1));

        let (master, child) = spawn_in_pty(&spec.argv, spec.env.as_deref(), &cwd, cols, rows)?;
        let pid = child.id();
        let pty = Arc::new(master);
        let meta = Arc::new(Mutex::new(Meta::default()));
        let attached = Arc::new(AtomicBool::new(false));
        let listener = Listener { meta: meta.clone(), attached: attached.clone(), pty: pty.clone() };
        let config = term::Config { scrolling_history: HISTORY, kitty_keyboard: true, ..Default::default() };
        let term = Term::new(config, &TermSize::new(cols as usize, rows as usize), listener);
        let now = now_ms();
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
            inner: Mutex::new(Inner { screen: Screen { term, parser: Processor::new(), cols, rows }, child: Some(child), client: None, exit: None }),
        });

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
        let mut inner = self.inner.lock().unwrap();
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
        let mut inner = self.inner.lock().unwrap();
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
        self.inner.lock().unwrap().exit.is_some()
    }

    /// Show this agent on a pane's connection, `cols` by `rows`: the pane it was on (if any)
    /// is let go, the screen is resized to fit and sent, then output flows. Returns the
    /// token that identifies this attachment, or `None` if the agent has ended.
    pub fn attach(&self, tx: Sender<Out>, token: u64, cols: u16, rows: u16) -> Option<u64> {
        let mut inner = self.inner.lock().unwrap();
        if inner.exit.is_some() {
            return None;
        }
        if let Some(old) = inner.client.take() {
            let _ = old.tx.send(Out::Close("[attached in another pane]".into()));
        }
        self.resize_locked(&mut inner, cols, rows);
        let title = self.meta.lock().unwrap().title.clone();
        let _ = tx.send(Out::Bytes(redraw::screen(&inner.screen.term, title.as_deref())));
        inner.client = Some(Client { token, tx });
        self.attached.store(true, Ordering::Relaxed);
        self.meta.lock().unwrap().bell = false;
        Some(token)
    }

    /// Let go of the pane attached with `token`, if it still is.
    pub fn detach(&self, token: u64) {
        let mut inner = self.inner.lock().unwrap();
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

    pub fn resize(&self, token: u64, cols: u16, rows: u16) {
        let mut inner = self.inner.lock().unwrap();
        if inner.client.as_ref().is_some_and(|c| c.token == token) {
            self.resize_locked(&mut inner, cols, rows);
        }
    }

    fn is_current(&self, token: u64) -> bool {
        self.inner.lock().unwrap().client.as_ref().is_some_and(|c| c.token == token)
    }

    fn resize_locked(&self, inner: &mut Inner, cols: u16, rows: u16) {
        let (cols, rows) = (cols.max(2), rows.max(1));
        if (cols, rows) == (inner.screen.cols, inner.screen.rows) {
            return;
        }
        inner.screen.term.resize(TermSize::new(cols as usize, rows as usize));
        inner.screen.cols = cols;
        inner.screen.rows = rows;
        set_size(self.pty.as_raw_fd(), cols, rows);
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
        let exit = self.inner.lock().unwrap().exit;
        let (title, bell) = {
            let m = self.meta.lock().unwrap();
            (m.title.clone(), m.bell)
        };
        let claude = (self.kind == "claude" && exit.is_none()).then(|| nex_host::claude::read(self.pid)).flatten();
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
            status: Status::Idle,
        };
        s.status = s.compute_status(now);
        s
    }
}

/// `claude` or `codex`, when `argv` runs one of them (directly or under node / bun).
pub fn detect_kind(argv: &[String]) -> Option<&'static str> {
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    let is = |s: &str, name: &str, script: &str| base(s) == name || s.ends_with(script);
    let pick = |s: &str| {
        if is(s, "claude", "@anthropic-ai/claude-code/cli.js") {
            Some("claude")
        } else if is(s, "codex", "@openai/codex/bin/codex.js") {
            Some("codex")
        } else {
            None
        }
    };
    match argv {
        [runtime, script, ..] if matches!(base(runtime).as_str(), "node" | "bun") => pick(script),
        [first, ..] => pick(first),
        [] => None,
    }
}

/// The login shell to start programs under, so they get the user's `PATH` (`~/.local/bin`,
/// nvm, …) even though the daemon was started by a non-interactive ssh command. Shells
/// whose `-l -c` works as in sh; anything else gets sh.
fn login_shell() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let base = shell.rsplit('/').next().unwrap_or("");
    if matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh" | "mksh" | "fish") && std::path::Path::new(&shell).exists() {
        shell
    } else {
        "/bin/sh".into()
    }
}

/// `argv` as one word for a shell: `'it'\''s'`.
pub fn quote(word: &str) -> String {
    if !word.is_empty() && word.chars().all(|c| c.is_ascii_alphanumeric() || "-_./=:@%+,".contains(c)) {
        return word.to_string();
    }
    format!("'{}'", word.replace('\'', r"'\''"))
}

/// Start `argv` in `cwd` under the login shell (which `exec`s it, so the program keeps the
/// shell's pid) on a new pty, as the leader of its own session with the pty as its terminal.
fn spawn_in_pty(argv: &[String], env: Option<&[(String, String)]>, cwd: &std::path::Path, cols: u16, rows: u16) -> io::Result<(File, Child)> {
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    // Raw pointers: macOS declares these `*mut`, Linux `*const`.
    let r = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut size) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(slave.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        // UTF-8 line editing (backspace over multi-byte characters), as terminals do.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(master.as_raw_fd(), &mut t) == 0 {
                t.c_iflag |= libc::IUTF8;
                libc::tcsetattr(master.as_raw_fd(), libc::TCSANOW, &t);
            }
        }
    }

    let mut cmd = match env {
        // The environment of the shell it was started from: run it directly in that.
        Some(env) => {
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..]).env_clear().envs(env.iter().filter(|(k, _)| !skip_var(k)).map(|(k, v)| (k, v)));
            cmd
        }
        None => {
            let script = format!("exec {}", argv.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" "));
            let mut cmd = Command::new(login_shell());
            cmd.args(["-l", "-c", &script]);
            cmd
        }
    };
    cmd.current_dir(cwd)
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave)
        .envs(agent_env());
    for key in ["SSH_TTY", "TMUX", "TMUX_PANE", "STY"] {
        cmd.env_remove(key);
    }
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // stdin is the pty: make it this session's terminal.
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            for sig in [libc::SIGCHLD, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGALRM, libc::SIGPIPE] {
                libc::signal(sig, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok((File::from(master), child))
}

/// Variables that describe the terminal a command was typed in, not the one the agent will
/// run in: left out of an environment passed along with `run`.
pub fn skip_var(key: &str) -> bool {
    matches!(key, "TERM" | "COLORTERM" | "TERM_PROGRAM" | "TERM_PROGRAM_VERSION" | "COLUMNS" | "LINES" | "SHLVL" | "PWD" | "OLDPWD" | "_" | "TMUX" | "TMUX_PANE" | "STY" | "WINDOWID" | "NEX_HOST")
        || key.starts_with("SSH_TTY")
}

/// What agents see of their terminal: nex's, as in a local pane.
fn agent_env() -> HashMap<&'static str, String> {
    HashMap::from([
        ("TERM", "xterm-256color".to_string()),
        ("COLORTERM", "truecolor".to_string()),
        ("TERM_PROGRAM", "nex".to_string()),
        ("TERM_PROGRAM_VERSION", proto::VERSION.to_string()),
        ("NEX_HOST", "1".to_string()),
    ])
}

fn set_size(fd: i32, cols: u16, rows: u16) {
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &size) };
}

/// The connection to an attached pane: sends what [`Agent`] queues for it, in order, so a
/// slow network never holds up reading the agent's output.
pub fn spawn_writer(mut stream: UnixStream) -> Sender<Out> {
    let (tx, rx) = mpsc::channel::<Out>();
    std::thread::spawn(move || {
        for out in rx {
            match out {
                Out::Bytes(b) => {
                    if stream.write_all(&b).is_err() {
                        break;
                    }
                }
                Out::Close(line) => {
                    let _ = stream.write_all(format!("{}\r\n{line}\r\n", redraw::RESET_MODES).as_bytes());
                    break;
                }
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
    });
    tx
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
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
