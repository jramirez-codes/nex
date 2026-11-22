//! Parsing git's output, and loading a real (temporary) repository.

use std::path::{Path, PathBuf};
use std::process::Command;

use super::git::{self, Change, LineKind, Status, parse_diff, parse_numstat, parse_status, word_diff};
use super::syntax::Token;

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

/// A one-hunk diff of `lines` (each with its `+`/`-`/` ` prefix), colored as `path`.
fn highlighted(path: &str, lines: &[&str]) -> git::FileDiff {
    let old = lines.iter().filter(|l| !l.starts_with('+')).count();
    let new = lines.iter().filter(|l| !l.starts_with('-')).count();
    let mut out = format!("@@ -1,{old} +1,{new} @@\n");
    for l in lines {
        out.push_str(l);
        out.push('\n');
    }
    let mut d = parse_diff(&out);
    super::syntax::highlight_file(&mut d, path, None);
    d
}

/// The token covering the first occurrence of `needle` in line `i`.
fn token_at(d: &git::FileDiff, i: usize, needle: &str) -> Option<Token> {
    let line = &d.hunks[0].lines[i];
    let at = line.text.find(needle).expect("needle in line");
    line.syntax.iter().find(|(r, _)| r.start <= at && at < r.end).map(|(_, t)| *t)
}

#[test]
fn syntax_rust_tokens() {
    let d = highlighted("src/main.rs", &[" fn main() {", "+    let s = \"hi\"; // note", "+    let n = 42;", " }"]);
    assert_eq!(token_at(&d, 0, "fn"), Some(Token::Keyword));
    assert_eq!(token_at(&d, 0, "main"), Some(Token::Function));
    assert_eq!(token_at(&d, 1, "hi"), Some(Token::String));
    assert_eq!(token_at(&d, 1, "note"), Some(Token::Comment));
    assert_eq!(token_at(&d, 2, "42"), Some(Token::Number));
}

#[test]
fn syntax_block_comment_spans_lines() {
    let d = highlighted("a.c", &[" /* start", "+   still a comment", "+*/", "+int x;"]);
    assert_eq!(token_at(&d, 1, "still"), Some(Token::Comment));
    assert_ne!(token_at(&d, 3, "int"), Some(Token::Comment));
}

#[test]
fn syntax_old_and_new_sides_are_separate() {
    // The removed line opens a string; the added line after it must not be inside it.
    let d = highlighted("a.py", &["-x = \"\"\"open", "+y = 1"]);
    assert_eq!(token_at(&d, 0, "open"), Some(Token::String));
    assert_eq!(token_at(&d, 1, "1"), Some(Token::Number));
}

#[test]
fn syntax_detection() {
    use super::syntax::syntax_for;
    assert!(syntax_for("notes.xyz", None).is_none());
    assert!(syntax_for("README", None).is_none());
    assert_eq!(syntax_for("Cargo.toml", None).map(|s| s.name.as_str()), Some("TOML"));
    assert!(syntax_for("web/app.tsx", None).is_some());
    assert!(syntax_for("Dockerfile", None).is_some());
    assert!(syntax_for("bin/run", Some("#!/bin/bash")).is_some());
    let d = highlighted("notes.xyz", &["+fn main() {}"]);
    assert!(d.hunks[0].lines[0].syntax.is_empty());
}

#[test]
fn syntax_reuses_unchanged_hunks() {
    let first = "@@ -1,1 +1,1 @@\n-let a = 1;\n+let a = 2;\n@@ -9,1 +9,1 @@\n-let b = 1;\n+let b = 2;\n";
    let mut prev = parse_diff(first);
    super::syntax::highlight_file(&mut prev, "x.rs", None);
    // Pretend the first hunk was colored differently last time: it must be copied, not re-read.
    prev.hunks[0].lines[0].syntax = vec![(0..3, Token::Comment)];
    let second = first.replace("let b = 2", "let b = 3");
    let mut next = parse_diff(&second);
    super::syntax::highlight_file(&mut next, "x.rs", Some(&prev));
    assert!(next.hunks.iter().all(|h| h.colored));
    assert_eq!(next.hunks[0].lines[0].syntax, vec![(0..3, Token::Comment)]);
    let line = &next.hunks[1].lines[1];
    assert!(line.syntax.iter().any(|(r, t)| *t == Token::Number && &line.text[r.clone()] == "3"));
    // A plain (not yet colored) previous diff is never copied from.
    let plain = parse_diff(first);
    let mut again = parse_diff(first);
    super::syntax::highlight_file(&mut again, "x.rs", Some(&plain));
    assert_eq!(token_at(&again, 0, "let"), Some(Token::Keyword));
}

#[test]
fn syntax_colorer_resumes_mid_hunk() {
    // One line per step: the parser state must carry over from one step to the next.
    let mut d = parse_diff("@@ -1,3 +1,4 @@\n /* start\n+   still a comment\n+*/\n+int x = 1;\n");
    let mut c = super::syntax::Colorer::new(&d, "a.c").unwrap();
    for i in 0..4 {
        assert!(!d.hunks[0].colored);
        assert_eq!(c.step(&mut d, 0, 1), i..i + 1);
    }
    assert!(d.hunks[0].colored);
    assert_eq!(c.step(&mut d, 0, 1), 0..0, "a colored hunk has nothing left");
    assert_eq!(token_at(&d, 1, "still"), Some(Token::Comment));
    assert_eq!(token_at(&d, 3, "1"), Some(Token::Number));
}

#[test]
fn syntax_colors_from_the_view_down_first() {
    let mut d = parse_diff("@@ -1,1 +1,1 @@\n+a\n@@ -5,1 +5,1 @@\n+b\n@@ -9,1 +9,1 @@\n+c\n@@ -13,1 +13,1 @@\n+d\n");
    let next = super::syntax::next;
    assert_eq!(next(&d, 2), Some(2));
    d.hunks[2].colored = true;
    assert_eq!(next(&d, 2), Some(3), "then the hunks below the view");
    d.hunks[3].colored = true;
    assert_eq!(next(&d, 2), Some(0), "then those above it");
    assert_eq!(next(&d, 99), Some(0), "a focus past the end is clamped");
    d.hunks[0].colored = true;
    d.hunks[1].colored = true;
    assert_eq!(next(&d, 2), None);
}

#[test]
fn big_diffs_show_first_and_color_in_chunks() {
    let Some(dir) = repo("stream") else { return };
    // Big enough that coloring it takes well over the first-paint budget.
    let body: String = (0..800).map(|i| format!("fn f{i}() -> u32 {{ \"s\".len() as u32 + {i} }} // c\n")).collect();
    std::fs::write(dir.join("big.rs"), body).unwrap();
    let ctx = egui::Context::default();
    let mut g = super::GitDiff::new(&ctx, dir.clone());
    let start = std::time::Instant::now();
    let mut saw_partial = false;
    loop {
        assert!(start.elapsed() < std::time::Duration::from_secs(60), "the diff never finished coloring");
        g.poll();
        if let Some((_, d)) = &g.diff
            && let Ok(d) = d.as_ref()
        {
            let colored = d.hunks[0].lines.iter().filter(|l| !l.syntax.is_empty()).count();
            saw_partial |= colored < d.hunks[0].lines.len();
            if g.diff_job.is_none() && d.hunks.iter().all(|h| h.colored) {
                let last = d.hunks[0].lines.last().unwrap();
                assert!(last.syntax.iter().any(|(_, t)| *t == Token::Comment));
                break;
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    assert!(saw_partial, "the diff showed before it was fully colored");
    let _ = std::fs::remove_dir_all(&dir);
}
