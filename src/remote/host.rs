//! `nex-host` on a pane's ssh host (see `nex-host/README.md`): installing it, watching the
//! agents it keeps running there, and ending one. (The command line that attaches a pane to
//! one is `SshTarget::attach_command`, in `target.rs`.)
//!
//! nex installs it in `~/.nex/bin` over its own batch connection (see the module above):
//! `uname` says which build the host needs, the version it prints says whether it's
//! already the right one, and the binary travels over ssh's stdin, gzipped when the host has
//! `gzip` (less than half the size). Then one long-lived `nex-host watch` per host streams its
//! agents as JSON lines, for the Agents pane. Starting the watch checks the version first, so
//! a host that's up to date takes one connection, not a check and then a watch.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use nex_host::proto::{self, Response, Session};

use crate::remote::error::last_line;
use crate::remote::ssh::host::RemoteHost;
use crate::remote::ssh::{Exec, ExecCloser};
use crate::remote::{RemoteError, SshTarget, posix_quote};

/// Where the program goes on the host, for the remote shell (which expands `~`). This and
/// the scripts below spell out [`nex_host::paths::INSTALL_PATH`] so they read as plain shell;
/// `install_paths_agree` checks they all name it.
pub(super) const REMOTE_BIN: &str = "~/.nex/bin/nex-host";
/// The same, for `sh` scripts.
pub(super) const REMOTE_BIN_SH: &str = "\"$HOME/.nex/bin/nex-host\"";
/// Wait this long before trying a host again after it failed, doubling up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_secs(3);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// Why a host can't be watched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// The connection failed, or the host wants a password nex's connections can't answer.
    Remote(RemoteError),
    /// nex has no `nex-host` built for it (`uname -sm` said this).
    Unsupported(String),
}

impl From<RemoteError> for HostError {
    fn from(e: RemoteError) -> Self {
        HostError::Remote(e)
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::Remote(RemoteError::NeedsLogin) => write!(f, "needs a login"),
            HostError::Remote(RemoteError::Other(e)) => write!(f, "{e}"),
            HostError::Unsupported(system) => write!(f, "no nex-host for {system}"),
        }
    }
}

/// The Rust target a host needs, from its `uname -sm`.
pub fn target_triple(uname: &str) -> Option<&'static str> {
    let mut words = uname.split_whitespace();
    let (os, arch) = (words.next()?, words.next()?);
    Some(match (os, arch) {
        ("Linux", "x86_64" | "amd64") => "x86_64-unknown-linux-musl",
        ("Linux", "aarch64" | "arm64") => "aarch64-unknown-linux-musl",
        ("Darwin", "arm64" | "aarch64") => "aarch64-apple-darwin",
        ("Darwin", "x86_64") => "x86_64-apple-darwin",
        _ => return None,
    })
}

/// The `nex-host` build for `triple`, from wherever nex finds one:
/// - `$NEX_HOST_DIR/nex-host-<triple>`
/// - next to nex: `../share/nex/host/` (the tarball and .deb), `../Resources/host/` (the
///   macOS app), `host/`
/// - in a development build, cargo's own `target/<triple>/{release,debug}/nex-host`.
pub fn local_binary(triple: &str) -> Option<PathBuf> {
    let name = format!("nex-host-{triple}");
    let mut candidates = Vec::new();
    if let Some(dir) = std::env::var_os("NEX_HOST_DIR") {
        candidates.push(PathBuf::from(dir).join(&name));
    }
    if let Some(dir) = std::env::current_exe().ok().and_then(|e| e.canonicalize().ok()).and_then(|e| e.parent().map(PathBuf::from)) {
        candidates.push(dir.join("../share/nex/host").join(&name));
        candidates.push(dir.join("../Resources/host").join(&name));
        candidates.push(dir.join("host").join(&name));
    }
    if cfg!(debug_assertions) {
        let target = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target").join(triple);
        candidates.push(target.join("release/nex-host"));
        candidates.push(target.join("debug/nex-host"));
    }
    candidates.into_iter().find(|p| p.is_file())
}

/// What the install check prints: `uname Linux x86_64`, `gzip yes` if the host can unpack a
/// gzipped binary, then if it's installed, `version nex-host 0.11.0 (protocol 1)` and
/// `shim on` (or `off`).
const CHECK_SCRIPT: &str = r#"echo "uname $(uname -sm)"; command -v gzip >/dev/null 2>&1 && echo "gzip yes"; p="$HOME/.nex/bin/nex-host"; if [ -x "$p" ]; then echo "version $("$p" --version 2>/dev/null)"; echo "shim $("$p" shim status 2>/dev/null)"; fi; true"#;

/// How the watch's script ends when the host needs this nex's nex-host first.
const NEEDS_INSTALL: i32 = 86;

