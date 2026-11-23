//! Running `git` and reading what it prints: the working tree's status (with per-file line
//! counts) and one file's unified diff, parsed into hunks with word-level highlights.
//! Everything here is blocking; `GitDiff` runs it on a background thread.
//!
//! For a folder on an ssh host the same `git` commands run there (`*_remote`, see
//! `remote::exec`), batched so a status is one trip; their output is parsed the same way.

use std::collections::HashMap;
use std::io::Read;
use std::ops::Range;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::remote::exec::{ExecError, Transport};
use crate::remote::{RemoteError, cd_word};
use crate::remote::posix_quote;

/// Longest a single `git` call may take before it is killed (a huge repo, a hung hook).
const GIT_TIMEOUT: Duration = Duration::from_secs(8);
/// Diff lines kept for one file; past this the rest is cut off with a note.
pub const MAX_DIFF_LINES: usize = 20_000;
/// Untracked files bigger than this aren't read to count their lines.
const MAX_COUNT_BYTES: u64 = 4 * 1024 * 1024;
/// Spaces a tab expands to in the diff view.
const TAB_WIDTH: usize = 4;
/// Lines longer than this (in chars) get no word-level highlights.
const MAX_EMPHASIS_CHARS: usize = 500;
/// Tokens per line compared for word-level highlights (the LCS is quadratic).
const MAX_EMPHASIS_TOKENS: usize = 240;

/// What every `git` runs with, here and on a host: never page, prompt or take locks, and
/// print in plain C-locale text.
const GIT_ENV: [(&str, &str); 4] = [("GIT_PAGER", "cat"), ("GIT_TERMINAL_PROMPT", "0"), ("GIT_OPTIONAL_LOCKS", "0"), ("LC_ALL", "C")];
/// The repository's root folder.
const TOPLEVEL_ARGS: [&str; 2] = ["rev-parse", "--show-toplevel"];
/// The status [`parse_status`] reads.
const STATUS_ARGS: [&str; 5] = ["status", "--porcelain=v2", "-z", "--branch", "--untracked-files=all"];
/// Line counts ([`parse_numstat`]) of the work tree against the index.
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

/// What happened to a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Modified,
    Added,
    Deleted,
    Renamed,
    Copied,
    TypeChanged,
    Untracked,
    Conflicted,
}

impl Change {
    fn from_code(c: char) -> Option<Change> {
        Some(match c {
            'M' => Change::Modified,
            'A' => Change::Added,
            'D' => Change::Deleted,
            'R' => Change::Renamed,
            'C' => Change::Copied,
            'T' => Change::TypeChanged,
            _ => return None,
        })
    }

    /// The one-letter badge shown in the list.
    pub fn letter(self) -> &'static str {
        match self {
            Change::Modified => "M",
            Change::Added => "A",
            Change::Deleted => "D",
            Change::Renamed => "R",
            Change::Copied => "C",
            Change::TypeChanged => "T",
            Change::Untracked => "U",
            Change::Conflicted => "!",
        }
    }

    pub fn describe(self) -> &'static str {
        match self {
            Change::Modified => "Modified",
            Change::Added => "Added",
            Change::Deleted => "Deleted",
            Change::Renamed => "Renamed",
            Change::Copied => "Copied",
            Change::TypeChanged => "Type changed",
            Change::Untracked => "Untracked",
            Change::Conflicted => "Conflict",
        }
    }
}

/// One changed file, staged or not. Paths are relative to the repository root, with `/`.
#[derive(Clone, Debug, PartialEq)]
pub struct FileChange {
    pub path: String,
    /// Where a renamed or copied file came from.
    pub orig: Option<String>,
    pub change: Change,
    pub staged: bool,
    /// Lines added and removed; `None` for a binary file (or one too big to count).
    pub counts: Option<(u32, u32)>,
}

impl FileChange {
    pub fn name(&self) -> &str {
        self.path.rsplit('/').next().unwrap_or(&self.path)
    }

