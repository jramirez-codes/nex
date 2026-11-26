//! Tests of the pipeline's parts that need no daemon: the file format and its checks
//! (`spec.rs`), the task templates, what joins a run (`files.rs`), and the examples that
//! ship with nex-host, which must all parse.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use nex_host::proto::Verdict;

use crate::pipeline::Outcome;
use crate::pipeline::files::joins;
use crate::pipeline::spec::{Next, Pipeline, after, fill, fill_each, parse_duration};

const FEATURE: &str = r#"
    worktree = true
    [[stage]]
    name = "plan"
    task = "Plan: {task}"
    [[stage]]
    name = "build"
    task = "Build it. Plan: {plan}"
    [[stage]]
    name = "tests"
    parallel = true
    task = "Test it."
    [[stage]]
    name = "review"
    kind = "codex"
    task = "Review. {build.verdict} {review}"
    retry = { stage = "build", max = 2 }
"#;

fn done(body: &str, verdict: Option<Verdict>) -> Outcome {
    Outcome { verdict, ..Outcome::text(body) }
}

#[test]
fn stages_run_in_groups() {
    let p = Pipeline::parse(FEATURE).unwrap();
    assert_eq!(p.groups(), vec![vec![0], vec![1, 2], vec![3]]);
    assert_eq!(p.stages[3].retry.as_ref().map(|r| (r.unless.as_str(), r.max)), Some(("approved", 2)));
}

#[test]
fn a_stage_can_have_a_worktree_of_its_own() {
    let p = Pipeline::parse("[[stage]]\nname='a'\ntask='x'\nworktree=true\n[[stage]]\nname='b'\ntask='y'\nparallel=true").unwrap();
    assert!(!p.worktree);
    assert_eq!(p.stages.iter().map(|s| s.worktree).collect::<Vec<_>>(), [true, false]);
}

#[test]
fn a_pipeline_ending_in_one_queue_can_be_joined() {
    let firehose = "[[stage]]\nname='build'\ntask='{task}'\nworktree=true\n[[stage]]\nname='merge'\ntask='{pr}'\neach=true\nworktree=true";
    assert!(Pipeline::parse(firehose).unwrap().joinable(), "the merger can have a worktree of its own");
    assert!(!Pipeline::parse(FEATURE).unwrap().joinable(), "no each stage");
    let two_steps = "[[stage]]\nname='plan'\ntask='x'\n[[stage]]\nname='build'\ntask='x'\n[[stage]]\nname='merge'\ntask='{pr}'\neach=true";
    assert!(!Pipeline::parse(two_steps).unwrap().joinable(), "a joined task would skip the plan");
}

#[test]
fn a_second_pipe_joins_the_run_going_in_its_repository() {
    use nex_host::proto::{Session, Status};
    let run = |id: &str, file: &str, cwd: &str, started: u64, exit: Option<i32>| Session {
        id: id.into(),
        kind: nex_host::proto::PIPELINE_KIND.into(),
        name: "feature".into(),
        cwd: cwd.into(),
        argv: vec!["nex-host".into(), "pipe-run".into(), file.into(), "a task".into()],
        pid: 1,
        started,
        title: None,
        bell: false,
        attached: false,
        last_output: 0,
        last_input: 0,
        exit,
        claude: None,
        running: None,
        status: Status::Idle,
        parent: None,
        role: None,
        last_message: 0,
        subagents: Vec::new(),
    };
    // Folders under /repo are one repository (its worktrees too); /other is another.
    let top = |d: &Path| if d.starts_with("/repo") || d.starts_with("/trees") { PathBuf::from("/repo/.git") } else { d.to_path_buf() };
    let file = Path::new("/cfg/feature.toml");
    let sessions = vec![
        run("old", "/cfg/feature.toml", "/repo", 1, None),
        run("new", "/cfg/feature.toml", "/repo", 2, None),
        run("ended", "/cfg/feature.toml", "/repo", 3, Some(0)),
        run("elsewhere", "/cfg/feature.toml", "/other", 4, None),
        run("another", "/cfg/smoke.toml", "/repo", 5, None),
    ];
    assert_eq!(joins(&sessions, file, Path::new("/trees/x"), &top).map(|s| s.id.as_str()), Some("new"), "the latest live one");
    assert_eq!(joins(&sessions, file, Path::new("/other"), &top).map(|s| s.id.as_str()), Some("elsewhere"));
    assert!(joins(&sessions, file, Path::new("/third"), &top).is_none(), "nothing going there: a run of its own");
    assert!(joins(&sessions, Path::new("/cfg/x.toml"), Path::new("/repo"), &top).is_none());
}

