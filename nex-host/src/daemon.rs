//! The background process that owns the agents: one per user, listening on
//! `~/.nex/run/sock`. Started by the first command that needs it (see `client::connect`), in
//! a session of its own so ssh hanging up doesn't reach it. It exits once it has no agents
//! and no one has been connected for a while.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto::{self, Request, Response};

use crate::agent::{self, Agent, Spec};
use crate::locked::Locked;

/// How long the daemon stays up with nothing to do.
const IDLE_EXIT: Duration = Duration::from_secs(30);
/// How long an exited agent stays listed if nobody dismisses it.
const EXITED_KEEP: Duration = Duration::from_secs(24 * 3600);
/// How often `watch` looks for changes.
const WATCH_TICK: Duration = Duration::from_millis(500);

struct State {
    agents: Mutex<HashMap<String, Arc<Agent>>>,
    clients: AtomicUsize,
    next_token: AtomicU64,
}

impl State {
    fn sessions(&self) -> Vec<proto::Session> {
        let now = agent::now_ms();
        let mut list: Vec<_> = self.agents.locked().values().map(|a| a.session(now)).collect();
        list.sort_by_key(|s| s.started);
        list
    }

    fn get(&self, id: &str) -> Option<Arc<Agent>> {
        self.agents.locked().get(id).cloned()
    }
}

/// Run the daemon. Returns when it's idle (or another daemon already runs).
pub fn run() -> io::Result<()> {
    let dir = paths::run_dir().ok_or_else(|| io::Error::other("no home folder"))?;
    std::fs::create_dir_all(&dir)?;
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;

    // One daemon at a time: whoever holds the lock owns the socket.
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(paths::lock().unwrap())?;
    if unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Ok(());
    }
    let sock = paths::socket().unwrap();
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;

    let state = Arc::new(State { agents: Mutex::new(HashMap::new()), clients: AtomicUsize::new(0), next_token: AtomicU64::new(1) });

    // Leave when there's nothing left to hold.
    let idle = state.clone();
    let idle_sock = sock.clone();
    std::thread::spawn(move || {
        let mut idle_since: Option<Instant> = None;
        loop {
            std::thread::sleep(Duration::from_secs(2));
            expire(&idle);
            let busy = !idle.agents.locked().is_empty() || idle.clients.load(Ordering::Relaxed) > 0;
            if busy {
                idle_since = None;
            } else if idle_since.get_or_insert_with(Instant::now).elapsed() >= idle_exit() {
                let _ = std::fs::remove_file(&idle_sock);
                std::process::exit(0);
            }
        }
    });

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let state = state.clone();
        state.clients.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || {
            if let Err(e) = serve(&state, stream) {
                log(&format!("connection: {e}"));
            }
            state.clients.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
}

/// [`IDLE_EXIT`], or `$NEX_HOST_IDLE_EXIT` seconds (for tests).
fn idle_exit() -> Duration {
    std::env::var("NEX_HOST_IDLE_EXIT").ok().and_then(|s| s.parse().ok()).map_or(IDLE_EXIT, Duration::from_secs)
}

/// Drop agents that exited long ago.
fn expire(state: &State) {
    let now = agent::now_ms();
    state.agents.locked().retain(|_, a| {
        let ended = a.ended_at.load(Ordering::Relaxed);
        ended == 0 || now.saturating_sub(ended) < EXITED_KEEP.as_millis() as u64
    });
}

fn send(stream: &mut UnixStream, r: &Response) -> io::Result<()> {
    let mut line = serde_json::to_string(r).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())
}

