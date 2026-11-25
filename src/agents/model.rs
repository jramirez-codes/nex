//! Which panes run an agent, and what each one is doing.
//!
//! Every process poll the app hands the [`Tracker`] one [`Probe`] per agent pane (what the
//! terminal knows about it, plus Claude Code's session file when there is one). The tracker
//! turns those into [`Agent`]s sorted for display, and remembers since when each has been
//! in its current state.
//!
//! The pane is drawn far more often than the agents change, so what a frame needs is made
//! when they do and shared after that: the list is [`Agents`] (each agent behind an `Arc`,
//! with a generation that changes only when the list does, for the view to cache on), and
//! the counts' wording is kept in a [`Tally`].

use std::collections::{HashMap, HashSet};
use std::ops::Deref;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use crate::layout::PaneId;

/// Output this recent (and not just the echo of typing) counts as working.
const BUSY_WINDOW: Duration = Duration::from_millis(nex_host::proto::BUSY_WINDOW_MS);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
    /// `nex-host pipe` running a pipeline on an ssh host: its stages are its team.
    Pipeline,
}

impl AgentKind {
    /// The agent a foreground process is, from its name and command line.
    pub fn detect(process: &str, argv: &[String]) -> Option<AgentKind> {
        if crate::session::claude::is_claude(argv) || process == "claude" {
            return Some(AgentKind::Claude);
        }
        let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
        let codex = |s: &str| base(s) == "codex" || s.ends_with("@openai/codex/bin/codex.js");
        match argv {
            _ if process == "codex" => Some(AgentKind::Codex),
            [first, ..] if codex(first) => Some(AgentKind::Codex),
            [runtime, script, ..] if matches!(base(runtime).as_str(), "node" | "bun") && codex(script) => Some(AgentKind::Codex),
            _ => None,
        }
    }

    /// The agent `nex-host` reports as `kind` (`claude`, `codex`, `pipeline`); other programs
    /// it holds aren't agents.
    pub fn from_name(kind: &str) -> Option<AgentKind> {
        match kind {
            "claude" => Some(AgentKind::Claude),
            "codex" => Some(AgentKind::Codex),
            nex_host::proto::PIPELINE_KIND => Some(AgentKind::Pipeline),
            _ => None,
        }
    }

    /// The process name its app tile is drawn for (see `ui::icons::app_tile`).
    pub fn process(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
            AgentKind::Pipeline => crate::ui::icons::PIPELINE_PROCESS,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
            AgentKind::Pipeline => "Pipeline",
        }
    }
}

/// Where an agent sits in a team: who started it (`nex-host spawn`, or a pipeline running
/// it as a stage), what for, and when, so a team's members keep the order they started in.
/// Agents on ssh hosts have one, and so does every sub-agent (one a Claude Code or Codex
/// agent runs inside its own process: the Task tool, a spawned thread).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Team {
    /// The card of the agent that started it.
    pub parent: Option<PaneId>,
    /// Its role (`reviewer`) or pipeline stage (`build`).
    pub role: Option<String>,
    /// When it started (unix ms).
    pub started: u64,
    /// It's a sub-agent of `parent`, inside that agent's process: it has no pane, program
    /// or bell of its own (see [`Source::Sub`]).
    pub sub: bool,
}

/// What an agent is doing. Ordered by how much it wants the user: the list shows agents
/// that need you first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    NeedsYou,
    /// It stopped working and you haven't looked at its pane since (see [`Tracker`]).
    Done,
    Working,
    Idle,
}

impl Status {
    pub const ALL: [Status; 4] = [Status::NeedsYou, Status::Done, Status::Working, Status::Idle];

