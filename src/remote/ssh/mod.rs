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
//! `russh` does the protocol, on a small tokio runtime of its own. Everything here blocks, so
//! it runs on background threads, never in a frame. A command's input and output are plain
//! `Read` / `Write` ([`Exec`]): tasks on the runtime move them to and from the channel.
//!
//! Host keys are trusted on first use, but only with the user's say: a host nex hasn't
//! seen fails with [`ConnectError::UnknownHost`] and its fingerprint. The user is shown that
//! fingerprint, and trusting it saves it to `known_hosts` in nex's config folder. A host
//! whose key changed is refused.

pub mod device;
pub mod host;
#[cfg(all(test, target_os = "linux"))]
mod sshd_tests;

use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::mpsc;
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use russh::ChannelMsg;
use russh::client::{self, Handle};
use russh::keys::{HashAlg, PrivateKey, PrivateKeyWithHashAlg, PublicKey};

/// Give up on a host that doesn't answer (or finish logging in) within this long.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(12);
/// Ping a quiet connection this often, and drop it after this many pings go unanswered:
/// a phone that changed networks finds out within a minute.
const KEEPALIVE: Duration = Duration::from_secs(15);
const KEEPALIVE_MAX: usize = 3;

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

/// A logged-in connection to a host. Commands on it run in channels of their own, any
/// number at once.
pub struct Client {
    handle: Handle<Checker>,
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
        let addr = (server.host.clone(), server.port);
        let user = server.user.clone();
        let result = runtime().block_on(async move {
            let connecting = async {
                let mut handle = client::connect(config, addr, checker).await?;
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
            (Ok(Ok((handle, true))), _) => Ok(Client { handle }),
        }
    }

    /// The connection has ended (the host hung up, or stopped answering keepalives).
    pub fn is_closed(&self) -> bool {
        self.handle.is_closed()
    }

    /// Run `command` (through the user's shell on the host), with its stdin and stdout as
    /// plain `Write` and `Read`. Blocks until the host has started it.
    pub fn exec(&self, command: &str) -> io::Result<Exec> {
        let command = command.to_string();
        let handle = &self.handle;
        let channel = runtime().block_on(async {
            let channel = handle.channel_open_session().await?;
            channel.exec(true, command).await?;
            Ok::<_, russh::Error>(channel)
        });
        let channel = channel.map_err(|e| io::Error::new(io::ErrorKind::ConnectionAborted, e.to_string()))?;
        Ok(Exec::start(channel))
    }

    /// Run `command`, feeding it `stdin`, and wait for it to end: its exit status and what it
    /// printed to stdout.
    pub fn run(&self, command: &str, stdin: &[u8]) -> io::Result<(u32, Vec<u8>)> {
        let mut exec = self.exec(command)?;
        exec.writer.write_all(stdin)?;
        drop(exec.writer);
        let mut out = Vec::new();
        exec.reader.read_to_end(&mut out)?;
        let status = exec.status.lock().unwrap_or_else(|e| e.into_inner()).unwrap_or(255);
        Ok((status, out))
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        let handle = &self.handle;
        let _ = runtime().block_on(async { handle.disconnect(russh::Disconnect::ByApplication, "", "en").await });
    }
}

/// A command running on the host: what it prints ([`ExecReader`]), what it reads
/// ([`ExecWriter`]; dropping it ends its input), and its exit status once it has ended.
pub struct Exec {
    pub reader: ExecReader,
    pub writer: ExecWriter,
    pub status: Arc<Mutex<Option<u32>>>,
}

impl Exec {
    /// Start the task that moves the channel's data to `reader` and `writer`'s data to it.
    fn start(channel: russh::Channel<client::Msg>) -> Exec {
        let (out_tx, out_rx) = mpsc::channel::<Vec<u8>>();
        let (in_tx, mut in_rx) = tokio::sync::mpsc::unbounded_channel::<Vec<u8>>();
        let status = Arc::new(Mutex::new(None));
        let exit = status.clone();
        let (mut read, write) = channel.split();
        runtime().spawn(async move {
            let mut input_open = true;
            loop {
                tokio::select! {
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
        Exec { reader: ExecReader { rx: out_rx, pending: Vec::new(), at: 0 }, writer: ExecWriter { tx: in_tx }, status }
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
