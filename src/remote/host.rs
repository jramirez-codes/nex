//! `nex-host` on a pane's ssh host (see `nex-host/README.md`): installing it, watching the
//! agents it keeps running there, and the commands that attach to one or drop one.
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

use super::{SshFailure, SshTarget, last_line, needs_login, posix_quote};

/// Where the program goes on the host, for the remote shell (which expands `~`). This and
/// the scripts below spell out [`nex_host::paths::INSTALL_PATH`] so they read as plain shell;
/// `install_paths_agree` checks they all name it.
const REMOTE_BIN: &str = "~/.nex/bin/nex-host";
/// The same, for `sh` scripts.
const REMOTE_BIN_SH: &str = "\"$HOME/.nex/bin/nex-host\"";
/// Wait this long before trying a host again after it failed, doubling up to [`RETRY_MAX`].
const RETRY_MIN: Duration = Duration::from_secs(3);
const RETRY_MAX: Duration = Duration::from_secs(60);

/// Why a host can't be watched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostError {
    /// It wants a password (or another prompt) nex's connections can't answer.
    NeedsLogin,
    /// nex has no `nex-host` built for it (`uname -sm` said this).
    Unsupported(String),
    Other(String),
}

impl From<SshFailure> for HostError {
    fn from(f: SshFailure) -> Self {
        match f {
            SshFailure::NeedsLogin => HostError::NeedsLogin,
            SshFailure::Other(e) => HostError::Other(e),
        }
    }
}

impl std::fmt::Display for HostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HostError::NeedsLogin => write!(f, "needs a login"),
            HostError::Unsupported(system) => write!(f, "no nex-host for {system}"),
            HostError::Other(e) => write!(f, "{e}"),
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
/// given) and returns what it printed. nex's own ssh client uses this (see `ssh/host.rs`).
pub fn ensure_with(mut run: impl FnMut(&str, Option<&[u8]>) -> Result<String, HostError>) -> Result<HostInfo, HostError> {
    let info = HostInfo::parse(&run(CHECK_SCRIPT, None)?);
    if info.current() {
        return Ok(info);
    }
    install(run, info)
}

/// The `nex-host` program for `triple`: built into the app where there is no folder of
/// builds beside it (Android, see `build.rs`), else from [`local_binary`].
fn binary(triple: &str) -> Result<Vec<u8>, String> {
    if let Some(bytes) = bundled(triple) {
        return Ok(bytes.to_vec());
    }
    let path = local_binary(triple).ok_or_else(|| format!("nex has no {triple} build"))?;
    std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(target_os = "android")]
fn bundled(triple: &str) -> Option<&'static [u8]> {
    let bytes: &'static [u8] = match triple {
        "x86_64-unknown-linux-musl" => include_bytes!(concat!(env!("OUT_DIR"), "/nex-host-x86_64-unknown-linux-musl")),
        "aarch64-unknown-linux-musl" => include_bytes!(concat!(env!("OUT_DIR"), "/nex-host-aarch64-unknown-linux-musl")),
        _ => return None,
    };
    // `build.rs` leaves the file empty when there was no build to bundle.
    (!bytes.is_empty()).then_some(bytes)
}

#[cfg(not(target_os = "android"))]
fn bundled(_triple: &str) -> Option<&'static [u8]> {
    None
}