    pub fn label(self) -> &'static str {
        match self {
            Status::NeedsYou => "Needs you",
            Status::Done => "Done",
            Status::Working => "Working",
            Status::Idle => "Idle",
        }
    }

    /// What `nex-host` says an agent on an ssh host is doing, or `None` once its program
    /// has ended (it's no longer listed).
    pub fn from_host(s: nex_host::proto::Status) -> Option<Status> {
        use nex_host::proto::Status as Host;
        match s {
            Host::NeedsYou => Some(Status::NeedsYou),
            Host::Working => Some(Status::Working),
            Host::Idle => Some(Status::Idle),
            Host::Exited => None,
        }
    }

    /// Claude Code's own word for its state (the session file's `status`), if nex knows it.
    /// The words are `nex-host`'s, so local and remote agents read them the same way.
    pub fn from_claude(status: &str) -> Option<Status> {
        nex_host::proto::Status::from_claude(status).and_then(Status::from_host)
    }
}

/// Where an agent runs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    /// In one of this machine's panes.
    Local,
    /// Held by `nex-host` on an ssh host (see `remote::host`): it keeps running when no pane
    /// shows it.
    Remote {
        /// The host's name (the card shows its folder instead).
        host: String,
        target: crate::remote::SshTarget,
        /// Its `nex-host` session id.
        session: String,
        /// The pane showing it right now, if one is.
        pane: Option<PaneId>,
    },
    /// Inside another agent's process: a sub-agent (see [`Team::sub`]). It has no terminal
    /// of its own; its tab follows its transcript (`nex_host::transcript`).
    Sub {
        /// The card of the agent running it.
        lead: PaneId,
        /// Its id within that agent (`nex_host::proto::SubAgent::id`).
        id: String,
        /// The pane showing it (its transcript's tab), if one is.
        pane: Option<PaneId>,
    },
}

/// The key a remote agent's card goes by: never a real pane's id (those count up from 1),
/// and the same for the same agent every poll.
pub fn remote_key(host: &str, session: &str) -> PaneId {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (host, session).hash(&mut h);
    h.finish() | (1 << 63)
}

/// Whether `key` is a remote agent's (see [`remote_key`]) rather than a pane's.
pub fn is_remote_key(key: PaneId) -> bool {
    key & (1 << 63) != 0
}

/// Whether `key` is a sub-agent's (see [`sub_key`]).
pub fn is_sub_key(key: PaneId) -> bool {
    key & (1 << 62) != 0 && !is_remote_key(key)
}

/// The key sub-agent `id` of the agent with card `lead` goes by: never a pane's or a remote
/// agent's (see [`remote_key`]), and the same every poll.
pub fn sub_key(lead: PaneId, id: &str) -> PaneId {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    (lead, id).hash(&mut h);
    (h.finish() & !(1 << 63)) | (1 << 62)
}

/// What the app knows about one agent pane at a poll.
#[derive(Clone, Debug)]
pub struct Probe {
    pub pane: PaneId,
    /// Position of its tab in the tab strip (0-based); `None` in the Agents / Files tab.
    pub tab: Option<usize>,
    pub kind: AgentKind,
    /// The pane's title without its leading icon (Claude Code puts the task there).
    pub title: String,
    /// The icon leading the title (Claude Code's ✳, or a spinner frame while it works).
    pub mark: Option<String>,
    pub cwd: Option<PathBuf>,
    /// The session file's `name` and `status` (Claude Code only).
    pub session_name: Option<String>,
    pub session_status: Option<String>,
    /// The program rang the bell or sent a notification the user hasn't seen.
    pub attention: bool,
    pub last_output: Instant,
    pub last_input: Instant,
    pub source: Source,
    /// What it's doing, when someone already worked it out (`nex-host` for remote agents).
    pub reported: Option<Status>,
    pub team: Team,
}

