//! The agent CLIs nex knows (Claude Code, Codex), each a module of its own behind one
//! interface, [`AgentCli`]: how to tell its command line, where it keeps its files, what its
//! session file and sub-agents are, how to print a sub-agent's transcript, what its screen
//! says, and how nex-host starts one on a task. The rest of nex and nex-host only ever ask
//! the [registry](ALL), so they work the same for every CLI.
//!
//! ```text
//! clis/
//!   mod.rs      the interface (AgentCli), a handle on one (Cli), the registry
//!   claude/     Claude Code: mod.rs (its AgentCli), session.rs (~/.claude/sessions/<pid>.json),
//!               subagents.rs (its Task tool's agents), transcript.rs, resume.rs
//!   codex/      Codex: mod.rs (its AgentCli), rollouts.rs (its threads), transcript.rs
//!   tests.rs    what every registered CLI must do
//! ```
//!
//! Adding a CLI is adding a module here that implements [`AgentCli`] and listing it in
//! [`ALL`].

use std::fmt;
use std::hash::{Hash, Hasher};
use std::ops::Deref;
use std::path::{Path, PathBuf};

use crate::proto::{Claude, Status, SubAgent};
use crate::transcript::Printer;

pub mod claude;
pub mod codex;

#[cfg(test)]
mod tests;

/// Claude Code.
pub const CLAUDE: Cli = Cli(&claude::ClaudeCode);
/// Codex.
pub const CODEX: Cli = Cli(&codex::Codex);

/// Every CLI nex knows, in the order they're tried.
pub const ALL: &[Cli] = &[CLAUDE, CODEX];

/// What `nex-host spawn` starts when it isn't told.
pub const DEFAULT: Cli = CLAUDE;

/// The CLI nex-host reports as `kind`.
pub fn by_kind(kind: &str) -> Option<Cli> {
    ALL.iter().copied().find(|c| c.kind() == kind)
}

/// The CLI `argv` runs (directly or under node / bun), if any.
pub fn detect(argv: &[String]) -> Option<Cli> {
    ALL.iter().copied().find(|c| c.program_len(argv).is_some())
}

/// Whose sub-agent transcript starts with `first_line`: the CLI that claims it, else Claude
/// Code (its transcripts have no header of their own).
pub fn of_transcript(first_line: &str) -> Cli {
    ALL.iter().copied().find(|c| c.claims_transcript(first_line)).unwrap_or(CLAUDE)
}

/// When process `pid` started, as session files record it to tell it from an earlier one
/// with the same pid ([`AgentCli::session`]): `/proc/<pid>/stat` field 22, on Linux.
/// Elsewhere `None`, and the pid alone has to match.
pub fn proc_start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after `(comm)` start at field 3 (state), so field 22 is the 20th of them.
    stat[stat.rfind(')')? + 1..].split_whitespace().nth(19).map(str::to_string)
}

/// The last part of a path: `/usr/bin/node` → `node`.
fn base(s: &str) -> &str {
    s.rsplit('/').next().unwrap_or(s)
}

/// A running agent, as its CLI's files are looked up for it.
#[derive(Clone, Copy, Debug, Default)]
pub struct Running<'a> {
    pub pid: u32,
    /// The folder it works in.
    pub cwd: &'a str,
    /// What its session file says, for a CLI that keeps one ([`AgentCli::session`]).
    pub session: Option<&'a Claude>,
}

