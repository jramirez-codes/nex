//! Git worktrees: making one for `spawn --worktree` (and pipelines), and `worktree ls` /
//! `worktree prune`.

use std::io;
use std::path::{Path, PathBuf};

use nex_host::proto::Request;

use crate::agentcli::args::Args;
use crate::agentcli::bad;
use crate::client;

/// A new git worktree of the repository `dir` is in, on a branch of its own, for `name`.
pub fn worktree(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(dir).args(args).output();
    let top = git(&["rev-parse", "--show-toplevel"])?;
    if !top.status.success() {
        return Err(bad(format!("--worktree: {} isn't in a git repository", dir.display())));
    }
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    let repo = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    let trees = nex_host::paths::worktrees().ok_or_else(|| io::Error::other("no home folder"))?;
    // The second it's made in, plus `-2`, `-3`… when another was made that second (two
    // features joining a run at once): no two share a branch or a folder.
    let stamp = crate::util::now_ms() / 1000;
    let taken = |b: &str, p: &Path| p.exists() || git(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_ok_and(|o| o.status.success());
    let (branch, path) = (1..)
        .map(|n| if n == 1 { format!("{name}-{stamp}") } else { format!("{name}-{stamp}-{n}") })
        .map(|s| (format!("nex/{s}"), trees.join(format!("{repo}-{s}"))))
        .find(|(b, p)| !taken(b, p))
        .ok_or_else(|| io::Error::other("--worktree: no branch name is free"))?;
    let out = git(&["worktree", "add", "-b", &branch, &path.to_string_lossy()])?;
    if !out.status.success() {
        return Err(io::Error::other(format!("git worktree add: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(path)
}

/// What `worktree prune` does with one worktree.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum Fate {
    Keep(&'static str),
    Remove,
}

/// Remove a worktree only when no agent is in it and nothing in it would be lost: its
/// changes are committed, and its branch is merged (or gone upstream, as a squash-merged PR
/// with `--delete-branch` leaves it). `all` drops that last condition, never the others.
pub(super) fn fate(in_use: bool, dirty: bool, done: bool, all: bool) -> Fate {
    if in_use {
        Fate::Keep("an agent is working in it")
    } else if dirty {
        Fate::Keep("uncommitted changes")
    } else if !done && !all {
        Fate::Keep("branch not merged")
    } else {
        Fate::Remove
    }
}

/// One worktree under `~/.nex/worktrees`, as git sees it.
struct Tree {
    path: PathBuf,
    /// The repository's `.git` (shared by all its worktrees).
    common: Option<PathBuf>,
    branch: Option<String>,
    dirty: bool,
}

fn git_in(dir: &Path, args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("git").arg("-C").arg(dir).args(args).output().ok()?;
    out.status.success().then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn git_dir(common: &Path, args: &[&str]) -> io::Result<std::process::Output> {
    std::process::Command::new("git").arg("--git-dir").arg(common).args(args).output()
}

fn trees() -> Vec<Tree> {
    let Some(dir) = nex_host::paths::worktrees() else { return vec![] };
    let mut out: Vec<Tree> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter(|e| e.path().is_dir())
        .map(|e| {
            let path = e.path();
            Tree {
                common: git_in(&path, &["rev-parse", "--path-format=absolute", "--git-common-dir"]).map(PathBuf::from),
                branch: git_in(&path, &["symbolic-ref", "--short", "HEAD"]),
                dirty: git_in(&path, &["status", "--porcelain"]).is_none_or(|s| !s.is_empty()),
                path,
            }
        })
        .collect();
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

/// The branch work lands on: origin's default, else `main`/`master`, else what the
/// repository's own checkout is on.
fn base_branch(common: &Path) -> Option<String> {
    let ok = |args: &[&str]| git_dir(common, args).ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string());
    ok(&["symbolic-ref", "--short", "refs/remotes/origin/HEAD"])
        .or_else(|| ["main", "master"].into_iter().find(|b| ok(&["rev-parse", "--verify", "--quiet", &format!("refs/heads/{b}")]).is_some()).map(Into::into))
        .or_else(|| common.parent().and_then(|top| git_in(top, &["symbolic-ref", "--short", "HEAD"])))
}

/// Its branch has nothing that isn't on the base branch, or its upstream was deleted.
fn branch_done(t: &Tree) -> bool {
    let (Some(common), Some(branch)) = (&t.common, &t.branch) else { return false };
    let merged = base_branch(common).is_some_and(|base| git_dir(common, &["merge-base", "--is-ancestor", branch, &base]).is_ok_and(|o| o.status.success()));
    let gone = git_dir(common, &["for-each-ref", "--format=%(upstream:track)", &format!("refs/heads/{branch}")])
        .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "[gone]");
    merged || gone
}

/// Folders live agents are working in.
fn busy_dirs() -> Vec<PathBuf> {
    client::request(&Request::List, false).ok().and_then(|r| r.sessions).unwrap_or_default().into_iter().filter(|s| s.exit.is_none()).map(|s| PathBuf::from(s.cwd)).collect()
}

/// `worktree ls`: the worktrees `--worktree` made. `worktree prune [--all] [--dry-run]`:
/// remove the ones [`fate`] says can go, and their branches.
pub fn worktrees(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &[], &["--all", "--dry-run"])?;
    let prune = match a.words.first().map(String::as_str) {
        None | Some("ls") | Some("list") => false,
        Some("prune") => true,
        _ => return Err(bad("usage: nex-host worktree ls | prune [--all] [--dry-run]")),
    };
    let found = trees();
    if found.is_empty() {
        println!("No worktrees.");
        return Ok(0);
    }
    if prune && !a.flag("--dry-run") {
        // So branches deleted upstream (merged PRs) show as gone. Once per repository.
        let mut fetched: Vec<&Path> = Vec::new();
        for c in found.iter().filter_map(|t| t.common.as_deref()) {
            if !fetched.contains(&c) {
                let _ = git_dir(c, &["fetch", "--prune", "--quiet"]);
                fetched.push(c);
            }
        }
    }
    let busy = busy_dirs();
    let mut removed = 0;
    for t in &found {
        let in_use = busy.iter().any(|b| b.starts_with(&t.path));
        let done = branch_done(t);
        let shown = nex_host::paths::tilde(&t.path);
        let branch = t.branch.as_deref().unwrap_or("(no branch)");
        let f = fate(in_use, t.dirty, done, a.flag("--all"));
        if !prune {
            let mut state = vec![];
            if in_use {
                state.push("in use");
            }
            if t.dirty {
                state.push("uncommitted changes");
            }
            state.push(if done { "merged" } else { "not merged" });
            println!("{shown}  {branch}  ({})", state.join(", "));
            continue;
        }
        match f {
            Fate::Keep(why) => println!("kept     {shown}  {branch}: {why}"),
            Fate::Remove if a.flag("--dry-run") => println!("would go {shown}  {branch}"),
            Fate::Remove => {
                let Some(common) = &t.common else {
                    std::fs::remove_dir_all(&t.path)?;
                    println!("removed  {shown}  (not a worktree any more)");
                    removed += 1;
                    continue;
                };
                let out = git_dir(common, &["worktree", "remove", &t.path.to_string_lossy()])?;
                if !out.status.success() {
                    println!("kept     {shown}: git worktree remove: {}", String::from_utf8_lossy(&out.stderr).trim());
                    continue;
                }
                if let Some(b) = &t.branch {
                    let _ = git_dir(common, &["branch", "-D", b]);
                }
                println!("removed  {shown}  {branch}");
                removed += 1;
            }
        }
    }
    if prune && !a.flag("--dry-run") {
        println!("{removed} removed.");
    }
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prune_never_loses_work() {
        assert_eq!(fate(false, false, true, false), Fate::Remove);
        assert_eq!(fate(false, false, false, false), Fate::Keep("branch not merged"));
        assert_eq!(fate(false, false, false, true), Fate::Remove, "--all");
        assert_eq!(fate(false, true, true, true), Fate::Keep("uncommitted changes"));
        assert_eq!(fate(true, false, true, true), Fate::Keep("an agent is working in it"));
    }
}
