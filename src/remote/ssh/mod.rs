//! nex's own ssh client, for a host nex connects to itself instead of through a pane's
//! `ssh`: the Android app (where there is no `ssh` to run) and `nex --remote`.
//!
//! ```text
//! ssh/
//!   mod.rs       this file: connecting, checking the host's key, and commands on the host
//!   device.rs    this device's key, put in the host's authorized_keys after one password login
//!   host.rs      one host and its connection, reconnected when it drops
//! ```
//!
//! A phone that sleeps loses its connections without being told: [`woke`] (called when the
//! app comes back) checks each one and ends those that don't answer, so their panes
//! reconnect at once instead of waiting a minute for keepalives to give up.
//!
//! `russh` does the protocol, on a small tokio runtime of its own. Everything here blocks, so
//! it runs on background threads, never in a frame. A command's input and output are plain
//! `Read` / `Write` ([`Exec`]): tasks on the runtime move them to and from the channel.
//!
//! A command is run by the user's login shell on the host, which needn't be a POSIX one (fish,
//! csh). Anything written in `sh` goes through [`Client::run_sh`] / [`Client::exec_sh`], which
//! hand it to `sh` as one quoted word; [`Client::run`] / [`Client::exec`] are for a plain
//! program and its arguments.
//!
//! Host keys are trusted on first use, but only with the user's say: a host nex hasn't
//! seen fails with [`ConnectError::UnknownHost`] and its fingerprint. The user is shown that
//! fingerprint, and trusting it saves it to `known_hosts` in nex's config folder. A host
//! whose key changed is refused.

pub mod device;
pub mod host;
#[cfg(all(test, target_os = "linux"))]
mod sshd_tests;

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::net::SocketAddr;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex, OnceLock, Weak};
use std::time::{Duration, Instant};

use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};

/// Give up on a host that doesn't answer (or finish logging in) within this long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
/// Give up on a TCP connection that doesn't open within this long (a network that isn't
/// back yet after the phone woke), so the next try comes soon.
const TCP_TIMEOUT: Duration = Duration::from_secs(5);
/// Ping a quiet connection this often, and drop it after this many pings go unanswered:
/// a phone that changed networks finds out within half a minute.
const KEEPALIVE: Duration = Duration::from_secs(10);
const KEEPALIVE_MAX: usize = 2;
/// How long a connection has to answer the check [`woke`] makes.
const WAKE_PING: Duration = Duration::from_secs(3);
/// [`woke`] checks the connections at most this often (resume and focus both call it).
const WAKE_DEBOUNCE: Duration = Duration::from_secs(2);

/// The host to connect to.
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub user: String,
}

impl std::fmt::Display for Server {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.port == 22 { write!(f, "{}@{}", self.user, self.host) } else { write!(f, "{}@{}:{}", self.user, self.host, self.port) }
    }
}

/// How to log in.
pub enum Login<'a> {
    Key(&'a PrivateKey),
    Password(&'a str),
}

/// Why a connection didn't come up.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConnectError {
    /// The host couldn't be reached, or the connection broke while setting up.
    Unreachable(String),
    /// nex hasn't seen this host before: its key's fingerprint (`SHA256:…`), for the user to
    /// check before trusting it.
    UnknownHost { fingerprint: String },
    /// The host's key isn't the one nex saved for it: someone may be in between.
    HostKeyChanged { fingerprint: String },
    /// The host turned the login down (wrong password, or the key isn't, or no longer is,
    /// in its authorized_keys).
    Denied,
}

impl std::fmt::Display for ConnectError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ConnectError::Unreachable(e) => write!(f, "couldn't reach it: {e}"),
            ConnectError::UnknownHost { fingerprint } => write!(f, "first connection (its key is {fingerprint})"),
            ConnectError::HostKeyChanged { fingerprint } => {
                write!(f, "its key changed (now {fingerprint}): if that's expected, remove it from nex's known_hosts")
            }
            ConnectError::Denied => write!(f, "the login was refused"),
        }
    }
}

/// The runtime russh runs on: two threads, started on first use.
fn runtime() -> &'static tokio::runtime::Runtime {
    static RT: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("ssh")
            .enable_all()
            .build()
            .expect("starting the ssh runtime")
    })
}

