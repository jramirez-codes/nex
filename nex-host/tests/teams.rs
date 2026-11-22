//! Agents working together end to end, with shell scripts standing in for Claude: three
//! builders each open a "PR" and tell a merger about it, worktrees keep them apart, and
//! `worktree prune` cleans up only what can't lose work.
#![cfg(unix)]

mod common;

use std::path::{Path, PathBuf};
use std::process::Command;

use common::{eventually, Host};
use nex_host::proto::{Message, MessageKind};

impl Host {
    /// `spawn` with `--json`: its id and the folder it works in.
    fn spawn(&self, args: &[&str]) -> (String, PathBuf) {
        let mut all = vec!["spawn", "--json"];
        all.extend(args);
        let v: serde_json::Value = serde_json::from_str(&self.run(&all)).unwrap();
        (v["id"].as_str().unwrap().to_string(), PathBuf::from(v["cwd"].as_str().unwrap()))
    }

    /// The messages in `id`'s mailbox record.
    fn mail(&self, id: &str) -> Vec<Message> {
        std::fs::read_to_string(self.home.join(format!(".nex/run/mail/{id}.jsonl")))
            .unwrap_or_default()
            .lines()
            .map(|l| serde_json::from_str(l).unwrap())
            .collect()
    }

    /// A git repository with one commit on `main`.
    fn repo(&self) -> PathBuf {
        let dir = self.home.join("repo");
        std::fs::create_dir_all(&dir).unwrap();
        git(&dir, &["init", "-q", "-b", "main"]);
        std::fs::write(dir.join("README"), "hi\n").unwrap();
        git(&dir, &["add", "."]);
        git(&dir, &["commit", "-q", "-m", "first"]);
        dir
    }
}

fn git(dir: &Path, args: &[&str]) -> String {
    let out = Command::new("git").arg("-C").arg(dir).args(["-c", "user.name=t", "-c", "user.email=t@t"]).args(args).output().unwrap();
    assert!(out.status.success(), "git {args:?}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn builders_report_their_prs_to_a_merger() {
    let h = Host::new();
    // `cat` stands in for the merger: what's typed into it just sits there.
    let (merger, _) = h.spawn(&["merger", "review and merge what comes in", "--", "cat"]);
    let mut builders = Vec::new();
    for (name, pr) in [("build-a", "101"), ("build-b", "102"), ("build-c", "#103")] {
        let script = format!("nex-host send merger 'PR ready: {name}' --pr '{pr}' && nex-host done 'opened a PR' --pr '{pr}'; sleep 30");
        let (id, _) = h.spawn(&[name, "build it", "--", "sh", "-c", &script]);
        builders.push((id, pr.trim_start_matches('#')));
    }

    // Each builder's message reached the merger, PR and all.
    eventually("three PR messages for the merger", || h.mail(&merger).iter().filter(|m| m.kind == MessageKind::Note).count() == 3);
    let notes: Vec<Message> = h.mail(&merger).into_iter().filter(|m| m.kind == MessageKind::Note).collect();
    for (id, pr) in &builders {
        let m = notes.iter().find(|m| m.from == *id).unwrap_or_else(|| panic!("nothing from {id}: {notes:#?}"));
        assert_eq!(m.pr.as_deref(), Some(*pr));
        assert!(m.from_name.starts_with("build-"));
    }

    // And each one's report, with its PR, went to whoever gave it the task: us.
    for (id, pr) in &builders {
        let out = h.run(&["recv", "--from", id, "--kind", "done", "--timeout", "10s", "--json"]);
        let m: Message = serde_json::from_str(&out).unwrap();
        assert_eq!((m.kind, m.pr.as_deref()), (MessageKind::Done, Some(*pr)));
    }
}

#[test]
fn parallel_pipeline_stages_get_worktrees_of_their_own() {
    let h = Host::new();
    let repo = h.repo();
    let pipes = h.home.join(".config/nex/pipelines");
    std::fs::create_dir_all(&pipes).unwrap();
    std::fs::write(
        pipes.join("team.toml"),
        "kind = \"cat\"\n\
         [[stage]]\nname = \"a\"\ntask = \"build a\"\nworktree = true\n\
         [[stage]]\nname = \"b\"\ntask = \"build b\"\nworktree = true\nparallel = true\n\
         [[stage]]\nname = \"merge\"\ntask = \"merge them\"\nparallel = true\n",
    )
    .unwrap();
    let out = h.cmd(&["pipe", "team", "go", "--bg"]).current_dir(&repo).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));

    let stage = |name: &str| h.sessions().into_iter().find(|s| s.name == name && s.kind == "cat");
    eventually("the three stages", || ["a", "b", "merge"].iter().all(|n| stage(n).is_some()));
    let (a, b, merge) = (stage("a").unwrap().cwd, stage("b").unwrap().cwd, stage("merge").unwrap().cwd);
    let trees = h.home.join(".nex/worktrees");
    assert!(Path::new(&a).starts_with(&trees) && Path::new(&b).starts_with(&trees), "{a} {b}");
    assert_ne!(a, b);
    assert_eq!(Path::new(&merge), repo.canonicalize().unwrap(), "a stage without worktree works in the run's folder");
    let branches = git(&repo, &["branch", "--list", "nex/*"]);
    assert!(branches.contains("nex/team-a-") && branches.contains("nex/team-b-"), "{branches}");
}