#[test]
fn an_each_stage_comes_last() {
    let ok = "[[stage]]\nname='a'\ntask='x'\n[[stage]]\nname='b'\ntask='y'\nparallel=true\n[[stage]]\nname='m'\ntask='{pr}'\neach=true";
    assert!(Pipeline::parse(ok).unwrap().stages[2].each);
    let e = |s: &str| Pipeline::parse(s).unwrap_err();
    assert!(e("[[stage]]\nname='m'\ntask='x'\neach=true").contains("each"), "not first");
    assert!(e("[[stage]]\nname='a'\ntask='x'\n[[stage]]\nname='m'\ntask='x'\neach=true\n[[stage]]\nname='c'\ntask='x'").contains("each"), "last");
    assert!(e("[[stage]]\nname='a'\ntask='x'\n[[stage]]\nname='m'\ntask='x'\neach=true\nparallel=true").contains("each"));
    assert!(e("[[stage]]\nname='pr'\ntask='x'").contains("can't be"));
}

#[test]
fn each_tasks_get_the_report() {
    let o = Outcome { files: vec!["a.rs".into()], pr: Some("7".into()), ..Outcome::text(" added --version \n") };
    assert_eq!(
        fill_each("PR {pr} from {from}: {report} [{files}] after {merged} {x}", "build-a", &o, "PR 5"),
        "PR 7 from build-a: added --version [a.rs] after PR 5 {x}"
    );
}

#[test]
fn bad_pipelines_say_why() {
    let e = |s: &str| Pipeline::parse(s).unwrap_err();
    assert!(e("worktree = true").contains("stage"));
    assert!(e("[[stage]]\nname='a'\ntask='x'\nparallel=true").contains("first stage"));
    assert!(e("[[stage]]\nname='a'\ntask='x'\n[[stage]]\nname='a'\ntask='y'").contains("unique"));
    assert!(e("[[stage]]\nname='task'\ntask='x'").contains("can't be"));
    assert!(e("[[stage]]\nname='a'\ntask='x'\nretry={stage='b'}\n[[stage]]\nname='b'\ntask='y'").contains("before it"));
    assert!(e("[[stage]]\nname='a b'\ntask='x'").contains("letters"));
    assert!(e("[[stage]]\nname='a'\ntask='x'\nbogus=1").contains("bogus"));
    assert!(e("timeout='soon'\n[[stage]]\nname='a'\ntask='x'").contains("timeout"));
}

#[test]
fn tasks_are_filled_from_reports() {
    let mut r = HashMap::new();
    r.insert(
        "plan".to_string(),
        Outcome {
            verdict: Some(Verdict::Approved),
            files: vec!["a.rs".into(), "b.rs".into()],
            pr: Some("42".into()),
            ..Outcome::text(" the plan \n")
        },
    );
    r.insert("docs".to_string(), done("d", None));
    assert_eq!(
        fill("Do {task}. Plan: {plan} ({plan.verdict}; {plan.files}; PR {plan.pr})", "X", &r),
        "Do X. Plan: the plan (approved; a.rs, b.rs; PR 42)"
    );
    assert_eq!(fill("{docs.pr}", "X", &r), "none");
    assert_eq!(fill("fn f() { {review} } {plan.bogus} {", "X", &r), "fn f() { {review} } {plan.bogus} {", "anything else is left alone");
}

