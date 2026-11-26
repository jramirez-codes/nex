//! The one choice every script nex runs on a host makes: through the system `ssh` with a
//! pane's options, or on a channel of nex's own connection (`ssh/`), for the host the window
//! is connected to itself. [`Transport`] names the way there; [`Transport::spawn`] starts a
//! script with its stdio piped to this side (a [`Process`], ended through its [`Running`]),
//! and [`Transport::output`] runs one to its end. `exec.rs`, `mic.rs` and `host.rs` build on
//! these two, whichever way the host is reached.

use std::io::{Read, Write};
use std::process::{Child, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};

use crate::remote::ssh::host::RemoteHost;
use crate::remote::ssh::{Client, Exec, ExecCloser};
use crate::remote::{RemoteError, SshTarget};

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

/// A script started on the host, its stdio piped to this side.
pub(super) struct Process {
    /// Its input, when [`Transport::spawn`] was asked for one (else it has none).
    pub(super) stdin: Option<Box<dyn Write + Send>>,
    pub(super) stdout: Box<dyn Read + Send>,
    /// Everything `ssh` wrote to stderr, sent once it has ended. A command on nex's own
    /// connection has none.
    pub(super) stderr: Option<Receiver<String>>,
    pub(super) running: Running,
}

/// A started script, as it is ended and as its exit is read.
pub(super) enum Running {
    /// An `ssh` process.
    Ssh { child: Child, target: SshTarget },
    /// A command on nex's own connection. `_keep` (the connection) is held until it's dropped.
    Nex { closer: ExecCloser, status: Arc<Mutex<Option<u32>>>, _keep: Box<dyn Send> },
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

    /// Start `script` under `sh` on the host, with a pipe for its input if `stdin`, and its
    /// stdout to read. `ssh`'s stderr is drained on a thread of its own, so it never blocks.
    pub(super) fn spawn(&self, script: &str, stdin: bool) -> Result<Process, RemoteError> {
        match self {
            Transport::Ssh(target) => {
                let mut child = target
                    .batch_command(script)
                    .stdin(if stdin { Stdio::piped() } else { Stdio::null() })
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| RemoteError::Other(format!("ssh: {e}")))?;
                let input = child.stdin.take().map(|s| Box::new(s) as Box<dyn Write + Send>);
                let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
                    let _ = child.kill();
                    return Err(RemoteError::from("couldn't talk to ssh"));
                };
                Ok(Process {
                    stdin: input,
                    stdout: Box::new(stdout),
                    stderr: Some(drain(stderr)),
                    running: Running::Ssh { child, target: target.clone() },
                })
            }
            Transport::Nex(host) => {
                let client = connect(host)?;
                // The command goes through the user's login shell: hand the script to `sh`
                // whatever that is.
                let exec = client.exec_sh(script).map_err(|e| RemoteError::Other(e.to_string()))?;
                // The connection stays up until the command is done with it.
                Ok(Process::nex(exec, stdin, client))
            }
        }
    }

    /// Run `script` under `sh` on the host to its end (feeding it `stdin`), and return what
    /// it printed. A script that exits non-zero is an error, as is ssh's own failure.
    pub(super) fn output(&self, script: &str, stdin: Option<&[u8]>) -> Result<Vec<u8>, RemoteError> {
        match self {
            Transport::Ssh(target) => {
                let mut child = target
                    .batch_command(script)
                    .stdin(if stdin.is_some() { Stdio::piped() } else { Stdio::null() })
                    .stdout(Stdio::piped())
                    .stderr(Stdio::piped())
                    .spawn()
                    .map_err(|e| RemoteError::Other(format!("ssh: {e}")))?;
                if let (Some(bytes), Some(mut pipe)) = (stdin, child.stdin.take()) {
                    pipe.write_all(bytes).map_err(|e| RemoteError::Other(format!("sending to the host: {e}")))?;
                }
                let out = child.wait_with_output().map_err(|e| RemoteError::Other(e.to_string()))?;
                if let Some(failure) = target.batch_failure(&out) {
                    return Err(failure);
                }
                Ok(out.stdout)
            }
            Transport::Nex(host) => {
                let (status, out) = connect(host)?.run_sh(script, stdin.unwrap_or_default()).map_err(|e| RemoteError::Other(e.to_string()))?;
                if status != 0 {
                    return Err(RemoteError::Other(format!("the host's script exited with {status}")));
                }
                Ok(out)
            }
        }
    }
}

/// nex's own connection, logged in again if it dropped.
fn connect(host: &RemoteHost) -> Result<Arc<Client>, RemoteError> {
    host.client().map_err(|e| RemoteError::Other(format!("Not connected to {}: {e}", host.server.host)))
}

/// Read `from` to its end on a thread of its own: the receiver gets it all once it has ended.
fn drain(mut from: impl Read + Send + 'static) -> Receiver<String> {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut text = String::new();
        let _ = from.read_to_string(&mut text);
        let _ = tx.send(text);
    });
    rx
}

