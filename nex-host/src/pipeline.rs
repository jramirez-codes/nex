//! Pipelines: a list of stages, each an agent given a task, run with the same calls an
//! agent delegating by hand makes (`spawn`, `send`, `recv`). There's no engine of its own:
//! a stage is "spawn (or send a new task to) this agent, then take its `done`".
//!
//! ```toml
//! # .nex/pipelines/feature.toml
//! worktree = true                       # one git worktree for the run, shared by the stages
//!                                       # (or `worktree = true` on a stage: one of its own)
//! [[stage]]
//! name = "plan"
//! task = "Plan this: {task}. Write PLAN.md."
//! [[stage]]
//! name = "build"
//! task = "Implement PLAN.md. The plan in short: {plan}"
//! [[stage]]
//! name = "tests"
//! parallel = true                       # runs alongside the stage before it
//! task = "Write tests for what PLAN.md describes."
//! [[stage]]
//! name = "review"
//! kind = "codex"
//! task = "Review the diff against PLAN.md. Finish with done --verdict approved|changes."
//! retry = { stage = "build", unless = "approved", max = 3 }
//! ```
//!
//! A last stage with `each = true` runs once per report from the stages before it, in the
//! order they arrive (first done, first served), one at a time, a fresh agent each time:
//!
//! ```toml
//! [[stage]]
//! name = "build-a"
//! worktree = true
//! task = "Build A, open a PR, then: nex-host done \"<what you did>\" --pr <number>"
//! [[stage]]
//! name = "build-b"
//! parallel = true
//! worktree = true
//! task = "…"
//! [[stage]]
//! name = "merge"
//! each = true
//! task = "Review PR {pr} from {from} ({report}); merged before it: {merged}. Merge it and report approved, or report changes saying what to fix."
//! ```
//!
//! An `each` stage's task also has `{from}`, `{report}`, `{pr}`, `{files}` and `{merged}`
//! (what it approved so far). When it reports `changes`, that goes back to the stage that
//! sent the report as a new task (it's still running, so it keeps its context), and its
//! next report joins the back of the line; up to three times.
//!
//! In a task, `{task}` is what the pipeline was run with, `{<stage>}` that stage's report,
//! `{<stage>.verdict}`, `{<stage>.files}` and `{<stage>.pr}` its fields. When `retry` sends the run back,
//! the stages from there on get their task again with the report that sent it back, and
//! a stage whose agent is still running gets it as a new message, so it keeps what it
//! learned the first time.
//!
//! `pipe` runs the runner as a nex-host session of its own (kind `pipeline`), so it keeps
//! going when the terminal closes and its stages show up in nex's Agents pane as its
//! children.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::Deserialize;

use nex_host::proto::{MessageKind, Request, Verdict};

use crate::client;

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Pipeline {
    /// The agent stages run when they don't say (default `claude`).
    #[serde(default)]
    pub kind: Option<String>,
    /// Run in a git worktree of its own.
    #[serde(default)]
    pub worktree: bool,
    /// How long a stage may take, like `90m` (default 2h).
    #[serde(default)]
    pub timeout: Option<String>,
    #[serde(rename = "stage")]
    pub stages: Vec<Stage>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Stage {
    pub name: String,
    pub task: String,
    #[serde(default)]
    pub kind: Option<String>,
    #[serde(default)]
    pub role: Option<String>,
    /// Runs at the same time as the stage before it.
    #[serde(default)]
    pub parallel: bool,
    /// The program to run instead of `kind`'s (the task is typed into it once it's up).
    #[serde(default)]
    pub command: Option<Vec<String>>,
    /// Last stage only: run once per report from the stages before it, first come first
    /// served, a fresh agent each time.
    #[serde(default)]
    pub each: bool,
    /// Works in a git worktree (and branch) of its own, so stages running at the same time
    /// don't edit one checkout. Made from the run's folder, and kept across retries.
    #[serde(default)]
    pub worktree: bool,
    #[serde(default)]
    pub retry: Option<Retry>,
}

/// Go back to `stage` unless this stage's verdict is `unless`, at most `max` times.
#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Retry {
    pub stage: String,
    #[serde(default = "approved")]
    pub unless: String,
    #[serde(default = "three")]
    pub max: u32,
}

/// What an `each` stage's task can use, so no stage can be called that.
const RESERVED: [&str; 6] = ["task", "from", "report", "pr", "files", "merged"];

