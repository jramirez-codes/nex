//! One shared ssh connection per host for nex's own traffic (the nex-host check and install,
//! the Agents watch, uploads, the mic), so only the first of them pays for a login: each one
//! after that opens a channel on the same connection, one round trip instead of a TCP
//! connect, a key exchange and authentication.
//!
//! Where the user's config already names a `ControlPath` for the host (nex's own sharing
//! block does, inside nex), that is the socket used, so nex's connections and the panes'
//! share one. Otherwise nex uses `~/.ssh/nex-%C`, the sharing block's path. When no master is
//! up there, nex starts one itself (`ssh -M -N -f`, logging in like every batch connection:
//! keys or an agent, never a prompt), which stays up [`PERSIST`] after its last session.
//! When that can't be done, connections go direct, as they did before.
//!
//! Whether the master is up is asked of its socket (`ssh -O check`, a process of its own),
//! and a yes is believed for [`CHECK_FOR`]: a listing, a Changes refresh and the commands
//! around them share one check instead of each paying for its own. A command that ssh itself
//! fails takes the answer back (see [`note_exit`]), so the next one asks again.

use std::collections::HashMap;
use std::path::Path;
use std::process::{ExitStatus, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::remote::target::BATCH_OPTS;
use crate::remote::{SHARING_SUPPORTED, SshTarget, ssh_command};

/// nex's control path, when the user's config has none for the host.
const OUR_PATH: &str = "~/.ssh/nex-%C";
/// How long a master nex started stays up after its last session ends.
const PERSIST: &str = "ControlPersist=120";
/// After starting a master failed, go direct for this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(60);
/// How long a master that answered is taken to still be up, without asking again. Short
/// against [`PERSIST`], and a master that dies inside it costs one command a direct
/// connection (or a failure that asks again, see [`note_exit`]).
const CHECK_FOR: Duration = Duration::from_secs(5);

/// What's known about sharing a connection to one host.
#[derive(Default)]
struct HostMux {
    /// `ssh -G` has been asked.
    resolved: bool,
    /// The control socket to use, expanded; `None` to leave multiplexing alone.
    path: Option<String>,
    /// nex may start the master itself (the user's config doesn't say `ControlMaster no`).
    may_start: bool,
    /// When starting one last failed.
    failed: Option<Instant>,
    /// When the master last answered at `path`.
    live: Option<Instant>,
}

/// What the mux asks of `ssh`. Tests put their own in its place.
trait Ssh: Send + Sync {
    /// Which socket to use for `target`, and whether nex may start a master on it.
    fn resolve(&self, target: &SshTarget) -> (Option<String>, bool);
    /// Whether a master is listening at `path`.
    fn check(&self, target: &SshTarget, path: &str) -> bool;
    /// Start a master at `path`; whether one is listening there now.
    fn start(&self, target: &SshTarget, path: &str) -> bool;
}

/// The `ssh` program.
struct SystemSsh;

impl Ssh for SystemSsh {
    fn resolve(&self, target: &SshTarget) -> (Option<String>, bool) {
        resolve(target)
    }
    fn check(&self, target: &SshTarget, path: &str) -> bool {
        check(target, Some(path))
    }
    fn start(&self, target: &SshTarget, path: &str) -> bool {
        start(target, path)
    }
}

/// No multiplexing: every connection goes direct.
struct Unshared;

impl Ssh for Unshared {
    fn resolve(&self, _: &SshTarget) -> (Option<String>, bool) {
        (None, false)
    }
    fn check(&self, _: &SshTarget, _: &str) -> bool {
        false
    }
    fn start(&self, _: &SshTarget, _: &str) -> bool {
        false
    }
}

/// The shared connections, one per host.
struct Mux {
    ssh: Box<dyn Ssh>,
    hosts: Mutex<HashMap<SshTarget, Arc<Mutex<HostMux>>>>,
}

impl Mux {
    fn new(ssh: Box<dyn Ssh>) -> Self {
        Mux { ssh, hosts: Mutex::default() }
    }

    fn host(&self, target: &SshTarget) -> Option<Arc<Mutex<HostMux>>> {
        Some(self.hosts.lock().ok()?.entry(target.clone()).or_default().clone())
    }

    /// See [`ensure_master`].
    fn ensure_master(&self, target: &SshTarget) -> Option<String> {
        let slot = self.host(target)?;
        // Held while starting the master, so two connections at once don't both start one.
        let mut h = slot.lock().ok()?;
        if !h.resolved {
            h.resolved = true;
            (h.path, h.may_start) = self.ssh.resolve(target);
        }
        let path = h.path.clone()?;
        if h.live.is_some_and(|t| t.elapsed() < CHECK_FOR) {
            return Some(path);
        }
        if self.ssh.check(target, &path) {
            h.live = Some(Instant::now());
            return Some(path);
        }
        h.live = None;
        if !h.may_start || h.failed.is_some_and(|t| t.elapsed() < RETRY_AFTER) {
            return None;
        }
        if self.ssh.start(target, &path) {
            h.failed = None;
            h.live = Some(Instant::now());
            Some(path)
        } else {
            h.failed = Some(Instant::now());
            None
        }
    }

    /// Stop believing the master to `target` is up: the next connection asks its socket.
    fn forget(&self, target: &SshTarget) {
        if let Some(slot) = self.host(target)
            && let Ok(mut h) = slot.lock()
        {
            h.live = None;
        }
    }
}

fn mux() -> &'static Mux {
    static MUX: OnceLock<Mux> = OnceLock::new();
    // Tests never connect anywhere.
    MUX.get_or_init(|| Mux::new(if cfg!(test) || !SHARING_SUPPORTED { Box::new(Unshared) } else { Box::new(SystemSsh) }))
}

