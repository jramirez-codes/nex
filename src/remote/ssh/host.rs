//! One host nex is connected to with its own client: logging in (the first time with the
//! password, which installs the device key, see `device.rs`), making sure `nex-host` is
//! there, and the connection itself, logged in again with the key when it drops.
//!
//! Every terminal pane on the host is a `nex-host` session reached through
//! `nex-host connect` (see `terminal/remote.rs`), so a dropped connection loses nothing: the
//! panes attach again once [`RemoteHost::client`] is back.

use std::sync::{Arc, Mutex};

use nex_host::proto::{self, Request, Response};
use russh::keys::PrivateKey;

use super::device::{self, Profile};
use super::{Client, ConnectError, Exec, Login, Server};
use crate::remote::host as nex_host_install;

/// What runs `nex-host connect` on the host (the remote shell expands `~`). A program and
/// its argument, which any login shell runs: no `sh` in between.
const CONNECT_COMMAND: &str = "~/.nex/bin/nex-host connect";

/// Why logging in didn't work.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LoginError {
    /// There's no usable device key (first time, or it expired or was turned down): the
    /// password is needed.
    NeedsPassword,
    /// The host is new to nex: the user should check its key's fingerprint and trust it.
    UnknownHost { fingerprint: String },
    WrongPassword,
    Failed(String),
}

impl std::fmt::Display for LoginError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoginError::NeedsPassword => write!(f, "Enter your password to set up this device."),
            LoginError::UnknownHost { fingerprint } => write!(f, "First connection to this server. Its key is {fingerprint}."),
            LoginError::WrongPassword => write!(f, "The server turned the password down."),
            LoginError::Failed(e) => write!(f, "{e}"),
        }
    }
}

impl From<ConnectError> for LoginError {
    fn from(e: ConnectError) -> Self {
        match e {
            ConnectError::UnknownHost { fingerprint } => LoginError::UnknownHost { fingerprint },
            other => LoginError::Failed(format!("Couldn't connect: {other}.")),
        }
    }
}

/// A host, logged in to with the device key.
pub struct RemoteHost {
    pub server: Server,
    key: PrivateKey,
    /// The live connection, if any: `None` (or closed) until [`Self::client`] logs in again.
    client: Mutex<Option<Arc<Client>>>,
}

impl RemoteHost {
    /// The connection, logging in again with the device key if it dropped. Blocks while it
    /// does; panes waiting on it queue behind the lock, so only one of them logs in.
    pub fn client(&self) -> Result<Arc<Client>, ConnectError> {
        let mut slot = self.client.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(c) = slot.as_ref().filter(|c| !c.is_closed()) {
            return Ok(c.clone());
        }
        *slot = None;
        let client = Arc::new(Client::connect(&self.server, Login::Key(&self.key), None)?);
        *slot = Some(client.clone());
        Ok(client)
    }

    /// A new connection to the host's `nex-host` daemon (started if it isn't running), as
    /// one command's input and output.
    pub fn connect_daemon(&self) -> std::io::Result<Exec> {
        let client = self.client().map_err(|e| std::io::Error::new(std::io::ErrorKind::NotConnected, e.to_string()))?;
        client.exec(CONNECT_COMMAND)
    }

    /// Send `request` to the host's daemon on a new connection and read its answer. The
    /// connection comes back too: for `attach`, the session's terminal stream follows.
    pub fn ask(&self, request: &Request) -> std::io::Result<(Response, Exec)> {
        use std::io::{BufRead, Write};
        let mut exec = self.connect_daemon()?;
        let mut line = serde_json::to_string(request).map_err(std::io::Error::other)?;
        line.push('\n');
        exec.writer.write_all(line.as_bytes())?;
        let mut answer = Vec::new();
        // One byte at a time: nothing after the answer's line may be taken from `exec`.
        std::io::BufReader::with_capacity(1, &mut exec.reader).read_until(b'\n', &mut answer)?;
        if answer.is_empty() {
            return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "nex-host didn't answer"));
        }
        let response = serde_json::from_slice(&answer)
            .map_err(|_| std::io::Error::other(format!("nex-host said: {}", String::from_utf8_lossy(&answer).trim())))?;
        Ok((response, exec))
    }

    /// The plain shells running on the host that no pane shows (the ones this device had
    /// open when it was last closed), oldest first.
    pub fn idle_shells(&self) -> Vec<String> {
        let Ok((r, _)) = self.ask(&Request::List) else { return Vec::new() };
        r.sessions.unwrap_or_default().into_iter().filter(|s| s.kind == proto::SHELL_KIND && !s.attached && s.exit.is_none()).map(|s| s.id).collect()
    }
}