/// How many times an `each` stage can send one report back.
const SEND_BACKS: u32 = 3;

fn approved() -> String {
    Verdict::Approved.label().into()
}

fn three() -> u32 {
    3
}

/// What a stage reported.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub body: String,
    pub verdict: Option<Verdict>,
    pub files: Vec<String>,
    pub pr: Option<String>,
}

impl Pipeline {
    pub fn parse(text: &str) -> Result<Pipeline, String> {
        let p: Pipeline = toml::from_str(text).map_err(|e| e.to_string())?;
        p.check()?;
        Ok(p)
    }

    fn check(&self) -> Result<(), String> {
        if self.stages.is_empty() {
            return Err("no stages ([[stage]])".into());
        }
        if self.stages[0].parallel {
            return Err(format!("{}: the first stage has nothing to run alongside", self.stages[0].name));
        }
        if self.timeout.as_deref().is_some_and(|t| parse_duration(t).is_none()) {
            return Err("timeout: like 90m, 2h or 600s".into());
        }
        let mut seen: Vec<&str> = Vec::new();
        for s in &self.stages {
            if s.name.is_empty() || !s.name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
                return Err(format!("`{}`: stage names are letters, digits, - and _", s.name));
            }
            if seen.contains(&s.name.as_str()) {
                return Err(format!("{}: stage names must be unique", s.name));
            }
            if RESERVED.contains(&s.name.as_str()) {
                return Err(format!("{}: stage names can't be {}", s.name, RESERVED.join(", ")));
            }
            seen.push(&s.name);
            if s.each {
                let last = std::ptr::eq(s, self.stages.last().unwrap());
                if !last || s.parallel || seen.len() == 1 || s.retry.is_some() || s.worktree {
                    return Err(format!("{}: an `each` stage is the last one, not parallel, after other stages, without retry or worktree", s.name));
                }
            }
            if let Some(r) = &s.retry {
                if !seen.contains(&r.stage.as_str()) {
                    return Err(format!("{}: retry goes back to `{}`, which isn't this stage or one before it", s.name, r.stage));
                }
                if Verdict::parse(&r.unless).is_none() {
                    return Err(format!("{}: retry.unless is approved, changes or failed", s.name));
                }
            }
        }
        Ok(())
    }

    /// The stages in the order they run: each group runs at once.
    pub fn groups(&self) -> Vec<Vec<usize>> {
        let mut groups: Vec<Vec<usize>> = Vec::new();
        for (i, s) in self.stages.iter().enumerate() {
            match groups.last_mut() {
                Some(g) if s.parallel => g.push(i),
                _ => groups.push(vec![i]),
            }
        }
        groups
    }

    fn group_of(&self, stage: &str) -> Option<usize> {
        let i = self.stages.iter().position(|s| s.name == stage)?;
        self.groups().iter().position(|g| g.contains(&i))
    }

    pub fn stage_timeout(&self) -> Duration {
        self.timeout.as_deref().and_then(parse_duration).unwrap_or(Duration::from_secs(2 * 3600))
    }
}

/// `template` with `{task}`, `{stage}`, `{stage.verdict}`, `{stage.files}` and `{stage.pr}`
/// filled in.
/// Anything else in braces stays as it is (code has braces too).
pub fn fill(template: &str, task: &str, results: &HashMap<String, Outcome>) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(open) = rest.find('{') {
        out.push_str(&rest[..open]);
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else {
            out.push_str(&rest[open..]);
            return out;
        };
        let key = &after[..close];
        let (name, field) = key.split_once('.').unwrap_or((key, ""));
        let value = match (name, field) {
            ("task", "") => Some(task.to_string()),
            _ => results.get(name).and_then(|o| match field {
                "" => Some(o.body.trim().to_string()),
                "verdict" => Some(o.verdict.map_or("none", Verdict::label).to_string()),
                "files" => Some(o.files.join(", ")),
                "pr" => Some(o.pr.clone().unwrap_or_else(|| "none".into())),
                _ => None,
            }),
        };
        match value {
            Some(v) => out.push_str(&v),
            None => {
                out.push('{');
                out.push_str(key);
                out.push('}');
            }
        }
        rest = &after[close + 1..];
    }
    out.push_str(rest);
    out
}

/// What to do after a group of stages has reported.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Run this group, and give these stages the report that sent the run back, if any.
    Run(usize, Option<String>),
    Finished,
    Failed(String),
}