    /// The folder part of the path, without the trailing `/` (empty at the root).
    pub fn folder(&self) -> &str {
        self.path.rfind('/').map_or("", |i| &self.path[..i])
    }
}

/// The repository's state, as of one `git status`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub root: PathBuf,
    /// The branch name, `HEAD (detached)` or the initial branch of an empty repository.
    pub branch: String,
    pub ahead: u32,
    pub behind: u32,
    pub staged: Vec<FileChange>,
    pub unstaged: Vec<FileChange>,
}

/// What loading a folder's status gave.
#[derive(Clone, Debug, PartialEq)]
pub enum Status {
    Ready(Snapshot),
    NotRepo,
    NoGit,
    Failed(String),
    /// The folder is on an ssh host nex can't log in to without a password.
    NeedsLogin,
}

impl From<RunError> for Status {
    fn from(e: RunError) -> Status {
        match e {
            RunError::NotFound => Status::NoGit,
            RunError::Timeout => Status::Failed("git took too long".into()),
            RunError::Io(e) => Status::Failed(e),
        }
    }
}

/// `git status` (and line counts) for the repository holding `dir`.
pub fn load_status(dir: &Path) -> Status {
    let root = match run(dir, &TOPLEVEL_ARGS) {
        Ok((true, out)) => PathBuf::from(String::from_utf8_lossy(&out).trim()),
        Ok((false, _)) => return Status::NotRepo,
        Err(e) => return e.into(),
    };
    let out = match run(&root, &STATUS_ARGS) {
        Ok((true, out)) => out,
        Ok((false, _)) => return Status::Failed("git status failed".into()),
        Err(RunError::Timeout) => return Status::Failed("git status took too long".into()),
        Err(e) => return e.into(),
    };
    let mut snap = parse_status(&String::from_utf8_lossy(&out));
    snap.root = root.clone();
    let numstat = |args: &[&str]| run(&root, args).ok().filter(|(ok, _)| *ok).map(|(_, out)| parse_numstat(&String::from_utf8_lossy(&out))).unwrap_or_default();
    let (staged, unstaged) = (numstat(&NUMSTAT_STAGED_ARGS), numstat(&NUMSTAT_ARGS));
    attach_counts(&mut snap, staged, unstaged, |path| count_lines(&root.join(path)));
    Status::Ready(snap)
}

