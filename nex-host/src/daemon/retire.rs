//! Handing over to a newer nex-host. Installing one replaces the program on disk, but a
//! daemon already running keeps serving the old code, and it owns the agents' terminals,
//! so it can't just be restarted. Instead it retires itself once nothing it holds is
//! running: it stops listening and exits, and the next command starts the new one.
//!
//! It's checked when an agent ends and on every idle tick while nothing runs (the check is
//! cached by the program file's identity, so it's cheap until the file changes).

use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use nex_host::{paths, proto};

use crate::daemon::state::State;
use crate::daemon::log;
use crate::util::Locked;

/// How long a retiring daemon lingers after it stops listening, so a request that reached it
/// just before is answered (with [`proto::RESTARTING`]) rather than cut off.
const LINGER: Duration = Duration::from_millis(300);
/// The longest `--version` may take.
const VERSION_TIMEOUT: Duration = Duration::from_secs(5);

/// The program this daemon was started from, taken when it starts (on Linux the link
/// reads `… (deleted)` once the file is replaced).
static EXE: OnceLock<Option<PathBuf>> = OnceLock::new();

pub(super) fn remember_exe() {
    EXE.get_or_init(|| std::env::current_exe().ok().map(|p| strip_deleted(&p)));
}

fn strip_deleted(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    PathBuf::from(s.strip_suffix(" (deleted)").unwrap_or(&s))
}

/// The version this daemon answers with: its own, or `$NEX_HOST_TEST_VERSION` (for tests,
/// to play an older daemon).
pub(super) fn own_version() -> String {
    std::env::var("NEX_HOST_TEST_VERSION").ok().filter(|v| !v.is_empty()).unwrap_or_else(proto::version_line)
}

/// The program file as last checked (device, inode, mtime, size), and whether it was newer.
type Checked = ((u64, u64, i64, u64), bool);

/// Whether a newer nex-host has replaced the program this daemon runs: the file at its
/// path says so with `--version`, or it's gone. A reinstall of the same version (or an
/// older one) doesn't count.
pub(super) fn stale() -> bool {
    static LAST: Mutex<Option<Checked>> = Mutex::new(None);
    let Some(Some(path)) = EXE.get() else { return false };
    let Ok(meta) = std::fs::metadata(path) else { return true };
    let key = (meta.dev(), meta.ino(), meta.mtime() * 1_000_000_000 + meta.mtime_nsec(), meta.size());
    let mut last = LAST.locked();
    if let Some((k, stale)) = *last
        && k == key
    {
        return stale;
    }
    let stale = version_of(path).is_some_and(|v| proto::older(&own_version(), &v));
    *last = Some((key, stale));
    stale
}

/// What `path --version` prints, if it answers in time.
fn version_of(path: &Path) -> Option<String> {
    let mut child = Command::new(path).arg("--version").stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => break,
            Ok(None) if start.elapsed() < VERSION_TIMEOUT => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    }
    let mut out = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take()?, &mut out).ok()?;
    Some(out.trim().to_string()).filter(|s| !s.is_empty())
}

/// Retire if a newer nex-host is installed and nothing is running: no agent (one that
/// exited doesn't count) and no `recv` waiting. Doesn't return when it retires.
pub(super) fn check(state: &State) {
    // Checked again as it's set (see `State::retire_if_idle`): once it is, nothing starts.
    if state.retiring() || !state.idle() || !stale() || !state.retire_if_idle() {
        return;
    }
    if let Some(sock) = paths::socket() {
        let _ = std::fs::remove_file(sock);
    }
    log(&format!("{} retiring: a newer nex-host is installed", own_version()));
    std::thread::sleep(LINGER);
    std::process::exit(0);
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;
    use crate::daemon::agent::Spec;
    use crate::daemon::lifecycle::launch;

    #[test]
    fn nothing_starts_once_retiring() {
        let state = Arc::new(State::in_memory());
        state.set_retiring();
        let spec = Spec { argv: vec!["true".into()], kind: None, name: None, cwd: "/".into(), cols: 80, rows: 24, env: None, parent: None, role: None };
        let e = launch(&state, "a1".into(), spec).unwrap_err();
        assert_eq!(e.to_string(), proto::RESTARTING);
        assert!(!state.has_agents());
    }

    #[test]
    fn a_replaced_programs_path_is_its_own() {
        assert_eq!(strip_deleted(Path::new("/h/.nex/bin/nex-host (deleted)")), PathBuf::from("/h/.nex/bin/nex-host"));
        assert_eq!(strip_deleted(Path::new("/h/.nex/bin/nex-host")), PathBuf::from("/h/.nex/bin/nex-host"));
    }
}