impl Probe {
    /// The best guess at what it's doing: Claude Code's own word first, then the bell, then
    /// its title spinner, then whether it's been printing on its own. The same rules as
    /// `nex_host::proto::Session::compute_status` uses for remote agents; keep them in step.
    pub fn status(&self, now: Instant) -> Status {
        if let Some(s) = self.reported {
            // A bell the pane showing it heard counts too.
            return if self.attention { Status::NeedsYou } else { s };
        }
        let reported = self.session_status.as_deref().and_then(Status::from_claude);
        if reported == Some(Status::NeedsYou) || self.attention {
            return Status::NeedsYou;
        }
        if let Some(s) = reported {
            return s;
        }
        if self.kind == AgentKind::Claude
            && let Some(mark) = &self.mark
        {
            let idle = mark.chars().eq([nex_host::proto::CLAUDE_IDLE_MARK]);
            return if idle { Status::Idle } else { Status::Working };
        }
        let recent = |t: Instant| now.saturating_duration_since(t) < BUSY_WINDOW;
        if recent(self.last_output) && !recent(self.last_input) { Status::Working } else { Status::Idle }
    }

    /// The pane it's on, if one shows it: its own, or for an agent on an ssh host, the pane
    /// attached to it.
    fn shown_pane(&self) -> Option<PaneId> {
        shown_pane(self.pane, &self.source)
    }

    /// What its title says it's doing right now, unless the title is only the program's
    /// name. A pipeline's is how its run stands (`nex_host::proto::pipeline_title`).
    fn task(&self) -> Option<&str> {
        let title = self.title.trim();
        let generic = title.is_empty()
            || title.eq_ignore_ascii_case(self.kind.label())
            || title.eq_ignore_ascii_case(self.kind.process());
        (!generic).then_some(title)
    }

    /// What to call it: the task in its title, else its session's name, else its folder. A
    /// pipeline keeps its name: its title changes with every report.
    fn name(&self) -> String {
        if self.kind != AgentKind::Pipeline
            && let Some(task) = self.task()
        {
            return task.to_string();
        }
        if let Some(n) = &self.session_name {
            return n.clone();
        }
        self.cwd
            .as_ref()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.kind.label().to_string())
    }
}

fn shown_pane(key: PaneId, source: &Source) -> Option<PaneId> {
    match source {
        Source::Local => Some(key),
        Source::Remote { pane, .. } | Source::Sub { pane, .. } => *pane,
    }
}

/// An agent as the Agents pane shows it.
#[derive(Clone, Debug, PartialEq)]
pub struct Agent {
    pub pane: PaneId,
    pub tab: Option<usize>,
    pub kind: AgentKind,
    pub name: String,
    /// What its terminal's title says it's doing, live (see [`Probe`]'s `task`); `None`
    /// while the title is only the program's name.
    pub task: Option<String>,
    /// The name its session goes by, when it has one (on an ssh host, `nex-host`'s: `build`,
    /// `build-2`).
    pub session: Option<String>,
    pub mark: Option<String>,
    pub cwd: Option<PathBuf>,
    pub status: Status,
    /// Since when it has been in `status`.
    pub since: Instant,
    pub source: Source,
    pub team: Team,
}

impl Agent {
    /// The pane it's on, if one shows it (see [`Probe`]'s).
    pub fn shown_pane(&self) -> Option<PaneId> {
        shown_pane(self.pane, &self.source)
    }

    /// Whether its app tile moves: a pipeline's does while it's running.
    pub fn tile_animates(&self) -> bool {
        tile_animates(self.kind, self.status)
    }

    /// Where its app tile's motion is at `time` (seconds), or `None` when the tile is still
    /// (see [`tile_phase`]).
    pub fn tile_phase(&self, animate: bool, time: f64) -> Option<f32> {
        tile_phase(self.kind, self.status, animate, time)
    }
}

/// Whether an agent of `kind` in `status` has a moving app tile: a pipeline that's running.
pub fn tile_animates(kind: AgentKind, status: Status) -> bool {
    kind == AgentKind::Pipeline && status == Status::Working
}

/// Where the app tile of an agent of `kind` in `status` is in its motion at `time` (seconds;
/// see [`icons::pipeline_phase`]), or `None` when it's still: it isn't a running pipeline, or
/// animations are off (`animate`).
///
/// [`icons::pipeline_phase`]: crate::ui::icons::pipeline_phase
pub fn tile_phase(kind: AgentKind, status: Status, animate: bool, time: f64) -> Option<f32> {
    (animate && tile_animates(kind, status)).then(|| crate::ui::icons::pipeline_phase(time))
}

