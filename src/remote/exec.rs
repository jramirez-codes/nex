//! Running shell scripts on the host a pane is connected to, for the panes that work there
//! without a shell of their own: a Files pane or a Changes pane opened from a terminal in an
//! `ssh` session (or on the host the window is connected to itself).
//!
//! Two ways there, one interface ([`Transport`], in `transport.rs`): [`Transport::Ssh`] rides
//! the pane's own `ssh` options and nex's shared connection (see `mux`), and
//! [`Transport::Nex`] opens a channel on the connection nex's own client keeps to the host
//! (`ssh/`). Scripts run under `sh` and only
//! their stdout and exit status come back, so scripts that want to explain a failure print
//! the reason to stdout.

use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::remote::error::last_line;
use crate::remote::transport::Process;
use crate::remote::{RemoteError, posix_quote};

pub use crate::remote::transport::Transport;

/// How long a quick script (a listing, a `git status`) may take before it's given up on.
pub const QUICK_TIMEOUT: Duration = Duration::from_secs(10);

/// Why a script didn't run to the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    /// No connection, ssh's own complaint, or the host wants a password nex's own
    /// connections can't type.
    Remote(RemoteError),
    Timeout,
    Cancelled,
}

impl From<RemoteError> for ExecError {
    fn from(e: RemoteError) -> Self {
        ExecError::Remote(e)
    }
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::Remote(RemoteError::NeedsLogin) => write!(f, "nex can't log in to the host without a password"),
            ExecError::Remote(RemoteError::Other(e)) => write!(f, "{e}"),
            ExecError::Timeout => write!(f, "The host took too long to answer"),
            ExecError::Cancelled => write!(f, "Cancelled"),
        }
    }
}

/// What a script printed, and how it exited.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Output {
    pub code: i32,
    pub out: Vec<u8>,
}

impl Output {
    pub fn ok(&self) -> bool {
        self.code == 0
    }

    pub fn text(&self) -> String {
        String::from_utf8_lossy(&self.out).into_owned()
    }

    /// The last line it printed: a failing script's reason.
    pub fn reason(&self) -> String {
        last_line(&self.text()).unwrap_or_else(|| format!("It failed on the host (exit status {})", self.code))
    }
}

/// A script started on the host.
struct Started {
    /// What it has printed so far.
    out: Arc<Mutex<Vec<u8>>>,
    /// Its exit status, once it has ended.
    done: mpsc::Receiver<Result<i32, ExecError>>,
    /// Stop it: kill the `ssh` running it, or close its channel on nex's own connection.
    abort: Box<dyn FnOnce() + Send>,
}

impl Transport {
    /// Run a quick `script` and wait for it (at most [`QUICK_TIMEOUT`]).
    pub fn run(&self, script: &str) -> Result<Output, ExecError> {
        let started = self.start(script)?;
        wait(started, Some(QUICK_TIMEOUT), None, &mut |_| {})
    }

    /// Run a long `script` (a copy, a delete) until it ends or `cancel` is set, when it's
    /// stopped on the host. `progress` sees what it has printed so far, every so often.
    pub fn run_job(&self, script: &str, cancel: &AtomicBool, progress: &mut dyn FnMut(&[u8])) -> Result<Output, ExecError> {
        // The script runs in the background of a shell that prints its pid first, so a
        // Cancel can end it (and everything it started) from a second connection.
        let wrapped = format!("printf '%s\\n' \"$$\"\ntrap 'trap - TERM; kill 0 2>/dev/null; exit 143' TERM\n( {script}\n) &\nwait $!\n");
        let started = self.start(&wrapped)?;
        let out = started.out.clone();
        let pid = || {
            let out = out.lock().unwrap_or_else(|e| e.into_inner());
            let line = out.split(|&b| b == b'\n').next().filter(|_| out.contains(&b'\n'))?;
            std::str::from_utf8(line).ok()?.trim().parse::<u32>().ok()
        };
        let strip = |bytes: &[u8]| -> Vec<u8> {
            match bytes.iter().position(|&b| b == b'\n') {
                Some(i) => bytes[i + 1..].to_vec(),
                None => Vec::new(),
            }
        };
        // Stopped on the host before this side lets go of it.
        let mut stop = || {
            if let Some(pid) = pid() {
                let _ = self.run(&format!("kill -TERM {pid} 2>/dev/null"));
            }
        };
        let result = wait(started, None, Some((cancel, &mut stop)), &mut |bytes| progress(&strip(bytes)));
        result.map(|o| Output { code: o.code, out: strip(&o.out) })
    }

