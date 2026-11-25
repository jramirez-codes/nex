//! A session the daemon holds and what it's doing: [`Session`], its [`Status`], what Claude
//! Code's session file says ([`Claude`]), its [`SubAgent`]s, and a pipeline's title.

use serde::{Deserialize, Serialize};

#[cfg(doc)]
use crate::proto::Request;

/// The kind of a session that is a plain shell (a [`Request::Launch`] with no `argv`), as
/// opposed to an agent: the Agents pane leaves these out, and the daemon forgets one as soon
/// as it exits.
pub const SHELL_KIND: &str = "shell";
/// The kind of a session running `nex-host pipe`: it takes its messages with `recv`, so
/// none are typed into it.
pub const PIPELINE_KIND: &str = "pipeline";
/// Output this recent (and not just the echo of typing) counts as working.
pub const BUSY_WINDOW_MS: u64 = 1500;
/// What sits between the two halves of the terminal title a pipeline's runner sets, for the
/// Agents pane's card: how its work stands (`2 building · 1 in review`), then the last
/// thing that happened. See [`pipeline_title`].
pub const TITLE_SEP: &str = " — ";
/// The most characters of the last event a pipeline puts in its title.
pub(super) const TITLE_EVENT_MAX: usize = 160;

/// The title a pipeline's runner sets: `counts`, then (when something has happened)
/// [`TITLE_SEP`] and `event` on one line, cut to a length a title can carry.
pub fn pipeline_title(counts: &str, event: &str) -> String {
    let shown: String = event.chars().filter(|c| c.is_whitespace() || !c.is_control()).collect();
    let line = shown.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut event: String = line.chars().take(TITLE_EVENT_MAX).collect();
    if line.chars().count() > TITLE_EVENT_MAX {
        event.push('…');
    }
    if event.is_empty() { counts.trim().to_string() } else { format!("{}{TITLE_SEP}{event}", counts.trim()) }
}

/// A [`pipeline_title`] taken apart again: its counts and its last event (either may be
/// empty).
pub fn split_pipeline_title(title: &str) -> (&str, &str) {
    let (counts, event) = title.split_once(TITLE_SEP).unwrap_or((title, ""));
    (counts.trim(), event.trim())
}

/// What an agent is doing, most urgent first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    NeedsYou,
    Working,
    Idle,
    /// Its program ended; the session stays listed until it's dismissed.
    Exited,
}

impl Status {
    /// Claude Code's own word for its state (its session file's `status`), if it's one nex knows.
    /// Never [`Status::Exited`]. nex uses this for its local agents too.
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

    pub fn label(self) -> &'static str {
        match self {
            Status::NeedsYou => "needs you",
            Status::Working => "working",
            Status::Idle => "idle",
            Status::Exited => "exited",
        }
    }
}

/// What Claude Code's session file says about the agent.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Claude {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
}

/// A sub-agent an agent is running inside its own process: Claude Code's Task tool, or a
/// thread Codex spawned. See `crate::subagents`.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SubAgent {
    /// Its id, unique within its agent.
    pub id: String,
    /// What it was asked to do (Claude Code's `description`).
    pub name: String,
    /// Its type (`Explore`), if it has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// When it started (unix ms).
    pub started: u64,
    /// It finished (it's shown with a tick for a while, then dropped).
    #[serde(default)]
    pub done: bool,
}

/// One agent the daemon holds.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: String,
    /// `claude`, `codex`, or the program's name.
    pub kind: String,
    pub name: String,
    pub cwd: String,
    pub argv: Vec<String>,
    pub pid: u32,
    /// Unix time in milliseconds.
    pub started: u64,
    /// The title the program last set.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// It rang the bell while no pane was attached.
    #[serde(default)]
    pub bell: bool,
    /// A pane is showing it right now.
    #[serde(default)]
    pub attached: bool,
    /// Unix ms of its last output and of the last key sent to it.
    pub last_output: u64,
    pub last_input: u64,
    /// Its exit code (128 + the signal when a signal ended it), once it has ended.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exit: Option<i32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub claude: Option<Claude>,
    /// For a [`SHELL_KIND`] session: the agent (`claude`, `codex`) its foreground job is,
    /// when someone typed one at its prompt. See [`Session::agent_kind`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub running: Option<String>,
    pub status: Status,
    /// The agent that started it with `spawn` (it ends when that one does).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
    /// What it's for, as `spawn --role` put it (`reviewer`), or its pipeline stage.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub role: Option<String>,
    /// Unix ms of the last message it was sent, 0 if none.
    #[serde(default)]
    pub last_message: u64,
    /// The sub-agents it is running (or just finished) inside its own process.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subagents: Vec<SubAgent>,
}

impl Session {
    /// The agent it is: its `kind`, or for a shell, the agent running in it (`running`).
    pub fn agent_kind(&self) -> &str {
        self.running.as_deref().unwrap_or(&self.kind)
    }

    /// The best guess at what it's doing at `now` (unix ms): ended, then Claude Code's own
    /// word, then the bell, then the title mark of a CLI that keeps one there
    /// (`AgentCli::title_status`: Claude Code's ✳ waiting, a spinner frame working),
    /// then whether it has been printing on its own. nex's `agents::model::Probe::status`
    /// applies the same rules to its local panes; keep the two in step.
    pub fn compute_status(&self, now: u64) -> Status {
        if self.exit.is_some() {
            return Status::Exited;
        }
        let reported = self.claude.as_ref().and_then(|c| c.status.as_deref()).and_then(Status::from_claude);
        if reported == Some(Status::NeedsYou) || self.bell {
            return Status::NeedsYou;
        }
        if let Some(s) = reported {
            return s;
        }
        if let Some(cli) = crate::clis::by_kind(self.agent_kind())
            && let Some(mark) = self.title.as_deref().and_then(title_mark)
            && let Some(s) = cli.title_status(mark.encode_utf8(&mut [0; 4]))
        {
            return s;
        }
        let recent = |t: u64| now.saturating_sub(t) < BUSY_WINDOW_MS;
        if recent(self.last_output) && !recent(self.last_input) { Status::Working } else { Status::Idle }
    }
}

/// The icon leading a title (`✳ Fixing tests` → `✳`), if it starts with one followed by a space.
fn title_mark(title: &str) -> Option<char> {
    let mut chars = title.chars();
    let first = chars.next()?;
    let icon = !first.is_alphanumeric() && !first.is_ascii() || first == '*';
    (icon && chars.next().is_none_or(char::is_whitespace)).then_some(first)
}
