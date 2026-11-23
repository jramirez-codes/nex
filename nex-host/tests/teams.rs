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
        assert_eq!(m.payload.pr.as_deref(), Some(*pr));
        assert!(m.from_name.starts_with("build-"));
    }

    // And each one's report, with its PR, went to whoever gave it the task: us.
    for (id, pr) in &builders {
        let out = h.run(&["recv", "--from", id, "--kind", "done", "--timeout", "10s", "--json"]);
        let m: Message = serde_json::from_str(&out).unwrap();
        assert_eq!((m.kind, m.payload.pr.as_deref()), (MessageKind::Done, Some(*pr)));
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
fn worktrees_made_in_the_same_second_get_their_own_branches() {
    // Two features joining a run at once used to collide on the branch name, which ended
    // the run and left every PR after it unreviewed.
    let h = Host::new();
    let repo = h.repo();
    let cwd = repo.to_string_lossy().into_owned();
    let dirs: Vec<_> = (0..3).map(|_| h.spawn(&["build", "x", "--worktree", "--cwd", &cwd, "--", "sleep", "60"]).1).collect();
    assert!(dirs[0] != dirs[1] && dirs[1] != dirs[2] && dirs[0] != dirs[2], "{dirs:?}");
    let branches: std::collections::HashSet<_> = dirs.iter().map(|d| git(d, &["symbolic-ref", "--short", "HEAD"]).trim().to_string()).collect();
    assert_eq!(branches.len(), 3, "{branches:?}");
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

/// A merger standing in for Claude: one agent for the whole run, taking one task after
/// another. It sends back anything that's a first try and approves the rest.
const MERGER: &str = r#"while read -r l; do t="$t $l"; case "$l" in *'</nex-message>'*) case "$t" in *'first try'*) nex-host done 'conflicts with main' --verdict changes;; *) nex-host done merged --verdict approved;; esac; t='';; esac; done"#;

#[test]
fn an_each_stage_takes_reports_first_come_first_served() {
    let h = Host::new();
    let pipes = h.home.join(".config/nex/pipelines");
    std::fs::create_dir_all(&pipes).unwrap();
    // Builders report in the order b, c, a. b's first try gets sent back (a "conflict"),
    // so it reports again, and is merged last. Then a reports a second PR, which is
    // reviewed too: every report gets in line, not only each builder's first.
    std::fs::write(
        pipes.join("prs.toml"),
        format!(
            r#"
            kind = "sh"
            [[stage]]
            name = "build-a"
            task = "build a"
            command = ["sh", "-c", "sleep 3; nex-host done 'built a' --pr 101; sleep 3; nex-host done 'a follow-up' --pr 104; sleep 60"]
            [[stage]]
            name = "build-b"
            parallel = true
            task = "build b"
            command = ["sh", "-c", "sleep 1; nex-host done 'first try' --pr 102; read -r x; read -r t; nex-host done 'rebased' --pr 102; sleep 60"]
            [[stage]]
            name = "build-c"
            parallel = true
            task = "build c"
            command = ["sh", "-c", "sleep 2; nex-host done 'built c' --pr '#103'; sleep 60"]
            [[stage]]
            name = "merge"
            each = true
            task = "Merge PR {{pr}} from {{from}} ({{report}}). Merged before it: {{merged}}."
            command = ["sh", "-c", {MERGER:?}]
            "#
        ),
    )
    .unwrap();
    let id = h.run(&["pipe", "prs", "go", "--bg"]).trim().to_string();

    let screen = || h.run(&["read", &id, "--lines", "60"]);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(60);
    while !screen().contains("merge approved 4") {
        assert!(std::time::Instant::now() < deadline, "the run never finished:\n{}", screen());
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let out = screen();
    assert!(out.contains("↺ build-b (PR 102) sent back (1/3): conflicts with main"), "{out}");
    assert!(out.contains("merge approved 4: build-c (PR 103), build-a (PR 101), build-b (PR 102), build-a (PR 104)"), "{out}");
    // One merger for the whole line, under the run like every stage.
    let mergers: Vec<_> = h.sessions().into_iter().filter(|s| s.name.starts_with("merge")).collect();
    assert_eq!(mergers.len(), 1, "{mergers:#?}");
    assert!(h.sessions().iter().filter(|s| s.id != id).all(|s| s.parent.as_deref() == Some(id.as_str())));
}

#[test]
fn another_pipe_of_a_pipeline_joins_the_run_going() {
    let h = Host::new();
    let pipes = h.home.join(".config/nex/pipelines");
    std::fs::create_dir_all(&pipes).unwrap();
    std::fs::write(
        pipes.join("feature.toml"),
        format!(
            r#"
            kind = "sh"
            [[stage]]
            name = "build"
            task = "build {{task}}"
            command = ["sh", "-c", "read -r t; case \"$t\" in *'build one'*) f=one;; *) f=two;; esac; sleep 1; nex-host done \"built $f\" --pr \"build $f\"; sleep 60"]
            [[stage]]
            name = "merge"
            each = true
            task = "Merge PR {{pr}}: {{report}}"
            command = ["sh", "-c", {MERGER:?}]
            "#
        ),
    )
    .unwrap();
    let first = h.run(&["pipe", "feature", "one", "--bg"]).trim().to_string();
    let screen = || h.run(&["read", &first, "--lines", "60"]);
    eventually("the first feature merged", || screen().contains("merge approved 1: build (PR build one)"));

    // A second feature in the same folder joins that run: one group, one merger.
    let second = h.run(&["pipe", "feature", "two", "--bg"]).trim().to_string();
    assert_eq!(second, first);
    eventually("the second feature merged", || screen().contains("merge approved 1: build-2 (PR build two)"));
    let sessions = h.sessions();
    assert_eq!(sessions.iter().filter(|s| s.kind == "pipeline").count(), 1, "{sessions:#?}");
    let mut names: Vec<&str> = sessions.iter().filter(|s| s.parent.as_deref() == Some(first.as_str())).map(|s| s.name.as_str()).collect();
    names.sort();
    assert_eq!(names, ["build", "build-2", "merge"]);
}

#[test]
fn a_pipe_from_an_agent_shows_under_it_and_outlives_it() {
    let h = Host::new();
    let pipes = h.home.join(".config/nex/pipelines");
    std::fs::create_dir_all(&pipes).unwrap();
    std::fs::write(
        pipes.join("feature.toml"),
        format!(
            r#"
            kind = "sh"
            [[stage]]
            name = "build"
            task = "build {{task}}"
            command = ["sh", "-c", "read -r t; sleep 1; nex-host done built --pr 1; sleep 60"]
            [[stage]]
            name = "merge"
            each = true
            task = "Merge PR {{pr}}: {{report}}"
            command = ["sh", "-c", {MERGER:?}]
            "#
        ),
    )
    .unwrap();
    let asker = h.launch(&["sleep", "60"]);
    let pipe = |task: &str| {
        let out = h.cmd(&["pipe", "feature", task, "--bg"]).env("NEX_AGENT_ID", &asker).output().unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap().trim().to_string()
    };
    let run = pipe("one");
    assert_eq!(h.session(&run).unwrap().parent.as_deref(), Some(asker.as_str()), "under the agent that asked");
    assert_eq!(pipe("two"), run, "the second joins the same run");
    eventually("both builders and the merger under the run", || h.sessions().iter().filter(|s| s.parent.as_deref() == Some(run.as_str())).count() == 3);

    // The asker ending doesn't end the run it asked for.
    h.run(&["kill", &asker]);
    eventually("the asker ended", || h.session(&asker).is_none_or(|s| s.exit.is_some()));
    std::thread::sleep(std::time::Duration::from_millis(500));
    assert!(h.session(&run).is_some_and(|s| s.exit.is_none()), "the pipeline is still running");
}