    /// Run each of `commands` in turn, in one trip to the host: what each one printed and
    /// how it exited. Their output stays apart byte for byte (NULs and all).
    pub fn run_many(&self, commands: &[String]) -> Result<Vec<Output>, ExecError> {
        if commands.is_empty() {
            return Ok(Vec::new());
        }
        let out = self.run(&many_script(commands))?;
        if !out.ok() {
            return Err(RemoteError::Other(out.reason()).into());
        }
        parse_many(&out.out, commands.len()).ok_or_else(|| RemoteError::from("The host's answer was cut short").into())
    }

    /// Run `script` until it ends or `cancel` is set, handing what it prints to `sink` as it
    /// arrives instead of keeping it (a file being downloaded). Returns its exit status. A
    /// `sink` error stops the script and comes back as the error.
    pub fn stream(&self, script: &str, cancel: &AtomicBool, sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>) -> Result<i32, ExecError> {
        stream_process(self.spawn(script, false)?, cancel, sink)
    }

    fn start(&self, script: &str) -> Result<Started, ExecError> {
        Ok(follow(self.spawn(script, false)?))
    }
}

/// Read a started script to its end, on threads of its own: what it prints as it comes, then
/// how it exited.
fn follow(process: Process) -> Started {
    let Process { stdout, stderr, running, .. } = process;
    let ssh = running.over_ssh();
    let out = Arc::new(Mutex::new(Vec::new()));
    let reader = {
        let out = out.clone();
        let mut stdout = stdout;
        std::thread::spawn(move || pump(&mut stdout, &out))
    };
    let running = Arc::new(Mutex::new(Some(running)));
    let (tx, done) = mpsc::channel();
    let waiter = running.clone();
    std::thread::spawn(move || {
        loop {
            let polled = waiter.lock().unwrap_or_else(|e| e.into_inner()).as_mut().map(|r| r.ended(reader.is_finished()));
            match polled {
                Some(Ok(false)) => std::thread::sleep(Duration::from_millis(5)),
                Some(Ok(true)) | None => break,
                Some(Err(e)) => {
                    let _ = tx.send(Err(RemoteError::Other(e.to_string()).into()));
                    return;
                }
            }
        }
        let _ = reader.join();
        let stderr = stderr.and_then(|r| r.recv().ok()).unwrap_or_default();
        let Some(running) = waiter.lock().unwrap_or_else(|e| e.into_inner()).take() else { return };
        // Let go of the connection (a command on nex's own) once the command is done with it.
        let (code, failure) = running.exit(&stderr);
        let code = code.unwrap_or(255);
        let _ = tx.send(if ssh { ssh_result(code, failure, &stderr) } else { Ok(code) });
    });
    // A command given up on (a timeout, a Cancel) is ended on the host: the `ssh` is killed,
    // or its channel on nex's own connection closed, which also ends the threads above. Left
    // alone it would run on, holding them for as long as it liked.
    let abort = Box::new(move || {
        if let Some(r) = running.lock().unwrap_or_else(|e| e.into_inner()).as_mut() {
            r.end();
        }
    });
    Started { out, done, abort }
}

/// [`Transport::stream`] for a started script: its stdout is read on a thread of its own and
/// handed over a chunk at a time (a few chunks ahead at most, so a slow `sink` slows the
/// read rather than filling memory).
fn stream_process(process: Process, cancel: &AtomicBool, sink: &mut dyn FnMut(&[u8]) -> std::io::Result<()>) -> Result<i32, ExecError> {
    let Process { stdout, stderr, running, .. } = process;
    let (tx, chunks) = mpsc::sync_channel::<Vec<u8>>(8);
    let mut stdout = stdout;
    std::thread::spawn(move || {
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match stdout.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(_) => break,
            }
        }
    });
    loop {
        if cancel.load(Ordering::Relaxed) {
            running.stop();
            return Err(ExecError::Cancelled);
        }
        match chunks.recv_timeout(Duration::from_millis(50)) {
            Ok(chunk) => {
                if let Err(e) = sink(&chunk) {
                    running.stop();
                    return Err(RemoteError::Other(e.to_string()).into());
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
        }
    }
    let stderr = stderr.and_then(|r| r.recv().ok()).unwrap_or_default();
    let ssh = running.over_ssh();
    let (code, failure) = running.exit(&stderr);
    let code = code.unwrap_or(255);
    if ssh { ssh_result(code, failure, &stderr) } else { Ok(code) }
}

/// What an `ssh` that exited with `code` comes to: the script's own status is the caller's
/// to read, and 255 is ssh's own failure, which `failure` (see [`RemoteError::of`]) and
/// `stderr` explain.
fn ssh_result(code: i32, failure: Option<RemoteError>, stderr: &str) -> Result<i32, ExecError> {
    if code != 255 {
        return Ok(code);
    }
    Err(match failure {
        Some(RemoteError::NeedsLogin) => RemoteError::NeedsLogin,
        _ => RemoteError::Other(last_line(stderr).unwrap_or_else(|| "ssh couldn't reach the host".into())),
    }
    .into())
}

/// Copy everything `from` gives into `to` as it arrives.
fn pump(from: &mut dyn Read, to: &Mutex<Vec<u8>>) {
    let mut buf = [0u8; 64 * 1024];
    loop {
        match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => to.lock().unwrap_or_else(|e| e.into_inner()).extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(_) => break,
        }
    }
}

