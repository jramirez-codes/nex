//! The working tree's status: what `git status` says changed (staged and not), and how many
//! lines each file gained and lost.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git_diff::git::{MAX_COUNT_BYTES, NUMSTAT_ARGS, NUMSTAT_STAGED_ARGS, RunError, STATUS_ARGS, TOPLEVEL_ARGS, run};

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
pub(super) fn attach_counts(snap: &mut Snapshot, staged: Numstat, unstaged: Numstat, untracked_lines: impl Fn(&str) -> Option<u32>) {
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
pub(super) fn count_lines(path: &Path) -> Option<u32> {
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

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};
    
    use super::*;

    #[test]
    fn status_sections_renames_and_branch() {
        let out = [
            "# branch.oid 1234",
            "# branch.head main",
            "# branch.upstream origin/main",
            "# branch.ab +2 -1",
            "1 .M N... 100644 100644 100644 aaa bbb src/app/mod.rs",
            "1 M. N... 100644 100644 100644 aaa bbb Cargo.toml",
            "1 MM N... 100644 100644 100644 aaa bbb both.rs",
            "1 A. N... 000000 100644 100644 000 bbb new file.rs",
            "2 R. N... 100644 100644 100644 aaa bbb R100 src/renamed.rs",
            "src/old.rs",
            "1 .D N... 100644 100644 000000 aaa bbb gone.rs",
            "u UU N... 100644 100644 100644 100644 a b c conflict.rs",
            "? notes/todo.md",
            "",
        ]
        .join("\0");
        let s = parse_status(&out);
        assert_eq!(s.branch, "main");
        assert_eq!((s.ahead, s.behind), (2, 1));
        let staged: Vec<(&str, Change)> = s.staged.iter().map(|f| (f.path.as_str(), f.change)).collect();
        assert_eq!(staged, vec![("Cargo.toml", Change::Modified), ("both.rs", Change::Modified), ("new file.rs", Change::Added), ("src/renamed.rs", Change::Renamed)]);
        assert_eq!(s.staged[3].orig.as_deref(), Some("src/old.rs"));
        let unstaged: Vec<(&str, Change)> = s.unstaged.iter().map(|f| (f.path.as_str(), f.change)).collect();
        assert_eq!(
            unstaged,
            vec![
                ("both.rs", Change::Modified),
                ("conflict.rs", Change::Conflicted),
                ("gone.rs", Change::Deleted),
                ("notes/todo.md", Change::Untracked),
                ("src/app/mod.rs", Change::Modified),
            ]
        );
        assert_eq!(s.unstaged[3].name(), "todo.md");
        assert_eq!(s.unstaged[3].folder(), "notes");
        assert_eq!(s.staged[0].folder(), "");
    }

    #[test]
    fn detached_and_unborn_heads() {
        assert_eq!(parse_status("# branch.head (detached)\0").branch, "HEAD (detached)");
        assert_eq!(parse_status("").branch, "HEAD");
    }

    #[test]
    fn numstat_counts_binaries_and_renames() {
        let out = "3\t1\tsrc/a.rs\0-\t-\timg.png\0" .to_string() + "5\t0\t\0old.rs\0new.rs\0";
        let n = parse_numstat(&out);
        assert_eq!(n, vec![("src/a.rs".into(), Some((3, 1))), ("img.png".into(), None), ("new.rs".into(), Some((5, 0)))]);
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