/// The control socket a batch connection to `target` should ride, making sure a master is
/// listening on it. `None`: connect directly. Blocks for a login when it starts the master.
pub(super) fn ensure_master(target: &SshTarget) -> Option<String> {
    mux().ensure_master(target)
}

/// A batch connection to `target` ended with `status`. When that is ssh's own failure (255,
/// or killed) rather than its script's, the master it rode may be gone: ask again next time.
pub(super) fn note_exit(target: &SshTarget, status: ExitStatus) {
    if ssh_failed(status) {
        mux().forget(target);
    }
}

/// Whether `status` is ssh's own failure: it exits 255 for those, and with its script's
/// status otherwise.
fn ssh_failed(status: ExitStatus) -> bool {
    matches!(status.code(), Some(255) | None)
}

/// The value for `-o`, quoted when the path has spaces (ssh splits option values on them).
pub(super) fn control_path_opt(path: &str) -> String {
    if path.contains(char::is_whitespace) { format!("ControlPath=\"{path}\"") } else { format!("ControlPath={path}") }
}

/// Which socket to use for `target`, and whether nex may start a master on it, from ssh's
/// own reading of the config (`ssh -G`, which never connects).
fn resolve(target: &SshTarget) -> (Option<String>, bool) {
    // The pane's own options choose for themselves (`-S`, `-o ControlPath=none`).
    let sets_control = target.opts.iter().any(|o| o == "-S" || o.to_ascii_lowercase().starts_with("control"));
    if sets_control {
        return (None, false);
    }
    let Some(theirs) = config(target, None) else { return (None, false) };
    match setting(&theirs, "controlpath") {
        Some(path) => (Some(path.to_string()), setting(&theirs, "controlmaster") != Some("false")),
        None => {
            // Our path, expanded by ssh so a stale socket there can be removed.
            let ours = config(target, Some(OUR_PATH)).and_then(|c| setting(&c, "controlpath").map(str::to_string));
            (ours, true)
        }
    }
}