/// Wait for a started script: until it ends, `timeout` passes, or `cancel`'s flag is set
/// (when its function runs first, to stop the script).
fn wait(
    started: Started,
    timeout: Option<Duration>,
    cancel: Option<(&AtomicBool, &mut dyn FnMut())>,
    progress: &mut dyn FnMut(&[u8]),
) -> Result<Output, ExecError> {
    let Started { out, done, abort } = started;
    let mut cancel = cancel;
    let deadline = timeout.map(|t| Instant::now() + t);
    let mut seen = 0;
    loop {
        match done.recv_timeout(Duration::from_millis(50)) {
            Ok(result) => {
                let code = result?;
                let out = std::mem::take(&mut *out.lock().unwrap_or_else(|e| e.into_inner()));
                return Ok(Output { code, out });
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(RemoteError::from("The connection to the host ended").into()),
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some((flag, stop)) = &mut cancel
            && flag.load(Ordering::Relaxed)
        {
            stop();
            abort();
            return Err(ExecError::Cancelled);
        }
        if deadline.is_some_and(|d| Instant::now() >= d) {
            abort();
            return Err(ExecError::Timeout);
        }
        let bytes = out.lock().unwrap_or_else(|e| e.into_inner()).clone();
        if bytes.len() != seen {
            seen = bytes.len();
            progress(&bytes);
        }
    }
}

/// A script that runs `commands` one after another, each one's output in a file of its own,
/// then prints them all: for each, `"<status> <bytes>\n"` and then exactly that many bytes.
fn many_script(commands: &[String]) -> String {
    let mut s = String::from("d=$(mktemp -d \"${TMPDIR:-/tmp}/nex.XXXXXX\") || exit 1\n");
    for (i, c) in commands.iter().enumerate() {
        s.push_str(&format!("( {c}\n) >\"$d/{i}\" 2>/dev/null </dev/null; echo $? >\"$d/{i}.s\"\n"));
    }
    s.push_str(&format!(
        "i=0; while [ $i -lt {n} ]; do printf '%s %s\\n' \"$(cat \"$d/$i.s\")\" \"$(wc -c <\"$d/$i\" | tr -d ' ')\"; cat \"$d/$i\"; i=$((i+1)); done\nrm -rf \"$d\"\n",
        n = commands.len()
    ));
    s
}

/// Split what [`many_script`] printed back into `n` outputs.
fn parse_many(mut bytes: &[u8], n: usize) -> Option<Vec<Output>> {
    let mut outs = Vec::with_capacity(n);
    for _ in 0..n {
        let nl = bytes.iter().position(|&b| b == b'\n')?;
        let head = std::str::from_utf8(&bytes[..nl]).ok()?;
        let (code, len) = head.trim().split_once(' ')?;
        let (code, len): (i32, usize) = (code.trim().parse().ok()?, len.trim().parse().ok()?);
        let rest = &bytes[nl + 1..];
        if rest.len() < len {
            return None;
        }
        outs.push(Output { code, out: rest[..len].to_vec() });
        bytes = &rest[len..];
    }
    Some(outs)
}

/// `path` quoted for the host's shell.
pub fn quote(path: &std::path::Path) -> String {
    posix_quote(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::remote::ssh::Exec;

    fn sh(script: &str) -> Vec<u8> {
        std::process::Command::new("sh").arg("-c").arg(script).output().unwrap().stdout
    }

    #[test]
    fn many_keeps_outputs_apart() {
        let cmds = vec!["printf 'a\\0b\\n\\n'".to_string(), "true".to_string(), "echo oops; exit 3".to_string()];
        let out = parse_many(&sh(&many_script(&cmds)), 3).unwrap();
        assert_eq!(out[0], Output { code: 0, out: b"a\0b\n\n".to_vec() });
        assert_eq!(out[1], Output { code: 0, out: Vec::new() });
        assert_eq!(out[2], Output { code: 3, out: b"oops\n".to_vec() });
    }

    /// Sends when dropped: [`follow`] lets go of what it keeps once the command has ended.
    struct Ended(mpsc::Sender<()>);

    impl Drop for Ended {
        fn drop(&mut self) {
            let _ = self.0.send(());
        }
    }

    /// A command on nex's own connection that never ends by itself, and when it has ended.
    fn hanging() -> (Started, mpsc::Receiver<()>) {
        let (tx, ended) = mpsc::channel();
        (follow(Process::nex(Exec::hanging(b"so far\n"), false, Ended(tx))), ended)
    }

    #[test]
    fn a_timeout_ends_the_command_on_nex_s_own_connection() {
        let (started, ended) = hanging();
        assert_eq!(wait(started, Some(Duration::from_millis(100)), None, &mut |_| {}), Err(ExecError::Timeout));
        ended.recv_timeout(Duration::from_secs(5)).expect("the command was ended, not left running");
    }

    #[test]
    fn a_cancel_ends_the_command_on_nex_s_own_connection() {
        let (started, ended) = hanging();
        let cancel = AtomicBool::new(false);
        let mut asked = false;
        let mut seen = Vec::new();
        // Cancelled once its first output shows.
        let result = wait(started, None, Some((&cancel, &mut || asked = true)), &mut |bytes| {
            seen = bytes.to_vec();
            cancel.store(true, Ordering::Relaxed);
        });
        assert_eq!(result, Err(ExecError::Cancelled));
        assert!(asked, "the script's own stop ran first");
        assert_eq!(seen, b"so far\n");
        ended.recv_timeout(Duration::from_secs(5)).expect("the command was ended, not left running");
    }

    /// A command that ends by itself is still read to its end.
    #[test]
    fn a_command_that_ends_gives_its_output() {
        let exec = Exec::hanging(b"done\n");
        exec.closer.close();
        let (tx, ended) = mpsc::channel();
        let out = wait(follow(Process::nex(exec, false, Ended(tx))), Some(Duration::from_secs(5)), None, &mut |_| {}).unwrap();
        assert_eq!(out.out, b"done\n");
        ended.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    /// A streamed command hands its output over as it comes, and a Cancel ends it.
    #[test]
    fn a_stream_hands_over_output_then_stops_on_cancel() {
        let (tx, ended) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let mut seen = Vec::new();
        let result = stream_process(Process::nex(Exec::hanging(b"chunk"), false, Ended(tx)), &cancel, &mut |bytes| {
            seen.extend_from_slice(bytes);
            cancel.store(true, Ordering::Relaxed);
            Ok(())
        });
        assert_eq!(result, Err(ExecError::Cancelled));
        assert_eq!(seen, b"chunk");
        ended.recv_timeout(Duration::from_secs(5)).expect("the command was ended, not left running");
    }

    /// A sink that can't take more (the phone's storage is full) stops the command.
    #[test]
    fn a_failing_sink_stops_the_stream() {
        let (tx, ended) = mpsc::channel();
        let cancel = AtomicBool::new(false);
        let result =
            stream_process(Process::nex(Exec::hanging(b"chunk"), false, Ended(tx)), &cancel, &mut |_| Err(std::io::Error::other("disk full")));
        assert_eq!(result, Err(RemoteError::from("disk full").into()));
        ended.recv_timeout(Duration::from_secs(5)).unwrap();
    }

    /// A command that ends by itself is streamed to its end.
    #[test]
    fn a_stream_reads_to_the_end() {
        let exec = Exec::hanging(b"all of it");
        exec.closer.close();
        let (tx, _ended) = mpsc::channel();
        let mut seen = Vec::new();
        let _ = stream_process(Process::nex(exec, false, Ended(tx)), &AtomicBool::new(false), &mut |b| {
            seen.extend_from_slice(b);
            Ok(())
        });
        assert_eq!(seen, b"all of it");
    }

    #[test]
    fn many_notices_a_short_answer() {
        assert_eq!(parse_many(b"0 10\nabc", 1), None);
        assert_eq!(parse_many(b"0 3\nabc", 2), None);
    }

    /// A script's own status and stderr are its caller's to read; only ssh's failure (255)
    /// is an error, and only then does `Permission denied (…)` mean the host wants a login.
    #[test]
    fn only_a_failed_ssh_needs_a_login() {
        let denied = "git@github.com: Permission denied (publickey).\n";
        assert_eq!(ssh_result(0, None, denied), Ok(0));
        assert_eq!(ssh_result(128, Some(RemoteError::NeedsLogin), denied), Ok(128));
        assert_eq!(ssh_result(255, Some(RemoteError::NeedsLogin), denied), Err(ExecError::Remote(RemoteError::NeedsLogin)));
        let refused = "ssh: connect to host x port 22: Connection refused\n";
        assert_eq!(ssh_result(255, Some(RemoteError::Other(refused.trim().into())), refused), Err(RemoteError::from(refused.trim()).into()));
        assert_eq!(ssh_result(255, None, ""), Err(RemoteError::from("ssh couldn't reach the host").into()));
    }

    #[test]
    fn exec_errors_read() {
        assert_eq!(ExecError::from(RemoteError::NeedsLogin).to_string(), "nex can't log in to the host without a password");
        assert_eq!(ExecError::from(RemoteError::from("No such directory")).to_string(), "No such directory");
    }
}
