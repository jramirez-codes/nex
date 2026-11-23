//! An `each` stage as a queue: every report from the stages before it gets in line, in
//! the order it arrives, and one reviewer takes them one at a time.
//!
//! A source (a stage's agent) is listened to for as long as the run lasts. Its first
//! report, its report after a send-back, and any later report (another PR) each get
//! reviewed. Sources come in batches: the run's own task is the first, and each
//! `pipe NAME TASK` that joins the run adds another. A batch's sender hears how it went
//! once every source in it is settled.
//!
//! This file is the bookkeeping only, with no I/O: [`Queue`] takes what arrived and says
//! what to do (an [`Action`]), and `run_each` in `run.rs` does it.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

use nex_host::proto::Verdict;

use crate::deliver::first_text_line as first_line;
use crate::pipeline::{Outcome, SEND_BACKS};

/// What the runner should do next.
#[derive(Debug, PartialEq, Eq)]
pub enum Action {
    /// Give this report to the reviewer. `task` is its batch's task, and `merged` is what
    /// was approved so far.
    Review { from: String, task: String, report: Outcome, merged: String },
    /// Send the work back to the source agent `to` as a new task.
    SendBack { to: String, body: String },
    /// Tell a batch's sender (`to`) how its batch went.
    Tell { to: String, body: String, verdict: Verdict },
    /// A line for the runner's screen.
    Say(String),
}

struct Source {
    id: String,
    name: String,
    batch: usize,
    /// It has a task it hasn't reported on, due by `deadline`.
    working: bool,
    deadline: Instant,
    /// Times it has been sent back since its last approval.
    tries: u32,
}

struct Batch {
    /// Who to tell when it's settled.
    origin: String,
    task: String,
    approved: Vec<(String, Option<String>)>,
    failed: Vec<String>,
    told: bool,
}

pub struct Queue {
    stage: String,
    timeout: Duration,
    /// The reviewer agent, once there is one.
    pub reviewer: Option<String>,
    sources: Vec<Source>,
    batches: Vec<Batch>,
    /// Reports waiting for the reviewer: (source index, report).
    line: VecDeque<(usize, Outcome)>,
    /// What the reviewer has now, and when it's due.
    reviewing: Option<(usize, Outcome, Instant)>,
    /// Everything approved, in order: (source name, PR).
    merged: Vec<(String, Option<String>)>,
}

impl Queue {
    pub fn new(stage: &str, timeout: Duration) -> Queue {
        Queue { stage: stage.into(), timeout, reviewer: None, sources: Vec::new(), batches: Vec::new(), line: VecDeque::new(), reviewing: None, merged: Vec::new() }
    }

    /// A new batch: `task` given by `origin`. Returns its index for [`Queue::add`].
    pub fn batch(&mut self, origin: &str, task: &str) -> usize {
        self.batches.push(Batch { origin: origin.into(), task: task.into(), approved: Vec::new(), failed: Vec::new(), told: false });
        self.batches.len() - 1
    }

    /// Agent `id` (stage `name`) is working on a task of `batch`.
    pub fn add(&mut self, id: &str, name: &str, batch: usize, now: Instant) {
        self.sources.push(Source { id: id.into(), name: name.into(), batch, working: true, deadline: now + self.timeout, tries: 0 });
    }

    /// Nothing in line or being reviewed, and no source working.
    pub fn idle(&self) -> bool {
        self.reviewing.is_none() && self.line.is_empty() && !self.sources.iter().any(|s| s.working)
    }

    /// Whether anything ended up not approved.
    pub fn failed(&self) -> bool {
        self.batches.iter().any(|b| !b.failed.is_empty())
    }

