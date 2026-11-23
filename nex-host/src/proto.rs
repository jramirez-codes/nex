//! What nex and `nex-host` say to each other.
//!
//! Every connection to the daemon's socket starts with one [`Request`] as a JSON line, and
//! the daemon answers with one [`Response`] line. Then:
//! - `watch` gets a new [`Response`] line with every session whenever something changes
//!   (and every [`HEARTBEAT_SECS`] regardless), until the client hangs up;
//! - `attach` turns into a terminal: the daemon sends the agent's screen and then its output
//!   as raw bytes, and the client sends [`frame`]s (keys, resizes) back;
//! - `recv` waits (up to its timeout) for a message before it answers;
//! - everything else is done after its answer.
//!
//! Agents talk to each other with [`Message`]s: `send` puts one in the recipient's mailbox
//! and types it into the recipient's terminal (`nex-host/src/deliver.rs` has the format it
//! arrives in); `recv` takes one out of the caller's mailbox instead.
//!
//! `nex-host connect` passes one such connection through its stdin and stdout, for clients
//! that reach the host over an ssh channel rather than a terminal (nex's own ssh client).

use serde::{Deserialize, Serialize};

/// Bumped whenever a change here would confuse the other side.
pub const PROTOCOL: u32 = 3;
/// The `nex-host` version, which nex compares to its own to decide whether to reinstall.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often `watch` resends the session list when nothing has changed: a client that hears
/// nothing for longer than this knows the connection is gone.
pub const HEARTBEAT_SECS: u64 = 15;
/// The kind of a session that is a plain shell (a [`Request::Launch`] with no `argv`), as
/// opposed to an agent: the Agents pane leaves these out, and the daemon forgets one as soon
/// as it exits.
pub const SHELL_KIND: &str = "shell";
/// The kind of a session running `nex-host pipe`: it takes its messages with `recv`, so
/// none are typed into it.
pub const PIPELINE_KIND: &str = "pipeline";
/// Output this recent (and not just the echo of typing) counts as working.
pub const BUSY_WINDOW_MS: u64 = 1500;
/// How long a `recv` waits when it doesn't say.
pub const RECV_DEFAULT_MS: u64 = 600 * 1000;
/// Claude Code's title glyph while it waits at its prompt; spinner frames replace it while
/// it works.
pub const CLAUDE_IDLE_MARK: char = '✳';
/// What sits between the two halves of the terminal title a pipeline's runner sets, for the
/// Agents pane's card: how its work stands (`2 building · 1 in review`), then the last
/// thing that happened. See [`pipeline_title`].
pub const TITLE_SEP: &str = " — ";
/// The most characters of the last event a pipeline puts in its title.
const TITLE_EVENT_MAX: usize = 160;

/// The title a pipeline's runner sets: `counts`, then (when something has happened)
/// [`TITLE_SEP`] and `event` on one line, cut to a length a title can carry.
pub fn pipeline_title(counts: &str, event: &str) -> String {
    let line: String = event.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut event: String = line.chars().filter(|c| !c.is_control()).take(TITLE_EVENT_MAX).collect();
    if event.len() < line.len() {
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

/// What `nex-host --version` prints: `nex-host 0.11.0 (protocol 2)`.
pub fn version_line() -> String {
    format!("nex-host {VERSION} (protocol {PROTOCOL})")
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
    /// word, then the bell, then Claude's title mark (✳ waiting, a spinner frame working),
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
        if self.agent_kind() == "claude"
            && let Some(mark) = self.title.as_deref().and_then(title_mark)
        {
            return if mark == CLAUDE_IDLE_MARK { Status::Idle } else { Status::Working };
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
    Kill { id: String },
    /// Drop an exited session from the list.
    Forget { id: String },
    /// Show a session: the connection becomes its terminal, `cols` by `rows`.
    Attach { id: String, cols: u16, rows: u16 },
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
}

/// What a message is for, which tells the agent reading it what's expected of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageKind {
    /// A job: do it, then report with `done`.
    Task,
    /// A question: answer it with `send --reply-to`.
    Ask,
    /// The answer to something the recipient asked.
    Reply,
    /// The sender finished the task it was given; this is its report.
    Done,
    /// For the recipient to take into account; no answer expected.
    Note,
}

impl MessageKind {
    pub const ALL: [MessageKind; 5] = [MessageKind::Task, MessageKind::Ask, MessageKind::Reply, MessageKind::Done, MessageKind::Note];

    pub fn label(self) -> &'static str {
        match self {
            MessageKind::Task => "task",
            MessageKind::Ask => "ask",
            MessageKind::Reply => "reply",
            MessageKind::Done => "done",
            MessageKind::Note => "note",
        }
    }

    pub fn parse(s: &str) -> Option<MessageKind> {
        Self::ALL.into_iter().find(|k| k.label() == s)
    }
}

/// How a `done` turned out: what pipelines branch on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Verdict {
    Approved,
    Changes,
    Failed,
}