impl Process {
    /// A command started on nex's own connection, as a [`Process`]. Without `stdin` its input
    /// is ended at once (there's nothing to send). `keep` is held until it's done.
    pub(super) fn nex(exec: Exec, stdin: bool, keep: impl Send + 'static) -> Process {
        let Exec { reader, writer, status, closer } = exec;
        let input = if stdin {
            Some(Box::new(writer) as Box<dyn Write + Send>)
        } else {
            drop(writer);
            None
        };
        Process { stdin: input, stdout: Box::new(reader), stderr: None, running: Running::Nex { closer, status, _keep: Box::new(keep) } }
    }
}

impl Running {
    /// End it now: kill the `ssh`, or close its channel (which also ends its stdout). Returns
    /// at once, and does nothing once it has ended.
    pub(super) fn end(&mut self) {
        match self {
            Running::Ssh { child, .. } => {
                let _ = child.kill();
            }
            Running::Nex { closer, .. } => closer.close(),
        }
    }

    /// [`end`](Self::end) it, and wait for an `ssh` to go.
    pub(super) fn stop(mut self) {
        self.end();
        if let Running::Ssh { child, .. } = &mut self {
            let _ = child.wait();
        }
    }

    /// Whether it has ended. `output_ended` says its stdout has been read to the end: all a
    /// command on nex's own connection has to go by.
    pub(super) fn ended(&mut self, output_ended: bool) -> std::io::Result<bool> {
        match self {
            Running::Ssh { child, .. } => Ok(child.try_wait()?.is_some()),
            Running::Nex { .. } => Ok(output_ended),
        }
    }

    /// It's over `ssh`, whose own failure shows in its exit status (255).
    pub(super) fn over_ssh(&self) -> bool {
        matches!(self, Running::Ssh { .. })
    }

    /// How it exited, once its stdout has ended: its exit status, and for an `ssh`, how ssh
    /// failed (see [`RemoteError::of`]) from what it wrote to `stderr`. Waits for an `ssh` to
    /// end; a command on nex's own connection is ended on the host too, if it hasn't.
    pub(super) fn exit(self, stderr: &str) -> (Option<i32>, Option<RemoteError>) {
        match self {
            Running::Ssh { mut child, target } => {
                let status = child.wait().ok();
                (status.and_then(|s| s.code()), status.and_then(|s| target.batch_exit(s, stderr)))
            }
            Running::Nex { closer, status, .. } => {
                closer.close();
                let code = *status.lock().unwrap_or_else(|e| e.into_inner());
                (code.map(|c| c as i32), None)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        let a = Transport::Ssh(SshTarget { opts: vec!["-p".into(), "2222".into()], destination: "me@box".into() });
        let b = Transport::Ssh(SshTarget {
            opts: vec!["-p".into(), "2222".into(), "-o".into(), "RemoteCommand=none".into()],
            destination: "me@box".into(),
        });
        let c = Transport::Ssh(SshTarget { opts: vec![], destination: "me@box".into() });
        assert_eq!(a, b);
        assert_ne!(a, c);
        assert_eq!(a.label(), "box");
    }

    /// A command on nex's own connection: its input ends unless asked for, its output reads
    /// through, and what it keeps goes when it's ended.
    #[test]
    fn a_nex_process_keeps_its_connection_until_done() {
        let (tx, gone) = mpsc::channel::<()>();
        struct Kept(mpsc::Sender<()>);
        impl Drop for Kept {
            fn drop(&mut self) {
                let _ = self.0.send(());
            }
        }
        let mut p = Process::nex(Exec::hanging(b"hi\n"), false, Kept(tx));
        assert!(p.stdin.is_none() && p.stderr.is_none() && !p.running.over_ssh());
        assert!(!p.running.ended(false).unwrap());
        p.running.end();
        let mut out = Vec::new();
        p.stdout.read_to_end(&mut out).unwrap();
        assert_eq!(out, b"hi\n");
        assert!(gone.try_recv().is_err(), "kept while running");
        assert_eq!(p.running.exit(""), (None, None));
        gone.try_recv().expect("let go once done");
    }

    /// `sh -c` as the stand-in for a host: stdout, stderr and the exit come back apart.
    #[cfg(unix)]
    #[test]
    fn drains_stderr_apart() {
        let mut child = std::process::Command::new("sh")
            .args(["-c", "echo out; echo err >&2; exit 3"])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        let err = drain(child.stderr.take().unwrap());
        let mut out = String::new();
        child.stdout.take().unwrap().read_to_string(&mut out).unwrap();
        assert_eq!((out.as_str(), err.recv().unwrap().as_str()), ("out\n", "err\n"));
        assert_eq!(child.wait().unwrap().code(), Some(3));
    }
}