    /// A `done` from agent `from`. `alive` says whether an agent is still running (a
    /// source that ended can't be sent back).
    pub fn done(&mut self, from: &str, o: Outcome, alive: &dyn Fn(&str) -> bool, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if self.reviewer.as_deref() == Some(from) {
            if let Some((i, report, _)) = self.reviewing.take() {
                self.verdict(i, report, o, alive, now, &mut out);
            }
        } else if let Some(i) = self.sources.iter().position(|s| s.id == from) {
            let s = &mut self.sources[i];
            s.working = false;
            let pr = pr_note(&o);
            if o.verdict == Some(Verdict::Failed) {
                out.push(Action::Say(format!("✗ {}{pr}: {}", s.name, first_line(&o.body))));
                let name = s.name.clone();
                self.batches[s.batch].failed.push(name);
            } else {
                if self.reviewing.is_some() || !self.line.is_empty() {
                    out.push(Action::Say(format!("  {}{pr} gets in line ({} ahead): {}", s.name, self.line.len() + 1, first_line(&o.body))));
                }
                self.line.push_back((i, o));
            }
        }
        self.advance(now, &mut out);
        out
    }

    /// Whatever is past its time: a source that didn't report, a reviewer that didn't.
    pub fn overdue(&mut self, alive: &dyn Fn(&str) -> bool, now: Instant) -> Vec<Action> {
        let mut out = Vec::new();
        if let Some((_, _, due)) = &self.reviewing
            && now >= *due
        {
            let (i, report, _) = self.reviewing.take().unwrap();
            let late = Outcome { verdict: Some(Verdict::Failed), ..Outcome::text(format!("{} didn't report in time", self.stage)) };
            self.verdict(i, report, late, alive, now, &mut out);
        }
        for s in self.sources.iter_mut().filter(|s| s.working && now >= s.deadline) {
            s.working = false;
            out.push(Action::Say(format!("✗ no report in time from {}", s.name)));
            self.batches[s.batch].failed.push(s.name.clone());
        }
        self.advance(now, &mut out);
        out
    }

    /// What the reviewer said about source `i`'s `report`.
    fn verdict(&mut self, i: usize, report: Outcome, v: Outcome, alive: &dyn Fn(&str) -> bool, now: Instant, out: &mut Vec<Action>) {
        let s = &mut self.sources[i];
        let pr = pr_note(&report);
        match v.verdict {
            Some(Verdict::Approved) => {
                out.push(Action::Say(format!("✓ {}{pr} approved: {}", s.name, first_line(&v.body))));
                s.tries = 0;
                self.merged.push((s.name.clone(), report.pr.clone()));
                self.batches[s.batch].approved.push((s.name.clone(), report.pr));
            }
            Some(Verdict::Changes) if s.tries < SEND_BACKS && alive(&s.id) => {
                s.tries += 1;
                s.working = true;
                s.deadline = now + self.timeout;
                out.push(Action::Say(format!("↺ {}{pr} sent back ({}/{SEND_BACKS}): {}", s.name, s.tries, first_line(&v.body))));
                let body = format!(
                    "{} sent your work back. What it said:\n{}\n\nFix it (rebase onto the main branch first if it conflicts), push, and report again with nex-host done (with --pr).",
                    self.stage,
                    v.body.trim()
                );
                out.push(Action::SendBack { to: s.id.clone(), body });
            }
            Some(Verdict::Changes) => {
                out.push(Action::Say(format!("✗ {}{pr} sent back too often (or its agent ended): {}", s.name, first_line(&v.body))));
                self.batches[s.batch].failed.push(s.name.clone());
            }
            _ => {
                out.push(Action::Say(format!("✗ {}{pr} not approved: {}", s.name, first_line(&v.body))));
                self.batches[s.batch].failed.push(s.name.clone());
            }
        }
    }