/// Parse `git status --porcelain=v2 -z --branch`. Files come out sorted by path.
pub fn parse_status(out: &str) -> Snapshot {
    let mut snap = Snapshot::default();
    let mut head = String::new();
    let mut fields = out.split('\0');
    while let Some(rec) = fields.next() {
        if let Some(h) = rec.strip_prefix("# ") {
            if let Some(name) = h.strip_prefix("branch.head ") {
                head = name.to_string();
            } else if let Some(ab) = h.strip_prefix("branch.ab ") {
                for part in ab.split(' ') {
                    if let Some(n) = part.strip_prefix('+') {
                        snap.ahead = n.parse().unwrap_or(0);
                    } else if let Some(n) = part.strip_prefix('-') {
                        snap.behind = n.parse().unwrap_or(0);
                    }
                }
            }
            continue;
        }
        let mut push = |path: &str, orig: Option<String>, x: char, y: char| {
            if let Some(change) = Change::from_code(x) {
                snap.staged.push(FileChange { path: path.to_string(), orig: orig.clone(), change, staged: true, counts: None });
            }
            if let Some(change) = Change::from_code(y) {
                // A rename only shows in the index; the work-tree side is a plain change.
                snap.unstaged.push(FileChange { path: path.to_string(), orig: None, change, staged: false, counts: None });
            }
        };
        match rec.as_bytes().first() {
            Some(b'1') => {
                let parts: Vec<&str> = rec.splitn(9, ' ').collect();
                if let (Some(xy), Some(path)) = (parts.get(1), parts.get(8)) {
                    let mut xy = xy.chars();
                    push(path, None, xy.next().unwrap_or('.'), xy.next().unwrap_or('.'));
                }
            }
            Some(b'2') => {
                let parts: Vec<&str> = rec.splitn(10, ' ').collect();
                let orig = fields.next().map(str::to_string);
                if let (Some(xy), Some(path)) = (parts.get(1), parts.get(9)) {
                    let mut xy = xy.chars();
                    push(path, orig, xy.next().unwrap_or('.'), xy.next().unwrap_or('.'));
                }
            }
            Some(b'u') => {
                if let Some(path) = rec.splitn(11, ' ').nth(10) {
                    snap.unstaged.push(FileChange { path: path.to_string(), orig: None, change: Change::Conflicted, staged: false, counts: None });
                }
            }
            Some(b'?') => {
                if let Some(path) = rec.get(2..).filter(|p| !p.is_empty()) {
                    snap.unstaged.push(FileChange { path: path.to_string(), orig: None, change: Change::Untracked, staged: false, counts: None });
                }
            }
            _ => {}
        }
    }
    snap.branch = match head.as_str() {
        "(detached)" => "HEAD (detached)".to_string(),
        "" => "HEAD".to_string(),
        name => name.to_string(),
    };
    snap.staged.sort_by(|a, b| a.path.cmp(&b.path));
    snap.unstaged.sort_by(|a, b| a.path.cmp(&b.path));
    snap
}

/// What [`parse_numstat`] gives: each path with its lines added and removed.
pub type Numstat = Vec<(String, Option<(u32, u32)>)>;

/// Parse `git diff --numstat -z`: `(path, Some((added, removed)))`, `None` counts for binary
/// files. A rename is reported under its new path.
pub fn parse_numstat(out: &str) -> Numstat {
    let mut result = Vec::new();
    let mut fields = out.split('\0');
    while let Some(rec) = fields.next() {
        let mut parts = rec.splitn(3, '\t');
        let (Some(a), Some(r), Some(path)) = (parts.next(), parts.next(), parts.next()) else { continue };
        let path = if path.is_empty() {
            // A rename: the old and new paths follow as fields of their own.
            let _orig = fields.next();
            fields.next().unwrap_or_default().to_string()
        } else {
            path.to_string()
        };
        let counts = a.parse().ok().zip(r.parse().ok());
        result.push((path, counts));
    }
    result
}

/// Give every file of `snap` its line counts: a tracked file's from the numstat of its side,
/// an untracked one's whole length, as `untracked_lines` counts it.
fn attach_counts(snap: &mut Snapshot, staged: Numstat, unstaged: Numstat, untracked_lines: impl Fn(&str) -> Option<u32>) {
    // The first entry for a path wins, should git ever list one twice.
    let by_path = |numstat: Numstat| {
        let mut map: HashMap<String, Option<(u32, u32)>> = HashMap::with_capacity(numstat.len());
        for (path, counts) in numstat {
            map.entry(path).or_insert(counts);
        }
        map
    };
    for (files, numstat) in [(&mut snap.staged, staged), (&mut snap.unstaged, unstaged)] {
        let counts = by_path(numstat);
        for f in files {
            f.counts = match f.change {
                Change::Untracked => untracked_lines(&f.path).map(|n| (n, 0)),
                _ => counts.get(&f.path).copied().flatten(),
            };
        }
    }
}

/// Lines in a text file, or `None` for a binary (or very big, or unreadable) one.
fn count_lines(path: &Path) -> Option<u32> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() > MAX_COUNT_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if is_binary(&bytes) {
        return None;
    }
    let lines = bytes.iter().filter(|&&b| b == b'\n').count();
    let trailing = !bytes.is_empty() && bytes.last() != Some(&b'\n');
    Some((lines + trailing as usize) as u32)
}

