//! Which panes run an agent, and what each one is doing.
//!
//! Every process poll the app hands the [`Tracker`] one [`Probe`] per agent pane (what the
//! terminal knows about it, plus Claude Code's session file when there is one). The tracker
//! turns those into [`Agent`]s sorted for display, and remembers since when each has been
//! in its current state.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
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
/// Only agents on ssh hosts have one; local panes' agents are always on their own.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Team {
    /// The card of the agent that started it.
    pub parent: Option<PaneId>,
    /// Its role (`reviewer`) or pipeline stage (`build`).
    pub role: Option<String>,
    /// When it started (unix ms).
    pub started: u64,
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

    /// What to call it: the task in its title, else its session's name, else its folder.
    fn name(&self) -> String {
        let title = self.title.trim();
        let generic = title.is_empty()
            || title.eq_ignore_ascii_case(self.kind.label())
            || title.eq_ignore_ascii_case(self.kind.process());
        if !generic {
            return title.to_string();
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
        Source::Remote { pane, .. } => *pane,
    }
}

/// An agent as the Agents pane shows it.
#[derive(Clone, Debug)]
pub struct Agent {
    pub pane: PaneId,
    pub tab: Option<usize>,
    pub kind: AgentKind,
    pub name: String,
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

/// The agents seen at the last poll, and since when each has been doing what it does.
///
/// It also works out what the probes alone can't: an agent that stopped working while
/// nobody was looking at its pane is [`Status::Done`] (not idle) until someone does, and one
/// that became done or started needing you unseen lights its pane's tab (see
/// [`Tracker::wants_you`]), whether or not the program rang the bell.
#[derive(Default)]
pub struct Tracker {
    agents: Vec<Agent>,
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
    /// idle, then by tab and pane.
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
                let finishes = p.kind != AgentKind::Pipeline;
                if finishes && status == Status::Idle && matches!(was, Some(Status::Working | Status::Done)) && !looked_at {
                    status = Status::Done;
                }
                let changed_into = was.is_some() && was != Some(status) && matches!(status, Status::Done | Status::NeedsYou);
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
                    mark: p.mark.clone(),
                    cwd: p.cwd.clone(),
                    status,
                    since: at,
                    source: p.source.clone(),
                    team: p.team.clone(),
                }
            })
            .collect();
        Self::sort(&mut agents);
        self.agents = agents;
        self.since = since;
        self.unseen = unseen;
    }

    fn sort(agents: &mut [Agent]) {
        agents.sort_by_key(|a| (a.status, a.tab.unwrap_or(usize::MAX), a.pane));
    }

    /// Someone is looking at `pane` (or at no pane: `None`), as of `now`. An agent on it that
    /// was done is idle now, and its tab stops asking for you. Cheap enough for every frame.
    pub fn mark_seen(&mut self, pane: Option<PaneId>, now: Instant) {
        self.seen = pane;
        let Some(pane) = pane else { return };
        let mut changed = false;
        for a in self.agents.iter_mut().filter(|a| a.shown_pane() == Some(pane)) {
            self.unseen.remove(&a.pane);
            if a.status == Status::Done {
                a.status = Status::Idle;
                a.since = now;
                self.since.insert(a.pane, (Status::Idle, now));
                changed = true;
            }
        }
        if changed {
            Self::sort(&mut self.agents);
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

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    /// How many agents are in each state (a pipeline isn't one: its stages are).
    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for a in self.agents.iter().filter(|a| a.kind != AgentKind::Pipeline) {
            match a.status {
                Status::NeedsYou => c.needs_you += 1,
                Status::Done => c.done += 1,
                Status::Working => c.working += 1,
                Status::Idle => c.idle += 1,
            }
        }
        c
    }
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
