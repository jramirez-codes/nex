//! A terminal whose shell runs on a host nex connects to itself (see `remote/ssh/`), as a
//! `nex-host` session: nothing runs on this machine.
//!
//! alacritty's event loop wants a PTY: something it can poll, read, write and resize, that
//! says when its child exits. [`RemotePty`] is one end of a socket pair for that, plus a
//! second pair whose only byte says "the shell exited". A thread ([`pump`]) holds the other
//! ends and carries them to the host over `nex-host connect`: the shell's output as it
//! comes, keys as input frames, resizes as resize frames.
//!
//! The session outlives the connection. When the connection drops (the phone slept, the
//! network changed), the pump says so in the pane, waits for the host to come back, and
//! attaches again; the daemon then redraws the screen. When the pane closes, the pump ends
//! the session; when the session ends (the shell exited), the pump ends the pane.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::ExitStatusExt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use alacritty_terminal::event::{OnResize, WindowSize};
use alacritty_terminal::tty::{ChildEvent, EventedPty, EventedReadWrite};
use nex_host::proto::{self, Request, Response};
use polling::{Event, PollMode, Poller};

use super::osc7::{AgentSlot, Alert, ScanReader, Slot};
use crate::remote::ssh::host::RemoteHost;
use crate::remote::ssh::{self, ExecWriter};

/// alacritty's poll key for a PTY's child events (`tty::unix`, not exported); the data key
/// comes from the event loop with each call.
const CHILD_KEY: usize = 1;
/// How long to wait between tries to reach a host that dropped, at most.
const RETRY_MAX: Duration = Duration::from_secs(10);
/// Right after the phone wakes the network comes back within seconds (a VPN's DNS last):
/// keep trying at least this often for [`WAKE_WINDOW`].
const RETRY_AFTER_WAKE: Duration = Duration::from_secs(1);
const WAKE_WINDOW: Duration = Duration::from_secs(30);
const RETRY_FIRST: Duration = Duration::from_millis(500);

/// The session a remote pane shows, once the host has started it: the pane keeps it to
/// attach again, and the session file keeps it to restore the pane.
pub type SessionSlot = Arc<Mutex<Option<String>>>;

/// The event loop's side.
pub struct RemotePty {
    io: UnixStream,
    reader: ScanReader<UnixStream>,
    exited: UnixStream,
    resize: Sender<(u16, u16)>,
}

impl EventedReadWrite for RemotePty {
    type Reader = ScanReader<UnixStream>;
    type Writer = UnixStream;

    unsafe fn register(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        // SAFETY: both sockets live as long as `self`, which the event loop deregisters
        // before dropping.
        unsafe {
            poll.add_with_mode(&self.io, interest, mode)?;
            poll.add_with_mode(&self.exited, Event::readable(CHILD_KEY), mode)
        }
    }

    fn reregister(&mut self, poll: &Arc<Poller>, interest: Event, mode: PollMode) -> io::Result<()> {
        poll.modify_with_mode(&self.io, interest, mode)?;
        poll.modify_with_mode(&self.exited, Event::readable(CHILD_KEY), mode)
    }

    fn deregister(&mut self, poll: &Arc<Poller>) -> io::Result<()> {
        poll.delete(&self.io)?;
        poll.delete(&self.exited)
    }

    fn reader(&mut self) -> &mut Self::Reader {
        &mut self.reader
    }

    fn writer(&mut self) -> &mut UnixStream {
        &mut self.io
    }
}

impl EventedPty for RemotePty {
    fn next_child_event(&mut self) -> Option<ChildEvent> {
        let mut code = [0u8; 1];
        match self.exited.read(&mut code) {
            Ok(1) => Some(ChildEvent::Exited(Some(std::process::ExitStatus::from_raw(i32::from(code[0]) << 8)))),
            // The pump is gone without a word: the shell is as good as ended.
            Ok(_) => Some(ChildEvent::Exited(None)),
            Err(_) => None,
        }
    }
}

impl OnResize for RemotePty {
    fn on_resize(&mut self, size: WindowSize) {
        let _ = self.resize.send((size.num_cols, size.num_lines));
    }
}

