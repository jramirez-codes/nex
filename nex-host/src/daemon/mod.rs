//! The background process that owns the agents: one per user, listening on
//! `~/.nex/run/sock`. Started by the first command that needs it (see `client::connect`), in
//! a session of its own so ssh hanging up doesn't reach it. It exits once it has no agents
//! and no one has been connected for a while, or (see `retire.rs`) once a newer nex-host
//! has been installed and none of its agents is running.
//!
//! ```text
//! daemon/
//!   mod.rs         this file: the daemon's state, and running it
//!   serve.rs       one connection: its request and the answer
//!   lifecycle.rs   agents starting (`spawn`), ending, and expiring
//!   mail.rs        messages: sending, typing them in, reminders
//!   retire.rs      handing over to a newer nex-host once nothing runs
//!   agent.rs       one agent: its program in a pty, and the screen it has drawn
//!   pty.rs         the pty and process an agent runs in, and the writer to its pane
//!   redraw.rs      an agent's screen as escape sequences, for a pane attaching
//!   broker.rs      the post office: addresses, mailboxes, `recv` waiting
//!   deliver.rs     how a message is written out and typed into an agent
//! ```

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::UnixListener;
use std::os::unix::net::UnixStream;
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto;

use crate::daemon::agent::Agent;
use crate::daemon::broker::{Broker, Peer};
use crate::util::{Locked, now_ms};

mod agent;
mod broker;
pub(crate) mod deliver;
mod lifecycle;
mod mail;
mod pty;
pub(crate) mod redraw;
mod retire;
mod serve;

/// How often the daemon looks for agents to expire and whether it's time to leave.
const IDLE_TICK: Duration = Duration::from_secs(2);
/// How long a new daemon waits for the lock while the one before it, no longer listening,
/// finishes retiring.
const LOCK_WAIT: Duration = Duration::from_secs(3);

struct State {
    agents: Mutex<HashMap<String, Arc<Agent>>>,
    clients: AtomicUsize,
    next_token: AtomicU64,
    broker: Broker,
    /// Messages to type in once their recipient can take them: started up, and not asking
    /// the user something.
    pending: Mutex<Vec<(String, String)>>,
    /// A newer nex-host is installed and this one is on its way out (see `retire.rs`): set
    /// under the `agents` lock, after which nothing is started.
    retiring: AtomicBool,
}

impl State {
    fn peers(&self) -> Vec<Peer> {
        self.agents.locked().values().map(|a| Peer { id: a.id.clone(), name: a.name.clone(), parent: a.parent.clone(), exited: a.exited() }).collect()
    }

    /// Drop a session, and its mail with it.
    fn remove(&self, id: &str) {
        self.agents.locked().remove(id);
        self.broker.forget(id);
    }

