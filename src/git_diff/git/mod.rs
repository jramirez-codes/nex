//! Running `git` and reading what it prints: the working tree's status (with per-file line
//! counts) and one file's unified diff, parsed into hunks with word-level highlights.
//! Everything here is blocking; `GitDiff` runs it on a background thread.
//!
//! For a folder on an ssh host the same `git` commands run there (`*_remote`, see
//! `remote::exec`), batched so a status is one trip; their output is parsed the same way.
//!
//! Here: `run` (one `git` call on this machine, with a timeout), `RunError` and what the
//! files share (the limits, git's environment and arguments). `status` (what changed, with
//! line counts), `diff` (one file's diff, parsed into hunks), `emphasis` (the words that
//! changed within a line), `remote` (the same commands on an ssh host) and `backend`
//! (`GitBackend`, which picks between here and the host once).

pub mod backend;
pub mod diff;
pub mod emphasis;
pub mod remote;
pub mod status;

use std::io::Read;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// Longest a single `git` call may take before it is killed (a huge repo, a hung hook).
const GIT_TIMEOUT: Duration = Duration::from_secs(8);
/// Diff lines kept for one file; past this the rest is cut off with a note.
pub const MAX_DIFF_LINES: usize = 20_000;
/// Untracked files bigger than this aren't read to count their lines.
const MAX_COUNT_BYTES: u64 = 4 * 1024 * 1024;

/// What every `git` runs with, here and on a host: never page, prompt or take locks, and
/// print in plain C-locale text.
const GIT_ENV: [(&str, &str); 4] = [("GIT_PAGER", "cat"), ("GIT_TERMINAL_PROMPT", "0"), ("GIT_OPTIONAL_LOCKS", "0"), ("LC_ALL", "C")];
/// The repository's root folder.
const TOPLEVEL_ARGS: [&str; 2] = ["rev-parse", "--show-toplevel"];
/// The status [`parse_status`](crate::git_diff::git::status::parse_status) reads.
const STATUS_ARGS: [&str; 5] = ["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"];
/// Line counts ([`parse_numstat`](crate::git_diff::git::status::parse_numstat)) of the work tree against the index.
const NUMSTAT_ARGS: [&str; 5] = ["diff", "--numstat", "-z", "--no-ext-diff", "--no-textconv"];
/// Line counts of the index against `HEAD`.
const NUMSTAT_STAGED_ARGS: [&str; 6] = ["diff", "--cached", "--numstat", "-z", "--no-ext-diff", "--no-textconv"];

#[derive(Debug)]
pub enum RunError {
    /// `git` isn't installed (or isn't on `PATH`).
    NotFound,
    /// It ran too long and was killed.
    Timeout,
    Io(String),
}

/// Run `git -C dir args…`, returning whether it exited successfully and what it printed.
pub fn run(dir: &Path, args: &[&str]) -> Result<(bool, Vec<u8>), RunError> {
    let mut child = Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(args)
        // Never page, prompt or pick up a user's color / external-diff settings.
        .envs(GIT_ENV)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| if e.kind() == std::io::ErrorKind::NotFound { RunError::NotFound } else { RunError::Io(e.to_string()) })?;
    let mut stdout = child.stdout.take().ok_or_else(|| RunError::Io("no stdout".into()))?;
    // Read on a thread so a big diff can't fill the pipe while we wait.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).map(|_| out)
    });
    let deadline = Instant::now() + GIT_TIMEOUT;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(3)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(RunError::Timeout);
            }
            Err(e) => return Err(RunError::Io(e.to_string())),
        }
    };
    let out = reader.join().map_err(|_| RunError::Io("reader panicked".into()))?.map_err(|e| RunError::Io(e.to_string()))?;
    Ok((status.success(), out))
}