/// Git's own test: a NUL in the first 8000 bytes.
fn is_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8000).any(|&b| b == 0)
}

/// Byte ranges within a line.
pub type Spans = Vec<Range<usize>>;

/// What a diff line is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Added,
    Removed,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub kind: LineKind,
    pub old_no: Option<u32>,
    pub new_no: Option<u32>,
    /// The text, tabs expanded, without the `+` / `-` / ` ` prefix.
    pub text: String,
    /// Byte ranges of `text` that changed within the line (word-level highlights).
    pub emphasis: Vec<Range<usize>>,
    /// Byte ranges of `text` by token kind, for syntax colors (empty for unknown languages).
    pub syntax: super::syntax::Tokens,
    /// The file ends here without a newline.
    pub no_newline: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Hunk {
    pub old_start: u32,
    pub old_len: u32,
    pub new_start: u32,
    pub new_len: u32,
    /// The function or section the hunk is in (`fn paint(...)`), as git guessed it.
    pub context: String,
    pub lines: Vec<Line>,
    /// Every line carries its syntax colors (they're worked out in the background).
    pub colored: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct FileDiff {
    pub hunks: Vec<Hunk>,
    pub binary: bool,
    /// More than `MAX_DIFF_LINES` lines: the rest was dropped.
    pub truncated: bool,
}

impl FileDiff {
    pub fn line_count(&self) -> usize {
        self.hunks.iter().map(|h| h.lines.len()).sum()
    }
}

/// The diff of `file`: the index against `HEAD` when staged, else the work tree against the
/// index (an untracked file against nothing).
pub fn load_diff(root: &Path, file: &FileChange) -> Result<FileDiff, String> {
    let args = diff_args(file, null_device());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    // `--no-index` exits with 1 when the files differ, which they always do here.
    match run(root, &args) {
        Ok((_, out)) => Ok(parse_diff(&String::from_utf8_lossy(&out))),
        Err(RunError::NotFound) => Err("git not found".into()),
        Err(RunError::Timeout) => Err("git diff took too long".into()),
        Err(RunError::Io(e)) => Err(e),
    }
}

/// `git diff …` for `file` (see [`load_diff`]), with `null` the null device.
fn diff_args(file: &FileChange, null: &str) -> Vec<String> {
    let base = ["--no-color", "--no-ext-diff", "--no-textconv", "-U3"];
    let mut args: Vec<&str> = vec!["diff"];
    match (file.staged, file.change) {
        (_, Change::Untracked) => {
            args.extend(["--no-index"]);
            args.extend(base);
            args.extend(["--", null, file.path.as_str()]);
        }
        (true, _) => {
            args.extend(["--cached", "-M"]);
            args.extend(base);
            args.push("--");
            if let Some(orig) = &file.orig {
                args.push(orig);
            }
            args.push(&file.path);
        }
        (false, _) => {
            args.extend(base);
            args.extend(["--", file.path.as_str()]);
        }
    }
    args.into_iter().map(String::from).collect()
}

/// Exit status of a remote script whose `cd` into the folder failed.
const NO_FOLDER: i32 = 3;

/// A command for the host's shell: `git args…` in `dir` (which may start with `~`), with
/// the same environment as [`run`] gives it here.
fn remote_git(dir: &str, args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|a| posix_quote(a)).collect();
    let env: Vec<String> = GIT_ENV.iter().map(|(name, value)| format!("{name}={value}")).collect();
    format!("cd -- {} 2>/dev/null || exit {NO_FOLDER}; {} git {}", cd_word(dir), env.join(" "), args.join(" "))
}

/// What a remote failure looks like in the pane.
fn remote_failure(e: ExecError) -> Status {
    match e {
        ExecError::Remote(RemoteError::NeedsLogin) => Status::NeedsLogin,
        other => Status::Failed(other.to_string()),
    }
}