/// The connections that are up, for [`woke`] to check.
fn live() -> &'static Mutex<Vec<Weak<Live>>> {
    static LIVE: OnceLock<Mutex<Vec<Weak<Live>>>> = OnceLock::new();
    LIVE.get_or_init(Mutex::default)
}

/// How many wakes there have been, and when the last one was.
type Wakes = (Mutex<(u64, Option<Instant>)>, Condvar);

/// Counts wakes, for [`wait_for_wake`]; and when the last one was.
fn wakes() -> &'static Wakes {
    static WAKES: OnceLock<Wakes> = OnceLock::new();
    WAKES.get_or_init(Default::default)
}

/// The app came back (the phone woke, the window got focus): check every connection with a
/// ping, end the ones that don't answer (their panes then reconnect), and cut short any
/// wait between tries to reach a host ([`wait_for_wake`]). Returns at once.
pub fn woke() {
    let (lock, cv) = wakes();
    {
        let mut w = lock.lock().unwrap_or_else(|e| e.into_inner());
        if w.1.is_some_and(|t| t.elapsed() < WAKE_DEBOUNCE) {
            return;
        }
        *w = (w.0.wrapping_add(1), Some(Instant::now()));
        cv.notify_all();
    }
    let clients: Vec<Arc<Live>> = {
        let mut live = live().lock().unwrap_or_else(|e| e.into_inner());
        live.retain(|c| c.strong_count() > 0);
        live.iter().filter_map(Weak::upgrade).collect()
    };
    // No connections, no runtime to start (a desktop window that never connected itself).
    for c in clients {
        runtime().spawn(async move {
            let answered = tokio::time::timeout(WAKE_PING, c.handle.send_ping()).await;
            // A ping also "returns" when the session ended under it: check that too.
            if !matches!(answered, Ok(Ok(()))) || c.handle.is_closed() {
                eprintln!("nex: a connection didn't survive the sleep; reconnecting");
                c.kill();
            }
        });
    }
}

/// The current wake count, to pass to [`wait_for_wake`].
// Remote panes (their only user) open on unix only.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn wake_count() -> u64 {
    wakes().0.lock().unwrap_or_else(|e| e.into_inner()).0
}

/// The app woke (see [`woke`]) within `within`.
// Remote panes (their only user) open on unix only.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn woke_within(within: Duration) -> bool {
    wakes().0.lock().unwrap_or_else(|e| e.into_inner()).1.is_some_and(|t| t.elapsed() < within)
}

/// Sleep for `d`, or until the app wakes after `seen` (a [`wake_count`]). True if it woke.
// Remote panes (their only user) open on unix only.
#[cfg_attr(not(unix), allow(dead_code))]
pub fn wait_for_wake(seen: u64, d: Duration) -> bool {
    let (lock, cv) = wakes();
    let guard = lock.lock().unwrap_or_else(|e| e.into_inner());
    let (guard, _) = cv.wait_timeout_while(guard, d, |w| w.0 == seen).unwrap_or_else(|e| e.into_inner());
    guard.0 != seen
}

/// The addresses each host last resolved to: on a phone just out of sleep, the DNS (a VPN's,
/// such as Tailscale's) can be the last thing to come back, while the host's address works.
type Addrs = Mutex<HashMap<(String, u16), Vec<SocketAddr>>>;

fn known_addrs() -> &'static Addrs {
    static ADDRS: OnceLock<Addrs> = OnceLock::new();
    ADDRS.get_or_init(Mutex::default)
}

/// Resolve `host`, falling back to the addresses it last had when the lookup fails.
async fn resolve(host: &str, port: u16) -> io::Result<Vec<SocketAddr>> {
    let key = (host.to_string(), port);
    match tokio::net::lookup_host((host, port)).await {
        Ok(addrs) => {
            let addrs: Vec<SocketAddr> = addrs.collect();
            if !addrs.is_empty() {
                known_addrs().lock().unwrap_or_else(|e| e.into_inner()).insert(key, addrs.clone());
            }
            Ok(addrs)
        }
        Err(e) => match known_addrs().lock().unwrap_or_else(|e| e.into_inner()).get(&key) {
            Some(addrs) => Ok(addrs.clone()),
            None => Err(e),
        },
    }
}

