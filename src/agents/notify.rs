//! Telling the user about agents while they look elsewhere: what the Agents tab's badge shows
//! ([`badge`]), and when to send a system notification ([`Notifier`], behind
//! `system-notifications`). Both only decide; the title bar draws the badge and
//! `platform::notify` sends the notification.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use crate::agents::model::{Agent, AgentKind, Counts, Status};
use crate::layout::PaneId;

/// What the Agents tab's badge shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// This many agents need you: amber, with the count where there's room for one.
    NeedsYou(usize),
    /// None needs you, but at least one finished and you haven't looked at it yet: a quieter dot.
    Done,
}

/// The badge for `counts`, or none: while the Agents tab is shown (it lists them already),
/// or while no agent needs you or is done.
pub fn badge(counts: Counts, agents_tab_shown: bool) -> Option<Badge> {
    if agents_tab_shown {
        None
    } else if counts.needs_you > 0 {
        Some(Badge::NeedsYou(counts.needs_you))
    } else if counts.done > 0 {
        Some(Badge::Done)
    } else {
        None
    }
}

/// The count as a badge shows it: `9+` past nine.
pub fn badge_count(n: usize) -> String {
    if n > 9 { "9+".into() } else { n.to_string() }
}

/// A system notification to send.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Note {
    pub title: String,
    pub body: String,
}

/// Where the user is looking, for [`Notifier::update`]: nothing is sent about what's on screen.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Looking {
    /// nex's window has focus.
    pub focused: bool,
    /// The Agents tab is shown.
    pub agents_tab: bool,
    /// The pane being looked at (the shown tab's focused one).
    pub pane: Option<PaneId>,
}

impl Looking {
    /// Whether the user can see `a` already: the window has focus and shows the Agents tab or
    /// the pane `a` is on.
    fn sees(&self, a: &Agent) -> bool {
        self.focused && (self.agents_tab || (self.pane.is_some() && a.shown_pane() == self.pane))
    }
}

/// Decides which changes in the agent list get a system notification: an agent starting to
/// need you, and a pipeline run finishing, once per change. What was already so the first
/// time it sees the list (at launch, or when the setting is turned on) isn't news.
#[derive(Default)]
pub struct Notifier {
    /// The list's generation last time ([`crate::agents::model::Agents::generation`]).
    generation: Option<u64>,
    /// The agents that needed you last time.
    needs_you: HashSet<PaneId>,
    /// Each pipeline's finishing line last time, if its run had finished.
    finished: HashMap<PaneId, String>,
}

impl Notifier {
    /// The notifications for `agents` (the list with `generation`) since the last call.
    /// Cheap when the list hasn't changed.
    pub fn update(&mut self, agents: &[Arc<Agent>], generation: u64, looking: Looking) -> Vec<Note> {
        if self.generation == Some(generation) {
            return Vec::new();
        }
        let first = self.generation.is_none();
        self.generation = Some(generation);
        let mut notes = Vec::new();
        let mut needs_you = HashSet::new();
        let mut finished = HashMap::new();
        for a in agents {
            if a.kind == AgentKind::Pipeline {
                let Some(line) = finish_line(a) else { continue };
                if !first && self.finished.get(&a.pane) != Some(&line) && !looking.sees(a) {
                    notes.push(Note { title: format!("{} finished", a.name), body: line.clone() });
                }
                finished.insert(a.pane, line);
            } else if a.status == Status::NeedsYou && !a.team.sub {
                if !first && !self.needs_you.contains(&a.pane) && !looking.sees(a) {
                    let body = a.task.clone().unwrap_or_else(|| format!("{} is waiting for you", a.kind.label()));
                    notes.push(Note { title: format!("{} needs you", a.name), body });
                }
                needs_you.insert(a.pane);
            }
        }
        self.needs_you = needs_you;
        self.finished = finished;
        notes
    }
}