/// [`load_status`] for `dir` on the host `t` reaches: one trip for the status and the
/// counts, and one more for untracked files' line counts.
pub fn load_status_remote(t: &Transport, dir: &Path) -> Status {
    let dir = dir.to_string_lossy();
    let commands: Vec<String> = [&TOPLEVEL_ARGS[..], &STATUS_ARGS, &NUMSTAT_STAGED_ARGS, &NUMSTAT_ARGS].iter().map(|args| remote_git(&dir, args)).collect();
    let outs = match t.run_many(&commands) {
        Ok(outs) => outs,
        Err(e) => return remote_failure(e),
    };
    let [top, status, staged, unstaged] = outs.as_slice() else { return Status::Failed("The host's answer was cut short".into()) };
    match top.code {
        0 => {}
        127 => return Status::NoGit,
        NO_FOLDER => return Status::Failed(format!("{dir} isn't there any more")),
        _ => return Status::NotRepo,
    }
    if !status.ok() {
        return Status::Failed("git status failed".into());
    }
    let root = PathBuf::from(top.text().trim());
    let mut snap = parse_status(&status.text());
    snap.root = root.clone();
    let numstat = |o: &crate::remote::exec::Output| if o.ok() { parse_numstat(&o.text()) } else { Vec::new() };
    let untracked: Vec<String> = snap.unstaged.iter().filter(|f| f.change == Change::Untracked).map(|f| f.path.clone()).take(MAX_REMOTE_COUNTS).collect();
    let counts = count_lines_remote(t, &root, &untracked);
    let lines: HashMap<&str, u32> = untracked.iter().zip(counts).filter_map(|(path, n)| Some((path.as_str(), n?))).collect();
    attach_counts(&mut snap, numstat(staged), numstat(unstaged), |path| lines.get(path).copied());
    Status::Ready(snap)
}

/// Untracked files on a host whose lines are counted (more would make one huge command).
const MAX_REMOTE_COUNTS: usize = 500;

/// [`count_lines`] for each of `files` (relative to `root`) on the host, in one trip.
fn count_lines_remote(t: &Transport, root: &Path, files: &[String]) -> Vec<Option<u32>> {
    if files.is_empty() {
        return Vec::new();
    }
    let script = count_script(&root.to_string_lossy(), files);
    let Ok(out) = t.run(&script) else { return Vec::new() };
    parse_counts(&out.text())
}

/// A script printing [`count_lines`]'s answer for each of `files` in the folder `root`, one
/// line each: the count, or `-`.
fn count_script(root: &str, files: &[String]) -> String {
    let quoted: Vec<String> = files.iter().map(|f| posix_quote(f)).collect();
    format!(
        r#"cd -- {root} 2>/dev/null || exit {NO_FOLDER}
for f in {files}; do
  if [ -f "$f" ] && [ "$(wc -c <"$f" | tr -d ' ')" -le {MAX_COUNT_BYTES} ] && [ "$(head -c 8000 "$f" | tr -cd '\000' | wc -c | tr -d ' ')" = 0 ]; then
    n=$(wc -l <"$f" | tr -d ' '); [ -n "$(tail -c 1 "$f")" ] && n=$((n+1)); printf '%s\n' "$n"
  else echo -; fi
done"#,
        root = posix_quote(root),
        files = quoted.join(" "),
    )
}

/// One line per file: its line count, or `-` (binary, too big, not a file).
fn parse_counts(text: &str) -> Vec<Option<u32>> {
    text.lines().map(|l| l.trim().parse().ok()).collect()
}

/// [`load_diff`] for a file in the repository at `root` on the host `t` reaches.
pub fn load_diff_remote(t: &Transport, root: &Path, file: &FileChange) -> Result<FileDiff, String> {
    let args = diff_args(file, "/dev/null");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = t.run(&remote_git(&root.to_string_lossy(), &args)).map_err(|e| e.to_string())?;
    match out.code {
        // `--no-index` exits with 1 when the files differ, which they always do here.
        0 | 1 => Ok(parse_diff(&out.text())),
        127 => Err("git not found on the host".into()),
        NO_FOLDER => Err("The repository isn't there any more".into()),
        code => Err(format!("git diff failed (exit status {code})")),
    }
}

