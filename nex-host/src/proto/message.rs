//! What agents send each other: a [`Message`], its [`MessageKind`], what it says
//! ([`Payload`]), and how a `done` turned out ([`Verdict`]).

use serde::{Deserialize, Serialize};

#[cfg(doc)]
use crate::proto::Request;

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
