//! The first line a client sends ([`Request`]) and the daemon's answer ([`Response`]).

use serde::{Deserialize, Serialize};

use crate::proto::{Message, MessageKind, Payload, Session};
#[cfg(doc)]
use crate::proto::{SHELL_KIND, USER, version_line};

/// How long a `recv` waits when it doesn't say.
pub const RECV_DEFAULT_MS: u64 = 600 * 1000;

/// A request: the first line a client sends.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Start `argv` in a new session (no `argv`: the user's login shell, interactive, as a
    /// [`SHELL_KIND`] session). Answered with its `id`.
    Launch {
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        name: Option<String>,
        /// Absolute, or starting with `~/`. The daemon's home when missing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cols: Option<u16>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        rows: Option<u16>,
        /// The environment to run it in (`run` sends the shell's own, so the agent finds
        /// what it would there: `PATH`, API keys). Without it, the user's login shell's.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<Vec<(String, String)>>,
        /// The agent it's started for (`pipe` from an agent): it shows under that one, in
        /// `ls` and nex's Agents pane. Unlike a spawned helper it outlives it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
    },
    List,
    Watch,
    /// End a session's program (it's forgotten once it has exited).
    Kill {
        id: String,
    },
    /// Drop an exited session from the list.
    Forget {
        id: String,
    },
    /// Show a session: the connection becomes its terminal, `cols` by `rows`.
    Attach {
        id: String,
        cols: u16,
        rows: u16,
    },
    /// Start an agent to do `task`: a [`Request::Launch`] whose first message is the task.
    /// Answered with its `id`.
    Spawn {
        name: String,
        /// `claude` (the default), `codex`, or anything else to run `argv`.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<String>,
        /// The command; empty: just `kind`.
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        argv: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        cwd: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        env: Option<Vec<(String, String)>>,
        /// Who's asking (their `NEX_AGENT_ID`); none when it's someone at a shell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        parent: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        role: Option<String>,
        task: String,
    },
    /// Send a message. Answered with its `id`, and in `text` when it's waiting to be typed
    /// in (the recipient is asking the user something).
    Send {
        /// The sender's `NEX_AGENT_ID`; none when it's someone at a shell.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        /// An id, a name, `@parent`, `@children`, `@task` (whoever gave the sender its current
        /// task) or `user`.
        to: String,
        kind: MessageKind,
        #[serde(flatten)]
        payload: Payload,
        /// Type it in even though the recipient is waiting for the user.
        #[serde(default)]
        force: bool,
    },
    /// Take the next message for `me` (none: [`USER`]) that matches, waiting up to
    /// `timeout_ms` for one. Answered with it in `messages`, or an error when time runs out.
    Recv {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        me: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        from: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply_to: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        kind: Option<MessageKind>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        timeout_ms: Option<u64>,
    },
    /// What a session's screen shows, as plain text: its last `lines` lines (scrollback
    /// included). Answered in `text`.
    Read {
        id: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        lines: Option<usize>,
    },
    /// The daemon's [`version_line`], answered in `text`. A daemon from before this request
    /// answers `bad request`: it's older than any client that asks.
    Version,
}

/// The daemon's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Response {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sessions: Option<Vec<Session>>,
    /// The host's home folder, with `list` and `watch`, for showing folders as `~/…`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// With `recv`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub messages: Option<Vec<Message>>,
    /// With `read`: the screen. With `send`: something the sender should know. With
    /// `version`: the daemon's version line.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// Added by `nex-host watch` (not the daemon) to each line when the daemon it watches is
    /// an older nex-host than the installed one, still running because agents are: it hands
    /// over once they're done. Its version line, or `older` when it's too old to say.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub daemon_version: Option<String>,
}

impl Response {
    pub fn ok() -> Self {
        Self { ok: true, ..Self::default() }
    }

    pub fn error(e: impl Into<String>) -> Self {
        Self { ok: false, error: Some(e.into()), ..Self::default() }
    }

    pub fn sessions(sessions: Vec<Session>) -> Self {
        let home = std::env::var("HOME").ok().filter(|h| !h.is_empty());
        Self { ok: true, sessions: Some(sessions), home, ..Self::default() }
    }
}