fn null_device() -> &'static str {
    if cfg!(windows) { "NUL" } else { "/dev/null" }
}

/// Parse unified diff output (one file's, possibly a merge conflict's combined diff).
pub fn parse_diff(out: &str) -> FileDiff {
    let mut diff = FileDiff::default();
    let mut old_no = 0u32;
    let mut new_no = 0u32;
    // A combined diff (`@@@`) has one prefix column per parent.
    let mut prefix_cols = 1usize;
    let mut total = 0usize;
    for raw in out.split('\n') {
        let raw = raw.strip_suffix('\r').unwrap_or(raw);
        if raw.starts_with("Binary files ") || raw == "GIT binary patch" {
            diff.binary = true;
            continue;
        }
        if let Some(h) = parse_hunk_header(raw) {
            prefix_cols = h.1;
            old_no = h.0.old_start;
            new_no = h.0.new_start;
            diff.hunks.push(h.0);
            continue;
        }
        let Some(hunk) = diff.hunks.last_mut() else { continue };
        // Every real diff line has a prefix; an empty one is the end of git's output.
        if raw.is_empty() {
            continue;
        }
        if raw.starts_with('\\') {
            if let Some(last) = hunk.lines.last_mut() {
                last.no_newline = true;
            }
            continue;
        }
        if raw.starts_with("diff --git ") || raw.starts_with("diff --cc ") || raw.starts_with("diff --combined ") {
            // A second file in the output: not expected, but don't fold it into this one.
            break;
        }
        if total >= MAX_DIFF_LINES {
            diff.truncated = true;
            break;
        }
        let split = raw.char_indices().nth(prefix_cols).map_or(raw.len(), |(i, _)| i);
        let (prefix, text) = raw.split_at(split);
        let kind = if prefix.contains('+') {
            LineKind::Added
        } else if prefix.contains('-') {
            LineKind::Removed
        } else {
            LineKind::Context
        };
        let (o, n) = match kind {
            LineKind::Context => (Some(old_no), Some(new_no)),
            LineKind::Added => (None, Some(new_no)),
            LineKind::Removed => (Some(old_no), None),
        };
        if o.is_some() {
            old_no += 1;
        }
        if n.is_some() {
            new_no += 1;
        }
        let (old_no_shown, new_no_shown) = if prefix_cols > 1 { (None, n) } else { (o, n) };
        hunk.lines.push(Line { kind, old_no: old_no_shown, new_no: new_no_shown, text: expand_tabs(text), emphasis: Vec::new(), syntax: Vec::new(), no_newline: false });
        total += 1;
    }
    for h in &mut diff.hunks {
        add_emphasis(&mut h.lines);
    }
    diff
}

/// `@@ -1,5 +1,6 @@ fn main()` (or a combined `@@@ … @@@`), and how many prefix columns its
/// lines have.
fn parse_hunk_header(line: &str) -> Option<(Hunk, usize)> {
    let ats = line.bytes().take_while(|&b| b == b'@').count();
    if ats < 2 || line.as_bytes().get(ats) != Some(&b' ') {
        return None;
    }
    let marker = &line[..ats];
    let rest = &line[ats + 1..];
    let end = rest.find(&format!(" {marker}"))?;
    let ranges: Vec<&str> = rest[..end].split(' ').collect();
    let context = rest[end + 1 + ats..].trim().to_string();
    let range = |s: &str| -> Option<(u32, u32)> {
        let s = &s[1..];
        let mut it = s.splitn(2, ',');
        let start = it.next()?.parse().ok()?;
        let len = it.next().map_or(Some(1), |l| l.parse().ok())?;
        Some((start, len))
    };
    let old = ranges.iter().find(|r| r.starts_with('-')).and_then(|r| range(r))?;
    let new = ranges.iter().find(|r| r.starts_with('+')).and_then(|r| range(r))?;
    let hunk = Hunk { old_start: old.0, old_len: old.1, new_start: new.0, new_len: new.1, context, lines: Vec::new(), colored: false };
    Some((hunk, ats - 1))
}