/// The panes the Agents pane marks "Viewing": those of the tab on screen (on the Agents tab,
/// the terminals opened there, not the pinned pane). Every pane of a split counts; the one
/// with focus is the primary.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Viewing {
    /// In the tab's layout order (only the focused one when it's zoomed).
    pub panes: Vec<PaneId>,
    /// The pane with focus (when that's the Agents pane itself, the one it was opened from).
    pub primary: Option<PaneId>,
}

/// How an agent is on screen: in the focused pane, or in another pane of the same tab.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seen {
    Primary,
    Shared,
}

impl Viewing {
    /// Whether `a` is on screen, by the pane that shows it (a helper by its lead's; an agent
    /// on an ssh host by the pane attached to it, so a detached one never is).
    pub fn of(&self, a: &Agent) -> Option<Seen> {
        let pane = a.shown_pane()?;
        if self.primary == Some(pane) {
            Some(Seen::Primary)
        } else if self.panes.contains(&pane) {
            Some(Seen::Shared)
        } else {
            None
        }
    }
}

/// The agents as the [`Tracker`] sorted them, shared rather than copied: whoever draws them
/// holds the same `Arc`s. Reads as a slice of them.
#[derive(Clone, Debug, Default)]
pub struct Agents {
    list: Vec<Arc<Agent>>,
    generation: u64,
}

impl Agents {
    /// `agents` as a list of its own, with a generation no other list has.
    pub fn new(agents: Vec<Agent>) -> Agents {
        Agents { list: agents.into_iter().map(Arc::new).collect(), generation: next_generation() }
    }

    /// Which list this is: the same number as long as nothing in it changed, and a new one
    /// (never used before, by any list) when something did. What's worked out from the list
    /// is good until this changes.
    pub fn generation(&self) -> u64 {
        self.generation
    }
}

impl Deref for Agents {
    type Target = [Arc<Agent>];

    fn deref(&self) -> &[Arc<Agent>] {
        &self.list
    }
}

/// A generation no [`Agents`] has had (the empty default's is 0).
fn next_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

/// How many agents are in each state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub needs_you: usize,
    pub done: usize,
    pub working: usize,
    pub idle: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.needs_you + self.done + self.working + self.idle
    }

    /// Each non-empty state as a phrase, most urgent first: "1 needs you", "2 working".
    pub fn parts(&self) -> Vec<(Status, String)> {
        Status::ALL
            .into_iter()
            .filter_map(|s| {
                let n = match s {
                    Status::NeedsYou => self.needs_you,
                    Status::Done => self.done,
                    Status::Working => self.working,
                    Status::Idle => self.idle,
                };
                let text = match s {
                    Status::NeedsYou => format!("{n} need{} you", if n == 1 { "s" } else { "" }),
                    Status::Done => format!("{n} done"),
                    Status::Working => format!("{n} working"),
                    Status::Idle => format!("{n} idle"),
                };
                (n > 0).then_some((s, text))
            })
            .collect()
    }

    /// One line for the pane's header: "1 needs you · 2 working · 1 idle".
    pub fn summary(&self) -> String {
        if self.total() == 0 {
            return "No agents running".into();
        }
        self.parts().into_iter().map(|(_, t)| t).collect::<Vec<_>>().join(" · ")
    }

    /// The most urgent part alone, for the title bar's narrow label: "1 needs you".
    pub fn headline(&self) -> String {
        self.parts().into_iter().next().map_or_else(|| "None running".into(), |(_, t)| t)
    }
}

/// The counts with their wording, written out once when the counts change: the title bar and
/// the sidebar show it every frame.
struct Tally {
    counts: Counts,
    summary: String,
    headline: String,
}

impl Tally {
    fn new(counts: Counts) -> Tally {
        Tally { counts, summary: counts.summary(), headline: counts.headline() }
    }
}

