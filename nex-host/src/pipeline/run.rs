//! Running a pipeline: starting each group of stages, taking their reports, and for an
//! `each` stage, feeding its queue (`each.rs`) until Enter ends the run.

use std::collections::HashMap;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use nex_host::proto::{MessageKind, Payload, Request, Verdict};

use crate::client;
use crate::pipeline::Outcome;
use crate::pipeline::each;
use crate::pipeline::spec::{Next, Pipeline, Stage, after, fill, fill_each};

/// How a run went: its exit code, and whether Enter already ended it (an `each` run
/// listens for more until then).
pub struct Ran {
    pub code: i32,
    pub ended: bool,
}

/// What a run keeps across its stages.
struct Ctx<'a> {
    p: &'a Pipeline,
    me: Option<String>,
    cwd: PathBuf,
    stem: String,
    env: Vec<(String, String)>,
    /// Stage name → the worktree it was given (kept across retries).
    trees: HashMap<String, PathBuf>,
}

impl Ctx<'_> {
    /// Start stage `s` on `body` in a fresh agent: in its worktree (a new one when `fresh`,
    /// else the one it had) or the run's folder, as one of the run's children.
    fn spawn(&mut self, s: &Stage, body: String, fresh: bool) -> io::Result<String> {
        let dir = if !s.worktree {
            self.cwd.clone()
        } else if let Some(t) = self.trees.get(&s.name).filter(|_| !fresh) {
            t.clone()
        } else {
            let t = crate::agentcli::worktree(&self.cwd, &format!("{}-{}", self.stem, s.name))?;
            println!("worktree ({}): {}", s.name, nex_host::paths::tilde(&t));
            self.trees.entry(s.name.clone()).or_insert_with(|| t.clone());
            t
        };
        let r = client::request(
            &Request::Spawn {
                name: s.name.clone(),
                kind: self.kind(s),
                argv: s.command.clone().unwrap_or_default(),
                cwd: Some(dir.to_string_lossy().into_owned()),
                env: Some(self.env.clone()),
                parent: self.me.clone(),
                role: Some(s.role.clone().unwrap_or_else(|| s.name.clone())),
                task: body,
            },
            true,
        )?;
        Ok(r.id.unwrap_or_default())
    }

    /// Give agent `id` a new task.
    fn task(&self, id: &str, body: String) -> io::Result<()> {
        client::request(&Request::Send { from: self.me.clone(), to: id.into(), kind: MessageKind::Task, payload: Payload::text(body), force: false }, true)?;
        Ok(())
    }

    fn kind(&self, s: &Stage) -> Option<String> {
        s.kind.clone().or_else(|| self.p.kind.clone())
    }
}