#[test]
fn prune_removes_only_worktrees_that_cant_lose_work() {
    let h = Host::new();
    let repo = h.repo();
    let cwd = repo.to_string_lossy().into_owned();
    let (merged, merged_dir) = h.spawn(&["merged", "x", "--worktree", "--cwd", &cwd, "--", "sleep", "60"]);
    let (ahead, ahead_dir) = h.spawn(&["ahead", "x", "--worktree", "--cwd", &cwd, "--", "sleep", "60"]);
    let (dirty, dirty_dir) = h.spawn(&["dirty", "x", "--worktree", "--cwd", &cwd, "--", "sleep", "60"]);

    // `merged`: its commit lands on main. `ahead`: a commit main doesn't have. `dirty`: an
    // uncommitted file.
    std::fs::write(merged_dir.join("a.txt"), "a\n").unwrap();
    git(&merged_dir, &["add", "."]);
    git(&merged_dir, &["commit", "-q", "-m", "a"]);
    let merged_branch = git(&merged_dir, &["symbolic-ref", "--short", "HEAD"]).trim().to_string();
    git(&repo, &["merge", "-q", "--ff-only", &merged_branch]);
    std::fs::write(ahead_dir.join("b.txt"), "b\n").unwrap();
    git(&ahead_dir, &["add", "."]);
    git(&ahead_dir, &["commit", "-q", "-m", "b"]);
    std::fs::write(dirty_dir.join("c.txt"), "c\n").unwrap();

    // While their agents run, nothing goes.
    let out = h.run(&["worktree", "prune"]);
    assert!(out.contains("0 removed") && out.matches("an agent is working in it").count() == 3, "{out}");

    for id in [&merged, &ahead, &dirty] {
        h.run(&["kill", id]);
    }
    eventually("agents gone", || h.sessions().iter().all(|s| s.exit.is_some()));
    let dry = h.run(&["worktree", "prune", "--dry-run"]);
    assert!(dry.contains("would go") && merged_dir.exists(), "{dry}");

    let out = h.run(&["worktree", "prune"]);
    assert!(out.contains("1 removed"), "{out}");
    assert!(!merged_dir.exists(), "the merged one is gone");
    assert!(!git(&repo, &["branch", "--list", &merged_branch]).contains(&merged_branch), "and its branch");
    assert!(ahead_dir.exists() && out.contains("branch not merged"), "{out}");
    assert!(dirty_dir.exists() && out.contains("uncommitted changes"), "{out}");

    // --all takes unmerged work too, but never uncommitted changes.
    h.run(&["worktree", "prune", "--all"]);
    assert!(!ahead_dir.exists() && dirty_dir.exists());
}