/// Start the watch if the host has this nex's nex-host (one that answers `shim status`),
/// saying first whether the shim is on. If not, print the install check and exit with
/// [`NEEDS_INSTALL`].
fn watch_script() -> String {
    let want = posix_quote(&proto::version_line());
    format!(
        r#"p="$HOME/.nex/bin/nex-host"; if [ -x "$p" ] && [ "$("$p" --version 2>/dev/null)" = {want} ] && s=$("$p" shim status 2>/dev/null) && [ -n "$s" ]; then echo "shim $s"; exec "$p" watch; fi; {CHECK_SCRIPT}; exit {NEEDS_INSTALL}"#
    )
}

/// Put the binary arriving on stdin (gzipped, with `gzip`) in place: a temporary file, then
/// a rename, so a daemon still running the old one keeps going and nothing half-written is
/// ever run.
fn install_script(gzip: bool) -> String {
    let unpack = if gzip { "gzip -dc" } else { "cat" };
    INSTALL_SCRIPT.replace("cat >", &format!("{unpack} >"))
}

const INSTALL_SCRIPT: &str = r#"d="$HOME/.nex/bin" && mkdir -p "$d" && chmod 700 "$HOME/.nex" && cat > "$d/nex-host.tmp" && chmod 700 "$d/nex-host.tmp" && mv -f "$d/nex-host.tmp" "$d/nex-host" && echo "version $("$d/nex-host" --version)" && echo "shim $("$d/nex-host" shim status)""#;

/// What the install check found on a host.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct HostInfo {
    pub uname: String,
    /// The installed `nex-host --version`, if it's installed.
    pub version: Option<String>,
    /// Its shim is in the shell's config (see `nex-host/src/shim.rs`). `None` from a
    /// nex-host too old to say.
    pub shim: Option<bool>,
    /// The host has `gzip`, so the binary can travel compressed.
    pub gzip: bool,
}

impl HostInfo {
    fn parse(out: &str) -> HostInfo {
        let mut info = HostInfo::default();
        for line in out.lines() {
            match line.trim().split_once(' ') {
                Some(("uname", v)) => info.uname = v.trim().to_string(),
                Some(("version", v)) if !v.trim().is_empty() => info.version = Some(v.trim().to_string()),
                Some(("shim", v)) => info.shim = matches!(v.trim(), "on" | "off").then(|| v.trim() == "on"),
                Some(("gzip", v)) => info.gzip = v.trim() == "yes",
                _ => {}
            }
        }
        info
    }

    /// It's this nex's nex-host. (One built before `shim` had the same version line: it
    /// doesn't answer `shim status`, so it's replaced too.)
    fn current(&self) -> bool {
        self.version.as_deref() == Some(proto::version_line().as_str()) && self.shim.is_some()
    }
}

/// Make sure the host has this nex's `nex-host`, installing it if not. Blocks. The watch does
/// the same, folded into starting up (see [`watch_script`]). Only the install test uses it.
#[cfg(all(test, target_os = "linux"))]
pub fn ensure(target: &SshTarget) -> Result<HostInfo, HostError> {
    ensure_with(|script, stdin| run(target, script, stdin))
}

/// [`ensure`] over any connection: `run` runs a script on the host (feeding it the bytes
/// given) and returns what it printed. Checks for `nex-host connect` too, which nex's own ssh
/// client (Android, `--remote`) needs: a build from before `connect` can carry the same version
/// line, so it's asked for by name, and replaced when it's missing.
pub fn ensure_with(mut run: impl FnMut(&str, Option<&[u8]>) -> Result<String, HostError>) -> Result<HostInfo, HostError> {
    let out = run(&format!("{CHECK_SCRIPT}; {CONNECT_CHECK}"), None)?;
    let info = HostInfo::parse(&out);
    if info.current() && out.lines().any(|l| l == "connect yes") {
        return Ok(info);
    }
    install(run, info)
}

/// Prints `connect yes` when the installed nex-host has `connect`.
const CONNECT_CHECK: &str = r#""$HOME/.nex/bin/nex-host" help 2>/dev/null | grep -q "nex-host connect" && echo "connect yes"; true"#;