/// Log in to `server`: with the device key when there's a valid one for it, else with
/// `password` (which installs a new key, valid for `key_days`). `trust` is the fingerprint
/// the user agreed to for a host nex hasn't seen. Then makes sure the host has nex-host.
/// Blocks for as long as all that takes (seconds, more when nex-host is installed).
pub fn login(server: &Server, password: Option<&str>, trust: Option<&str>, key_days: u32) -> Result<RemoteHost, LoginError> {
    let profile = device::load_profile().filter(|p| p.server == *server).unwrap_or_else(|| Profile::new(server.clone()));
    let key = device::load_key().filter(|_| profile.key_valid(device::now()));
    let (client, key) = match (key, password) {
        // The key first, even with a password at hand: the password is for setting it up.
        (Some(key), _) => match Client::connect(server, Login::Key(&key), trust) {
            Ok(client) => (client, key),
            Err(ConnectError::Denied) if password.is_some() => with_password(server, profile, password.unwrap_or_default(), trust, key_days)?,
            Err(ConnectError::Denied) => return Err(LoginError::NeedsPassword),
            Err(e) => return Err(e.into()),
        },
        (None, Some(password)) => with_password(server, profile, password, trust, key_days)?,
        (None, None) => return Err(LoginError::NeedsPassword),
    };
    // The check and install scripts are `sh`: not for the login shell to read.
    nex_host_install::ensure_with(|script, stdin| {
        let (status, out) = client.run_sh(script, stdin.unwrap_or_default()).map_err(|e| crate::remote::RemoteError::Other(e.to_string()))?;
        let out = String::from_utf8_lossy(&out).into_owned();
        if status == 0 { Ok(out) } else { Err(crate::remote::RemoteError::Other(format!("the install script failed ({status})")).into()) }
    })
    .map_err(|e| LoginError::Failed(format!("Couldn't set up nex-host on the server: {e}.")))?;
    Ok(RemoteHost { server: server.clone(), key, client: Mutex::new(Some(Arc::new(client))) })
}

/// Log in with the password, put a new device key on the host, and check that the key
/// works by logging in with it. Returns that key login.
fn with_password(server: &Server, mut profile: Profile, password: &str, trust: Option<&str>, key_days: u32) -> Result<(Client, PrivateKey), LoginError> {
    let client = match Client::connect(server, Login::Password(password), trust) {
        Ok(c) => c,
        Err(ConnectError::Denied) => return Err(LoginError::WrongPassword),
        Err(e) => return Err(e.into()),
    };
    let failed = |what: &str, e: String| LoginError::Failed(format!("Couldn't {what}: {e}."));
    let (_, version) = client.run_sh(device::VERSION_COMMAND, b"").map_err(|e| failed("ask the server's ssh version", e.to_string()))?;
    let now = device::now();
    let expires = device::expiry(now, key_days);
    let host_expiry = device::supports_expiry(&String::from_utf8_lossy(&version)).then_some(expires);
    let key = device::new_key(&profile.comment).map_err(|e| failed("make this device's key", e.to_string()))?;
    let line = device::authorized_line(&key, &profile.comment, host_expiry).map_err(|e| failed("write the key", e))?;
    let (status, _) = client.run_sh(&device::install_script(&line, &profile.comment), b"").map_err(|e| failed("add the key on the server", e.to_string()))?;
    if status != 0 {
        return Err(failed("add the key to ~/.ssh/authorized_keys on the server", format!("the script failed ({status})")));
    }
    drop(client);
    // The key's own login, so a server that ignores it (permissions on ~/.ssh, an sshd that
    // only takes passwords) is found out now rather than at the next connection.
    let client = Client::connect(server, Login::Key(&key), None).map_err(|e| match e {
        ConnectError::Denied => LoginError::Failed(
            "The server didn't accept this device's key. Check that ~/.ssh is only writable by you and that sshd allows keys (PubkeyAuthentication).".into(),
        ),
        other => other.into(),
    })?;
    profile.key_expires = Some(expires);
    device::save_profile(&profile).map_err(|e| failed("save the login", e.to_string()))?;
    Ok((client, key))
}