/// Send this nex's `nex-host` to a host the install check said `info` about. Blocks.
fn install(mut run: impl FnMut(&str, Option<&[u8]>) -> Result<String, HostError>, info: HostInfo) -> Result<HostInfo, HostError> {
    let HostInfo { uname, gzip, .. } = info;
    let triple = target_triple(&uname).ok_or_else(|| HostError::Unsupported(uname.clone()))?;
    let mut bytes = binary(triple).map_err(|e| HostError::Unsupported(format!("{uname} ({e})")))?;
    if gzip {
        bytes = gzipped(&bytes).map_err(|e| HostError::Other(format!("compressing nex-host: {e}")))?;
    }
    let installed = HostInfo { uname, gzip, ..HostInfo::parse(&run(&install_script(gzip), Some(&bytes))?) };
    if installed.current() {
        Ok(installed)
    } else {
        Err(HostError::Other(format!("installed nex-host says {:?}", installed.version.unwrap_or_default())))
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
        .map_err(|e| HostError::Other(format!("ssh: {e}")))?;
    if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
        pipe.write_all(bytes).map_err(|e| HostError::Other(format!("sending nex-host: {e}")))?;
    }
    let out = child.wait_with_output().map_err(|e| HostError::Other(e.to_string()))?;
    let stderr = String::from_utf8_lossy(&out.stderr);
    // A login prompt counts even when ssh exits 0 (unlike `SshFailure::of`).
    if needs_login(&stderr) {
        return Err(HostError::NeedsLogin);
    }
    if let Some(failure) = SshFailure::of(out.status, &stderr) {
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

impl SshTarget {
    /// The same host without the options nex adds to its own panes' command lines, so a
    /// pane nex opened and the one it came from count as one host.
    pub fn normalized(&self) -> SshTarget {
        let mut opts = Vec::with_capacity(self.opts.len());
        let mut i = 0;
        while i < self.opts.len() {
            if self.opts[i] == "-o" && self.opts.get(i + 1).is_some_and(|v| v == "RemoteCommand=none") {
                i += 2;
                continue;
            }
            opts.push(self.opts[i].clone());
            i += 1;
        }
        SshTarget { opts, destination: self.destination.clone() }
    }

    /// The command line for a pane showing agent `id` on this host: `ssh -t` running
    /// `nex-host attach`. When the agent ends (or another pane takes it), ssh ends, and so
    /// does the pane.
    pub fn attach_command(&self, id: &str) -> Vec<String> {
        let mut ssh: Vec<String> = ["ssh", "-t", "-o", "RemoteCommand=none"].map(String::from).to_vec();
        ssh.extend(self.opts.iter().cloned());
        ssh.push("--".into());
        ssh.push(self.destination.clone());
        ssh.push(format!("{REMOTE_BIN} attach {}", posix_quote(id)));
        ssh
    }

    /// The agent a pane running `argv` (an `ssh` command line) attaches to, if it's one of
    /// [`attach_command`](Self::attach_command)'s.
    pub fn attach_id(argv: &[String]) -> Option<String> {
        let words: Vec<&str> = argv.last()?.split_whitespace().collect();
        match words.as_slice() {
            [bin, "attach", id] if bin.ends_with("nex-host") => Some(id.trim_matches('\'').to_string()),
            _ => None,
        }
    }
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
}

impl Default for Snapshot {
    fn default() -> Self {
        Self { state: State::Connecting, sessions: Vec::new(), home: None, updated: None, shim: None }
    }
}

/// One host being watched: a thread that installs `nex-host` if needed, then keeps a
/// `nex-host watch` running, reconnecting when it drops. Dropping it stops the watch.
pub struct Watch {
    snapshot: Arc<Mutex<Snapshot>>,
    stop: Arc<AtomicBool>,
    child: Arc<Mutex<Option<Child>>>,
}

impl Watch {
    pub fn start(target: SshTarget, wake: impl Fn() + Send + 'static) -> Self {
        let w = Watch { snapshot: Arc::default(), stop: Arc::default(), child: Arc::default() };
        let (snapshot, stop, child) = (w.snapshot.clone(), w.stop.clone(), w.child.clone());
        let _ = std::thread::Builder::new().name("nex-host-watch".into()).spawn(move || watch(&target, &snapshot, &stop, &child, &wake));
        w
    }

    /// A watch that connects nowhere, showing `snapshot`, for tests.
    #[cfg(test)]
    pub fn fake(snapshot: Snapshot) -> Self {
        Watch { snapshot: Arc::new(Mutex::new(snapshot)), stop: Arc::default(), child: Arc::default() }
    }

    pub fn snapshot(&self) -> Snapshot {
        self.snapshot.lock().map(|s| s.clone()).unwrap_or_default()
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
    let mut retry = RETRY_MIN;
    while !stop.load(Ordering::Relaxed) {
        let child = target.batch_command(&watch_script()).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn();
        let mut child = match child {
            Ok(c) => c,
            Err(e) => {
                set_state(State::Failed(HostError::Other(format!("ssh: {e}"))));
                sleep(stop, retry);
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
            if let Ok(mut s) = snapshot.lock() {
                s.state = State::Live;
                s.sessions = r.sessions.unwrap_or_default();
                s.home = r.home.or(s.home.take());
                s.updated = Some(Instant::now());
            }
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
        if needs_login(&err) {
            set_state(State::Failed(HostError::NeedsLogin));
        } else if status.and_then(|s| s.code()) == Some(NEEDS_INSTALL) {
            match install(|script, stdin| run(target, script, stdin), HostInfo::parse(&check)) {
                Ok(info) => {
                    if let Ok(mut s) = snapshot.lock() {
                        s.shim = info.shim;
                    }
                    // Straight back to watching, now that it's there.
                    retry = RETRY_MIN;
                    continue;
                }
                Err(e) => {
                    let wait = if matches!(e, HostError::Unsupported(_)) { RETRY_MAX * 5 } else { retry };
                    set_state(State::Failed(e));
                    sleep(stop, wait);
                    retry = (retry * 2).min(RETRY_MAX);
                    continue;
                }
            }
        } else {
            set_state(State::Failed(HostError::Other(last_line(&err).unwrap_or_else(|| "connection lost".into()))));
        }
        retry = if lines > 0 { RETRY_MIN } else { (retry * 2).min(RETRY_MAX) };
        sleep(stop, retry);
    }
}

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

    fn target(dest: &str) -> SshTarget {
        SshTarget { opts: vec!["-p".into(), "2222".into()], destination: dest.into() }
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

    #[test]
    fn attach_commands_round_trip() {
        let argv = target("me@devbox").attach_command("3fa91c0e");
        assert_eq!(argv, ["ssh", "-t", "-o", "RemoteCommand=none", "-p", "2222", "--", "me@devbox", "~/.nex/bin/nex-host attach 3fa91c0e"]);
        assert_eq!(SshTarget::attach_id(&argv).as_deref(), Some("3fa91c0e"));
        // The pane's ssh reads as a session on the same host, so uploads and the mic work.
        assert_eq!(SshTarget::from_argv(&argv).map(|t| t.normalized()), Some(target("me@devbox")));
        assert_eq!(SshTarget::attach_id(&["ssh".into(), "devbox".into()]), None);
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