/// Open a TCP connection to `server`, giving up after [`TCP_TIMEOUT`].
async fn open_tcp(server: &Server) -> io::Result<tokio::net::TcpStream> {
    let addrs = resolve(&server.host, server.port).await?;
    let opened = tokio::time::timeout(TCP_TIMEOUT, tokio::net::TcpStream::connect(&addrs[..])).await;
    let socket = opened.map_err(|_| io::Error::new(io::ErrorKind::TimedOut, "the network didn't answer"))??;
    let _ = socket.set_nodelay(true);
    Ok(socket)
}

/// `known_hosts` in nex's config folder (kept apart from `~/.ssh`, which on a desktop
/// belongs to the system's `ssh`).
pub fn known_hosts_path() -> Option<PathBuf> {
    Some(crate::platform::config_dir()?.join("known_hosts"))
}

/// What the host's key check found, kept for the error the connection fails with.
#[derive(Clone, Debug, Default)]
enum KeyCheck {
    #[default]
    NotYet,
    Known,
    Unknown(String),
    Changed(String),
    Failed(String),
}

/// russh's callbacks: checks the host's key against `known_hosts`.
struct Checker {
    server: Server,
    /// The fingerprint the user agreed to trust, for a host seen the first time.
    trust: Option<String>,
    found: Arc<Mutex<KeyCheck>>,
}

impl client::Handler for Checker {
    type Error = russh::Error;

    async fn check_server_key(&mut self, key: &russh::keys::PublicKeyOrCertificate) -> Result<bool, Self::Error> {
        let russh::keys::PublicKeyOrCertificate::PublicKey { key, .. } = key else {
            *self.found.lock().unwrap_or_else(|e| e.into_inner()) = KeyCheck::Failed("it offered a certificate, which nex doesn't check yet".into());
            return Ok(false);
        };
        let verdict = check_key(&self.server, key, self.trust.as_deref());
        let ok = matches!(verdict, KeyCheck::Known);
        *self.found.lock().unwrap_or_else(|e| e.into_inner()) = verdict;
        Ok(ok)
    }
}

/// `SHA256:…`, as `ssh` shows a key.
pub fn fingerprint(key: &PublicKey) -> String {
    key.fingerprint(HashAlg::Sha256).to_string()
}

/// Is `key` the one saved for `server`? A new host's key is saved when `trust` is its
/// fingerprint.
fn check_key(server: &Server, key: &PublicKey, trust: Option<&str>) -> KeyCheck {
    let Some(path) = known_hosts_path() else {
        return KeyCheck::Failed("no folder to keep known hosts in".into());
    };
    let fp = fingerprint(key);
    match russh::keys::check_known_hosts_path(&server.host, server.port, key, &path) {
        Ok(true) => KeyCheck::Known,
        Ok(false) if trust == Some(fp.as_str()) => match russh::keys::known_hosts::learn_known_hosts_path(&server.host, server.port, key, &path) {
            Ok(()) => KeyCheck::Known,
            Err(e) => KeyCheck::Failed(format!("saving its key: {e}")),
        },
        Ok(false) => KeyCheck::Unknown(fp),
        Err(russh::keys::Error::KeyChanged { .. }) => KeyCheck::Changed(fp),
        Err(e) => KeyCheck::Failed(format!("reading known_hosts: {e}")),
    }
}

/// The command that runs `script` under `sh`. A command sent to a host is run by the user's
/// login shell there, which may be fish or csh: they don't read `sh` (`a=b`, `$(…)`, `2>&1`),
/// but every shell runs a program with one quoted word the same way.
pub fn sh_command(script: &str) -> String {
    format!("sh -c {}", crate::remote::posix_quote(script))
}

/// A logged-in connection to a host. Commands on it run in channels of their own, any
/// number at once.
pub struct Client {
    live: Arc<Live>,
}

/// A connection, as [`woke`] sees it.
struct Live {
    handle: Handle<Checker>,
    /// Found dead by [`woke`]: treated as closed, and its commands are ended.
    dead: AtomicBool,
    killed: tokio::sync::watch::Sender<bool>,
}

