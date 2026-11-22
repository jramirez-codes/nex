//! Parsing git's output, and loading a real (temporary) repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::git::{self, Change, LineKind, Status, parse_diff, parse_numstat, parse_status, word_diff};

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
fn rewritten_lines_get_no_word_highlights() {
    assert!(word_diff("let total = items.len();", "return Err(\"nope\".into());").is_none());
    assert!(word_diff("foo(a, b)", "foo(a, c)").is_some());
}

#[test]
fn binary_files() {
    let d = parse_diff("diff --git a/x.png b/x.png\nindex 1..2 100644\nBinary files a/x.png and b/x.png differ\n");
    assert!(d.binary);
    assert!(d.hunks.is_empty());
}

#[test]
fn combined_conflict_diff() {
    let d = parse_diff("diff --cc c.rs\nindex 1,2..0\n--- a/c.rs\n+++ b/c.rs\n@@@ -1,1 -1,1 +1,5 @@@\n++<<<<<<< HEAD\n +ours\n++=======\n+ theirs\n++>>>>>>> b\n");
    assert_eq!(d.hunks.len(), 1);
    let h = &d.hunks[0];
    assert_eq!(h.lines.len(), 5);
    assert_eq!(h.lines[1].text, "ours");
    assert!(h.lines.iter().all(|l| l.kind == LineKind::Added));
    assert_eq!(h.lines[0].new_no, Some(1));
}

/// A fresh repository in the temp folder, with one commit of `a.txt`.
fn repo(name: &str) -> Option<PathBuf> {
    let dir = std::env::temp_dir().join(format!("nex-diff-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(&dir).args(["-c", "user.name=nex", "-c", "user.email=nex@example.com", "-c", "commit.gpgsign=false"]).args(args).output();
    // No git on this machine: nothing to test against.
    git(&["init", "-q", "-b", "main"]).ok().filter(|o| o.status.success())?;
    std::fs::write(dir.join("a.txt"), "one\ntwo\nthree\n").unwrap();
    std::fs::write(dir.join("gone.txt"), "bye\n").unwrap();
    git(&["add", "."]).unwrap();
    git(&["commit", "-q", "-m", "first"]).unwrap();
    Some(dir)
}

fn sh(dir: &Path, args: &[&str]) {
    Command::new("git").arg("-C").arg(dir).args(args).output().unwrap();
}

#[test]
fn loads_a_real_repository() {
    let Some(dir) = repo("real") else { return };
    std::fs::write(dir.join("a.txt"), "one\n2\nthree\nfour\n").unwrap();
    std::fs::write(dir.join("staged.txt"), "s1\ns2\n").unwrap();
    sh(&dir, &["add", "staged.txt"]);
    std::fs::remove_file(dir.join("gone.txt")).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::fs::write(dir.join("sub/new.txt"), "x\ny\nz").unwrap();

    // From a subfolder too: the whole repository is shown.
    let Status::Ready(s) = git::load_status(&dir.join("sub")) else { panic!("not loaded") };
    assert_eq!(s.branch, "main");
    assert_eq!(s.root.canonicalize().unwrap(), dir.canonicalize().unwrap());
    let staged: Vec<_> = s.staged.iter().map(|f| (f.path.as_str(), f.change, f.counts)).collect();
    assert_eq!(staged, vec![("staged.txt", Change::Added, Some((2, 0)))]);
    let unstaged: Vec<_> = s.unstaged.iter().map(|f| (f.path.as_str(), f.change, f.counts)).collect();
    assert_eq!(
        unstaged,
        vec![("a.txt", Change::Modified, Some((2, 1))), ("gone.txt", Change::Deleted, Some((0, 1))), ("sub/new.txt", Change::Untracked, Some((3, 0)))]
    );

    let d = git::load_diff(&s.root, &s.unstaged[0]).unwrap();
    assert_eq!(d.hunks.len(), 1);
    let added: Vec<&str> = d.hunks[0].lines.iter().filter(|l| l.kind == LineKind::Added).map(|l| l.text.as_str()).collect();
    assert_eq!(added, vec!["2", "four"]);

    let d = git::load_diff(&s.root, &s.unstaged[2]).unwrap();
    let lines: Vec<&str> = d.hunks[0].lines.iter().map(|l| l.text.as_str()).collect();
    assert_eq!(lines, vec!["x", "y", "z"]);
    assert!(d.hunks[0].lines[2].no_newline);

    let d = git::load_diff(&s.root, &s.staged[0]).unwrap();
    assert_eq!(d.hunks[0].lines.len(), 2);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn folders_outside_a_repository() {
    let dir = std::env::temp_dir().join(format!("nex-diff-{}-plain", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // The temp folder itself could sit inside a repository on some machines; only check
    // what can be known.
    match git::load_status(&dir) {
        Status::NotRepo | Status::NoGit | Status::Ready(_) => {}
        Status::Failed(e) => panic!("{e}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