/// The `nex-host` program for `triple`: built into the app where there is no folder of
/// builds beside it (the phone apps, see `build.rs`), else from [`local_binary`].
fn binary(triple: &str) -> Result<Vec<u8>, String> {
    if let Some(bytes) = bundled(triple) {
        return Ok(bytes.to_vec());
    }
    let path = local_binary(triple).ok_or_else(|| format!("nex has no {triple} build"))?;
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(any(target_os = "android", target_os = "ios"))]
fn bundled(triple: &str) -> Option<&'static [u8]> {
    let bytes: &'static [u8] = match triple {
        "x86_64-unknown-linux-musl" => include_bytes!(concat!(env!("OUT_DIR"), "/nex-host-x86_64-unknown-linux-musl")),
        "aarch64-unknown-linux-musl" => include_bytes!(concat!(env!("OUT_DIR"), "/nex-host-aarch64-unknown-linux-musl")),
        "aarch64-apple-darwin" => include_bytes!(concat!(env!("OUT_DIR"), "/nex-host-aarch64-apple-darwin")),
        "x86_64-apple-darwin" => include_bytes!(concat!(env!("OUT_DIR"), "/nex-host-x86_64-apple-darwin")),
        _ => return None,
    };
    // `build.rs` leaves the file empty when there was no build to bundle.
    (!bytes.is_empty()).then_some(bytes)
}

#[cfg(not(any(target_os = "android", target_os = "ios")))]
fn bundled(_triple: &str) -> Option<&'static [u8]> {
    None
}

/// Send this nex's `nex-host` to a host the install check said `info` about. Blocks.
fn install(mut run: impl FnMut(&str, Option<&[u8]>) -> Result<String, HostError>, info: HostInfo) -> Result<HostInfo, HostError> {
    let HostInfo { uname, gzip, .. } = info;
    let triple = target_triple(&uname).ok_or_else(|| HostError::Unsupported(uname.clone()))?;
    let mut bytes = binary(triple).map_err(|e| HostError::Unsupported(format!("{uname} ({e})")))?;
    if gzip {
        bytes = gzipped(&bytes).map_err(|e| RemoteError::Other(format!("compressing nex-host: {e}")))?;
    }
    let installed = HostInfo { uname, gzip, ..HostInfo::parse(&run(&install_script(gzip), Some(&bytes))?) };
    if installed.current() {
        Ok(installed)
    } else {
        Err(RemoteError::Other(format!("installed nex-host says {:?}", installed.version.unwrap_or_default())).into())
    }
}

/// `bytes` as a gzip stream, as small as it gets (once per install, about 50 ms for nex-host).
fn gzipped(bytes: &[u8]) -> std::io::Result<Vec<u8>> {
    let mut gz = flate2::write::GzEncoder::new(Vec::with_capacity(bytes.len() / 2), flate2::Compression::best());
    gz.write_all(bytes)?;
    gz.finish()
}