impl Live {
    fn kill(&self) {
        self.dead.store(true, Ordering::Relaxed);
        self.killed.send_replace(true);
    }
}

impl Client {
    /// Connect to `server` and log in. `trust` is the fingerprint the user agreed to, for a
    /// host nex hasn't seen. Blocks.
    pub fn connect(server: &Server, login: Login<'_>, trust: Option<&str>) -> Result<Client, ConnectError> {
        let found = Arc::new(Mutex::new(KeyCheck::NotYet));
        let checker = Checker { server: server.clone(), trust: trust.map(String::from), found: found.clone() };
        let config = Arc::new(client::Config {
            keepalive_interval: Some(KEEPALIVE),
            keepalive_max: KEEPALIVE_MAX,
            nodelay: true,
            ..Default::default()
        });
        let user = server.user.clone();
        let result = runtime().block_on(async move {
            let connecting = async {
                let socket = open_tcp(server).await.map_err(russh::Error::from)?;
                let mut handle = client::connect_stream(config, socket, checker).await?;
                let auth = match login {
                    Login::Key(key) => {
                        let hash = handle.best_supported_rsa_hash().await?.flatten();
                        handle.authenticate_publickey(user, PrivateKeyWithHashAlg::new(Arc::new(key.clone()), hash)).await?
                    }
                    Login::Password(password) => handle.authenticate_password(user, password).await?,
                };
                Ok::<_, russh::Error>((handle, auth.success()))
            };
            tokio::time::timeout(CONNECT_TIMEOUT, connecting).await
        });
        let key_check = std::mem::take(&mut *found.lock().unwrap_or_else(|e| e.into_inner()));
        match (result, key_check) {
            (_, KeyCheck::Unknown(fingerprint)) => Err(ConnectError::UnknownHost { fingerprint }),
            (_, KeyCheck::Changed(fingerprint)) => Err(ConnectError::HostKeyChanged { fingerprint }),
            (_, KeyCheck::Failed(e)) => Err(ConnectError::Unreachable(e)),
            (Err(_), _) => Err(ConnectError::Unreachable("it didn't answer in time".into())),
            (Ok(Err(e)), _) => Err(ConnectError::Unreachable(e.to_string())),
            (Ok(Ok((_, false))), _) => Err(ConnectError::Denied),
            (Ok(Ok((handle, true))), _) => {
                let conn = Arc::new(Live { handle, dead: AtomicBool::new(false), killed: tokio::sync::watch::Sender::new(false) });
                let mut all = live().lock().unwrap_or_else(|e| e.into_inner());
                all.retain(|c| c.strong_count() > 0);
                all.push(Arc::downgrade(&conn));
                Ok(Client { live: conn })
            }
        }
    }

    /// The connection has ended (the host hung up, stopped answering keepalives, or didn't
    /// answer the check after a wake).
    pub fn is_closed(&self) -> bool {
        self.live.dead.load(Ordering::Relaxed) || self.live.handle.is_closed()
    }

    /// Run `command` (through the user's shell on the host), with its stdin and stdout as
    /// plain `Write` and `Read`. Blocks until the host has started it.
    pub fn exec(&self, command: &str) -> io::Result<Exec> {
        let command = command.to_string();
        let handle = &self.live.handle;
        let channel = runtime().block_on(async {
            let channel = handle.channel_open_session().await?;
            channel.exec(true, command).await?;
            Ok::<_, russh::Error>(channel)
        });
        let channel = channel.map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        Ok(Exec::start(channel, self.live.killed.subscribe()))
    }

    /// Run `script` under `sh`, whatever the user's shell on the host is (see [`sh_command`]):
    /// [`Self::exec`] for anything written in `sh`.
    pub fn exec_sh(&self, script: &str) -> io::Result<Exec> {
        self.exec(&sh_command(script))
    }

    /// Run `command`, feeding it `stdin`, and wait for it to end: its exit status and what it
    /// printed to stdout.
    pub fn run(&self, command: &str, stdin: &[u8]) -> io::Result<(u32, Vec<u8>)> {
        Self::finish(self.exec(command)?, stdin)
    }

