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

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use super::{BATCH_OPTS, SHARING_SUPPORTED, SshTarget};

/// nex's control path, when the user's config has none for the host.
const OUR_PATH: &str = "~/.ssh/nex-%C";
/// How long a master nex started stays up after its last session ends.
const PERSIST: &str = "ControlPersist=120";
/// After starting a master failed, go direct for this long before trying again.
const RETRY_AFTER: Duration = Duration::from_secs(60);

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
}

fn hosts() -> &'static Mutex<HashMap<SshTarget, Arc<Mutex<HostMux>>>> {
    static HOSTS: OnceLock<Mutex<HashMap<SshTarget, Arc<Mutex<HostMux>>>>> = OnceLock::new();
    HOSTS.get_or_init(Default::default)
}

/// The control socket a batch connection to `target` should ride, making sure a master is
/// listening on it. `None`: connect directly. Blocks for a login when it starts the master.
pub(super) fn ensure_master(target: &SshTarget) -> Option<String> {
    // Tests never connect anywhere.
    if !SHARING_SUPPORTED || cfg!(test) {
        return None;
    }
    let slot = hosts().lock().ok()?.entry(target.clone()).or_default().clone();
    // Held while starting the master, so two connections at once don't both start one.
    let mut h = slot.lock().ok()?;
    if !h.resolved {
        h.resolved = true;
        (h.path, h.may_start) = resolve(target);
    }
    let path = h.path.clone()?;
    if check(target, &path) {
        return Some(path);
    }
    if !h.may_start || h.failed.is_some_and(|t| t.elapsed() < RETRY_AFTER) {
        return None;
    }
    if start(target, &path) {
        h.failed = None;
        Some(path)
    } else {
        h.failed = Some(Instant::now());
        None
    }
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
    let mut ssh = Command::new("ssh");
    ssh.env("TERM_PROGRAM", "nex").arg("-G");
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

/// Whether a master is listening at `path` (asks the socket only: quick, never prompts).
fn check(target: &SshTarget, path: &str) -> bool {
    Command::new("ssh")
        .env("TERM_PROGRAM", "nex")
        .args(["-o", "BatchMode=yes", "-o", &control_path_opt(path), "-O", "check"])
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
    let started = Command::new("ssh")
        .env("TERM_PROGRAM", "nex")
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
    started && check(target, path)
}

#[cfg(test)]
mod tests {
    use super::*;

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
