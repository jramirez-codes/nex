//! Talking to the daemon: connecting (starting it first when it isn't running), and one
//! request with its answer.
//!
//! A command that may start the daemon first asks the running one its version. One older
//! than this program (a newer nex-host has been installed since it started) is replaced
//! when none of its agents is running; while some are, it's kept, and it hands over by
//! itself once they finish (see `daemon/retire.rs`).

use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto::{self, RESTARTING, Request, Response};

/// How long to wait between tries at connecting to a daemon that's starting.
const START_RETRY: Duration = Duration::from_millis(60);
/// How long an old daemon told to go has to let go of its lock.
const RETIRE_WAIT: Duration = Duration::from_secs(5);
/// What stands for the version of a daemon too old to say.
pub const OLDER: &str = "older";

/// The running daemon, next to this program.
#[derive(Debug, PartialEq, Eq)]
pub enum Daemon {
    /// The same nex-host, or a newer one.
    Current,
    /// An older one: its version line, or [`OLDER`].
    Older(String),
}

impl Daemon {
    /// What it answered `version` with. One from before `version` calls it a bad request.
    pub fn judge(r: &Response) -> Daemon {
        match (&r.text, &r.error) {
            (Some(v), _) if r.ok && proto::older(v, &proto::version_line()) => Daemon::Older(v.clone()),
            (_, Some(e)) if !r.ok && e.starts_with("bad request") => Daemon::Older(OLDER.into()),
            _ => Daemon::Current,
        }
    }
}

/// Ask the running daemon (starting one if `start` and none is) its version.
pub fn daemon(start: bool) -> io::Result<Daemon> {
    let mut s = open(start)?;
    ask(&mut s, &Request::Version).map(|r| Daemon::judge(&r))
}

/// Connect to the daemon. With `start`, start one if none is running and wait for it, and
/// replace a running one that's older than this program if it has nothing running.
pub fn connect(start: bool) -> io::Result<UnixStream> {
    if start && let Ok(Daemon::Older(old)) = daemon(true) {
        replace(&old);
    }
    open(start)
}

/// An older daemon is running: end it if none of its sessions is running (the next
/// connection starts this one), or say it's still there.
fn replace(old: &str) {
    let Some(mut s) = paths::socket().and_then(|p| UnixStream::connect(p).ok()) else { return };
    let pid = peer_pid(&s);
    let Ok(r) = ask(&mut s, &Request::List) else { return };
    let running = r.sessions.unwrap_or_default().iter().filter(|s| s.exit.is_none()).count();
    if running > 0 {
        let agents = if running == 1 { "1 agent".to_string() } else { format!("{running} agents") };
        eprintln!("nex-host: {} is still running {agents}; it switches to {} when they finish", short(old), short(&proto::version_line()));
        return;
    }
    let Some(pid) = pid.filter(|&p| p > 1 && p != std::process::id() as i32) else { return };
    unsafe { libc::kill(pid, libc::SIGTERM) };
    if let Some(lock) = paths::lock() {
        wait_for_lock(&lock, RETIRE_WAIT);
    }
}

/// `nex-host 0.12.2 (protocol 3)` as `nex-host 0.12.2`; [`OLDER`] as `an older nex-host`.
fn short(line: &str) -> String {
    if line == OLDER {
        return "an older nex-host".into();
    }
    line.split(" (protocol").next().unwrap_or(line).to_string()
}

/// Wait (up to `limit`) until nobody holds the daemon's lock: the old daemon has exited.
fn wait_for_lock(path: &Path, limit: Duration) {
    let Ok(f) = std::fs::OpenOptions::new().write(true).open(path) else { return };
    let start = Instant::now();
    while start.elapsed() < limit {
        if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
            // Closing the file lets go of it again.
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The process at the other end of `s`.
#[cfg(any(target_os = "linux", target_os = "android"))]
fn peer_pid(s: &UnixStream) -> Option<i32> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, (&raw mut cred).cast(), &mut len) };
    (r == 0 && cred.pid > 0).then_some(cred.pid)
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn peer_pid(s: &UnixStream) -> Option<i32> {
    let mut pid: libc::pid_t = 0;
    let mut len = std::mem::size_of::<libc::pid_t>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_LOCAL, libc::LOCAL_PEERPID, (&raw mut pid).cast(), &mut len) };
    (r == 0 && pid > 0).then_some(pid)
}

#[cfg(not(any(target_os = "linux", target_os = "android", target_os = "macos", target_os = "ios")))]
fn peer_pid(_: &UnixStream) -> Option<i32> {
    None
}

/// Connect to whatever daemon is running. With `start`, start one if none is and wait for it.
fn open(start: bool) -> io::Result<UnixStream> {
    let sock = paths::socket().ok_or_else(|| io::Error::other("no home folder ($HOME is unset)"))?;
    match UnixStream::connect(&sock) {
        Ok(s) => return Ok(s),
        Err(e) if !start => return Err(io::Error::new(e.kind(), "nex-host isn't running (no sessions)")),
        Err(_) => {}
    }
    spawn_daemon()?;
    let mut last = None;
    for _ in 0..80 {
        std::thread::sleep(START_RETRY);
        match UnixStream::connect(&sock) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("the daemon didn't start")))
}