/// The line a pipeline's title ends on once its run is over (`✓ finished. …`, `✗ stopped: …`),
/// or `None` while it runs. See `nex_host::proto::pipeline_title`.
fn finish_line(a: &Agent) -> Option<String> {
    let (counts, event) = nex_host::proto::split_pipeline_title(a.task.as_deref()?);
    // A title with no counts has lost the separator's leading space to trimming.
    let event = if event.is_empty() { counts.strip_prefix('—').map_or("", str::trim) } else { event };
    let rest = event.strip_prefix('✓').or_else(|| event.strip_prefix('✗'))?.trim_start();
    (rest.starts_with("finished") || rest.starts_with("stopped")).then(|| event.to_string())
}

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use super::*;
    use crate::agents::model::{Source, Team};

    fn agent(pane: PaneId, kind: AgentKind, status: Status, task: Option<&str>) -> Arc<Agent> {
        Arc::new(Agent {
            pane,
            tab: Some(0),
            kind,
            name: format!("agent {pane}"),
            task: task.map(str::to_string),
            session: None,
            mark: None,
            cwd: None,
            status,
            since: Instant::now(),
            source: Source::Local,
            team: Team::default(),
        })
    }

    fn claude(pane: PaneId, status: Status) -> Arc<Agent> {
        agent(pane, AgentKind::CLAUDE, status, Some("Fixing tests"))
    }

    fn pipeline(pane: PaneId, title: &str) -> Arc<Agent> {
        agent(pane, AgentKind::Pipeline, Status::Working, Some(title))
    }

    fn counts(needs_you: usize, done: usize) -> Counts {
        Counts { needs_you, done, working: 1, idle: 1 }
    }

    #[test]
    fn badge_is_amber_for_needs_you_quiet_for_done() {
        assert_eq!(badge(counts(2, 1), false), Some(Badge::NeedsYou(2)));
        assert_eq!(badge(counts(0, 3), false), Some(Badge::Done));
        assert_eq!(badge(counts(0, 0), false), None);
    }

    #[test]
    fn badge_hides_while_the_agents_tab_is_shown() {
        assert_eq!(badge(counts(2, 1), true), None);
        assert_eq!(badge(counts(0, 1), true), None);
    }

    #[test]
    fn badge_count_caps_at_nine() {
        assert_eq!(badge_count(3), "3");
        assert_eq!(badge_count(10), "9+");
    }

    /// Feed `n` each list in turn, with a fresh generation each time.
    fn run(n: &mut Notifier, lists: &[Vec<Arc<Agent>>], looking: Looking) -> Vec<Vec<Note>> {
        thread_local!(static GEN: std::cell::Cell<u64> = const { std::cell::Cell::new(100) });
        lists
            .iter()
            .map(|l| {
                let g = GEN.with(|g| {
                    g.set(g.get() + 1);
                    g.get()
                });
                n.update(l, g, looking)
            })
            .collect()
    }

    #[test]
    fn needing_you_notifies_once_per_change() {
        let mut n = Notifier::default();
        let notes = run(
            &mut n,
            &[
                vec![claude(1, Status::Working)],
                vec![claude(1, Status::NeedsYou)],
                vec![claude(1, Status::NeedsYou)],
                vec![claude(1, Status::Working)],
                vec![claude(1, Status::NeedsYou)],
            ],
            Looking::default(),
        );
        let sent: Vec<usize> = notes.iter().map(Vec::len).collect();
        assert_eq!(sent, [0, 1, 0, 0, 1]);
        assert_eq!(notes[1][0], Note { title: "agent 1 needs you".into(), body: "Fixing tests".into() });
    }

    #[test]
    fn the_same_generation_is_not_looked_at_again() {
        let mut n = Notifier::default();
        n.update(&[claude(1, Status::Working)], 1, Looking::default());
        assert_eq!(n.update(&[claude(1, Status::NeedsYou)], 1, Looking::default()), []);
    }

    #[test]
    fn what_was_so_at_the_start_is_not_news() {
        let mut n = Notifier::default();
        let notes =
            run(&mut n, &[vec![claude(1, Status::NeedsYou), pipeline(2, "stage 1 of 1: build — ✓ finished. build said: ok")]], Looking::default());
        assert_eq!(notes, [vec![]]);
    }

    #[test]
    fn nothing_is_sent_about_what_is_on_screen() {
        let lists = [vec![claude(1, Status::Working)], vec![claude(1, Status::NeedsYou)]];
        let on_tab = Looking { focused: true, agents_tab: true, pane: None };
        assert!(run(&mut Notifier::default(), &lists, on_tab).concat().is_empty());
        let on_pane = Looking { focused: true, agents_tab: false, pane: Some(1) };
        assert!(run(&mut Notifier::default(), &lists, on_pane).concat().is_empty());
        // Another pane, or the window in the background: it's news.
        let elsewhere = Looking { focused: true, agents_tab: false, pane: Some(7) };
        assert_eq!(run(&mut Notifier::default(), &lists, elsewhere).concat().len(), 1);
        let away = Looking { focused: false, agents_tab: true, pane: Some(1) };
        assert_eq!(run(&mut Notifier::default(), &lists, away).concat().len(), 1);
    }

    #[test]
    fn sub_agents_and_pipelines_needing_you_are_not_sent() {
        let mut sub = (*claude(2, Status::Working)).clone();
        sub.team.sub = true;
        let mut sub_needs = sub.clone();
        sub_needs.status = Status::NeedsYou;
        let mut pipe_needs = (*pipeline(3, "stage 1 of 2: build")).clone();
        pipe_needs.status = Status::NeedsYou;
        let notes = run(
            &mut Notifier::default(),
            &[vec![Arc::new(sub), pipeline(3, "stage 1 of 2: build")], vec![Arc::new(sub_needs), Arc::new(pipe_needs)]],
            Looking::default(),
        );
        assert!(notes.concat().is_empty());
    }

    #[test]
    fn a_pipeline_run_finishing_notifies_once() {
        let done = "stage 2 of 2: review — ✓ finished. review said: approved";
        let notes = run(
            &mut Notifier::default(),
            &[
                vec![pipeline(5, "stage 1 of 2: build — worktree (build): ~/x")],
                vec![pipeline(5, "stage 2 of 2: review — ✓ build [approved] (PR 7): done")],
                vec![pipeline(5, done)],
                vec![pipeline(5, done)],
            ],
            Looking::default(),
        );
        let sent: Vec<usize> = notes.iter().map(Vec::len).collect();
        assert_eq!(sent, [0, 0, 1, 0]);
        assert_eq!(notes[2][0], Note { title: "agent 5 finished".into(), body: "✓ finished. review said: approved".into() });
    }

    #[test]
    fn each_batch_of_a_queue_run_finishing_is_its_own_change() {
        let notes = run(
            &mut Notifier::default(),
            &[
                vec![pipeline(5, "1 building")],
                vec![pipeline(5, "1 merged — ✓ finished. merge approved 1: build (PR 7)")],
                vec![pipeline(5, "1 merged · 1 building — ✓ build-2 (PR 8): done")],
                vec![pipeline(5, "1 merged · 1 failed — ✗ finished. merge approved 1: build (PR 7); not approved: build-2")],
            ],
            Looking::default(),
        );
        let sent: Vec<usize> = notes.iter().map(Vec::len).collect();
        assert_eq!(sent, [0, 1, 0, 1]);
    }

    #[test]
    fn a_stopped_run_counts_as_finished() {
        let notes = run(
            &mut Notifier::default(),
            &[vec![pipeline(5, "stage 1 of 2: build")], vec![pipeline(5, "stage 1 of 2: build — ✗ stopped: build failed")]],
            Looking::default(),
        );
        assert_eq!(notes[1].len(), 1);
    }

    #[test]
    fn finish_lines_are_only_the_run_ending() {
        let line = |t: &str| finish_line(&pipeline(1, t));
        assert_eq!(line("stage 1 of 1: build — ✓ finished. build said: ok").as_deref(), Some("✓ finished. build said: ok"));
        assert_eq!(line("— ✗ stopped: no agent"), Some("✗ stopped: no agent".into()));
        assert_eq!(line("stage 1 of 1: build — ✓ build (PR 7): done"), None);
        assert_eq!(line("stage 1 of 1: build"), None);
        assert_eq!(line("finished — nothing yet"), None);
    }
}
