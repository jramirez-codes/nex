//! Running shell scripts on the host a pane is connected to, for the panes that work there
//! without a shell of their own: a Files pane or a Changes pane opened from a terminal in an
//! `ssh` session (or on the host the window is connected to itself).
//!
//! Two ways there, one interface: [`Transport::Ssh`] rides the pane's own `ssh` options and
//! nex's shared connection (see `mux`), and [`Transport::Nex`] opens a channel on the
//! connection nex's own client keeps to the host (`ssh/`). Scripts run under `sh` and only
//! their stdout and exit status come back, so scripts that want to explain a failure print
//! the reason to stdout.

use std::io::Read;
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::ssh::host::RemoteHost;
use super::{SshTarget, last_line, needs_login, posix_quote};

/// How long a quick script (a listing, a `git status`) may take before it's given up on.
pub const QUICK_TIMEOUT: Duration = Duration::from_secs(10);

/// How a pane reaches its host.
pub enum Transport {
    /// With the `ssh` program, the way the terminal pane it came from is connected.
    Ssh(SshTarget),
    /// Over nex's own connection to the host the window is a client of.
    Nex(Arc<RemoteHost>),
}

impl std::fmt::Debug for Transport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Transport({})", self.key())
    }
}

impl PartialEq for Transport {
    fn eq(&self, other: &Self) -> bool {
        self.key() == other.key()
    }
}

/// Why a script didn't run to the end.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ExecError {
    /// The host wants a password nex's own connections can't type.
    NeedsLogin,
    Timeout,
    Cancelled,
    /// No connection, or ssh's own complaint.
    Failed(String),
}

impl std::fmt::Display for ExecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ExecError::NeedsLogin => write!(f, "nex can't log in to the host without a password"),
            ExecError::Timeout => write!(f, "The host took too long to answer"),
            ExecError::Cancelled => write!(f, "Cancelled"),
            ExecError::Failed(e) => write!(f, "{e}"),
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
    /// Stop waiting for it (end the local side of the connection).
    abort: Box<dyn FnOnce() + Send>,
}

impl Transport {
    /// Identifies the host and the way there: two panes with the same key are on the same
    /// host, and paths from one mean the same files in the other.
    pub fn key(&self) -> String {
        match self {
            Transport::Ssh(t) => {
                let t = t.normalized();
                format!("ssh:{}:{}", t.opts.join(" "), t.destination)
            }
            Transport::Nex(h) => format!("nex:{}", h.server),
        }
    }

    /// The host's name, for titles and messages.
    pub fn label(&self) -> String {
        match self {
            Transport::Ssh(t) => t.host().to_string(),
            Transport::Nex(h) => h.server.host.clone(),
        }
    }

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
        let wrapped = format!(
            "printf '%s\\n' \"$$\"\ntrap 'trap - TERM; kill 0 2>/dev/null; exit 143' TERM\n( {script}\n) &\nwait $!\n"
        );
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
            return Err(ExecError::Failed(out.reason()));
        }
        parse_many(&out.out, commands.len()).ok_or_else(|| ExecError::Failed("The host's answer was cut short".into()))
    }

    fn start(&self, script: &str) -> Result<Started, ExecError> {
        match self {
            Transport::Ssh(target) => start_ssh(target, script),
            Transport::Nex(host) => start_nex(host, script),
        }
    }
}