/// What the pump needs to start or find the session.
pub struct Start {
    pub host: Arc<RemoteHost>,
    /// The session to attach to; `None` starts a new shell.
    pub session: SessionSlot,
    /// Closing the pane leaves the session running (an agent) instead of ending it.
    pub keep: bool,
    /// What a new session runs instead of a shell (`None`: a shell).
    pub command: Option<Vec<String>>,
    /// Where a new shell starts on the host (`None`: home).
    pub cwd: Option<String>,
    pub cols: u16,
    pub rows: u16,
}

/// The PTY for a new remote pane, and the pump thread behind it. `osc7`, `alert` and `agent`
/// pick up what the shell reports, as for a local pane.
pub fn open(start: Start, osc7: Slot, alert: Alert, agent: AgentSlot) -> io::Result<RemotePty> {
    let (io, peer) = UnixStream::pair()?;
    let (exited, exit_tx) = UnixStream::pair()?;
    io.set_nonblocking(true)?;
    exited.set_nonblocking(true)?;
    let reader = ScanReader::new(io.try_clone()?, osc7, alert, agent);
    let (resize, resizes) = mpsc::channel();
    std::thread::Builder::new().name("remote pane".into()).spawn(move || pump(start, peer, exit_tx, resizes))?;
    Ok(RemotePty { io, reader, exited, resize })
}

/// Carry one pane to its session on the host until one of them ends.
fn pump(start: Start, pane: UnixStream, mut exit_tx: UnixStream, resizes: Receiver<(u16, u16)>) {
    let Start { host, session, keep, command, cwd, cols, rows } = start;
    let size = Arc::new(Mutex::new((cols, rows)));
    // Where keys and resizes go: the attached connection's input, while there is one.
    let to_host: Arc<Mutex<Option<ExecWriter>>> = Arc::default();
    let pane_closed = Arc::new(AtomicBool::new(false));

    // Keys: everything the pane writes, framed, to whichever connection is attached.
    let keys = {
        let (mut pane, to_host, closed) = (pane.try_clone(), to_host.clone(), pane_closed.clone());
        std::thread::Builder::new().name("remote pane keys".into()).spawn(move || {
            let Ok(pane) = pane.as_mut() else { return };
            let mut buf = vec![0u8; 16 * 1024];
            loop {
                match pane.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => {
                        if let Some(w) = to_host.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                            let _ = w.write_all(&proto::frame(proto::FRAME_INPUT, &buf[..n]));
                        }
                    }
                }
            }
            // The pane is gone: dropping the input ends the attached connection.
            closed.store(true, Ordering::Relaxed);
            to_host.lock().unwrap_or_else(|e| e.into_inner()).take();
        })
    };
    // Resizes, framed the same way; the last size is kept for attaching again.
    {
        let (to_host, size) = (to_host.clone(), size.clone());
        let _ = std::thread::Builder::new().name("remote pane resize".into()).spawn(move || {
            for (cols, rows) in resizes {
                *size.lock().unwrap_or_else(|e| e.into_inner()) = (cols, rows);
                if let Some(w) = to_host.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
                    let _ = w.write_all(&proto::resize_frame(cols, rows));
                }
            }
        });
    }

    let mut out = pane;
    let say = |out: &mut UnixStream, line: &str| {
        let _ = out.write_all(format!("\r\n\x1b[2m[{line}]\x1b[0m\r\n").as_bytes());
    };
    let mut wait = RETRY_FIRST;
    // What the pane last said about a host it can't reach, so retries don't repeat it.
    let mut said: Option<String> = None;
    loop {
        if pane_closed.load(Ordering::Relaxed) {
            break;
        }
        // Taken before trying, so a wake during the try still cuts the wait after it short.
        let wakes = ssh::wake_count();
        let (cols, rows) = *size.lock().unwrap_or_else(|e| e.into_inner());
        let id = session.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let attached = match id {
            Some(id) => attach(&host, &id, cols, rows),
            None => launch(&host, cwd.clone(), command.clone(), cols, rows).and_then(|id| {
                *session.lock().unwrap_or_else(|e| e.into_inner()) = Some(id.clone());
                attach(&host, &id, cols, rows)
            }),
        };
        let mut exec = match attached {
            Ok(exec) => exec,
            Err(Attach::Gone(why)) => {
                say(&mut out, &why);
                break;
            }
            Err(Attach::Unreachable(why)) => {
                // Try again until the host is back (or the pane is closed).
                let line = format!("can't reach {}: {why}; trying again", host.server);
                if said.as_deref() != Some(line.as_str()) {
                    say(&mut out, &line);
                    said = Some(line);
                }
                wait = retry_wait(wait, ssh::wait_for_wake(wakes, wait), ssh::woke_within(WAKE_WINDOW));
                continue;
            }
        };
        if said.take().is_some() {
            eprintln!("nex: back on {}", host.server);
        }
        wait = RETRY_FIRST;
        *to_host.lock().unwrap_or_else(|e| e.into_inner()) = Some(exec.writer.clone());
        drop(exec.writer);
        // The session's output, until the connection ends.
        let _ = io::copy(&mut exec.reader, &mut out);
        to_host.lock().unwrap_or_else(|e| e.into_inner()).take();
        if pane_closed.load(Ordering::Relaxed) {
            break;
        }
        if daemon_let_go(*exec.status.lock().unwrap_or_else(|e| e.into_inner())) {
            // `nex-host connect` exited, so the daemon let go: the shell ended, or another
            // pane took the session. Either way this pane is done.
            break;
        }
        eprintln!("nex: lost the connection to {}; reconnecting", host.server);
        say(&mut out, &format!("lost the connection to {}; reconnecting", host.server));
    }
    if !pane_closed.load(Ordering::Relaxed) {
        // Tell the event loop the shell ended; it closes the pane, and the keys thread ends
        // with it.
        let _ = exit_tx.write_all(&[0]);
        return;
    }
    // Closing the pane ends its shell, as closing a local one would; an agent keeps running,
    // whether it was picked in the Agents pane or typed at this shell's prompt.
    if !keep
        && let Some(id) = session.lock().unwrap_or_else(|e| e.into_inner()).clone()
        && !runs_agent(&host, &id)
    {
        let _ = request(&host, &Request::Kill { id });
    }
    let _ = keys;
}

