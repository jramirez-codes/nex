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

use crate::agent::{self, Agent};
use crate::broker::{Broker, Peer};
use crate::locked::Locked;

mod lifecycle;
mod mail;
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

    retire::remember_exe();
    // One daemon at a time: whoever holds the lock owns the socket. Another that holds it
    // and is listening is the daemon; one that isn't listening is retiring, and lets go soon.
    let sock = paths::socket().unwrap();
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(paths::lock().unwrap())?;
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
        let state = state.clone();
        state.clients.fetch_add(1, Ordering::Relaxed);
        std::thread::spawn(move || {
            if let Err(e) = serve::serve(&state, stream) {
                log(&format!("connection: {e}"));
            }
            state.clients.fetch_sub(1, Ordering::Relaxed);
        });
    }
    Ok(())
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