/// Run the pipeline in `file` on `task`, here: what `pipe` starts in a session of its own.
/// `enter` gets a line each time Enter is pressed.
pub fn run(file: &Path, task: &str, enter: &Receiver<()>) -> io::Result<Ran> {
    let text = std::fs::read_to_string(file)?;
    let p = Pipeline::parse(&text).map_err(|e| io::Error::other(format!("{}: {e}", file.display())))?;
    let me = std::env::var("NEX_AGENT_ID").ok().filter(|s| !s.is_empty());
    let mut cwd = std::env::current_dir()?;
    let stem = file.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_else(|| "pipeline".into());
    if p.worktree {
        cwd = crate::agentcli::worktree(&cwd, &stem)?;
        println!("worktree: {}", nex_host::paths::tilde(&cwd));
    }
    let mut ctx = Ctx { p: &p, me: me.clone(), cwd, stem, env: std::env::vars().collect(), trees: HashMap::new() };
    let timeout = p.stage_timeout();
    let groups = p.groups();
    let mut agents: HashMap<String, String> = HashMap::new();
    let mut results: HashMap<String, Outcome> = HashMap::new();
    let mut loops: HashMap<String, u32> = HashMap::new();
    let mut step = Next::Run(0, None);
    loop {
        let (g, sent_back_by) = match step {
            Next::Run(g, by) => (g, by),
            Next::Finished => break,
            Next::Failed(why) => {
                println!("\n✗ stopped: {why}");
                report(me.as_deref(), &format!("The pipeline stopped: {why}"), Some(Verdict::Failed));
                return Ok(Ran { code: 1, ended: false });
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
            let id = match agents.get(&s.name) {
                Some(id) if alive(id) => {
                    ctx.task(id, body)?;
                    id.clone()
                }
                _ => ctx.spawn(s, body, false)?,
            };
            println!("▸ {} ({}) is on it: {id}", s.name, ctx.kind(s).as_deref().unwrap_or("claude"));
            agents.insert(s.name.clone(), id.clone());
            running.push((s.name.clone(), id));
        }
        if let Some(each) = groups.get(g + 1).map(|n| &p.stages[n[0]]).filter(|s| s.each) {
            let code = run_each(&mut ctx, each, task, &results, running, enter)?;
            return Ok(Ran { code, ended: true });
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
    Ok(Ran { code: 0, ended: false })
}

/// The `each` stage: put every report from the run's agents in line as it comes, and give
/// them to one agent of `each`'s, one at a time (see `each.rs`). Tasks sent to the run
/// itself (another `pipe` joining it) start the stages before `each` again, on that task,
/// with their reports joining the same line. Runs until Enter; returns the exit code.
fn run_each(ctx: &mut Ctx, each: &Stage, task: &str, results: &HashMap<String, Outcome>, running: Vec<(String, String)>, enter: &Receiver<()>) -> io::Result<i32> {
    let p = ctx.p;
    let me = ctx.me.clone();
    let mut q = each::Queue::new(&each.name, p.stage_timeout());
    let now = Instant::now();
    // The run's own task came from whoever started it (nobody gives a pipeline its task,
    // so that's its parent, or the user).
    let first = q.batch("@parent", task);
    for (name, id) in &running {
        q.add(id, name, first, now);
    }
    let alive_fn: &dyn Fn(&str) -> bool = &alive;
    let mut said_idle = false;
    loop {
        if enter.try_recv().is_ok() {
            break;
        }
        let mut actions = q.overdue(alive_fn, Instant::now());
        match recv_any(me.as_deref())? {
            Some(m) if m.kind == MessageKind::Task && p.joinable() => {
                let body = message_body(&m.more, m.payload.body);
                let joined = body.trim().to_string();
                println!("\n▸ {} joins with: {}", m.from_name, crate::deliver::first_line(&joined));
                let b = q.batch(&m.from, &joined);
                for &i in &p.groups()[0] {
                    let s = &p.stages[i];
                    // One join that can't start doesn't end the run: the others are still
                    // in line, and their reports still need the reviewer.
                    let id = match ctx.spawn(s, fill(&s.task, &joined, results), true) {
                        Ok(id) => id,
                        Err(e) => {
                            println!("✗ {} couldn't start: {e}", s.name);
                            let payload = Payload { verdict: Some(Verdict::Failed), ..Payload::text(format!("The pipeline couldn't start {} on your task: {e}", s.name)) };
                            let _ = client::request(&Request::Send { from: me.clone(), to: m.from.clone(), kind: MessageKind::Done, payload, force: false }, true);
                            continue;
                        }
                    };
                    // Its own name (`build-2`), so the line tells the features apart.
                    let name = name_of(&id).unwrap_or_else(|| s.name.clone());
                    println!("▸ {name} ({}) is on it: {id}", ctx.kind(s).as_deref().unwrap_or("claude"));
                    q.add(&id, &name, b, Instant::now());
                }
            }
            Some(m) if m.kind == MessageKind::Done => {
                let from = m.from.clone();
                actions.extend(q.done(&from, outcome(m), alive_fn, Instant::now()));
            }
            _ => {}
        }
        for a in actions {
            match a {
                each::Action::Say(line) => println!("{line}"),
                each::Action::Tell { to, body, verdict } => {
                    if me.is_some() {
                        let payload = Payload { verdict: Some(verdict), ..Payload::text(body) };
                        let _ = client::request(&Request::Send { from: me.clone(), to, kind: MessageKind::Done, payload, force: false }, true);
                    }
                }
                each::Action::SendBack { to, body } => ctx.task(&to, body)?,
                each::Action::Review { from, task, report, merged } => {
                    let body = fill_each(&fill(&each.task, &task, results), &from, &report, &merged);
                    let id = match q.reviewer.clone() {
                        Some(id) if alive(&id) => {
                            ctx.task(&id, body)?;
                            id
                        }
                        _ => ctx.spawn(each, body, false)?,
                    };
                    println!("  {} ({}) is on it: {id}", each.name, ctx.kind(each).as_deref().unwrap_or("claude"));
                    q.reviewer = Some(id);
                }
            }
        }
        if q.idle() && !said_idle {
            let more = if p.joinable() { " Waiting for more (nex-host pipe with this pipeline, from this repository, adds to this run)." } else { "" };
            println!("\nThe line is empty.{more} Press Enter to end the run and its agents.");
        }
        said_idle = q.idle();
    }
    Ok(if q.failed() { 1 } else { 0 })
}

/// What a `done` reported: its fields, and its whole body.
fn outcome(m: nex_host::proto::Message) -> Outcome {
    let body = message_body(&m.more, m.payload.body);
    Outcome { body, reply_to: None, ..m.payload }
}

/// A message's whole body: what was cut short is in the file `more` names.
fn message_body(more: &Option<String>, body: String) -> String {
    match more {
        Some(path) => std::fs::read_to_string(nex_host::paths::expand_home(path)).unwrap_or(body),
        None => body,
    }
}

/// The next message for the run, if one comes within a second.
fn recv_any(me: Option<&str>) -> io::Result<Option<nex_host::proto::Message>> {
    let r = client::request(&Request::Recv { me: me.map(Into::into), from: None, reply_to: None, kind: None, timeout_ms: Some(1000) }, true);
    match r {
        Ok(r) => Ok(r.messages.and_then(|m| m.into_iter().next())),
        Err(e) if e.to_string().starts_with("timed out") => Ok(None),
        Err(e) => Err(e),
    }
}

/// The name agent `id` goes by.
fn name_of(id: &str) -> Option<String> {
    client::request(&Request::List, false).ok().and_then(|r| r.sessions)?.into_iter().find(|s| s.id == id).map(|s| s.name)
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
            return Ok(Outcome { verdict: Some(Verdict::Failed), ..Outcome::text(format!("{id} didn't report in time")) });
        }
        let r = client::request(
            &Request::Recv { me: me.map(Into::into), from: Some(id.into()), reply_to: None, kind: Some(MessageKind::Done), timeout_ms: Some(left.as_millis() as u64) },
            true,
        );
        match r {
            Ok(r) => {
                if let Some(m) = r.messages.and_then(|m| m.into_iter().next()) {
                    return Ok(outcome(m));
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
            &Request::Send { from: me.map(Into::into), to: "@task".into(), kind: MessageKind::Done, payload: Payload { verdict, ..Payload::text(body) }, force: false },
            true,
        );
    }
}