/// Where the run goes after group `g`: back where a `retry` says (while it has tries
/// left), on to the next group, or stopped by a stage that failed.
pub fn after(p: &Pipeline, g: usize, results: &HashMap<String, Outcome>, loops: &mut HashMap<String, u32>) -> Next {
    let groups = p.groups();
    for &i in &groups[g] {
        let s = &p.stages[i];
        let verdict = results.get(&s.name).and_then(|o| o.verdict);
        if let Some(r) = &s.retry {
            let ok = verdict == Verdict::parse(&r.unless);
            if !ok {
                let n = loops.entry(s.name.clone()).or_insert(0);
                if *n >= r.max {
                    return Next::Failed(format!("{} still says {} after {} retries", s.name, verdict.map_or("nothing", Verdict::label), r.max));
                }
                *n += 1;
                let back = p.group_of(&r.stage).unwrap_or(g);
                return Next::Run(back, Some(s.name.clone()));
            }
        } else if verdict == Some(Verdict::Failed) {
            return Next::Failed(format!("{} failed", s.name));
        }
    }
    if g + 1 < groups.len() { Next::Run(g + 1, None) } else { Next::Finished }
}

/// `90m`, `2h`, `30s`, `500ms`, or seconds.
pub fn parse_duration(s: &str) -> Option<Duration> {
    let s = s.trim();
    let split = s.find(|c: char| !c.is_ascii_digit()).unwrap_or(s.len());
    let (n, unit) = s.split_at(split);
    let n: u64 = n.parse().ok()?;
    Some(match unit.trim() {
        "" | "s" => Duration::from_secs(n),
        "ms" => Duration::from_millis(n),
        "m" => Duration::from_secs(n * 60),
        "h" => Duration::from_secs(n * 3600),
        _ => return None,
    })
}

/// The pipeline file `name` means: a path, else `.nex/pipelines/<name>.toml` in this folder
/// or one above it, else `~/.config/nex/pipelines/<name>.toml`.
pub fn find(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_file() {
        return std::fs::canonicalize(p).ok();
    }
    let file = format!("{name}.toml");
    let cwd = std::env::current_dir().ok()?;
    for dir in cwd.ancestors() {
        let candidate = dir.join(".nex/pipelines").join(&file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?;
    let candidate = config.join("nex/pipelines").join(&file);
    candidate.is_file().then_some(candidate)
}

/// Every pipeline `find` can see, by name.
pub fn list() -> Vec<(String, PathBuf)> {
    let mut dirs: Vec<PathBuf> = std::env::current_dir().map(|c| c.ancestors().map(|d| d.join(".nex/pipelines")).collect()).unwrap_or_default();
    if let Some(config) = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config"))) {
        dirs.push(config.join("nex/pipelines"));
    }
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "toml")
                && let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned())
                && !out.iter().any(|(n, _)| *n == stem)
            {
                out.push((stem, path));
            }
        }
    }
    out.sort();
    out
}