/// Start `nex-host daemon` in a session of its own, with nothing of this process's attached
/// (so the ssh command that ran us can finish).
fn spawn_daemon() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).current_dir("/");
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    // Not waited for: it outlives us, and init adopts it.
    cmd.spawn().map(drop)
}

/// Send `request` on `stream` and read the answer line (byte by byte, so nothing after it is
/// consumed: for `attach` the terminal stream follows).
pub fn ask(stream: &mut UnixStream, request: &Request) -> io::Result<Response> {
    let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    read_response(stream)
}

pub fn read_response(stream: &mut impl Read) -> io::Result<Response> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte)? {
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon hung up")),
            _ if byte[0] == b'\n' => break,
            _ => buf.push(byte[0]),
        }
    }
    serde_json::from_slice(&buf).map_err(io::Error::other)
}

/// Connect (starting the daemon if `start`), ask, and fail on an error answer.
pub fn request(request: &Request, start: bool) -> io::Result<Response> {
    let r = retrying(request, || {
        let mut stream = connect(start)?;
        ask(&mut stream, request)
    })?;
    if r.ok { Ok(r) } else { Err(io::Error::other(r.error.unwrap_or_else(|| "failed".into()))) }
}

/// `once`, and once more if it started something on a daemon that's handing over to a newer
/// one (the second try starts the new one).
fn retrying(request: &Request, mut once: impl FnMut() -> io::Result<Response>) -> io::Result<Response> {
    let r = once()?;
    let starts = matches!(request, Request::Launch { .. } | Request::Spawn { .. });
    if starts && !r.ok && r.error.as_deref().is_some_and(|e| e.contains(RESTARTING)) {
        return once();
    }
    Ok(r)
}

/// `line` (a [`Response`] from the daemon) marked as coming from an older daemon, `old`.
pub fn mark_older(line: &str, old: &str) -> String {
    match serde_json::from_str::<serde_json::Value>(line) {
        Ok(serde_json::Value::Object(mut o)) => {
            o.insert("daemon_version".into(), old.into());
            serde_json::Value::Object(o).to_string()
        }
        _ => line.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    #[test]
    fn an_old_daemons_bad_request_reads_as_older() {
        let r = Response::error("bad request: unknown variant `version`, expected one of `launch`, `list`");
        assert_eq!(Daemon::judge(&r), Daemon::Older(OLDER.into()));
        let ours = Response { ok: true, text: Some(proto::version_line()), ..Response::default() };
        assert_eq!(Daemon::judge(&ours), Daemon::Current);
        let old = Response { ok: true, text: Some("nex-host 0.0.1 (protocol 3)".into()), ..Response::default() };
        assert_eq!(Daemon::judge(&old), Daemon::Older("nex-host 0.0.1 (protocol 3)".into()));
        let newer = Response { ok: true, text: Some("nex-host 999.0.0 (protocol 3)".into()), ..Response::default() };
        assert_eq!(Daemon::judge(&newer), Daemon::Current, "a newer daemon is left alone");
        assert_eq!(Daemon::judge(&Response::error("something else")), Daemon::Current);
        assert_eq!(short("nex-host 0.12.2 (protocol 3)"), "nex-host 0.12.2");
        assert_eq!(short(OLDER), "an older nex-host");
    }

    #[test]
    fn a_launch_on_a_retiring_daemon_is_tried_again() {
        let launch =
            Request::Launch { argv: vec!["claude".into()], kind: None, name: None, cwd: None, cols: None, rows: None, env: None, parent: None };
        let tries = Cell::new(0);
        let r = retrying(&launch, || {
            tries.set(tries.get() + 1);
            Ok(if tries.get() == 1 { Response::error(RESTARTING) } else { Response { ok: true, id: Some("a1".into()), ..Response::default() } })
        })
        .unwrap();
        assert_eq!((tries.get(), r.id.as_deref()), (2, Some("a1")));

        // Once only, and only what starts something.
        tries.set(0);
        let r = retrying(&launch, || {
            tries.set(tries.get() + 1);
            Ok(Response::error(RESTARTING))
        })
        .unwrap();
        assert_eq!((tries.get(), r.ok), (2, false));
        tries.set(0);
        let _ = retrying(&Request::List, || {
            tries.set(tries.get() + 1);
            Ok(Response::error(RESTARTING))
        });
        assert_eq!(tries.get(), 1);
    }

    #[test]
    fn watch_lines_are_marked_with_the_old_version() {
        let line = serde_json::to_string(&Response::sessions(Vec::new())).unwrap();
        let r: Response = serde_json::from_str(&mark_older(&line, "nex-host 0.12.2 (protocol 3)")).unwrap();
        assert_eq!(r.daemon_version.as_deref(), Some("nex-host 0.12.2 (protocol 3)"));
        assert!(r.sessions.is_some());
        assert_eq!(mark_older("not json", "x"), "not json");
    }
}