/// Run `script` on the host over a batch connection (feeding it `stdin`), and return what
/// it printed.
fn run(target: &SshTarget, script: &str, stdin: Option<&[u8]>) -> Result<String, HostError> {
    let mut child = target
        .batch_command(script)
        .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| RemoteError::Other(format!("ssh: {e}")))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes).map_err(|e| RemoteError::Other(format!("sending nex-host: {e}")))?;
    }
    let out = child.wait_with_output().map_err(|e| RemoteError::Other(e.to_string()))?;
    if let Some(failure) = target.batch_failure(&out) {
        return Err(failure.into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// End an agent on the host. Blocks.
pub fn kill(target: &SshTarget, id: &str) -> Result<(), HostError> {
    run(target, &format!("{REMOTE_BIN_SH} kill {}", posix_quote(id)), None).map(drop)
}

/// Put the shim in the host's shell config (see `nex-host/src/shim.rs`). Blocks.
pub fn install_shim(target: &SshTarget) -> Result<(), HostError> {
    run(target, &format!("{REMOTE_BIN_SH} shim install"), None).map(drop)
}


/// How watching a host is going.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum State {
    /// Checking or installing `nex-host`, or connecting.
    Connecting,
    Live,
    Failed(HostError),
}

/// What's known about a host's agents.
#[derive(Clone, Debug)]
pub struct Snapshot {
    pub state: State,
    /// As of the last line `watch` sent (kept while reconnecting: the agents are still
    /// running on the host).
    pub sessions: Vec<Session>,
    /// The host's home folder, for `~/…`.
    pub home: Option<String>,
    /// When the last line came.
    pub updated: Option<Instant>,
    /// Whether the shim is on, once the install check has run.
    pub shim: Option<bool>,
    /// The version of the daemon `watch` reaches when it's older than the installed
    /// nex-host: kept running for its agents, it hands over once they finish.
    pub daemon_version: Option<String>,
}

impl Snapshot {
    /// Take in a line `watch` sent.
    fn update(&mut self, r: Response) {
        self.state = State::Live;
        self.sessions = r.sessions.unwrap_or_default();
        self.home = r.home.or(self.home.take());
        self.daemon_version = r.daemon_version;
        self.updated = Some(Instant::now());
    }
}

impl Default for Snapshot {
    fn default() -> Self {
        Self { state: State::Connecting, sessions: Vec::new(), home: None, updated: None, shim: None, daemon_version: None }
    }
}

/// Take a line `watch` sent into the snapshot.
fn apply_response(snapshot: &Mutex<Snapshot>, r: Response) {
    if let Ok(mut s) = snapshot.lock() {
        s.update(r);
    }
}

/// How long to wait before trying a host again: [`RETRY_MIN`], doubling up to [`RETRY_MAX`]
/// while it keeps failing.
struct Backoff {
    wait: Duration,
}

impl Backoff {
    fn new() -> Self {
        Backoff { wait: RETRY_MIN }
    }

    /// The wait before the next try.
    fn wait(&self) -> Duration {
        self.wait
    }

    /// The host failed again: wait twice as long next time.
    fn grow(&mut self) {
        self.wait = (self.wait * 2).min(RETRY_MAX);
    }

    /// The host answered: start over.
    fn reset(&mut self) {
        self.wait = RETRY_MIN;
    }

    /// A watch ended. The wait before the next one: short again if this one `heard` from the
    /// host, longer than the last if it didn't.
    fn ended(&mut self, heard: bool) -> Duration {
        if heard { self.reset() } else { self.grow() }
        self.wait
    }
}

/// One host being watched: a thread that installs `nex-host` if needed, then keeps a
/// `nex-host watch` running, reconnecting when it drops. Dropping it stops the watch.
pub struct Watch {
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    /// The `ssh` running the watch, killed to stop it.
    child: Arc<Mutex<Option<Child>>>,
    /// The watch's command on nex's own connection ([`Watch::start_direct`]), closed to stop it.
    closer: Arc<Mutex<Option<ExecCloser>>>,
}

impl Watch {
    pub fn start(target: SshTarget, wake: impl Fn() + Send + 'static) -> Self {
        let w = Watch::fake(Snapshot::default());
        let (snapshot, stop, child) = (w.snapshot.clone(), w.stop.clone(), w.child.clone());
        let _ = std::thread::Builder::new().name("nex-host-watch".into()).spawn(move || watch(&target, &snapshot, &stop, &child, &wake));
        w
    }

    /// Watch the host the window is connected to itself (Android, `--remote`), over that
    /// connection: there's no `ssh` program to run, and login already put this nex's
    /// `nex-host` there.
    pub fn start_direct(host: Arc<RemoteHost>, wake: impl Fn() + Send + 'static) -> Self {
        let w = Watch::fake(Snapshot::default());
        let (snapshot, stop, closer) = (w.snapshot.clone(), w.stop.clone(), w.closer.clone());
        let _ = std::thread::Builder::new().name("nex-host-watch".into()).spawn(move || watch_direct(&host, &snapshot, &stop, &closer, &wake));
        w
    }

    /// A watch that connects nowhere, showing `snapshot`: what the others start from, and
    /// for tests.
    pub fn fake(snapshot: Snapshot) -> Self {
        Watch { snapshot: Arc::new(Mutex::new(snapshot)), stop: Arc::default(), child: Arc::default(), closer: Arc::default() }
    }

    /// A copy of everything known about the host, its sessions included: for showing them.
    /// To only look at it (its state, whether it has sessions), use [`Self::with_snapshot`].
    pub fn snapshot(&self) -> Snapshot {
        self.with_snapshot(Snapshot::clone)
    }

    /// What `read` makes of the snapshot, looked at in place: nothing is copied. `read` runs
    /// with the snapshot locked, so it should only look.
    pub fn with_snapshot<R>(&self, read: impl FnOnce(&Snapshot) -> R) -> R {
        match self.snapshot.lock() {
            Ok(s) => read(&s),
            Err(_) => read(&Snapshot::default()),
        }
    }

    /// The shim was just installed (by nex).
    pub fn set_shim(&self, on: bool) {
        if let Ok(mut s) = self.snapshot.lock() {
            s.shim = Some(on);
        }
    }
}

impl Drop for Watch {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Ok(mut c) = self.child.lock()
            && let Some(c) = c.as_mut()
        {
            let _ = c.kill();
        }
        // The direct watch's thread is reading the command's output: closing it ends that read.
        if let Ok(mut c) = self.closer.lock()
            && let Some(c) = c.take()
        {
            c.close();
        }
    }
}