impl Verdict {
    pub const ALL: [Verdict; 3] = [Verdict::Approved, Verdict::Changes, Verdict::Failed];

    pub fn label(self) -> &'static str {
        match self {
            Verdict::Approved => "approved",
            Verdict::Changes => "changes",
            Verdict::Failed => "failed",
        }
    }

    pub fn parse(s: &str) -> Option<Verdict> {
        Self::ALL.into_iter().find(|v| v.label() == s)
    }
}

/// What a message says: its text, and the fields that go with it. Its keys sit at the top
/// level of a [`Request::Send`] and of a [`Message`], beside theirs.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Payload {
    pub body: String,
    /// The message it answers.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_to: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub verdict: Option<Verdict>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub files: Vec<String>,
    /// The pull request it's about: a number or a URL, as the sender gave it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pr: Option<String>,
}

impl Payload {
    /// Just a text, no fields.
    pub fn text(body: impl Into<String>) -> Payload {
        Payload { body: body.into(), ..Payload::default() }
    }
}

/// The address of someone at a shell rather than an agent: messages to it are only taken
/// with `recv`.
pub const USER: &str = "user";

/// One message, as kept in its recipient's mailbox.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    /// `m` and a number.
    pub id: String,
    /// The sender's session id, or [`USER`].
    pub from: String,
    pub from_name: String,
    pub to: String,
    pub to_name: String,
    pub kind: MessageKind,
    /// Its text (cut short when it was long: the whole of it is in the file at `more`) and
    /// fields.
    #[serde(flatten)]
    pub payload: Payload,
    /// The first line of the message it answers, so it makes sense on its own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub re: Option<String>,
    /// Where the whole text is when `body` was cut short, and how many lines it has.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub more_lines: Option<usize>,
    /// Unix ms.
    pub sent: u64,
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
    /// With `read`: the screen. With `send`: something the sender should know.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
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

/// What `attach` prints to tell the terminal it's in (nex) which agent it shows:
/// `ESC ] 7700 ; nex-agent=<id> BEL`, and the same with no id when it stops. Other
/// terminals ignore it.
pub const AGENT_TAG: &str = "7700;nex-agent=";

/// The tag sequence for `id` (empty: no agent any more).
pub fn agent_tag(id: &str) -> String {
    format!("\x1b]{AGENT_TAG}{id}\x07")
}

/// Keys typed into an attached session.
pub const FRAME_INPUT: u8 = 0;
/// The attached pane's new size: columns then rows, each a big-endian `u16`.
pub const FRAME_RESIZE: u8 = 1;
/// The largest frame either side accepts.
pub const FRAME_MAX: usize = 1 << 20;

/// A frame from an attached client: its kind, the payload's length (big-endian `u32`), then
/// the payload.
pub fn frame(kind: u8, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + payload.len());
    out.push(kind);
    out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
    out.extend_from_slice(payload);
    out
}