    /// [`Self::run`] for a `script` written in `sh`, run under `sh` (see [`sh_command`]).
    pub fn run_sh(&self, script: &str, stdin: &[u8]) -> io::Result<(u32, Vec<u8>)> {
        Self::finish(self.exec_sh(script)?, stdin)
    }

    /// Feed a started command `stdin` and wait for it to end.
    fn finish(mut exec: Exec, stdin: &[u8]) -> io::Result<(u32, Vec<u8>)> {
        exec.writer.write_all(stdin)?;
        drop(exec.writer);
        let mut out = Vec::new();
        exec.reader.read_to_end(&mut out)?;
        let status = exec.status.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(255);
        Ok((status, out))
    }
}

impl Drop for Live {
    fn drop(&mut self) {
        // Nobody is listening at the other end of a dead one: saying goodbye would only wait.
        // And blocking on the runtime from one of its own tasks (a wake check that held the
        // last reference) would panic: the connection just closes without a goodbye then.
        if self.dead.load(Ordering::Relaxed) || tokio::runtime::Handle::try_current().is_ok() {
            return;
        }
        let handle = &self.handle;
        let _ = runtime().block_on(async {
            tokio::time::timeout(WAKE_PING, handle.disconnect(russh::Disconnect::ByApplication, "", "en")).await
        });
    }
}

/// A command running on the host: what it prints ([`ExecReader`]), what it reads
/// ([`ExecWriter`]; dropping it ends its input), its exit status once it has ended, and a way
/// to end it from outside ([`ExecCloser`]).
pub struct Exec {
    pub reader: ExecReader,
    pub writer: ExecWriter,
    pub status: Arc<Mutex<Option<u32>>>,
    pub closer: ExecCloser,
}

/// Ends a command from outside: sends it `SIGTERM` and closes its channel. Ending its input
/// isn't always enough (a command blocked opening a FIFO never reads it). Cheap to clone.
#[derive(Clone)]
pub struct ExecCloser {
    close: Arc<tokio::sync::Notify>,
}

impl ExecCloser {
    /// End the command. Returns at once; does nothing once it has ended.
    pub fn close(&self) {
        // A permit is kept if the task isn't waiting right now, so this is never missed.
        self.close.notify_one();
    }
}

impl Exec {
    /// Start the task that moves the channel's data to `reader` and `writer`'s data to it,
    /// until the channel closes, `killed` says the connection is dead, or the closer is used.
    fn start(channel: russh::Channel<client::Msg>, mut killed: tokio::sync::watch::Receiver<bool>) -> Exec {
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let (in_tx, mut in_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let status = Arc::new(Mutex::new(None));
        let exit = status.clone();
        let (mut read, write) = channel.split();
        let close = Arc::new(tokio::sync::Notify::new());
        let closing = close.clone();
        runtime().spawn(async move {
            let mut input_open = true;
            // Ends only when the connection is found dead (not when the client is dropped).
            let dead = async move {
                if killed.wait_for(|dead| *dead).await.is_err() {
                    std::future::pending::<()>().await;
                }
            };
            tokio::pin!(dead);
            loop {
                tokio::select! {
                    () = &mut dead => break,
                    () = closing.notified() => {
                        let _ = write.signal(russh::Sig::TERM).await;
                        let _ = write.close().await;
                        break;
                    }
                    msg = read.wait() => match msg {
                        Some(ChannelMsg::Data { data }) => {
                            // Once the reader is gone, output is just dropped until the end.
                            let _ = out_tx.send(data.to_vec());
                        }
                        Some(ChannelMsg::ExitStatus { exit_status }) => {
                            *exit.lock().unwrap_or_else(|e| e.into_inner()) = Some(exit_status);
                        }
                        Some(ChannelMsg::Close) | None => break,
                        Some(_) => {}
                    },
                    data = in_rx.recv(), if input_open => match data {
                        Some(bytes) => {
                            if write.data_bytes(bytes).await.is_err() {
                                break;
                            }
                        }
                        None => {
                            input_open = false;
                            let _ = write.eof().await;
                        }
                    },
                }
            }
            // Dropping `out_tx` ends the reader.
        });
        Exec { reader: ExecReader { rx: out_rx, pending: Vec::new(), at: 0 }, writer: ExecWriter { tx: in_tx }, status, closer: ExecCloser { close } }
    }
}

#[cfg(test)]
impl Exec {
    /// A command with no connection behind it, for tests: it prints `output` and then runs
    /// until its closer is used, as one that never ends by itself would.
    pub fn hanging(output: &[u8]) -> Exec {
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let (in_tx, _) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let close = Arc::new(tokio::sync::Notify::new());
        let closing = close.clone();
        let _ = out_tx.send(output.to_vec());
        runtime().spawn(async move {
            closing.notified().await;
            // Dropping `out_tx` ends the reader, as the closed channel does.
            drop(out_tx);
        });
        Exec { reader: ExecReader { rx: out_rx, pending: Vec::new(), at: 0 }, writer: ExecWriter { tx: in_tx }, status: Arc::default(), closer: ExecCloser { close } }
    }
}

/// A command's output. Reads block until something arrives, and return 0 once it has ended.
pub struct ExecReader {
    rx: mpsc::Receiver<Vec<u8>>,
    pending: Vec<u8>,
    at: usize,
}

impl Read for ExecReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.at >= self.pending.len() {
            match self.rx.recv() {
                Ok(data) => {
                    self.pending = data;
                    self.at = 0;
                }
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.pending.len() - self.at);
        buf[..n].copy_from_slice(&self.pending[self.at..self.at + n]);
        self.at += n;
        Ok(n)
    }
}