/// Tabs to spaces (to the next multiple of `TAB_WIDTH`), so columns line up.
fn expand_tabs(s: &str) -> String {
    if !s.contains('\t') {
        return s.to_string();
    }
    let mut out = String::with_capacity(s.len() + 8);
    let mut col = 0;
    for c in s.chars() {
        if c == '\t' {
            let n = TAB_WIDTH - col % TAB_WIDTH;
            out.extend(std::iter::repeat_n(' ', n));
            col += n;
        } else {
            out.push(c);
            col += 1;
        }
    }
    out
}

/// Pair each run of removed lines with the added lines right after it, and mark the words
/// that differ between each pair.
fn add_emphasis(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let add_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        let pairs = (add_start - del_start).min(i - add_start);
        for k in 0..pairs {
            let (old, new) = (del_start + k, add_start + k);
            if let Some((a, b)) = word_diff(&lines[old].text, &lines[new].text) {
                lines[old].emphasis = a;
                lines[new].emphasis = b;
            }
        }
    }
}

/// Byte ranges that changed in `old` and in `new`, or `None` when the lines are too long or
/// too different for highlights to help (the whole line is then simply changed).
pub fn word_diff(old: &str, new: &str) -> Option<(Spans, Spans)> {
    if old.chars().count() > MAX_EMPHASIS_CHARS || new.chars().count() > MAX_EMPHASIS_CHARS {
        return None;
    }
    let a = tokens(old);
    let b = tokens(new);
    if a.len() > MAX_EMPHASIS_TOKENS || b.len() > MAX_EMPHASIS_TOKENS || a.is_empty() || b.is_empty() {
        return None;
    }
    let ta: Vec<&str> = a.iter().map(|r| &old[r.clone()]).collect();
    let tb: Vec<&str> = b.iter().map(|r| &new[r.clone()]).collect();
    // LCS table, from the end.
    let (n, m) = (ta.len(), tb.len());
    let mut dp = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if ta[i] == tb[j] { dp[at(i + 1, j + 1)] + 1 } else { dp[at(i + 1, j)].max(dp[at(i, j + 1)]) };
        }
    }
    let mut keep_a = vec![false; n];
    let mut keep_b = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if ta[i] == tb[j] {
            keep_a[i] = true;
            keep_b[j] = true;
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    // Mostly rewritten: highlighting nearly every word is just noise.
    let common: usize = keep_a.iter().zip(&ta).filter(|(k, t)| **k && !t.trim().is_empty()).map(|(_, t)| t.len()).sum();
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if common * 10 < longest * 4 {
        return None;
    }
    Some((changed_ranges(&a, &keep_a), changed_ranges(&b, &keep_b)))
}

/// The ranges of tokens not kept, merged where they touch.
fn changed_ranges(spans: &[Range<usize>], keep: &[bool]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (k, span) in spans.iter().enumerate() {
        if keep[k] {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == span.start => last.end = span.end,
            _ => out.push(span.clone()),
        }
    }
    out
}

/// Words (letters, digits, `_`), runs of whitespace, and single other characters, as byte
/// ranges.
fn tokens(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<(usize, u8)> = None;
    let class = |c: char| if c.is_alphanumeric() || c == '_' { 1 } else if c.is_whitespace() { 2 } else { 0 };
    for (i, c) in s.char_indices() {
        let k = class(c);
        match start {
            Some((_, sk)) if sk == k && k != 0 => {}
            Some((st, _)) => {
                out.push(st..i);
                start = Some((i, k));
            }
            None => start = Some((i, k)),
        }
    }
    if let Some((st, _)) = start {
        out.push(st..s.len());
    }
    out
}