/// Run the pipeline in `file` on `task`, here: what `pipe` starts in a session of its own.
pub fn run(file: &Path, task: &str) -> io::Result<i32> {
    let text = std::fs::read_to_string(file)?;
    let p = Pipeline::parse(&text).map_err(|e| io::Error::other(format!("{}: {e}", file.display())))?;
    let me = std::env::var("NEX_AGENT_ID").ok().filter(|s| !s.is_empty());
    let mut cwd = std::env::current_dir()?;
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "pipeline".into());
    if p.worktree {
        cwd = crate::agentcli::worktree(&cwd, &stem)?;
        println!("worktree: {}", nex_host::paths::tilde(&cwd));
    }
    let env: Vec<(String, String)> = std::env::vars().collect();
    let timeout = p.stage_timeout();
    let groups = p.groups();
    let mut agents: HashMap<String, String> = HashMap::new();
    let mut results: HashMap<String, Outcome> = HashMap::new();
    let mut loops: HashMap<String, u32> = HashMap::new();
    let mut trees: HashMap<String, PathBuf> = HashMap::new();
    let mut step = Next::Run(0, None);
    loop {
        let (g, sent_back_by) = match step {
            Next::Run(g, by) => (g, by),
            Next::Finished => break,
            Next::Failed(why) => {
                println!("\n✗ stopped: {why}");
                report(me.as_deref(), &format!("The pipeline stopped: {why}"), Some(Verdict::Failed));
                return Ok(1);
            }
        };
        // Start (or re-task) every stage of the group, then take each one's report.
        let mut running = Vec::new();
        for &i in &groups[g] {
            let s = &p.stages[i];
            let mut body = fill(&s.task, task, &results);
            if let Some(by) = &sent_back_by
                && let Some(o) = results.get(by)
            {
                body.push_str(&format!("\n\nThis is another go: {by} sent it back ({}). What it said:\n{}", o.verdict.map_or("no verdict", Verdict::label), o.body.trim()));
            }
            let kind = s.kind.clone().or_else(|| p.kind.clone());
            let id = match agents.get(&s.name) {
                Some(id) if alive(id) => {
                    client::request(
                        &Request::Send { from: me.clone(), to: id.clone(), kind: MessageKind::Task, body, reply_to: None, verdict: None, files: vec![], pr: None, force: false },
                        true,
                    )?;
                    id.clone()
                }
                _ => {
                    let dir = match (s.worktree, trees.get(&s.name)) {
                        (false, _) => cwd.clone(),
                        (true, Some(t)) => t.clone(),
                        (true, None) => {
                            let t = crate::agentcli::worktree(&cwd, &format!("{stem}-{}", s.name))?;
                            println!("worktree ({}): {}", s.name, nex_host::paths::tilde(&t));
                            trees.insert(s.name.clone(), t.clone());
                            t
                        }
                    };
                    let r = client::request(
                        &Request::Spawn {
                            name: s.name.clone(),
                            kind: kind.clone(),
                            argv: s.command.clone().unwrap_or_default(),
                            cwd: Some(dir.to_string_lossy().into_owned()),
                            env: Some(env.clone()),
                            parent: me.clone(),
                            role: Some(s.role.clone().unwrap_or_else(|| s.name.clone())),
                            task: body,
                        },
                        true,
                    )?;
                    r.id.unwrap_or_default()
                }
            };
            println!("▸ {} ({}) is on it: {id}", s.name, kind.as_deref().unwrap_or("claude"));
            agents.insert(s.name.clone(), id.clone());
            running.push((s.name.clone(), id));
        }
        if let Some(each) = groups.get(g + 1).map(|n| &p.stages[n[0]]).filter(|s| s.each) {
            let (merged, failed) = run_each(&p, each, task, &results, &cwd, &env, me.as_deref(), running)?;
            let summary = format!(
                "{each_name} approved {}: {}{}",
                merged.len(),
                if merged.is_empty() { "nothing".to_string() } else { merged.iter().map(|(n, pr)| pr.as_ref().map_or_else(|| n.clone(), |p| format!("{n} (PR {p})"))).collect::<Vec<_>>().join(", ") },
                if failed.is_empty() { String::new() } else { format!("; not approved: {}", failed.join(", ")) },
                each_name = each.name,
            );
            println!("\n{} finished. {summary}", if failed.is_empty() { "✓" } else { "✗" });
            report(me.as_deref(), &format!("The pipeline finished. {summary}"), Some(if failed.is_empty() { Verdict::Approved } else { Verdict::Failed }));
            return Ok(if failed.is_empty() { 0 } else { 1 });
        }
        for (name, id) in running {
            let o = wait_done(me.as_deref(), &id, timeout)?;
            let mark = match o.verdict {
                Some(Verdict::Failed) => "✗",
                Some(Verdict::Changes) => "↺",
                _ => "✓",
            };
            let verdict = o.verdict.map(|v| format!(" [{}]", v.label())).unwrap_or_default();
            let pr = o.pr.as_ref().map(|p| format!(" (PR {p})")).unwrap_or_default();
            println!("{mark} {name}{verdict}{pr}: {}", crate::deliver::first_line(&o.body));
            results.insert(name, o);
        }
        step = after(&p, g, &results, &mut loops);
        if let Next::Run(back, Some(by)) = &step {
            let to = &p.stages[groups[*back][0]].name;
            println!("↺ {by} sent it back to {to} ({}/{})", loops[by], p.stages.iter().find(|s| s.name == *by).and_then(|s| s.retry.as_ref()).map_or(0, |r| r.max));
        }
    }
    let last = &p.stages[groups[groups.len() - 1][0]].name;
    let summary = results.get(last).map(|o| o.body.trim().to_string()).unwrap_or_default();
    println!("\n✓ finished. {last} said:\n{summary}");
    report(me.as_deref(), &format!("The pipeline finished. {last} said:\n{summary}"), Some(Verdict::Approved));
    Ok(0)
}