/// Did the attached stream end because the daemon let go, rather than the connection
/// dropping? Only `nex-host connect` exiting reports a status; a connection that dropped (or
/// was found dead after a sleep) ends without one, even before the client notices it's
/// closed.
fn daemon_let_go(status: Option<u32>) -> bool {
    status.is_some()
}

/// The wait before the next try to reach the host: back to the start after a wake,
/// otherwise doubling, to [`RETRY_AFTER_WAKE`] at most just after a wake and [`RETRY_MAX`]
/// otherwise.
fn retry_wait(wait: Duration, woke_during: bool, recently_woke: bool) -> Duration {
    if woke_during {
        return RETRY_FIRST;
    }
    (wait * 2).min(if recently_woke { RETRY_AFTER_WAKE } else { RETRY_MAX })
}

enum Attach {
    /// The session isn't there any more (it ended while nobody was attached).
    Gone(String),
    /// The host can't be reached right now.
    Unreachable(String),
}

/// [`RemoteHost::ask`], with its errors sorted for the pump.
fn request(host: &RemoteHost, request: &Request) -> Result<(Response, crate::remote::ssh::Exec), Attach> {
    host.ask(request).map_err(|e| match e.kind() {
        io::ErrorKind::Other => Attach::Gone(e.to_string()),
        _ => Attach::Unreachable(e.to_string()),
    })
}

/// Start a shell on the host (or `command`, under [`VIEW_KIND`]); its session id.
fn launch(host: &RemoteHost, cwd: Option<String>, command: Option<Vec<String>>, cols: u16, rows: u16) -> Result<String, Attach> {
    if let Some(argv) = command {
        let req =
            Request::Launch { argv, kind: Some(VIEW_KIND.into()), name: None, cwd, cols: Some(cols), rows: Some(rows), env: None, parent: None };
        let (r, _) = request(host, &req)?;
        return match r.id {
            Some(id) if r.ok => Ok(id),
            _ => Err(Attach::Gone(format!("couldn't start it on {}: {}", host.server, r.error.unwrap_or_default()))),
        };
    }
    let shell = |argv: Vec<String>| Request::Launch {
        argv,
        kind: Some(proto::SHELL_KIND.into()),
        name: None,
        cwd: cwd.clone(),
        cols: Some(cols),
        rows: Some(rows),
        env: None,
        parent: None,
    };
    let (mut r, _) = request(host, &shell(Vec::new()))?;
    if !r.ok && r.error.as_deref().is_some_and(|e| e.contains("nothing to run")) {
        // A daemon from before plain shells (still running the old nex-host after the new
        // one was installed): ask for the login shell by name.
        (r, _) = request(host, &shell(OLD_DAEMON_SHELL.iter().map(|s| s.to_string()).collect()))?;
    }
    match r.id {
        Some(id) if r.ok => Ok(id),
        _ => Err(Attach::Gone(format!("couldn't start a shell on {}: {}", host.server, r.error.unwrap_or_default()))),
    }
}