fn watch(target: &SshTarget, snapshot: &Mutex<Snapshot>, stop: &AtomicBool, slot: &Mutex<Option<Child>>, wake: &dyn Fn()) {
    let set_state = |state: State| {
        if let Ok(mut s) = snapshot.lock()
            && s.state != state
        {
            s.state = state;
            drop(s);
            wake();
        }
    };
    let mut retry = Backoff::new();
    while !stop.load(Ordering::Relaxed) {
        let child = target.batch_command(&watch_script()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                set_state(State::Failed(RemoteError::Other(format!("ssh: {e}")).into()));
                sleep(stop, retry.wait());
                continue;
            }
        };
        let (Some(stdout), mut stderr) = (child.stdout.take(), child.stderr.take()) else { continue };
        if let Ok(mut c) = slot.lock() {
            *c = Some(child);
        }
        let mut lines = 0;
        // The lines that aren't the watch's: `shim on` ahead of it, or the install check.
        let mut check = String::new();
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            let Ok(r) = serde_json::from_str::<Response>(&line) else {
                if let Some(shim) = HostInfo::parse(&line).shim
                    && let Ok(mut s) = snapshot.lock()
                {
                    s.shim = Some(shim);
                }
                check.push_str(&line);
                check.push('\n');
                continue;
            };
            lines += 1;
            apply_response(snapshot, r);
            wake();
        }
        let status = slot.lock().ok().and_then(|mut c| c.take()).and_then(|mut c| c.wait().ok());
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mut err = String::new();
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_string(&mut err);
        }
        if status.and_then(|s| target.batch_exit(s, &err)) == Some(RemoteError::NeedsLogin) {
            set_state(State::Failed(RemoteError::NeedsLogin.into()));
        } else if status.and_then(|s| s.code()) == Some(NEEDS_INSTALL) {
            match install(|script, stdin| run(target, script, stdin), HostInfo::parse(&check)) {
                Ok(info) => {
                    if let Ok(mut s) = snapshot.lock() {
                        s.shim = info.shim;
                    }
                    // Straight back to watching, now that it's there.
                    retry.reset();
                    continue;
                }
                Err(e) => {
                    let wait = if matches!(e, HostError::Unsupported(_)) { RETRY_MAX * 5 } else { retry.wait() };
                    set_state(State::Failed(e));
                    sleep(stop, wait);
                    retry.grow();
                    continue;
                }
            }
        } else {
            set_state(State::Failed(RemoteError::Other(last_line(&err).unwrap_or_else(|| "connection lost".into())).into()));
        }
        sleep(stop, retry.ended(lines > 0));
    }
}

/// What [`Watch::start_direct`]'s thread runs: `nex-host watch` on the connection, again
/// whenever it ends, until told to stop.
fn watch_direct(host: &RemoteHost, snapshot: &Mutex<Snapshot>, stop: &AtomicBool, closer: &Mutex<Option<ExecCloser>>, wake: &dyn Fn()) {
    let fail = |why: String| {
        if let Ok(mut s) = snapshot.lock() {
            s.state = State::Failed(RemoteError::Other(why).into());
        }
        wake();
    };
    let mut retry = Backoff::new();
    while !stop.load(Ordering::Relaxed) {
        let exec = match host.client() {
            // An `sh` script, so not for the login shell to read.
            Ok(client) => client.exec_sh(DIRECT_WATCH),
            Err(e) => Err(std::io::Error::other(e.to_string())),
        };
        let exec = match exec {
            Ok(e) => e,
            Err(e) => {
                fail(e.to_string());
                sleep(stop, retry.wait());
                retry.grow();
                continue;
            }
        };
        let lines = read_direct(exec, snapshot, stop, closer, wake);
        if stop.load(Ordering::Relaxed) {
            break;
        }
        fail("connection lost".into());
        sleep(stop, retry.ended(lines > 0));
    }
}

/// Read one run of `nex-host watch` on nex's own connection into `snapshot`, until it ends or
/// the [`Watch`] is dropped: `closer` holds the command's closer meanwhile, which
/// [`Watch`]'s `Drop` uses to end it (and so this read, which would otherwise wait for a line
/// that may never come). Returns how many lines it read.
fn read_direct(exec: Exec, snapshot: &Mutex<Snapshot>, stop: &AtomicBool, closer: &Mutex<Option<ExecCloser>>, wake: &dyn Fn()) -> usize {
    if let Ok(mut c) = closer.lock() {
        *c = Some(exec.closer.clone());
    }
    // Dropped before the closer was there to use: end the command here.
    if stop.load(Ordering::Relaxed) {
        exec.closer.close();
    }
    let mut lines = 0;
    for line in BufReader::new(exec.reader).lines() {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let Ok(line) = line else { break };
        let Ok(r) = serde_json::from_str::<Response>(&line) else { continue };
        lines += 1;
        apply_response(snapshot, r);
        wake();
    }
    if let Ok(mut c) = closer.lock() {
        *c = None;
    }
    // Ended on the host too, however the read ended (nothing happens if it already has).
    exec.closer.close();
    lines
}

/// The watch, on a host the window is connected to itself. An `sh` script (`exec`, `$HOME`):
/// it runs with `Client::exec_sh`.
const DIRECT_WATCH: &str = "exec \"$HOME/.nex/bin/nex-host\" watch";