/// The (stage, PR) of each report an `each` stage approved, in order.
type Approved = Vec<(String, Option<String>)>;

/// The `each` stage: take the reports of `running` as they come, and give each one to a
/// fresh agent of `each`'s, one at a time. What it approves goes in `merged`; what it
/// sends back goes back to whoever reported it. Returns (stage, pr) approved, in order,
/// and the stages that weren't.
#[allow(clippy::too_many_arguments)]
fn run_each(
    p: &Pipeline,
    each: &Stage,
    task: &str,
    results: &HashMap<String, Outcome>,
    cwd: &Path,
    env: &[(String, String)],
    me: Option<&str>,
    running: Vec<(String, String)>,
) -> io::Result<(Approved, Vec<String>)> {
    let timeout = p.stage_timeout();
    let kind = each.kind.clone().or_else(|| p.kind.clone());
    let mut waiting: HashMap<String, String> = running.into_iter().map(|(name, id)| (id, name)).collect();
    let mut merged: Approved = Vec::new();
    let mut failed: Vec<String> = Vec::new();
    let mut sent_back: HashMap<String, u32> = HashMap::new();
    let mut n = 0;
    while !waiting.is_empty() {
        let Some((id, o)) = wait_any(me, &waiting, timeout)? else {
            let late: Vec<String> = waiting.into_values().collect();
            println!("✗ no report in time from {}", late.join(", "));
            failed.extend(late);
            break;
        };
        let name = waiting.remove(&id).unwrap_or_default();
        let pr = o.pr.as_ref().map(|p| format!(" (PR {p})")).unwrap_or_default();
        if o.verdict == Some(Verdict::Failed) {
            println!("✗ {name}{pr}: {}", crate::deliver::first_line(&o.body));
            failed.push(name);
            continue;
        }
        println!("▸ {name}{pr} is next: {}", crate::deliver::first_line(&o.body));
        n += 1;
        let done_so_far = if merged.is_empty() {
            "nothing yet".to_string()
        } else {
            merged.iter().map(|(n, pr)| pr.as_ref().map_or_else(|| n.clone(), |p| format!("PR {p} ({n})"))).collect::<Vec<_>>().join(", ")
        };
        let body = fill_each(&fill(&each.task, task, results), &name, &o, &done_so_far);
        let r = client::request(
            &Request::Spawn {
                name: format!("{}-{n}", each.name),
                kind: kind.clone(),
                argv: each.command.clone().unwrap_or_default(),
                cwd: Some(cwd.to_string_lossy().into_owned()),
                env: Some(env.to_vec()),
                parent: me.map(Into::into),
                role: Some(each.role.clone().unwrap_or_else(|| each.name.clone())),
                task: body,
            },
            true,
        )?;
        let agent = r.id.unwrap_or_default();
        println!("  {}-{n} ({}) is on it: {agent}", each.name, kind.as_deref().unwrap_or("claude"));
        let verdict = wait_done(me, &agent, timeout)?;
        // Fresh for each: this one's done.
        let _ = client::request(&Request::Kill { id: agent.clone() }, false);
        match verdict.verdict {
            Some(Verdict::Approved) => {
                println!("✓ {name}{pr} approved: {}", crate::deliver::first_line(&verdict.body));
                merged.push((name, o.pr));
            }
            Some(Verdict::Changes) => {
                let tries = sent_back.entry(name.clone()).or_insert(0);
                *tries += 1;
                if *tries > SEND_BACKS || !alive(&id) {
                    println!("✗ {name}{pr} sent back too often (or its agent ended): {}", crate::deliver::first_line(&verdict.body));
                    failed.push(name);
                    continue;
                }
                println!("↺ {name}{pr} sent back ({tries}/{SEND_BACKS}): {}", crate::deliver::first_line(&verdict.body));
                let body = format!(
                    "{} sent your work back. What it said:\n{}\n\nFix it (rebase onto the main branch first if it conflicts), push, and report again with nex-host done (with --pr).",
                    each.name,
                    verdict.body.trim()
                );
                client::request(
                    &Request::Send { from: me.map(Into::into), to: id.clone(), kind: MessageKind::Task, body, reply_to: None, verdict: None, files: vec![], pr: None, force: false },
                    true,
                )?;
                waiting.insert(id, name);
            }
            _ => {
                println!("✗ {name}{pr} not approved: {}", crate::deliver::first_line(&verdict.body));
                failed.push(name);
            }
        }
    }
    Ok((merged, failed))
}