    /// Start the next review if the reviewer is free, and tell each batch that has just
    /// settled how it went.
    fn advance(&mut self, now: Instant, out: &mut Vec<Action>) {
        for b in 0..self.batches.len() {
            let busy = |i: usize| self.sources[i].batch == b;
            let open = self.sources.iter().any(|s| s.batch == b && s.working)
                || self.line.iter().any(|(i, _)| busy(*i))
                || self.reviewing.as_ref().is_some_and(|(i, ..)| busy(*i));
            let batch = &mut self.batches[b];
            if open {
                batch.told = false;
                continue;
            }
            if batch.told || (batch.approved.is_empty() && batch.failed.is_empty()) {
                continue;
            }
            batch.told = true;
            let summary = format!(
                "{} approved {}: {}{}",
                self.stage,
                batch.approved.len(),
                if batch.approved.is_empty() { "nothing".to_string() } else { batch.approved.iter().map(|(n, pr)| pr.as_ref().map_or_else(|| n.clone(), |p| format!("{n} (PR {p})"))).collect::<Vec<_>>().join(", ") },
                if batch.failed.is_empty() { String::new() } else { format!("; not approved: {}", batch.failed.join(", ")) },
            );
            let ok = batch.failed.is_empty();
            out.push(Action::Say(format!("\n{} finished. {summary}", if ok { "✓" } else { "✗" })));
            out.push(Action::Tell { to: batch.origin.clone(), body: format!("The pipeline finished. {summary}"), verdict: if ok { Verdict::Approved } else { Verdict::Failed } });
        }
        if self.reviewing.is_some() {
            return;
        }
        let Some((i, report)) = self.line.pop_front() else { return };
        let s = &self.sources[i];
        out.push(Action::Say(format!("▸ {}{} is next: {}", s.name, pr_note(&report), first_line(&report.body))));
        let merged = if self.merged.is_empty() {
            "nothing yet".to_string()
        } else {
            self.merged.iter().map(|(n, pr)| pr.as_ref().map_or_else(|| n.clone(), |p| format!("PR {p} ({n})"))).collect::<Vec<_>>().join(", ")
        };
        out.push(Action::Review { from: s.name.clone(), task: self.batches[s.batch].task.clone(), report: report.clone(), merged });
        self.reviewing = Some((i, report, now + self.timeout));
    }
}