/// A command's input. Writes never block (they queue for the channel); one fails once the
/// command has ended.
#[derive(Clone)]
pub struct ExecWriter {
    tx: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
}

impl Write for ExecWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.tx.send(buf.to_vec()).map_err(|_| io::Error::new(io::ErrorKind::BrokenPipe, "the command has ended"))?;
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn servers_read_like_ssh_destinations() {
        let s = Server { host: "home.lan".into(), port: 22, user: "jr".into() };
        assert_eq!(s.to_string(), "jr@home.lan");
        let s = Server { port: 2222, ..s };
        assert_eq!(s.to_string(), "jr@home.lan:2222");
    }

    /// A script reaches `sh` as one quoted word, which fish and csh pass on as `sh` would
    /// (nothing of it is left outside the quotes for them to read their own way), and `sh`
    /// gets it as it was written.
    #[test]
    fn scripts_are_one_quoted_word_for_sh() {
        let script = "a='it''s'; t=$(printf '%s' \"$a\") && echo \"$t $HOME\" 2>&1";
        let command = sh_command(script);
        let word = command.strip_prefix("sh -c ").expect("sh runs it");
        // Outside the single quotes there is only `\'`, an escaped quote.
        let mut quoted = false;
        let mut chars = word.chars();
        while let Some(c) = chars.next() {
            match c {
                '\'' => quoted = !quoted,
                '\\' if !quoted => assert_eq!(chars.next(), Some('\'')),
                c => assert!(quoted, "{c:?} is outside the quotes in {word}"),
            }
        }
        assert!(!quoted);
        #[cfg(unix)]
        {
            let out = std::process::Command::new("sh").arg("-c").arg(&command).env("HOME", "/h").output().unwrap();
            assert_eq!(String::from_utf8_lossy(&out.stdout), "its /h\n");
        }
    }

    #[test]
    fn a_hanging_command_ends_when_closed() {
        let Exec { mut reader, closer, .. } = Exec::hanging(b"hi\n");
        let mut buf = [0u8; 8];
        assert_eq!(reader.read(&mut buf).unwrap(), 3);
        closer.close();
        assert_eq!(reader.read(&mut buf).unwrap(), 0);
    }

    #[test]
    fn exec_reader_hands_out_chunks_then_ends() {
        let (tx, rx) = mpsc::channel();
        let mut r = ExecReader { rx, pending: Vec::new(), at: 0 };
        tx.send(b"hello".to_vec()).unwrap();
        tx.send(b" world".to_vec()).unwrap();
        drop(tx);
        let mut buf = [0u8; 3];
        assert_eq!(r.read(&mut buf).unwrap(), 3);
        assert_eq!(&buf, b"hel");
        let mut rest = Vec::new();
        r.read_to_end(&mut rest).unwrap();
        assert_eq!(rest, b"lo world");
    }
}