/// Sleep `d`, waking early when told to stop.
fn sleep(stop: &AtomicBool, d: Duration) {
    let end = Instant::now() + d;
    while !stop.load(Ordering::Relaxed) && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(200));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_watch_line_from_an_older_daemon_says_so() {
        let mut snap = Snapshot::default();
        let line = r#"{"ok":true,"sessions":[],"home":"/home/u","daemon_version":"nex-host 0.12.2 (protocol 3)"}"#;
        snap.update(serde_json::from_str(line).unwrap());
        assert_eq!((snap.state.clone(), snap.daemon_version.as_deref()), (State::Live, Some("nex-host 0.12.2 (protocol 3)")));
        // The new daemon's lines don't: the update is done.
        snap.update(serde_json::from_str(r#"{"ok":true,"sessions":[]}"#).unwrap());
        assert_eq!((snap.daemon_version, snap.home.as_deref()), (None, Some("/home/u")));
    }

    #[test]
    fn a_watch_line_goes_into_the_shared_snapshot() {
        let snapshot = Mutex::new(Snapshot { shim: Some(true), ..Default::default() });
        apply_response(&snapshot, serde_json::from_str(r#"{"ok":true,"sessions":[],"home":"/home/u"}"#).unwrap());
        let s = snapshot.lock().unwrap();
        assert_eq!((s.state.clone(), s.home.as_deref(), s.shim), (State::Live, Some("/home/u"), Some(true)));
        assert!(s.updated.is_some());
    }

    #[test]
    fn backoff_doubles_to_a_ceiling_and_starts_over() {
        let mut b = Backoff::new();
        assert_eq!(b.wait(), RETRY_MIN);
        b.grow();
        assert_eq!(b.wait(), RETRY_MIN * 2);
        // A watch that heard nothing waits longer each time, up to the ceiling.
        assert_eq!(b.ended(false), RETRY_MIN * 4);
        for _ in 0..10 {
            b.ended(false);
        }
        assert_eq!(b.wait(), RETRY_MAX);
        // One that heard from the host starts over.
        assert_eq!(b.ended(true), RETRY_MIN);
    }

    /// Each error says ssh's failure in its own words.
    #[test]
    fn host_errors_read() {
        assert_eq!(HostError::from(RemoteError::NeedsLogin).to_string(), "needs a login");
        assert_eq!(HostError::from(RemoteError::Other("connection lost".into())).to_string(), "connection lost");
        assert_eq!(HostError::Unsupported("FreeBSD amd64".into()).to_string(), "no nex-host for FreeBSD amd64");
    }

    #[test]
    fn install_paths_agree() {
        let path = nex_host::paths::INSTALL_PATH;
        assert_eq!(REMOTE_BIN, format!("~/{path}"));
        assert_eq!(REMOTE_BIN_SH, format!("\"$HOME/{path}\""));
        assert!(CHECK_SCRIPT.contains(&format!("p=\"$HOME/{path}\"")));
        assert!(watch_script().contains(&format!("p=\"$HOME/{path}\"")));
        let (dir, file) = path.rsplit_once('/').unwrap();
        assert!(INSTALL_SCRIPT.starts_with(&format!("d=\"$HOME/{dir}\"")));
        assert!(INSTALL_SCRIPT.contains(&format!("\"$d/{file}\"")));
    }

    fn session(id: &str) -> Session {
        serde_json::from_value(serde_json::json!({
            "id": id, "kind": "claude", "name": id, "cwd": "/", "argv": ["claude"], "pid": 1, "started": 0,
            "last_output": 0, "last_input": 0, "status": "idle"
        }))
        .unwrap()
    }

    /// The direct watch's read, on a thread as `watch_direct` runs it; the receiver hears
    /// how many lines it read once it has ended.
    fn reading(w: &Watch, exec: Exec) -> std::sync::mpsc::Receiver<usize> {
        let (snapshot, stop, closer) = (w.snapshot.clone(), w.stop.clone(), w.closer.clone());
        let (tx, ended) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(read_direct(exec, &snapshot, &stop, &closer, &|| {}));
        });
        ended
    }

    /// Dropping the watch ends the direct connection's `nex-host watch`, and the thread
    /// reading it, though no line is on its way.
    #[test]
    fn dropping_a_direct_watch_stops_its_command() {
        let line = serde_json::to_string(&serde_json::json!({ "ok": true, "sessions": [session("a")], "home": "/home/me" })).unwrap();
        let w = Watch::fake(Snapshot::default());
        let ended = reading(&w, Exec::hanging(format!("{line}\n").as_bytes()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !w.with_snapshot(|s| s.state == State::Live) {
            assert!(Instant::now() < deadline, "the watch's line never arrived");
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(w.with_snapshot(|s| (s.sessions.len(), s.home.clone())), (1, Some("/home/me".into())));
        // Quiet from here on: the read is waiting, and stays so while the watch lives.
        assert!(ended.recv_timeout(Duration::from_millis(200)).is_err(), "still watching");
        let closer = w.closer.clone();
        drop(w);
        assert_eq!(ended.recv_timeout(Duration::from_secs(5)).expect("the read ended with the watch"), 1);
        assert!(closer.lock().unwrap().is_none(), "nothing kept of the ended command");
    }

    /// A watch dropped while its thread was still connecting: the command that then starts
    /// is ended at once.
    #[test]
    fn a_direct_watch_dropped_while_connecting_stops_too() {
        let w = Watch::fake(Snapshot::default());
        w.stop.store(true, Ordering::Relaxed);
        let ended = reading(&w, Exec::hanging(b""));
        assert_eq!(ended.recv_timeout(Duration::from_secs(5)).expect("the read ended"), 0);
    }

    /// Looking at a snapshot copies nothing (the sessions stay where they are), where
    /// `snapshot()` copies every session on every call.
    #[test]
    fn with_snapshot_looks_without_copying() {
        let sessions: Vec<Session> = (0..200).map(|i| session(&format!("s{i}"))).collect();
        let w = Watch::fake(Snapshot { state: State::Live, sessions, ..Default::default() });
        let held = w.with_snapshot(|s| s.sessions.as_ptr());
        assert_eq!(w.with_snapshot(|s| s.sessions.as_ptr()), held, "the watch's own sessions, in place");
        assert_ne!(w.snapshot().sessions.as_ptr(), held, "a snapshot is a copy");
        assert_eq!(w.with_snapshot(|s| s.sessions.len()), 200);
    }

    #[test]
    fn picks_the_build_a_host_needs() {
        assert_eq!(target_triple("Linux x86_64"), Some("x86_64-unknown-linux-musl"));
        assert_eq!(target_triple("Linux aarch64\n"), Some("aarch64-unknown-linux-musl"));
        assert_eq!(target_triple("Darwin arm64"), Some("aarch64-apple-darwin"));
        assert_eq!(target_triple("Darwin x86_64"), Some("x86_64-apple-darwin"));
        assert_eq!(target_triple("FreeBSD amd64"), None);
        assert_eq!(target_triple(""), None);
    }

    #[test]
    fn reads_the_install_check() {
        let v = proto::version_line();
        let info = HostInfo::parse(&format!("uname Linux x86_64\nversion {v}\nshim on\n"));
        assert_eq!(info, HostInfo { uname: "Linux x86_64".into(), version: Some(v.clone()), shim: Some(true), gzip: false });
        assert!(HostInfo::parse("uname Linux x86_64\ngzip yes\n").gzip);
        assert!(info.current());
        let missing = HostInfo::parse("uname Darwin arm64\n");
        assert_eq!(missing.version, None);
        assert!(!missing.current() && missing.shim.is_none());
        // One from before `shim`, with the same version: replaced.
        let old = HostInfo::parse(&format!("uname Linux x86_64\nversion {v}\nshim \n"));
        assert_eq!(old.shim, None);
        assert!(!old.current());
    }

    /// The install's remote half, run here with a local `sh`: a gzipped binary arrives whole.
    #[test]
    #[cfg(unix)]
    fn gzipped_install_unpacks() {
        let home = std::env::temp_dir().join(format!("nex-gz-install-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let program = b"#!/bin/sh\n[ \"$1\" = --version ] && echo 'nex-host test'\n[ \"$1\" = shim ] && echo off\ntrue\n".repeat(50);
        let packed = gzipped(&program).unwrap();
        assert!(packed.len() < program.len() / 4);
        let mut sh = std::process::Command::new("sh")
            .args(["-c", &install_script(true)])
            .env("HOME", &home)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        sh.stdin.take().unwrap().write_all(&packed).unwrap();
        let out = sh.wait_with_output().unwrap();
        assert!(out.status.success());
        assert_eq!(std::fs::read(home.join(".nex/bin/nex-host")).unwrap(), program);
        let info = HostInfo::parse(&String::from_utf8_lossy(&out.stdout));
        assert_eq!((info.version.as_deref(), info.shim), (Some("nex-host test"), Some(false)));
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// A nex-host from before `connect` (same version line) is caught by name.
    #[test]
    #[cfg(unix)]
    fn connect_check_finds_a_nex_host_without_connect() {
        use std::os::unix::fs::PermissionsExt;
        for (help, expect) in [("  nex-host connect   one connection", true), ("  nex-host watch", false)] {
            let home = std::env::temp_dir().join(format!("nex-connect-check-{}-{expect}", std::process::id()));
            let bin = home.join(".nex/bin");
            std::fs::create_dir_all(&bin).unwrap();
            let p = bin.join("nex-host");
            std::fs::write(&p, format!("#!/bin/sh\necho '{help}'\n")).unwrap();
            std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o700)).unwrap();
            let out = std::process::Command::new("sh").args(["-c", CONNECT_CHECK]).env("HOME", &home).output().unwrap();
            assert!(out.status.success());
            assert_eq!(String::from_utf8_lossy(&out.stdout).lines().any(|l| l == "connect yes"), expect);
            let _ = std::fs::remove_dir_all(&home);
        }
    }

    /// The watch's script on a host without nex-host: the install check, and the exit code
    /// that says to install.
    #[test]
    #[cfg(unix)]
    fn watch_script_asks_for_an_install() {
        let home = std::env::temp_dir().join(format!("nex-watch-script-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let out = std::process::Command::new("sh").args(["-c", &watch_script()]).env("HOME", &home).output().unwrap();
        assert_eq!(out.status.code(), Some(NEEDS_INSTALL));
        let info = HostInfo::parse(&String::from_utf8_lossy(&out.stdout));
        assert!(!info.uname.is_empty() && info.version.is_none());
        std::fs::remove_dir_all(&home).unwrap();
    }

    /// The whole path against a stand-in `ssh` that runs the "remote" side here, in a home
    /// of its own: install, a second check that doesn't reinstall, and the watch hearing an
    /// agent start and end. Changes `PATH`, so it runs on its own:
    /// `cargo build -p nex-host --release --target x86_64-unknown-linux-musl` (or the
    /// aarch64 one), then `cargo test installs_and_watches -- --ignored`.
    #[test]
    #[ignore = "needs a built nex-host and changes PATH"]
    #[cfg(target_os = "linux")]
    fn installs_and_watches_a_host() {
        use std::os::unix::fs::PermissionsExt;

        let root = std::env::temp_dir().join(format!("nex-fake-host-{}", std::process::id()));
        let (bin, home) = (root.join("bin"), root.join("home"));
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        // Skip ssh's options and the destination; run the command with the fake home.
        let ssh = "#!/bin/sh\nwhile [ $# -gt 0 ]; do [ \"$1\" = -- ] && { shift; break; }; shift; done\nshift\nexport HOME=\"$FAKE_HOME\" NEX_HOST_IDLE_EXIT=2\nexec sh -c \"$1\"\n";
        std::fs::write(bin.join("ssh"), ssh).unwrap();
        std::fs::set_permissions(bin.join("ssh"), std::fs::Permissions::from_mode(0o755)).unwrap();
        unsafe {
            std::env::set_var("PATH", format!("{}:{}", bin.display(), std::env::var("PATH").unwrap_or_default()));
            std::env::set_var("FAKE_HOME", &home);
        }

        let t = SshTarget { opts: vec![], destination: "fake".into() };
        assert_eq!(ensure(&t).expect("install").shim, Some(false), "no shim until asked");
        install_shim(&t).expect("shim");
        assert!(std::fs::read_to_string(home.join(".bashrc")).unwrap_or_default().contains("run -- claude") || home.join(".config/fish/conf.d/nex-host.fish").exists());
        assert_eq!(ensure(&t).unwrap().shim, Some(true));
        let installed = home.join(".nex/bin/nex-host");
        let first = std::fs::metadata(&installed).unwrap().modified().unwrap();
        ensure(&t).expect("already installed");
        assert_eq!(std::fs::metadata(&installed).unwrap().modified().unwrap(), first, "not sent again");

        let w = Watch::start(t.clone(), || {});
        let wait = |what: &str, ok: &dyn Fn(&Snapshot) -> bool| {
            let end = Instant::now() + Duration::from_secs(15);
            while !ok(&w.snapshot()) {
                assert!(Instant::now() < end, "timed out waiting for {what}: {:?}", w.snapshot());
                std::thread::sleep(Duration::from_millis(100));
            }
        };
        wait("the watch", &|s| s.state == State::Live);
        let id = run(&t, &format!("{REMOTE_BIN_SH} launch --name demo -- sh -c 'sleep 30'"), None).unwrap().trim().to_string();
        wait("the agent", &|s| s.sessions.iter().any(|a| a.id == id && a.name == "demo"));
        assert_eq!(w.snapshot().home.as_deref(), Some(home.to_str().unwrap()));
        kill(&t, &id).unwrap();
        wait("it to go", &|s| s.sessions.is_empty());
        drop(w);
        let _ = std::fs::remove_dir_all(&root);
    }
}