fn pr_note(o: &Outcome) -> String {
    o.pr.as_ref().map(|p| format!(" (PR {p})")).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(body: &str, pr: &str) -> Outcome {
        Outcome { pr: Some(pr.into()), ..Outcome::text(body) }
    }

    fn says(v: Verdict, body: &str) -> Outcome {
        Outcome { verdict: Some(v), ..Outcome::text(body) }
    }

    fn reviews(a: &[Action]) -> Vec<(&str, Option<&str>)> {
        a.iter().filter_map(|a| if let Action::Review { from, report, .. } = a { Some((from.as_str(), report.pr.as_deref())) } else { None }).collect()
    }

    fn tells(a: &[Action]) -> Vec<&str> {
        a.iter().filter_map(|a| if let Action::Tell { body, .. } = a { Some(body.as_str()) } else { None }).collect()
    }

    const T: Duration = Duration::from_secs(60);
    const ALIVE: &dyn Fn(&str) -> bool = &|_| true;

    #[test]
    fn every_report_is_reviewed_including_after_a_send_back_and_later_ones() {
        let now = Instant::now();
        let mut q = Queue::new("merge", T);
        let b = q.batch("user", "add a stats command");
        q.add("b1", "build", b, now);
        q.reviewer = Some("rev".into());

        let a = q.done("b1", report("opened it", "7"), ALIVE, now);
        assert_eq!(reviews(&a), [("build", Some("7"))]);
        let a = q.done("rev", says(Verdict::Changes, "conflicts"), ALIVE, now);
        assert!(a.contains(&Action::SendBack { to: "b1".into(), body: a.iter().find_map(|a| if let Action::SendBack { body, .. } = a { Some(body.clone()) } else { None }).unwrap() }));
        assert!(tells(&a).is_empty(), "still open: it was sent back");
        assert!(!q.idle());

        // The re-report is reviewed too.
        let a = q.done("b1", report("rebased", "7"), ALIVE, now);
        assert_eq!(reviews(&a), [("build", Some("7"))]);
        let a = q.done("rev", says(Verdict::Approved, "merged"), ALIVE, now);
        assert_eq!(tells(&a), ["The pipeline finished. merge approved 1: build (PR 7)"]);
        assert!(q.idle());

        // And a later report from the same agent (another PR) is reviewed as well.
        let a = q.done("b1", report("and a follow-up", "8"), ALIVE, now);
        let Some(Action::Review { merged, .. }) = a.iter().find(|a| matches!(a, Action::Review { .. })) else { panic!("{a:?}") };
        assert_eq!(merged, "PR 7 (build)");
        let a = q.done("rev", says(Verdict::Approved, "merged"), ALIVE, now);
        assert_eq!(tells(&a), ["The pipeline finished. merge approved 2: build (PR 7), build (PR 8)"]);
    }

    #[test]
    fn one_review_at_a_time_first_come_first_served() {
        let now = Instant::now();
        let mut q = Queue::new("merge", T);
        let b = q.batch("user", "go");
        for (id, name) in [("a", "build-a"), ("b", "build-b"), ("c", "build-c")] {
            q.add(id, name, b, now);
        }
        q.reviewer = Some("rev".into());
        assert_eq!(reviews(&q.done("b", report("b", "2"), ALIVE, now)), [("build-b", Some("2"))]);
        assert!(reviews(&q.done("c", report("c", "3"), ALIVE, now)).is_empty(), "the reviewer is busy");
        assert!(reviews(&q.done("a", report("a", "1"), ALIVE, now)).is_empty());
        assert_eq!(reviews(&q.done("rev", says(Verdict::Approved, "ok"), ALIVE, now)), [("build-c", Some("3"))]);
        assert_eq!(reviews(&q.done("rev", says(Verdict::Approved, "ok"), ALIVE, now)), [("build-a", Some("1"))]);
        let a = q.done("rev", says(Verdict::Approved, "ok"), ALIVE, now);
        assert_eq!(tells(&a), ["The pipeline finished. merge approved 3: build-b (PR 2), build-c (PR 3), build-a (PR 1)"]);
    }

    #[test]
    fn joined_batches_share_the_reviewer_and_hear_about_their_own_work() {
        let now = Instant::now();
        let mut q = Queue::new("merge", T);
        let first = q.batch("user", "feature one");
        q.add("b1", "build", first, now);
        let second = q.batch("lead", "feature two");
        q.add("b2", "build-2", second, now);
        q.reviewer = Some("rev".into());
        q.done("b1", report("one", "1"), ALIVE, now);
        let a = q.done("b2", report("two", "2"), ALIVE, now);
        assert!(reviews(&a).is_empty(), "in line behind build");
        let a = q.done("rev", says(Verdict::Approved, "ok"), ALIVE, now);
        assert_eq!(tells(&a), ["The pipeline finished. merge approved 1: build (PR 1)"]);
        let Some(Action::Review { task, .. }) = a.iter().find(|a| matches!(a, Action::Review { .. })) else { panic!("{a:?}") };
        assert_eq!(task, "feature two", "the review gets its own batch's task");
        let a = q.done("rev", says(Verdict::Approved, "ok"), ALIVE, now);
        assert!(a.contains(&Action::Tell { to: "lead".into(), body: "The pipeline finished. merge approved 1: build-2 (PR 2)".into(), verdict: Verdict::Approved }));
        assert!(q.idle() && !q.failed());
    }

    #[test]
    fn failures_settle_a_batch_as_not_approved() {
        let now = Instant::now();
        let mut q = Queue::new("merge", T);
        let b = q.batch("user", "go");
        q.add("a", "build-a", b, now);
        q.add("b", "build-b", b, now);
        q.reviewer = Some("rev".into());
        q.done("a", says(Verdict::Failed, "couldn't build"), ALIVE, now);
        q.done("b", report("b", "2"), ALIVE, now);
        let dead: &dyn Fn(&str) -> bool = &|_| false;
        let a = q.done("rev", says(Verdict::Changes, "fix it"), dead, now);
        assert_eq!(tells(&a), ["The pipeline finished. merge approved 0: nothing; not approved: build-a, build-b"], "an ended agent can't be sent back");
        assert!(q.failed());
    }

    #[test]
    fn late_sources_and_reviewers_time_out() {
        let now = Instant::now();
        let mut q = Queue::new("merge", T);
        let b = q.batch("user", "go");
        q.add("a", "build-a", b, now);
        q.add("b", "build-b", b, now);
        q.reviewer = Some("rev".into());
        q.done("a", report("a", "1"), ALIVE, now);
        assert!(q.overdue(ALIVE, now + T / 2).is_empty());
        let a = q.overdue(ALIVE, now + T);
        assert!(a.contains(&Action::Say("✗ no report in time from build-b".into())), "{a:?}");
        assert_eq!(tells(&a), ["The pipeline finished. merge approved 0: nothing; not approved: build-a, build-b"]);
        assert!(q.idle());
    }
}