/// `ssh -G` for `target` (with `control` as its control path), or `None` if it failed.
fn config(target: &SshTarget, control: Option<&str>) -> Option<String> {
    let mut ssh = ssh_command();
    ssh.arg("-G");
    if let Some(path) = control {
        ssh.args(["-o", &control_path_opt(path)]);
    }
    let out = ssh.args(&target.opts).arg("--").arg(&target.destination).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
}

/// The value of `key` in `ssh -G` output.
fn setting<'a>(config: &'a str, key: &str) -> Option<&'a str> {
    config.lines().find_map(|l| l.strip_prefix(key)?.strip_prefix(' ')).map(str::trim).filter(|v| !v.is_empty() && *v != "none")
}

/// Whether a master is listening at `path`, or with `None`, at the control path the config
/// names for the host (asks the socket only: quick, never prompts).
pub(super) fn check(target: &SshTarget, path: Option<&str>) -> bool {
    let mut ssh = ssh_command();
    ssh.args(["-o", "BatchMode=yes"]);
    if let Some(path) = path {
        ssh.args(["-o", &control_path_opt(path)]);
    }
    ssh.args(["-O", "check"])
        .args(&target.opts)
        .arg("--")
        .arg(&target.destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Start a master at `path`: `ssh -f` goes to the background once it's logged in, and the
/// master lasts until [`PERSIST`] after its last session.
fn start(target: &SshTarget, path: &str) -> bool {
    let socket = Path::new(path);
    // Nobody answered at the socket, so whatever is there is left from a master that died.
    // ssh won't listen over it: it would carry on as a plain connection in the background.
    #[cfg(unix)]
    if std::fs::symlink_metadata(socket).is_ok_and(|m| std::os::unix::fs::FileTypeExt::is_socket(&m.file_type())) {
        let _ = std::fs::remove_file(socket);
    }
    if let Some(dir) = socket.parent().filter(|d| !d.as_os_str().is_empty() && !d.exists()) {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        if b.create(dir).is_err() {
            return false;
        }
    }
    let started = ssh_command()
        .args(["-M", "-N", "-f"])
        .args(BATCH_OPTS)
        .args(["-o", PERSIST, "-o", &control_path_opt(path)])
        // A dropped network ends the master (and the sessions on it) within a minute, so the
        // watch notices and reconnects rather than waiting on a dead connection.
        .args(["-o", "ServerAliveInterval=15", "-o", "ServerAliveCountMax=3"])
        .args(&target.opts)
        .arg("--")
        .arg(&target.destination)
        // Not pipes: the master keeps them open in the background, and reading one would
        // wait for it to end. A login that fails shows in the direct connection made instead.
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    started && check(target, Some(path))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

    #[test]
    fn reads_ssh_config_output() {
        let c = "user me\ncontrolmaster auto\ncontrolpath /home/me/.ssh/nex-ab12\ncontrolpersist 60\n";
        assert_eq!(setting(c, "controlpath"), Some("/home/me/.ssh/nex-ab12"));
        assert_eq!(setting(c, "controlmaster"), Some("auto"));
        assert_eq!(setting("controlmaster false\n", "controlpath"), None);
        assert_eq!(setting("controlpath none\n", "controlpath"), None);
        // Not a prefix of a longer key.
        assert_eq!(setting("controlpersist 60\n", "controlp"), None);
    }

    #[test]
    fn quotes_paths_with_spaces() {
        assert_eq!(control_path_opt("/home/me/.ssh/nex-ab"), "ControlPath=/home/me/.ssh/nex-ab");
        assert_eq!(control_path_opt("/Users/my name/.ssh/nex-ab"), "ControlPath=\"/Users/my name/.ssh/nex-ab\"");
    }

    #[test]
    fn leaves_the_panes_own_choice_alone() {
        let t = |opts: &[&str]| SshTarget { opts: opts.iter().map(|s| s.to_string()).collect(), destination: "host".into() };
        assert_eq!(resolve(&t(&["-S", "/tmp/sock"])), (None, false));
        assert_eq!(resolve(&t(&["-o", "ControlPath=none"])), (None, false));
    }

    /// A stand-in for ssh that counts what it's asked, with a master that is up or not.
    #[derive(Default)]
    struct Counting {
        checks: AtomicUsize,
        starts: AtomicUsize,
        up: AtomicBool,
    }

    impl Ssh for Arc<Counting> {
        fn resolve(&self, _: &SshTarget) -> (Option<String>, bool) {
            (Some("/tmp/nex-test-sock".into()), true)
        }
        fn check(&self, _: &SshTarget, _: &str) -> bool {
            self.checks.fetch_add(1, Ordering::Relaxed);
            self.up.load(Ordering::Relaxed)
        }
        fn start(&self, _: &SshTarget, _: &str) -> bool {
            self.starts.fetch_add(1, Ordering::Relaxed);
            self.up.store(true, Ordering::Relaxed);
            true
        }
    }

    fn counting() -> (Arc<Counting>, Mux, SshTarget) {
        let ssh = Arc::new(Counting::default());
        (ssh.clone(), Mux::new(Box::new(ssh)), SshTarget { opts: vec![], destination: "host".into() })
    }

    /// What the cache is for: a burst of commands asks the socket once, not once each.
    #[test]
    fn a_burst_of_commands_checks_the_master_once() {
        let (ssh, mux, t) = counting();
        ssh.up.store(true, Ordering::Relaxed);
        for _ in 0..20 {
            assert_eq!(mux.ensure_master(&t).as_deref(), Some("/tmp/nex-test-sock"));
        }
        assert_eq!((ssh.checks.load(Ordering::Relaxed), ssh.starts.load(Ordering::Relaxed)), (1, 0));
        // Another host has its own answer.
        let other = SshTarget { opts: vec![], destination: "other".into() };
        mux.ensure_master(&other);
        assert_eq!(ssh.checks.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn a_master_nex_started_is_not_checked_again_at_once() {
        let (ssh, mux, t) = counting();
        assert!(mux.ensure_master(&t).is_some() && mux.ensure_master(&t).is_some());
        assert_eq!((ssh.checks.load(Ordering::Relaxed), ssh.starts.load(Ordering::Relaxed)), (1, 1));
    }

    #[test]
    fn a_failed_command_or_a_few_seconds_ask_again() {
        let (ssh, mux, t) = counting();
        ssh.up.store(true, Ordering::Relaxed);
        mux.ensure_master(&t);
        mux.forget(&t);
        mux.ensure_master(&t);
        assert_eq!(ssh.checks.load(Ordering::Relaxed), 2, "asked again after a failure");
        // The answer has aged past `CHECK_FOR`.
        let Some(then) = Instant::now().checked_sub(CHECK_FOR) else { return };
        mux.host(&t).unwrap().lock().unwrap().live = Some(then);
        mux.ensure_master(&t);
        assert_eq!((ssh.checks.load(Ordering::Relaxed), ssh.starts.load(Ordering::Relaxed)), (3, 0));
    }

    /// Only ssh's own failure says anything about the master: a script's status doesn't.
    #[cfg(unix)]
    #[test]
    fn only_sshs_own_failure_drops_the_answer() {
        let exit = |code: i32| std::process::Command::new("sh").args(["-c", &format!("exit {code}")]).status().unwrap();
        assert!(!ssh_failed(exit(0)) && !ssh_failed(exit(1)) && ssh_failed(exit(255)));
    }

    /// ssh's own reading: with no path in the config, nex's, expanded.
    #[test]
    fn uses_our_path_by_default() {
        let t = SshTarget { opts: vec!["-F".into(), "/dev/null".into()], destination: "example.invalid".into() };
        if let (Some(path), may_start) = resolve(&t) {
            assert!(path.contains("/.ssh/nex-") && !path.contains('%') && !path.starts_with('~'), "{path}");
            assert!(may_start);
        }
    }
}