impl Default for Tally {
    fn default() -> Tally {
        Tally::new(Counts::default())
    }
}

/// The agents seen at the last poll, and since when each has been doing what it does.
///
/// It also works out what the probes alone can't: an agent that stopped working while
/// nobody was looking at its pane is [`Status::Done`] (not idle) until someone does, and one
/// that became done or started needing you unseen lights its pane's tab (see
/// [`Tracker::wants_you`]), whether or not the program rang the bell.
#[derive(Default)]
pub struct Tracker {
    agents: Agents,
    /// How many of them are in each state, in numbers and words, as of the last change.
    tally: Tally,
    /// Each pipeline's status as its card shows it (see [`Tracker::pipeline_status`]), as of
    /// the last change.
    pipelines: HashMap<PaneId, Status>,
    since: HashMap<PaneId, (Status, Instant)>,
    /// The pane being looked at (the shown tab's focused pane, window focused).
    seen: Option<PaneId>,
    /// Agents that became done or started needing you since their pane was last looked at.
    unseen: HashSet<PaneId>,
    /// An agent joined `unseen` since [`Tracker::take_alerted`] was last called.
    alerted: bool,
}

impl Tracker {
    /// Replace the list with what `probes` say at `now`, sorted: needs you, done, working,
    /// idle, then by tab and pane. A poll that finds every agent as it was keeps the list
    /// (and its generation) as it is.
    pub fn update(&mut self, probes: Vec<Probe>, now: Instant) {
        let mut since = HashMap::with_capacity(probes.len());
        let mut unseen = HashSet::new();
        let mut agents: Vec<Agent> = probes
            .into_iter()
            .map(|p| {
                let looked_at = self.seen.is_some() && p.shown_pane() == self.seen;
                let was = self.since.get(&p.pane).map(|&(s, _)| s);
                let mut status = p.status(now);
                // A pipeline prints a line per stage: that's not it finishing (its stages
                // finishing is what counts, and they have cards of their own).
                // A sub-agent says itself whether it finished, and its agent's pane is where
                // anything that wants you shows.
                let finishes = p.kind != AgentKind::Pipeline && !p.team.sub;
                if finishes && status == Status::Idle && matches!(was, Some(Status::Working | Status::Done)) && !looked_at {
                    status = Status::Done;
                }
                let changed_into = !p.team.sub && was.is_some() && was != Some(status) && matches!(status, Status::Done | Status::NeedsYou);
                if !looked_at && (changed_into || self.unseen.contains(&p.pane)) {
                    self.alerted |= !self.unseen.contains(&p.pane);
                    unseen.insert(p.pane);
                }
                let at = match self.since.get(&p.pane) {
                    Some(&(was, at)) if was == status => at,
                    _ => now,
                };
                since.insert(p.pane, (status, at));
                Agent {
                    pane: p.pane,
                    tab: p.tab,
                    kind: p.kind,
                    name: p.name(),
                    task: p.task().map(str::to_string),
                    session: p.session_name.clone(),
                    mark: p.mark.clone(),
                    cwd: p.cwd.clone(),
                    status,
                    since: at,
                    source: p.source.clone(),
                    team: p.team.clone(),
                }
            })
            .collect();
        agents.sort_by_key(order);
        self.since = since;
        self.unseen = unseen;
        let same = agents.len() == self.agents.len() && agents.iter().zip(self.agents.iter()).all(|(new, old)| *new == **old);
        if !same {
            self.agents = Agents::new(agents);
            self.recount();
        }
    }

    /// The list changed: count it again, and reword the counts if they came out different.
    fn recount(&mut self) {
        self.pipelines = pipeline_statuses(&self.agents);
        let mut c = Counts::default();
        for a in self.agents.iter().filter(|a| a.kind != AgentKind::Pipeline && !a.team.sub) {
            match a.status {
                Status::NeedsYou => c.needs_you += 1,
                Status::Done => c.done += 1,
                Status::Working => c.working += 1,
                Status::Idle => c.idle += 1,
            }
        }
        if c != self.tally.counts {
            self.tally = Tally::new(c);
        }
    }