#[test]
fn review_sends_it_back_until_approved_or_out_of_tries() {
    let p = Pipeline::parse(FEATURE).unwrap();
    let mut loops = HashMap::new();
    let mut r = HashMap::new();
    r.insert("plan".into(), done("p", None));
    assert_eq!(after(&p, 0, &r, &mut loops), Next::Run(1, None));
    r.insert("build".into(), done("b", None));
    r.insert("tests".into(), done("t", None));
    assert_eq!(after(&p, 1, &r, &mut loops), Next::Run(2, None));
    r.insert("review".into(), done("fix x", Some(Verdict::Changes)));
    assert_eq!(after(&p, 2, &r, &mut loops), Next::Run(1, Some("review".into())), "back to build (and tests, its group)");
    assert_eq!(after(&p, 2, &r, &mut loops), Next::Run(1, Some("review".into())));
    assert!(matches!(after(&p, 2, &r, &mut loops), Next::Failed(why) if why.contains("after 2 retries")));
    let mut loops = HashMap::new();
    r.insert("review".into(), done("lgtm", Some(Verdict::Approved)));
    assert_eq!(after(&p, 2, &r, &mut loops), Next::Finished);
}

#[test]
fn mockups_designers_run_together_and_the_critic_sends_them_back() {
    let p = Pipeline::parse(include_str!("../../examples/mockups/pipeline.toml")).unwrap();
    let names: Vec<_> = p.stages.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names, ["calm", "bold", "dense", "critic"]);
    assert_eq!(p.groups(), vec![vec![0, 1, 2], vec![3]]);
    let mut r = HashMap::new();
    for (name, file) in [("calm", "mockups/x-calm/index.html"), ("bold", "mockups/x-bold/index.html"), ("dense", "mockups/x-dense/index.html")] {
        r.insert(name.to_string(), Outcome { files: vec![file.into()], ..Outcome::text(name) });
    }
    let mut loops = HashMap::new();
    assert_eq!(after(&p, 0, &r, &mut loops), Next::Run(1, None));
    let task = fill(&p.stages[3].task, "a pricing page", &r);
    assert!(task.contains("a pricing page") && task.contains("mockups/x-bold/index.html") && task.contains("mockups/<slug>-review.md"), "{task}");
    r.insert("critic".into(), done("bold: fix contrast", Some(Verdict::Changes)));
    assert_eq!(after(&p, 1, &r, &mut loops), Next::Run(0, Some("critic".into())), "back to all three designers");
    r.insert("critic".into(), done("calm wins", Some(Verdict::Approved)));
    assert_eq!(after(&p, 1, &r, &mut loops), Next::Finished);
}

#[test]
fn a_failed_stage_stops_the_run() {
    let p = Pipeline::parse(FEATURE).unwrap();
    let mut r = HashMap::new();
    r.insert("plan".into(), done("can't", Some(Verdict::Failed)));
    assert_eq!(after(&p, 0, &r, &mut HashMap::new()), Next::Failed("plan failed".into()));
}

#[test]
fn durations() {
    assert_eq!(parse_duration("90m"), Some(Duration::from_secs(5400)));
    assert_eq!(parse_duration("2h"), Some(Duration::from_secs(7200)));
    assert_eq!(parse_duration("45"), Some(Duration::from_secs(45)));
    assert_eq!(parse_duration("500ms"), Some(Duration::from_millis(500)));
    assert_eq!(parse_duration("soon"), None);
}

#[test]
fn every_example_pipeline_parses() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let mut files: Vec<PathBuf> =
        std::fs::read_dir(root.join("examples")).unwrap().map(|e| e.unwrap().path().join("pipeline.toml")).filter(|p| p.exists()).collect();
    files.sort();
    let names: Vec<_> = files.iter().filter_map(|p| p.parent()?.file_name()?.to_str()).collect();
    assert!(names.contains(&"feature") && names.contains(&"pr-team") && names.contains(&"mockups"), "examples went missing: {names:?}");
    files.push(root.join("../tools/demo/agents/pipeline.toml"));
    for file in files {
        let text = std::fs::read_to_string(&file).unwrap_or_else(|e| panic!("{}: {e}", file.display()));
        if let Err(e) = Pipeline::parse(&text) {
            panic!("{}: {e}", file.display());
        }
    }
}