    fn sessions(&self) -> Vec<proto::Session> {
        let now = now_ms();
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

    // The daemon's output goes nowhere: a panic is written to its log instead.
    let before = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log(&panic_line(info));
        before(info);
    }));

    retire::remember_exe();
    // One daemon at a time: whoever holds the lock owns the socket. Another that holds it
    // and is listening is the daemon; one that isn't listening is retiring, and lets go soon.
    let sock = dir.join("sock");
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(dir.join("lock"))?;
    let waiting = Instant::now();
    while unsafe { libc::flock(lock.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        if UnixStream::connect(&sock).is_ok() || waiting.elapsed() >= LOCK_WAIT {
            return Ok(());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    let _ = std::fs::remove_file(&sock);
    let listener = UnixListener::bind(&sock)?;
    std::fs::set_permissions(&sock, std::fs::Permissions::from_mode(0o600))?;

    let state = Arc::new(State {
        agents: Mutex::new(HashMap::new()),
        clients: AtomicUsize::new(0),
        next_token: AtomicU64::new(1),
        broker: Broker::new(),
        pending: Mutex::new(Vec::new()),
        retiring: AtomicBool::new(false),
    });

    let postman = state.clone();
    std::thread::spawn(move || mail::deliver_loop(&postman));

    // Leave when there's nothing left to hold.
    let idle = state.clone();
    let idle_sock = sock.clone();
    std::thread::spawn(move || {
        let mut idle_since: Option<Instant> = None;
        loop {
            std::thread::sleep(IDLE_TICK);
            lifecycle::expire(&idle);
            retire::check(&idle);
            let busy = !idle.agents.locked().is_empty() || idle.clients.load(Ordering::Relaxed) > 0;
            if busy {
                idle_since = None;
            } else if idle_since.get_or_insert_with(Instant::now).elapsed() >= lifecycle::idle_exit() {
                let _ = std::fs::remove_file(&idle_sock);
                std::process::exit(0);
            }
        }
    });

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let client = Connected::new(state.clone());
        std::thread::spawn(move || {
            if let Err(e) = serve::serve(&client.0, stream) {
                log(&format!("connection: {e}"));
            }
        });
    }
    Ok(())
}

/// One connection, counted in [`State::clients`] for as long as this lives: until its
/// thread is done with it, whether that returned or panicked.
struct Connected(Arc<State>);

impl Connected {
    fn new(state: Arc<State>) -> Self {
        state.clients.fetch_add(1, Ordering::Relaxed);
        Self(state)
    }
}

impl Drop for Connected {
    fn drop(&mut self) {
        self.0.clients.fetch_sub(1, Ordering::Relaxed);
    }
}

/// A panic as one line of the daemon's log: what it said and where.
fn panic_line(info: &std::panic::PanicHookInfo) -> String {
    let payload = info.payload();
    let message = payload.downcast_ref::<&str>().copied().or_else(|| payload.downcast_ref::<String>().map(String::as_str)).unwrap_or("(no message)");
    match info.location() {
        Some(at) => format!("panic at {}:{}: {message}", at.file(), at.line()),
        None => format!("panic: {message}"),
    }
}

/// A short id nobody else has: 8 hex digits.
fn new_id(state: &State) -> String {
    loop {
        let mut bytes = [0u8; 4];
        if std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes)).is_err() {
            let n = now_ms() ^ (u64::from(std::process::id()) << 20) ^ state.next_token.fetch_add(1, Ordering::Relaxed);
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
        let _ = writeln!(f, "{} {msg}", now_ms() / 1000);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> Arc<State> {
        Arc::new(State {
            agents: Mutex::new(HashMap::new()),
            clients: AtomicUsize::new(0),
            next_token: AtomicU64::new(1),
            broker: Broker::default(),
            pending: Mutex::new(Vec::new()),
            retiring: AtomicBool::new(false),
        })
    }

    #[test]
    fn a_connection_that_panics_is_no_longer_counted() {
        let state = state();
        let client = Connected::new(state.clone());
        let also = Connected::new(state.clone());
        assert_eq!(state.clients.load(Ordering::Relaxed), 2);
        let served = std::thread::spawn(move || {
            let _client = client;
            panic!("serving went wrong");
        });
        assert!(served.join().is_err());
        assert_eq!(state.clients.load(Ordering::Relaxed), 1, "the one that panicked is gone");
        drop(also);
        assert_eq!(state.clients.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn a_panic_is_one_line_saying_what_and_where() {
        // The hook is the whole process's: other tests' panics pass through it meanwhile,
        // so this looks only for its own.
        static LINES: Mutex<Vec<String>> = Mutex::new(Vec::new());
        let before: Arc<dyn Fn(&std::panic::PanicHookInfo) + Send + Sync> = Arc::from(std::panic::take_hook());
        let chained = before.clone();
        std::panic::set_hook(Box::new(move |info| {
            let line = panic_line(info);
            if line.contains("the daemon tripped") { LINES.locked().push(line) } else { chained(info) }
        }));
        let line = line!() + 1;
        let _ = std::thread::spawn(|| panic!("the daemon tripped on {}", 7)).join();
        let _ = std::thread::spawn(|| std::panic::panic_any(7)).join();
        std::panic::set_hook(Box::new(move |info| before(info)));
        assert_eq!(*LINES.locked(), [format!("panic at {}:{line}: the daemon tripped on 7", file!())]);
    }
}