/// An `each` stage's task with `{from}`, `{report}`, `{pr}`, `{files}` and `{merged}` filled in.
pub fn fill_each(template: &str, from: &str, o: &Outcome, merged: &str) -> String {
    template
        .replace("{from}", from)
        .replace("{pr}", o.pr.as_deref().unwrap_or("none"))
        .replace("{files}", &o.files.join(", "))
        .replace("{merged}", merged)
        .replace("{report}", o.body.trim())
}

/// The next `done` from any of `from` (id → name), waiting up to `timeout`; none if time
/// runs out. Reports from anyone else are passed over.
fn wait_any(me: Option<&str>, from: &HashMap<String, String>, timeout: Duration) -> io::Result<Option<(String, Outcome)>> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(None);
        }
        let r = client::request(&Request::Recv { me: me.map(Into::into), from: None, reply_to: None, kind: Some(MessageKind::Done), timeout_ms: Some(left.as_millis() as u64) }, true);
        match r {
            Ok(r) => {
                if let Some(m) = r.messages.and_then(|m| m.into_iter().next())
                    && from.contains_key(&m.from)
                {
                    let body = match &m.more {
                        Some(path) => std::fs::read_to_string(nex_host::paths::expand_home(path)).unwrap_or(m.body),
                        None => m.body,
                    };
                    return Ok(Some((m.from, Outcome { body, verdict: m.verdict, files: m.files, pr: m.pr })));
                }
            }
            Err(e) if e.to_string().starts_with("timed out") => {}
            Err(e) => return Err(e),
        }
    }
}

fn alive(id: &str) -> bool {
    client::request(&Request::List, false).ok().and_then(|r| r.sessions).is_some_and(|s| s.iter().any(|s| s.id == id && s.exit.is_none()))
}

/// The next `done` from `id`, waiting up to `timeout` (in turns, as one `recv` waits an
/// hour at most).
fn wait_done(me: Option<&str>, id: &str, timeout: Duration) -> io::Result<Outcome> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            return Ok(Outcome { body: format!("{id} didn't report in time"), verdict: Some(Verdict::Failed), files: vec![], pr: None });
        }
        let r = client::request(
            &Request::Recv { me: me.map(Into::into), from: Some(id.into()), reply_to: None, kind: Some(MessageKind::Done), timeout_ms: Some(left.as_millis() as u64) },
            true,
        );
        match r {
            Ok(r) => {
                if let Some(m) = r.messages.and_then(|m| m.into_iter().next()) {
                    let body = match &m.more {
                        Some(path) => std::fs::read_to_string(nex_host::paths::expand_home(path)).unwrap_or(m.body),
                        None => m.body,
                    };
                    return Ok(Outcome { body, verdict: m.verdict, files: m.files, pr: m.pr });
                }
            }
            Err(e) if e.to_string().starts_with("timed out") => {}
            Err(e) => return Err(e),
        }
    }
}

/// Tell whoever gave this pipeline its task (if an agent did) how it went.
fn report(me: Option<&str>, body: &str, verdict: Option<Verdict>) {
    if me.is_some() {
        let _ = client::request(
            &Request::Send { from: me.map(Into::into), to: "@task".into(), kind: MessageKind::Done, body: body.into(), reply_to: None, verdict, files: vec![], pr: None, force: false },
            true,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
        Outcome { body: body.into(), verdict, files: vec![], pr: None }
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
        let o = Outcome { body: " added --version \n".into(), verdict: None, files: vec!["a.rs".into()], pr: Some("7".into()) };
        assert_eq!(fill_each("PR {pr} from {from}: {report} [{files}] after {merged} {x}", "build-a", &o, "PR 5"), "PR 7 from build-a: added --version [a.rs] after PR 5 {x}");
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
        r.insert("plan".to_string(), Outcome { body: " the plan \n".into(), verdict: Some(Verdict::Approved), files: vec!["a.rs".into(), "b.rs".into()], pr: Some("42".into()) });
        r.insert("docs".to_string(), done("d", None));
        assert_eq!(fill("Do {task}. Plan: {plan} ({plan.verdict}; {plan.files}; PR {plan.pr})", "X", &r), "Do X. Plan: the plan (approved; a.rs, b.rs; PR 42)");
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
}
