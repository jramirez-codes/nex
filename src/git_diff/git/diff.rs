//! One file's unified diff: asking git for it and parsing it into hunks and lines.

use std::ops::Range;
use std::path::Path;

use crate::git_diff::git::emphasis::add_emphasis;
use crate::git_diff::git::status::{Change, FileChange};
use crate::git_diff::git::{MAX_DIFF_LINES, RunError, run};

/// Spaces a tab expands to in the diff view.
const TAB_WIDTH: usize = 4;

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
    pub syntax: crate::git_diff::syntax::Tokens,
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
pub(super) fn diff_args(file: &FileChange, null: &str) -> Vec<String> {
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
        hunk.lines.push(Line {
            kind,
            old_no: old_no_shown,
            new_no: new_no_shown,
            text: expand_tabs(text),
            emphasis: Vec::new(),
            syntax: Vec::new(),
            no_newline: false,
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    const DIFF: &str = "diff --git a/src/a.rs b/src/a.rs
index 111..222 100644
--- a/src/a.rs
+++ b/src/a.rs
@@ -1,4 +1,5 @@ fn main() {
 fn main() {
-    let x = 1;
+    let x = 2;
+\tlog(x);
 }
\x20
@@ -20 +21,2 @@
-end
+end
+tail
\\ No newline at end of file
";

    #[test]
    fn hunks_lines_and_numbers() {
        let d = parse_diff(DIFF);
        assert!(!d.binary && !d.truncated);
        assert_eq!(d.hunks.len(), 2);
        let h = &d.hunks[0];
        assert_eq!((h.old_start, h.old_len, h.new_start, h.new_len), (1, 4, 1, 5));
        assert_eq!(h.context, "fn main() {");
        let kinds: Vec<LineKind> = h.lines.iter().map(|l| l.kind).collect();
        use LineKind::*;
        assert_eq!(kinds, vec![Context, Removed, Added, Added, Context, Context]);
        let nums: Vec<(Option<u32>, Option<u32>)> = h.lines.iter().map(|l| (l.old_no, l.new_no)).collect();
        assert_eq!(nums, vec![(Some(1), Some(1)), (Some(2), None), (None, Some(2)), (None, Some(3)), (Some(3), Some(4)), (Some(4), Some(5))]);
        // Tabs expand to the next stop, so columns line up.
        assert_eq!(h.lines[3].text, "    log(x);");
        // An empty context line is kept (its prefix is a space).
        assert_eq!(h.lines[5].text, "");

        let h = &d.hunks[1];
        assert_eq!((h.old_start, h.old_len, h.new_start, h.new_len), (20, 1, 21, 2));
        assert!(h.context.is_empty());
        assert!(h.lines.last().unwrap().no_newline);
        assert!(!h.lines[1].no_newline);
    }

    #[test]
    fn changed_words_are_highlighted() {
        let d = parse_diff(DIFF);
        let h = &d.hunks[0];
        let (old, new) = (&h.lines[1], &h.lines[2]);
        assert_eq!(old.emphasis.iter().map(|r| &old.text[r.clone()]).collect::<Vec<_>>(), vec!["1"]);
        assert_eq!(new.emphasis.iter().map(|r| &new.text[r.clone()]).collect::<Vec<_>>(), vec!["2"]);
        // A line paired with nothing (the extra addition) has no highlights of its own.
        assert!(h.lines[3].emphasis.is_empty());
    }

    #[test]
    fn binary_files() {
        let d = parse_diff("diff --git a/x.png b/x.png\nindex 1..2 100644\nBinary files a/x.png and b/x.png differ\n");
        assert!(d.binary);
        assert!(d.hunks.is_empty());
    }

    #[test]
    fn combined_conflict_diff() {
        let d = parse_diff(
            "diff --cc c.rs\nindex 1,2..0\n--- a/c.rs\n+++ b/c.rs\n@@@ -1,1 -1,1 +1,5 @@@\n++<<<<<<< HEAD\n +ours\n++=======\n+ theirs\n++>>>>>>> b\n",
        );
        assert_eq!(d.hunks.len(), 1);
        let h = &d.hunks[0];
        assert_eq!(h.lines.len(), 5);
        assert_eq!(h.lines[1].text, "ours");
        assert!(h.lines.iter().all(|l| l.kind == LineKind::Added));
        assert_eq!(h.lines[0].new_no, Some(1));
    }
}