    /// Someone is looking at `pane` (or at no pane: `None`), as of `now`. An agent on it that
    /// was done is idle now, and its tab stops asking for you. Cheap enough for every frame.
    pub fn mark_seen(&mut self, pane: Option<PaneId>, now: Instant) {
        self.seen = pane;
        let Some(pane) = pane else { return };
        let mut changed = false;
        for a in self.agents.list.iter_mut().filter(|a| a.shown_pane() == Some(pane)) {
            self.unseen.remove(&a.pane);
            if a.status == Status::Done {
                let a = Arc::make_mut(a);
                a.status = Status::Idle;
                a.since = now;
                self.since.insert(a.pane, (Status::Idle, now));
                changed = true;
            }
        }
        if changed {
            self.agents.list.sort_by_key(|a| order(a));
            self.agents.generation = next_generation();
            self.recount();
        }
    }

    /// Whether an agent shown in `pane` became done or started needing you since the pane
    /// was last looked at: its tab gets the attention dot.
    pub fn wants_you(&self, pane: PaneId) -> bool {
        !self.unseen.is_empty() && self.agents.iter().any(|a| self.unseen.contains(&a.pane) && a.shown_pane() == Some(pane))
    }

    /// Whether an agent started wanting you since the last call (to flag the window).
    pub fn take_alerted(&mut self) -> bool {
        std::mem::take(&mut self.alerted)
    }

    pub fn agents(&self) -> &Agents {
        &self.agents
    }

    /// The status of the pipeline with key `pane`, as its card in the Agents pane shows it: a
    /// pipeline with stages is as far along as they are (its runner printing doesn't count;
    /// see [`groups`]), one inside a team its own. `None` if it isn't a pipeline.
    ///
    /// [`groups`]: super::group::groups
    pub fn pipeline_status(&self, pane: PaneId) -> Option<Status> {
        self.pipelines.get(&pane).copied()
    }

    /// How many agents are in each state (a pipeline isn't one: its stages are; nor is a
    /// sub-agent: its agent is). Counted when the list changes, not here.
    pub fn counts(&self) -> Counts {
        self.tally.counts
    }

    /// [`Counts::summary`] of [`Tracker::counts`], written out when the counts last changed.
    pub fn summary(&self) -> &str {
        &self.tally.summary
    }

    /// [`Counts::headline`] of [`Tracker::counts`], written out when the counts last changed.
    pub fn headline(&self) -> &str {
        &self.tally.headline
    }
}

/// Each pipeline's status as the Agents pane shows it: a team's lead by its stages, any
/// other by its own.
fn pipeline_statuses(agents: &[Arc<Agent>]) -> HashMap<PaneId, Status> {
    if !agents.iter().any(|a| a.kind == AgentKind::Pipeline) {
        return HashMap::new();
    }
    let leads: HashMap<PaneId, Status> = super::group::groups(agents).iter().map(|g| (g.key(), g.lead.status)).collect();
    agents
        .iter()
        .filter(|a| a.kind == AgentKind::Pipeline)
        .map(|a| (a.pane, leads.get(&a.pane).copied().unwrap_or(a.status)))
        .collect()
}

/// The order the list is in: by status, then tab, then pane.
fn order(a: &Agent) -> (Status, usize, PaneId) {
    (a.status, a.tab.unwrap_or(usize::MAX), a.pane)
}

/// A short elapsed time: "now", "45s", "12m", "3h", "2d".
pub fn elapsed(d: Duration) -> String {
    let s = d.as_secs();
    match s {
        0..5 => "now".into(),
        5..60 => format!("{s}s"),
        60..3600 => format!("{}m", s / 60),
        3600..86400 => format!("{}h", s / 3600),
        _ => format!("{}d", s / 86400),
    }
}