fn serve(state: &Arc<State>, stream: UnixStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut stream = stream;
    let request: Request = match serde_json::from_str(line.trim()) {
        Ok(r) => r,
        Err(e) => return send(&mut stream, &Response::error(format!("bad request: {e}"))),
    };
    match request {
        Request::Launch { argv, kind, name, cwd, cols, rows, env } => {
            let id = new_id(state);
            let spec = Spec {
                argv,
                kind,
                name,
                cwd: cwd.as_deref().map(paths::expand_home).unwrap_or_else(|| paths::expand_home("~")),
                cols: cols.unwrap_or(120),
                rows: rows.unwrap_or(40),
                env,
            };
            let on_end_state = Arc::downgrade(state);
            // Killed sessions are forgotten when they end, and so are shells: a shell that
            // exited has nothing left to show.
            let ended = move |a: &Arc<Agent>| {
                if (a.killed.load(Ordering::Relaxed) || a.kind == proto::SHELL_KIND)
                    && let Some(s) = on_end_state.upgrade()
                {
                    s.agents.locked().remove(&a.id);
                }
            };
            match Agent::spawn(id.clone(), spec, ended) {
                Ok(a) => {
                    state.agents.locked().insert(id.clone(), a);
                    send(&mut stream, &Response { ok: true, id: Some(id), ..Response::default() })
                }
                Err(e) => send(&mut stream, &Response::error(format!("couldn't start it: {e}"))),
            }
        }
        Request::List => send(&mut stream, &Response::sessions(state.sessions())),
        Request::Watch => watch(state, &mut stream),
        Request::Kill { id } => match state.get(&id) {
            Some(a) if a.exited() => {
                state.agents.locked().remove(&id);
                send(&mut stream, &Response::ok())
            }
            Some(a) => {
                a.kill();
                send(&mut stream, &Response::ok())
            }
            None => send(&mut stream, &Response::error(format!("no session {id}"))),
        },
        Request::Forget { id } => match state.get(&id) {
            Some(a) if a.exited() => {
                state.agents.locked().remove(&id);
                send(&mut stream, &Response::ok())
            }
            Some(_) => send(&mut stream, &Response::error("it's still running: kill it first")),
            None => send(&mut stream, &Response::error(format!("no session {id}"))),
        },
        Request::Attach { id, cols, rows } => {
            let Some(agent) = state.get(&id) else {
                return send(&mut stream, &Response::error(format!("no session {id}")));
            };
            if agent.exited() {
                return send(&mut stream, &Response::error("it has exited"));
            }
            send(&mut stream, &Response::ok())?;
            let token = state.next_token.fetch_add(1, Ordering::Relaxed);
            let tx = agent::spawn_writer(stream.try_clone()?);
            if agent.attach(tx, token, cols, rows).is_none() {
                return Ok(());
            }
            // Read through the buffer: a client may send keys right behind its request.
            let result = pump(&agent, token, &mut reader);
            agent.detach(token);
            result
        }
    }
}

/// Frames from an attached pane, until it hangs up.
fn pump(agent: &Agent, token: u64, stream: &mut impl Read) -> io::Result<()> {
    let mut head = [0u8; 5];
    let mut payload = Vec::new();
    loop {
        if stream.read_exact(&mut head).is_err() {
            return Ok(());
        }
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if len > proto::FRAME_MAX {
            return Err(io::Error::other("frame too large"));
        }
        payload.resize(len, 0);
        stream.read_exact(&mut payload)?;
        match head[0] {
            proto::FRAME_INPUT => agent.input(token, &payload),
            proto::FRAME_RESIZE if len == 4 => {
                let cols = u16::from_be_bytes([payload[0], payload[1]]);
                let rows = u16::from_be_bytes([payload[2], payload[3]]);
                agent.resize(token, cols, rows);
            }
            _ => {}
        }
    }
}

/// Send the session list now, again whenever it changes, and every heartbeat regardless.
fn watch(state: &State, stream: &mut UnixStream) -> io::Result<()> {
    let mut last = String::new();
    let mut sent = Instant::now();
    loop {
        let line = serde_json::to_string(&Response::sessions(state.sessions())).map_err(io::Error::other)?;
        if line != last || sent.elapsed() >= Duration::from_secs(proto::HEARTBEAT_SECS) {
            stream.write_all(line.as_bytes())?;
            stream.write_all(b"\n")?;
            last = line;
            sent = Instant::now();
        }
        std::thread::sleep(WATCH_TICK);
    }
}

/// A short id nobody else has: 8 hex digits.
fn new_id(state: &State) -> String {
    loop {
        let mut bytes = [0u8; 4];
        if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_err() {
            let n = agent::now_ms() ^ (u64::from(std::process::id()) << 20) ^ state.next_token.fetch_add(1, Ordering::Relaxed);
            bytes = (n as u32).to_be_bytes();
        }
        let id: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        if !state.agents.locked().contains_key(&id) {
            return id;
        }
    }
}

/// Note something in `~/.nex/run/daemon.log` (the daemon has no terminal to print to).
pub fn log(msg: &str) {
    let Some(path) = paths::log() else { return };
    // Keep it small: start over past 1 MB.
    if std::fs::metadata(&path).is_ok_and(|m| m.len() > 1 << 20) {
        let _ = std::fs::remove_file(&path);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {msg}", agent::now_ms() / 1000);
    }
}