/// The kind of session a pane runs a command in (a sub-agent's tab): neither a shell to
/// adopt again nor an agent to list.
pub const VIEW_KIND: &str = "view";

/// The user's shell, interactive, for a daemon that needs a command.
const OLD_DAEMON_SHELL: &[&str] = &["sh", "-c", "exec \"${SHELL:-/bin/sh}\" -i"];

/// Is an agent (`claude`, `codex`) the foreground job of session `id`'s shell? Then closing
/// its pane leaves it running, and it stays in the Agents pane. A host that can't be asked
/// counts as no, so the shell ends as before.
fn runs_agent(host: &RemoteHost, id: &str) -> bool {
    let Ok((r, _)) = request(host, &Request::List) else { return false };
    r.sessions.is_some_and(|all| agent_in(&all, id))
}

/// Whether `sessions` (a `list` reply) says session `id` has an agent running in it.
fn agent_in(sessions: &[proto::Session], id: &str) -> bool {
    sessions.iter().any(|s| s.id == id && s.exit.is_none() && s.running.is_some())
}

/// Attach to session `id`: the connection, now carrying its screen.
fn attach(host: &RemoteHost, id: &str, cols: u16, rows: u16) -> Result<crate::remote::ssh::Exec, Attach> {
    let (r, exec) = request(host, &Request::Attach { id: id.into(), cols, rows })?;
    if r.ok { Ok(exec) } else { Err(Attach::Gone(r.error.unwrap_or_else(|| "the session ended".into()))) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell(id: &str, running: Option<&str>, exit: Option<i32>) -> proto::Session {
        proto::Session {
            id: id.into(),
            kind: proto::SHELL_KIND.into(),
            name: "me".into(),
            cwd: "/".into(),
            argv: Vec::new(),
            pid: 1,
            started: 0,
            title: None,
            bell: false,
            attached: true,
            last_output: 0,
            last_input: 0,
            exit,
            claude: None,
            running: running.map(str::to_string),
            status: proto::Status::Idle,
            parent: None,
            role: None,
            last_message: 0,
            subagents: Vec::new(),
        }
    }

    #[test]
    fn a_shell_with_an_agent_in_it_is_kept() {
        let all = [shell("a1", Some("claude"), None), shell("b2", None, None), shell("c3", Some("codex"), Some(0))];
        assert!(agent_in(&all, "a1"));
        // A plain shell, one that ended, or one the host doesn't know ends as before.
        assert!(!agent_in(&all, "b2"));
        assert!(!agent_in(&all, "c3"));
        assert!(!agent_in(&all, "zz"));
    }

    #[test]
    fn only_an_exit_status_means_the_daemon_let_go() {
        assert!(daemon_let_go(Some(0)));
        assert!(daemon_let_go(Some(1)));
        // The connection dropped (or a sleep killed it): attach again.
        assert!(!daemon_let_go(None));
    }

    #[test]
    fn retries_back_off_but_stay_quick_after_a_wake() {
        let mut w = RETRY_FIRST;
        for _ in 0..10 {
            w = retry_wait(w, false, false);
        }
        assert_eq!(w, RETRY_MAX);
        assert_eq!(retry_wait(w, true, false), RETRY_FIRST);
        assert_eq!(retry_wait(Duration::from_secs(4), false, true), RETRY_AFTER_WAKE);
        assert_eq!(retry_wait(RETRY_FIRST, false, true), RETRY_AFTER_WAKE);
    }
}