/// A resize frame.
pub fn resize_frame(cols: u16, rows: u16) -> Vec<u8> {
    let mut p = cols.to_be_bytes().to_vec();
    p.extend_from_slice(&rows.to_be_bytes());
    frame(FRAME_RESIZE, &p)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn session() -> Session {
        Session {
            id: "a1b2c3d4".into(),
            kind: "claude".into(),
            name: "nex".into(),
            cwd: "/home/u/code/nex".into(),
            argv: vec!["claude".into()],
            pid: 42,
            started: 1_000,
            title: None,
            bell: false,
            attached: false,
            last_output: 0,
            last_input: 0,
            exit: None,
            claude: None,
            running: None,
            status: Status::Idle,
            parent: None,
            role: None,
            last_message: 0,
            subagents: Vec::new(),
        }
    }

    #[test]
    fn protocol_2_sessions_still_parse() {
        let old = r#"{"id":"a","kind":"claude","name":"n","cwd":"/","argv":[],"pid":1,"started":0,"last_output":0,"last_input":0,"status":"idle"}"#;
        let s: Session = serde_json::from_str(old).unwrap();
        assert_eq!((s.parent, s.role, s.last_message), (None, None, 0));
        assert!(s.subagents.is_empty());
    }

    #[test]
    fn message_requests_are_tagged_json() {
        let r: Request = serde_json::from_str(r#"{"op":"send","to":"@parent","kind":"done","body":"ok","verdict":"approved"}"#).unwrap();
        assert!(matches!(r, Request::Send { kind: MessageKind::Done, payload: Payload { verdict: Some(Verdict::Approved), .. }, force: false, .. }));
        let r: Request = serde_json::from_str(r#"{"op":"recv","from":"build","timeout_ms":5}"#).unwrap();
        assert!(matches!(r, Request::Recv { me: None, timeout_ms: Some(5), .. }));
        assert_eq!(MessageKind::parse("ask"), Some(MessageKind::Ask));
        assert_eq!(Verdict::parse("changes"), Some(Verdict::Changes));
    }

    #[test]
    fn payloads_sit_at_the_top_level() {
        let payload = Payload { body: "ok".into(), reply_to: Some("m9".into()), verdict: Some(Verdict::Changes), files: vec!["a.rs".into()], pr: Some("42".into()) };
        let send = Request::Send { from: Some("a1".into()), to: "@task".into(), kind: MessageKind::Done, payload: payload.clone(), force: true };
        let line = r#"{"op":"send","from":"a1","to":"@task","kind":"done","body":"ok","reply_to":"m9","verdict":"changes","files":["a.rs"],"pr":"42","force":true}"#;
        assert_eq!(serde_json::to_string(&send).unwrap(), line);
        assert_eq!(serde_json::from_str::<Request>(line).unwrap(), send);
        let bare = r#"{"op":"send","to":"lead","kind":"note","body":"hi"}"#;
        let r: Request = serde_json::from_str(bare).unwrap();
        assert_eq!(r, Request::Send { from: None, to: "lead".into(), kind: MessageKind::Note, payload: Payload::text("hi"), force: false });
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"op":"send","to":"lead","kind":"note","body":"hi","force":false}"#);

        let m = Message {
            id: "m12".into(),
            from: "b7".into(),
            from_name: "reviewer".into(),
            to: "a1".into(),
            to_name: "lead".into(),
            kind: MessageKind::Done,
            payload,
            re: Some("Review it".into()),
            more: Some("~/.nex/run/mail/m12.md".into()),
            more_lines: Some(40),
            sent: 7,
        };
        let json = serde_json::to_value(&m).unwrap();
        let want = serde_json::json!({
            "id": "m12", "from": "b7", "from_name": "reviewer", "to": "a1", "to_name": "lead", "kind": "done",
            "body": "ok", "reply_to": "m9", "re": "Review it", "verdict": "changes", "files": ["a.rs"], "pr": "42",
            "more": "~/.nex/run/mail/m12.md", "more_lines": 40, "sent": 7,
        });
        assert_eq!(json, want);
        assert_eq!(serde_json::from_value::<Message>(want).unwrap(), m);
        let old = r#"{"id":"m1","from":"user","from_name":"user","to":"a","to_name":"a","kind":"note","body":"x","sent":0}"#;
        assert_eq!(serde_json::from_str::<Message>(old).unwrap().payload, Payload::text("x"));
    }

    #[test]
    fn requests_are_tagged_json() {
        let r = Request::Attach { id: "x".into(), cols: 80, rows: 24 };
        let line = serde_json::to_string(&r).unwrap();
        assert_eq!(line, r#"{"op":"attach","id":"x","cols":80,"rows":24}"#);
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), r);
        assert_eq!(serde_json::from_str::<Request>(r#"{"op":"list"}"#).unwrap(), Request::List);
        let launch: Request = serde_json::from_str(r#"{"op":"launch","argv":["claude"]}"#).unwrap();
        assert!(matches!(launch, Request::Launch { cwd: None, .. }));
        let launch: Request = serde_json::from_str(r#"{"op":"launch","argv":[],"kind":"shell"}"#).unwrap();
        assert!(matches!(launch, Request::Launch { ref argv, .. } if argv.is_empty()));
        assert!(matches!(launch, Request::Launch { cwd: None, parent: None, .. }));
        let launch = Request::Launch { argv: vec!["x".into()], kind: None, name: None, cwd: None, cols: None, rows: None, env: None, parent: Some("a1".into()) };
        let line = serde_json::to_string(&launch).unwrap();
        assert!(line.contains(r#""parent":"a1""#), "{line}");
        assert_eq!(serde_json::from_str::<Request>(&line).unwrap(), launch);
    }

    #[test]
    fn a_pipelines_title_is_its_counts_then_its_last_event() {
        let t = pipeline_title("2 building · 1 in review", "\n✓ build (PR 7) approved:\tmerged\n");
        assert_eq!(t, "2 building · 1 in review — ✓ build (PR 7) approved: merged");
        assert_eq!(split_pipeline_title(&t), ("2 building · 1 in review", "✓ build (PR 7) approved: merged"));
        // Nothing said yet: the counts alone. An event that has a dash of its own splits once.
        assert_eq!(pipeline_title("1 building", "  "), "1 building");
        assert_eq!(split_pipeline_title("1 building"), ("1 building", ""));
        assert_eq!(split_pipeline_title("1 merged — ✓ done — really"), ("1 merged", "✓ done — really"));
        assert_eq!(split_pipeline_title(&pipeline_title("", "✗ stopped")), ("", "✗ stopped"));
        let long = pipeline_title("1 building", &"é".repeat(500));
        assert_eq!(split_pipeline_title(&long).1.chars().count(), TITLE_EVENT_MAX + 1, "cut, with an ellipsis");
    }

    #[test]
    fn sessions_round_trip() {
        let mut s = session();
        s.claude = Some(Claude { status: Some("busy".into()), ..Claude::default() });
        let r = Response::sessions(vec![s]);
        let back: Response = serde_json::from_str(&serde_json::to_string(&r).unwrap()).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn status_prefers_exit_then_claude_then_bell_then_title_then_output() {
        let mut s = session();
        s.exit = Some(0);
        s.bell = true;
        assert_eq!(s.compute_status(10_000), Status::Exited);

        let mut s = session();
        s.claude = Some(Claude { status: Some("waiting_for_permission".into()), ..Claude::default() });
        assert_eq!(s.compute_status(10_000), Status::NeedsYou);
        s.claude = Some(Claude { status: Some("busy".into()), ..Claude::default() });
        assert_eq!(s.compute_status(10_000), Status::Working);
        s.bell = true;
        assert_eq!(s.compute_status(10_000), Status::NeedsYou);

        let mut s = session();
        s.title = Some("✳ Fix the tests".into());
        assert_eq!(s.compute_status(10_000), Status::Idle);
        s.title = Some("✻ Fix the tests".into());
        assert_eq!(s.compute_status(10_000), Status::Working);
        s.title = Some("Fix the tests".into());
        s.last_output = 9_500;
        assert_eq!(s.compute_status(10_000), Status::Working);
        s.last_input = 9_400;
        assert_eq!(s.compute_status(10_000), Status::Idle, "just the echo of typing");
        s.last_input = 0;
        assert_eq!(s.compute_status(20_000), Status::Idle);
    }

    #[test]
    fn frames_carry_their_length() {
        assert_eq!(frame(FRAME_INPUT, b"hi"), vec![0, 0, 0, 0, 2, b'h', b'i']);
        assert_eq!(resize_frame(120, 40), vec![1, 0, 0, 0, 4, 0, 120, 0, 40]);
    }
}
