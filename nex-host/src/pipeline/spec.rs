//! A pipeline file: its stages and the order they run in, how their tasks are filled in
//! from earlier reports, and where a run goes after each group of stages.

use std::collections::HashMap;
use std::time::Duration;

use serde::Deserialize;

use nex_host::proto::Verdict;

use crate::pipeline::Outcome;

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
    /// served, one at a time, by one agent.
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

/// How long a stage may take when the pipeline doesn't say.
const STAGE_TIMEOUT: Duration = Duration::from_secs(2 * 3600);

fn approved() -> String {
    Verdict::Approved.label().into()
}

fn three() -> u32 {
    3
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
                let last = self.stages.last().is_some_and(|l| std::ptr::eq(s, l));
                if !last || s.parallel || seen.len() == 1 || s.retry.is_some() {
                    return Err(format!("{}: an `each` stage is the last one, not parallel, after other stages, without retry", s.name));
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

    /// Whether another `pipe` of it can join a run that's going: it ends in an `each`
    /// stage, and the stages before that all run at once (so a joined task is just those).
    pub fn joinable(&self) -> bool {
        self.stages.last().is_some_and(|s| s.each) && self.groups().len() == 2
    }

    pub(super) fn group_of(&self, stage: &str) -> Option<usize> {
        let i = self.stages.iter().position(|s| s.name == stage)?;
        self.groups().iter().position(|g| g.contains(&i))
    }

    pub(super) fn stage_timeout(&self) -> Duration {
        self.timeout.as_deref().and_then(parse_duration).unwrap_or(STAGE_TIMEOUT)
    }
}

/// `template` with `{task}`, `{stage}`, `{stage.verdict}`, `{stage.files}` and `{stage.pr}`
/// filled in.
/// Anything else in braces stays as it is (code has braces too).
pub(super) fn fill(template: &str, task: &str, results: &HashMap<String, Outcome>) -> String {
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
pub(super) enum Next {
    /// Run this group, and give these stages the report that sent the run back, if any.
    Run(usize, Option<String>),
    Finished,
    Failed(String),
}

/// Where the run goes after group `g`: back where a `retry` says (while it has tries
/// left), on to the next group, or stopped by a stage that failed.
pub(super) fn after(p: &Pipeline, g: usize, results: &HashMap<String, Outcome>, loops: &mut HashMap<String, u32>) -> Next {
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

/// An `each` stage's task with `{from}`, `{report}`, `{pr}`, `{files}` and `{merged}` filled in.
pub(super) fn fill_each(template: &str, from: &str, o: &Outcome, merged: &str) -> String {
    template
        .replace("{from}", from)
        .replace("{pr}", o.pr.as_deref().unwrap_or("none"))
        .replace("{files}", &o.files.join(", "))
        .replace("{merged}", merged)
        .replace("{report}", o.body.trim())
}