#[cfg(test)]
mod remote_tests {
    use super::*;

    #[test]
    fn remote_git_commands() {
        assert_eq!(
            remote_git("~/my repo", &["diff", "--", "a b.txt"]),
            "cd -- ~/'my repo' 2>/dev/null || exit 3; GIT_PAGER=cat GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 LC_ALL=C git diff -- 'a b.txt'"
        );
    }

    #[test]
    fn counts_lines_like_here() {
        let dir = std::env::temp_dir().join(format!("nex-remote-counts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("two"), "a\nb\n").unwrap();
        std::fs::write(dir.join("no newline"), "a\nb").unwrap();
        std::fs::write(dir.join("bin"), b"a\0b").unwrap();
        let files = ["two", "no newline", "bin", "missing"];
        // The same script as on a host, run here by `sh`.
        let script = count_script(&dir.to_string_lossy(), &files.map(String::from));
        let out = std::process::Command::new("sh").arg("-c").arg(script).output().unwrap();
        let counts = parse_counts(&String::from_utf8_lossy(&out.stdout));
        assert_eq!(counts, [Some(2), Some(2), None, None]);
        for f in ["two", "no newline"] {
            assert_eq!(counts[files.iter().position(|x| *x == f).unwrap()], count_lines(&dir.join(f)));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn file(path: &str, change: Change, staged: bool) -> FileChange {
        FileChange { path: path.into(), orig: None, change, staged, counts: None }
    }

    #[test]
    fn counts_attach_by_path_and_side() {
        let mut snap = Snapshot {
            staged: vec![file("a.rs", Change::Modified, true), file("img.png", Change::Added, true), file("gone.rs", Change::Deleted, true)],
            unstaged: vec![file("a.rs", Change::Modified, false), file("new.txt", Change::Untracked, false), file("big.bin", Change::Untracked, false)],
            ..Snapshot::default()
        };
        let staged = vec![("img.png".to_string(), None), ("a.rs".to_string(), Some((3, 1))), ("a.rs".to_string(), Some((9, 9)))];
        // `new.txt` in the numstat is ignored: an untracked file's count is its length.
        let unstaged = vec![("a.rs".to_string(), Some((0, 7))), ("new.txt".to_string(), Some((5, 5)))];
        attach_counts(&mut snap, staged, unstaged, |path| (path == "new.txt").then_some(12));
        let counts = |files: &[FileChange]| files.iter().map(|f| f.counts).collect::<Vec<_>>();
        assert_eq!(counts(&snap.staged), [Some((3, 1)), None, None]);
        assert_eq!(counts(&snap.unstaged), [Some((0, 7)), Some((12, 0)), None]);
    }

    /// A status with thousands of files: the counts are matched in one pass, not by
    /// searching the whole numstat for every file.
    #[test]
    fn counts_attach_fast_for_many_files() {
        let n = 40_000u32;
        let path = |i: u32| format!("src/some/deep/folder/file-{i:06}.rs");
        let mut snap = Snapshot { unstaged: (0..n).map(|i| file(&path(i), Change::Modified, false)).collect(), ..Snapshot::default() };
        let numstat: Numstat = (0..n).rev().map(|i| (path(i), Some((i, 1)))).collect();
        let start = Instant::now();
        attach_counts(&mut snap, Vec::new(), numstat, |_| None);
        let took = start.elapsed();
        assert!(snap.unstaged.iter().enumerate().all(|(i, f)| f.counts == Some((i as u32, 1))));
        // Searching per file is 800 million string compares here: 26 s in a debug build, where
        // this takes about 0.1 s.
        assert!(took < Duration::from_secs(5), "attaching counts took {took:?}");
    }
}
