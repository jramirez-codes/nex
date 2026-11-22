//! Which panes run an agent, and what each one is doing.
//!
//! Every process poll the app hands the [`Tracker`] one [`Probe`] per agent pane (what the
//! terminal knows about it, plus Claude Code's session file when there is one). The tracker
//! turns those into [`Agent`]s sorted for display, and remembers since when each has been
//! in its current state.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::layout::PaneId;

/// Output this recent (and not just the echo of typing) counts as working.
const BUSY_WINDOW: Duration = Duration::from_millis(1500);
/// Claude Code's title glyph while it waits at its prompt; spinner frames replace it while
/// it works.
const CLAUDE_IDLE_MARK: &str = "✳";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AgentKind {
    Claude,
    Codex,
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

    /// The process name its app tile is drawn for (see `ui::icons::app_tile`).
    pub fn process(self) -> &'static str {
        match self {
            AgentKind::Claude => "claude",
            AgentKind::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            AgentKind::Claude => "Claude Code",
            AgentKind::Codex => "Codex",
        }
    }
}

/// What an agent is doing. Ordered by how much it wants the user: the list shows agents
/// that need you first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Status {
    NeedsYou,
    Working,
    Idle,
}

impl Status {
    pub const ALL: [Status; 3] = [Status::NeedsYou, Status::Working, Status::Idle];

    pub fn label(self) -> &'static str {
        match self {
            Status::NeedsYou => "Needs you",
            Status::Working => "Working",
            Status::Idle => "Idle",
        }
    }

    /// Claude Code's own word for its state (the session file's `status`), if nex knows it.
    pub fn from_claude(status: &str) -> Option<Status> {
        let s = status.to_ascii_lowercase();
        let has = |words: &[&str]| words.iter().any(|w| s.contains(w));
        if has(&["wait", "permission", "input", "blocked", "attention", "approval"]) {
            Some(Status::NeedsYou)
        } else if has(&["busy", "running", "working", "thinking", "active", "streaming", "tool"]) {
            Some(Status::Working)
        } else if has(&["idle", "ready", "done", "complete"]) {
            Some(Status::Idle)
        } else {
            None
        }
    }
}

/// Where an agent runs. Only this machine's panes for now; agents on an ssh host are next.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Source {
    Local,
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
}

impl Probe {
    /// The best guess at what it's doing: Claude Code's own word first, then the bell, then
    /// its title spinner, then whether it's been printing on its own.
    pub fn status(&self, now: Instant) -> Status {
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
            return if mark == CLAUDE_IDLE_MARK { Status::Idle } else { Status::Working };
        }
        let recent = |t: Instant| now.saturating_duration_since(t) < BUSY_WINDOW;
        if recent(self.last_output) && !recent(self.last_input) { Status::Working } else { Status::Idle }
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
    #[allow(dead_code, reason = "only local agents so far; ssh hosts' agents are next")]
    pub source: Source,
}

/// How many agents are in each state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Counts {
    pub needs_you: usize,
    pub working: usize,
    pub idle: usize,
}

impl Counts {
    pub fn total(&self) -> usize {
        self.needs_you + self.working + self.idle
    }

    /// Each non-empty state as a phrase, most urgent first: "1 needs you", "2 working".
    pub fn parts(&self) -> Vec<(Status, String)> {
        Status::ALL
            .into_iter()
            .filter_map(|s| {
                let n = match s {
                    Status::NeedsYou => self.needs_you,
                    Status::Working => self.working,
                    Status::Idle => self.idle,
                };
                let text = match s {
                    Status::NeedsYou => format!("{n} need{} you", if n == 1 { "s" } else { "" }),
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
#[derive(Default)]
pub struct Tracker {
    agents: Vec<Agent>,
    since: HashMap<PaneId, (Status, Instant)>,
}

impl Tracker {
    /// Replace the list with what `probes` say at `now`, sorted: needs you, working, idle,
    /// then by tab and pane.
    pub fn update(&mut self, probes: Vec<Probe>, now: Instant) {
        let mut since = HashMap::with_capacity(probes.len());
        let mut agents: Vec<Agent> = probes
            .into_iter()
            .map(|p| {
                let status = p.status(now);
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
                    source: Source::Local,
                }
            })
            .collect();
        agents.sort_by_key(|a| (a.status, a.tab.unwrap_or(usize::MAX), a.pane));
        self.agents = agents;
        self.since = since;
    }

    pub fn agents(&self) -> &[Agent] {
        &self.agents
    }

    pub fn counts(&self) -> Counts {
        let mut c = Counts::default();
        for a in &self.agents {
            match a.status {
                Status::NeedsYou => c.needs_you += 1,
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