/// What nex-host writes for a CLI so it finds the agents' manual (`nex-host skill install`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Manual {
    /// A file of its own, at `path` in the CLI's home, holding `text`.
    File { path: &'static str, text: String },
    /// A marked block holding `text`, in the file at `path` in the CLI's home, which the user
    /// has their own lines in too.
    Block { path: &'static str, text: String },
}

/// One agent CLI. Each lives in a module of its own under `clis/`; see [`ALL`].
///
/// Everything a CLI writes is its own state, not an interface: implementations read it
/// leniently, and anything unexpected means "nothing".
pub trait AgentCli: Sync {
    /// What nex-host reports as the agent's `kind`, `--kind` names, and the program's name
    /// (`claude`).
    fn kind(&self) -> &'static str;
    /// What it's called on screen (`Claude Code`).
    fn label(&self) -> &'static str;
    /// The end of the path of the script its npm package runs under node or bun.
    fn script(&self) -> &'static str;
    /// The variable naming its home folder (`CLAUDE_CONFIG_DIR`), and the folder in the
    /// user's home it is otherwise (`.claude`).
    fn home_var(&self) -> &'static str;
    fn home_folder(&self) -> &'static str;

    /// How many leading words of `argv` are this program: itself (1) or `node <script>` (2).
    /// `None` if `argv` doesn't run it.
    fn program_len(&self, argv: &[String]) -> Option<usize> {
        let is = |s: &str| base(s) == self.kind() || s.ends_with(self.script());
        match argv {
            [runtime, script, ..] if matches!(base(runtime), "node" | "bun") => is(script).then_some(2),
            [first, ..] => is(first).then_some(1),
            [] => None,
        }
    }

    /// Its home folder when the user's is `user_home`: the absolute path in
    /// [`home_var`](AgentCli::home_var), else [`home_folder`](AgentCli::home_folder) there.
    fn home_in(&self, user_home: Option<PathBuf>) -> Option<PathBuf> {
        match std::env::var_os(self.home_var()).map(PathBuf::from).filter(|p| p.is_absolute()) {
            Some(p) => Some(p),
            None => Some(user_home?.join(self.home_folder())),
        }
    }

    /// [`home_in`](AgentCli::home_in) with `$HOME` (on an ssh host).
    fn home(&self) -> Option<PathBuf> {
        self.home_in(std::env::var_os("HOME").map(PathBuf::from))
    }

    /// What its session file says about process `pid` (started at `start`, as
    /// [`proc_start`] reads it, when known), for a CLI that keeps one.
    fn session(&self, _home: &Path, _pid: u32, _start: Option<&str>) -> Option<Claude> {
        None
    }

    /// The sub-agents `agent` runs inside its own process (or just finished), as of `now`
    /// (unix ms). See [`crate::subagents`].
    fn subagents(&self, home: &Path, agent: &Running, now: u64) -> Vec<SubAgent>;

    /// The transcript file of `agent`'s sub-agent `id`.
    fn subagent_transcript(&self, home: &Path, agent: &Running, id: &str) -> Option<PathBuf>;

    /// Whether a sub-agent transcript whose first line is `first_line` is one of its own.
    /// See [`of_transcript`].
    fn claims_transcript(&self, _first_line: &str) -> bool {
        false
    }

    /// Print one line of its transcript (parsed JSON) with `out`. Anything it doesn't know
    /// prints nothing.
    fn render(&self, line: &serde_json::Value, out: &mut Printer);

    /// What a sub-agent's tab is called, from its transcript at `path` and what's beside it.
    fn transcript_title(&self, path: &Path) -> Option<String>;

    /// A file beside the transcript at `path` that says when the sub-agent has finished
    /// (read with [`finished_in`](AgentCli::finished_in) whenever it changes), for a CLI
    /// that doesn't say so in the transcript itself.
    fn finish_file(&self, _path: &Path) -> Option<PathBuf> {
        None
    }

    /// Whether [`finish_file`](AgentCli::finish_file) `file` says it has finished.
    fn finished_in(&self, _file: &Path) -> bool {
        false
    }

    /// What it's doing, from the icon leading its terminal title, for a CLI that marks it
    /// there.
    fn title_status(&self, _mark: &str) -> Option<Status> {
        None
    }

    /// What its screen shows while it asks the user something before it has a session file
    /// to say so (a start-up dialog). A CLI with any waits for its first screen to settle
    /// before a message is typed in, so the message can't answer one.
    fn dialog_marks(&self) -> &'static [&'static str] {
        &[]
    }

    /// Whether `nex-host spawn` hands it its task as the command line's last argument
    /// (it starts on it at once), rather than typing it in once it's up.
    fn task_on_command_line(&self) -> bool {
        true
    }

    /// Arguments after the task, when nex-host chose the command line, so it can report
    /// back without stopping at a permission prompt.
    fn report_args(&self) -> &'static [&'static str] {
        &[]
    }

    /// Its command line rewritten to pick up conversation `session_id` (else its latest)
    /// when nex restores the pane it ran in. `None`: run it again as it was.
    fn resume(&self, _argv: &[String], _session_id: Option<&str>) -> Option<Vec<String>> {
        None
    }

    /// Where `nex-host skill install` puts the agents' `manual` for it, if anywhere.
    fn manual(&self, _manual: &str) -> Option<Manual> {
        None
    }
}

/// A registered CLI: a handle that can be copied, compared and hashed (by its
/// [`kind`](AgentCli::kind)).
#[derive(Clone, Copy)]
pub struct Cli(&'static dyn AgentCli);

impl Deref for Cli {
    type Target = dyn AgentCli;

    fn deref(&self) -> &Self::Target {
        self.0
    }
}

impl PartialEq for Cli {
    fn eq(&self, other: &Cli) -> bool {
        self.kind() == other.kind()
    }
}

impl Eq for Cli {}

impl Hash for Cli {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind().hash(state);
    }
}

impl fmt::Debug for Cli {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Cli({})", self.kind())
    }
}