/// Run `script` with `ssh`, reading its output as it comes.
fn start_ssh(target: &SshTarget, script: &str) -> Result<Started, ExecError> {
    let mut child = target
        .batch_command(script)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ExecError::Failed(format!("ssh: {e}")))?;
    let out = Arc::new(Mutex::new(Vec::new()));
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();
    let reader = {
        let out = out.clone();
        std::thread::spawn(move || {
            if let Some(mut s) = stdout {
                pump(&mut s, &out);
            }
        })
    };
    let errors = std::thread::spawn(move || {
        let mut text = String::new();
        if let Some(mut s) = stderr {
            let _ = s.read_to_string(&mut text);
        }
        text
    });
    let child = Arc::new(Mutex::new(child));
    let (tx, done) = mpsc::channel();
    let waiter = child.clone();
    std::thread::spawn(move || {
        let status = loop {
            let polled = waiter.lock().unwrap_or_else(|e| e.into_inner()).try_wait();
            match polled {
                Ok(Some(status)) => break status,
                Ok(None) => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => {
                    let _ = tx.send(Err(ExecError::Failed(e.to_string())));
                    return;
                }
            }
        };
        let _ = reader.join();
        let stderr = errors.join().unwrap_or_default();
        let code = status.code().unwrap_or(255);
        // A login prompt counts even when the script itself got to run.
        let result = if needs_login(&stderr) {
            Err(ExecError::NeedsLogin)
        } else if code == 255 {
            // ssh's own failure (the script's would have its own status).
            Err(ExecError::Failed(last_line(&stderr).unwrap_or_else(|| "ssh couldn't reach the host".into())))
        } else {
            Ok(code)
        };
        let _ = tx.send(result);
    });
    let abort = Box::new(move || {
        let _ = child.lock().unwrap_or_else(|e| e.into_inner()).kill();
    });
    Ok(Started { out, done, abort })
}

/// Run `script` on a channel of nex's own connection to `host`.
fn start_nex(host: &Arc<RemoteHost>, script: &str) -> Result<Started, ExecError> {
    let client = host.client().map_err(|e| ExecError::Failed(format!("Not connected to {}: {e}", host.server.host)))?;
    // The command goes through the user's login shell: hand the script to `sh` whatever that is.
    let exec = client.exec(&format!("sh -c {}", posix_quote(script))).map_err(|e| ExecError::Failed(e.to_string()))?;
    let crate::remote::ssh::Exec { mut reader, writer, status, .. } = exec;
    // Nothing to send: end its input.
    drop(writer);
    let out = Arc::new(Mutex::new(Vec::new()));
    let (tx, done) = mpsc::channel();
    let o = out.clone();
    std::thread::spawn(move || {
        pump(&mut reader, &o);
        let code = status.lock().unwrap_or_else(|e| e.into_inner()).map_or(255, |c| c as i32);
        let _ = tx.send(Ok(code));
        // Keep the connection up until the command is done with it.
        drop(client);
    });
    // The channel ends with the command; there's nothing to kill on this side.
    Ok(Started { out, done, abort: Box::new(|| {}) })
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
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(ExecError::Failed("The connection to the host ended".into())),
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

/// A remote directory as a word for `cd`: quoted, with a leading `~` left for the shell.
pub fn cd_word(dir: &str) -> String {
    if dir.is_empty() || dir == "~" {
        return "~".into();
    }
    super::remote_dir(dir)
}

/// `path` quoted for the host's shell.
pub fn quote(path: &std::path::Path) -> String {
    posix_quote(&path.to_string_lossy())
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn many_notices_a_short_answer() {
        assert_eq!(parse_many(b"0 10\nabc", 1), None);
        assert_eq!(parse_many(b"0 3\nabc", 2), None);
    }

    #[test]
    fn cd_words() {
        assert_eq!(cd_word(""), "~");
        assert_eq!(cd_word("~"), "~");
        assert_eq!(cd_word("~/my dir"), "~/'my dir'");
        assert_eq!(cd_word("/srv/app"), "/srv/app");
    }

    #[test]
    fn keys() {
        let a = Transport::Ssh(SshTarget { opts: vec!["-p".into(), "2222".into()], destination: "me@box".into() });
        let b = Transport::Ssh(SshTarget { opts: vec!["-p".into(), "2222".into(), "-o".into(), "RemoteCommand=none".into()], destination: "me@box".into() });
        let c = Transport::Ssh(SshTarget { opts: vec![], destination: "me@box".into() });
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.label(), "box");
    }
}
