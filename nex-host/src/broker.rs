//! The daemon's post office: who an address means, every agent's mailbox, and `recv`
//! waiting on one.
//!
//! Everything agents say to each other is a [`Message`]. Starting an agent with a task,
//! reporting back (`done`), asking and answering are all messages; what differs is their
//! [`MessageKind`]. Each one goes in its recipient's mailbox (kept in memory, and appended to
//! `~/.nex/run/mail/<id>.jsonl` as a record) and is typed into the recipient's terminal
//! (`daemon.rs` does that, `deliver.rs` says how), unless a `recv` is waiting for it: then
//! the `recv` gets it instead, so an agent never sees one message twice.

use std::collections::HashMap;
use std::io::Write;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto::{Message, MessageKind, USER};

use crate::locked::Locked;

/// What address resolution needs to know about an agent.
#[derive(Clone, Debug)]
pub struct Peer {
    pub id: String,
    pub name: String,
    pub parent: Option<String>,
    pub exited: bool,
}

/// Who `to` means, for a message from `me` (none: someone at a shell), whose current task
/// came from `task_from`:
/// - `user`: someone at a shell (taken with `recv`)
/// - `@me`, `@parent`, `@children`
/// - `@task`: whoever gave `me` its current task; its parent, or `user`, if nobody did
/// - an id, or a name (one of `me`'s children first, then anyone's)
pub fn resolve(peers: &[Peer], me: Option<&str>, task_from: Option<&str>, to: &str) -> Result<Vec<String>, String> {
    let to = to.trim();
    let mine = || me.and_then(|m| peers.iter().find(|p| p.id == m));
    let need_me = || mine().ok_or_else(|| format!("`{to}` only means something from an agent (NEX_AGENT_ID isn't set)"));
    match to {
        "" => Err("no recipient".into()),
        USER => Ok(vec![USER.into()]),
        "@me" => Ok(vec![need_me()?.id.clone()]),
        "@parent" => Ok(vec![need_me()?.parent.clone().unwrap_or_else(|| USER.into())]),
        "@task" => match task_from {
            Some(from) => Ok(vec![from.to_string()]),
            None => Ok(vec![need_me()?.parent.clone().unwrap_or_else(|| USER.into())]),
        },
        "@children" => {
            let me = need_me()?;
            let kids: Vec<String> = peers.iter().filter(|p| p.parent.as_deref() == Some(&me.id) && !p.exited).map(|p| p.id.clone()).collect();
            if kids.is_empty() { Err("you have no running children".into()) } else { Ok(kids) }
        }
        _ => {
            if let Some(p) = peers.iter().find(|p| p.id == to) {
                return if p.exited { Err(format!("{} ({}) has exited", p.name, p.id)) } else { Ok(vec![p.id.clone()]) };
            }
            let named: Vec<&Peer> = peers.iter().filter(|p| p.name == to).collect();
            let live: Vec<&Peer> = named.iter().copied().filter(|p| !p.exited).collect();
            let kids: Vec<&Peer> = live.iter().copied().filter(|p| me.is_some() && p.parent.as_deref() == me).collect();
            match (kids.as_slice(), live.as_slice()) {
                ([one], _) | ([], [one]) => Ok(vec![one.id.clone()]),
                ([], []) if !named.is_empty() => Err(format!("{to} has exited")),
                ([], []) => Err(format!("no agent called {to} (see nex-host ls)")),
                (_, many) => {
                    let ids: Vec<&str> = many.iter().map(|p| p.id.as_str()).collect();
                    Err(format!("more than one agent is called {to}: use an id ({})", ids.join(", ")))
                }
            }
        }
    }
}

/// The one agent `who` (an id or a name) means, exited or not: `me`'s child of that name
/// first, then a running one, then any.
pub fn lookup(peers: &[Peer], me: Option<&str>, who: &str) -> Option<String> {
    if let Some(p) = peers.iter().find(|p| p.id == who) {
        return Some(p.id.clone());
    }
    let named: Vec<&Peer> = peers.iter().filter(|p| p.name == who).collect();
    let pick = |f: &dyn Fn(&Peer) -> bool| -> Option<String> {
        match named.iter().filter(|p| f(p)).collect::<Vec<_>>().as_slice() {
            [one] => Some(one.id.clone()),
            _ => None,
        }
    };
    pick(&|p| me.is_some() && p.parent.as_deref() == me).or_else(|| pick(&|p| !p.exited)).or_else(|| pick(&|_| true))
}

/// What a `recv` is waiting for. `None` matches anything.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Filter {
    pub from: Option<String>,
    pub reply_to: Option<String>,
    pub kind: Option<MessageKind>,
}

impl Filter {
    pub fn matches(&self, m: &Message) -> bool {
        self.from.as_ref().is_none_or(|f| *f == m.from) && self.reply_to.as_ref().is_none_or(|r| m.reply_to.as_ref() == Some(r)) && self.kind.is_none_or(|k| k == m.kind)
    }
}

/// The task an agent is working on: its message id, who gave it, and whether it has
/// reported on it yet.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub from: String,
    pub reported: bool,
}

struct Entry {
    msg: Message,
    /// A `recv` has had it.
    taken: bool,
}

#[derive(Default)]
struct Mail {
    next_id: u64,
    boxes: HashMap<String, Vec<Entry>>,
    /// The `recv`s waiting: whose mailbox, and for what.
    waiting: Vec<(u64, String, Filter)>,
    next_wait: u64,
    tasks: HashMap<String, Task>,
}

#[derive(Default)]
pub struct Broker {
    mail: Mutex<Mail>,
    arrived: Condvar,
    /// Record mailboxes on disk (off in tests).
    pub record: bool,
}

/// Whether a posted message should still be typed in.
#[derive(Debug, PartialEq, Eq)]
pub enum Posted {
    /// Nobody's waiting for it: type it into the recipient's terminal.
    Type,
    /// A `recv` takes it.
    Taken,
}

impl Broker {
    pub fn new() -> Self {
        Self { record: true, ..Self::default() }
    }

    /// A message id nobody has had.
    pub fn next_id(&self) -> String {
        let mut mail = self.mail.locked();
        mail.next_id += 1;
        format!("m{}", mail.next_id)
    }

    /// The message with this id, in anyone's mailbox.
    pub fn find(&self, id: &str) -> Option<Message> {
        self.mail.locked().boxes.values().flatten().find(|e| e.msg.id == id).map(|e| e.msg.clone())
    }

    pub fn task(&self, agent: &str) -> Option<Task> {
        self.mail.locked().tasks.get(agent).cloned()
    }

    /// Put `m` in its recipient's mailbox. A task becomes the recipient's current one; a
    /// `done` marks the sender's as reported.
    pub fn post(&self, m: Message) -> Posted {
        let mut mail = self.mail.locked();
        match m.kind {
            MessageKind::Task => {
                mail.tasks.insert(m.to.clone(), Task { id: m.id.clone(), from: m.from.clone(), reported: false });
            }
            MessageKind::Done => {
                if let Some(t) = mail.tasks.get_mut(&m.from) {
                    t.reported = true;
                }
            }
            _ => {}
        }
        if self.record {
            record(&m);
        }
        let waited = mail.waiting.iter().any(|(_, me, f)| *me == m.to && f.matches(&m));
        mail.boxes.entry(m.to.clone()).or_default().push(Entry { msg: m, taken: false });
        drop(mail);
        self.arrived.notify_all();
        if waited { Posted::Taken } else { Posted::Type }
    }

    /// The first message for `me` matching `filter` that no `recv` has had, waiting up to
    /// `timeout` for one.
    pub fn take(&self, me: &str, filter: &Filter, timeout: Duration) -> Option<Message> {
        let deadline = Instant::now() + timeout;
        let mut mail = self.mail.locked();
        mail.next_wait += 1;
        let ticket = mail.next_wait;
        mail.waiting.push((ticket, me.to_string(), filter.clone()));
        let found = loop {
            if let Some(e) = mail.boxes.get_mut(me).and_then(|b| b.iter_mut().find(|e| !e.taken && filter.matches(&e.msg))) {
                e.taken = true;
                break Some(e.msg.clone());
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break None;
            }
            mail = self.arrived.wait_timeout(mail, left).unwrap_or_else(std::sync::PoisonError::into_inner).0;
        };
        mail.waiting.retain(|(t, ..)| *t != ticket);
        found
    }

    /// Forget an agent that's gone for good: its mailbox, its task, its record on disk.
    pub fn forget(&self, agent: &str) {
        let mut mail = self.mail.locked();
        if let Some(entries) = mail.boxes.remove(agent)
            && self.record
        {
            for e in entries {
                if let Some(more) = paths::mail_dir().map(|d| d.join(format!("{}.md", e.msg.id))) {
                    let _ = std::fs::remove_file(more);
                }
            }
        }
        mail.tasks.remove(agent);
        if self.record
            && let Some(d) = paths::mail_dir()
        {
            let _ = std::fs::remove_file(d.join(format!("{agent}.jsonl")));
        }
    }
}

/// Append `m` to its recipient's record.
fn record(m: &Message) {
    let Some(dir) = paths::mail_dir() else { return };
    if std::fs::create_dir_all(&dir).is_err() || m.to.contains('/') {
        return;
    }
    let Ok(line) = serde_json::to_string(m) else { return };
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join(format!("{}.jsonl", m.to))) {
        let _ = writeln!(f, "{line}");
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    fn peer(id: &str, name: &str, parent: Option<&str>) -> Peer {
        Peer { id: id.into(), name: name.into(), parent: parent.map(Into::into), exited: false }
    }

    fn peers() -> Vec<Peer> {
        vec![
            peer("lead1", "lead", None),
            peer("rev1", "reviewer", Some("lead1")),
            peer("bld1", "build", Some("lead1")),
            peer("lead2", "lead2", None),
            peer("rev2", "reviewer", Some("lead2")),
            Peer { exited: true, ..peer("old1", "old", Some("lead1")) },
        ]
    }

    #[test]
    fn addresses() {
        let p = peers();
        let r = |me: Option<&str>, to: &str| resolve(&p, me, None, to);
        assert_eq!(r(Some("rev1"), "@parent"), Ok(vec!["lead1".into()]));
        assert_eq!(r(Some("lead1"), "@parent"), Ok(vec![USER.into()]), "started at a shell");
        assert_eq!(r(Some("lead1"), "@children"), Ok(vec!["rev1".into(), "bld1".into()]), "not the exited one");
        assert_eq!(r(Some("lead1"), "reviewer"), Ok(vec!["rev1".into()]), "my own child first");
        assert_eq!(r(Some("lead2"), "reviewer"), Ok(vec!["rev2".into()]));
        assert!(r(Some("bld1"), "reviewer").unwrap_err().contains("rev1, rev2"), "ambiguous elsewhere");
        assert_eq!(r(None, "bld1"), Ok(vec!["bld1".into()]));
        assert!(r(None, "old").unwrap_err().contains("exited"));
        assert!(r(None, "old1").unwrap_err().contains("exited"));
        assert!(r(None, "nobody").unwrap_err().contains("nex-host ls"));
        assert!(r(None, "@parent").is_err(), "needs an agent");
        assert_eq!(r(None, "user"), Ok(vec![USER.into()]));
        assert_eq!(resolve(&p, Some("rev1"), Some("bld1"), "@task"), Ok(vec!["bld1".into()]));
        assert_eq!(resolve(&p, Some("rev1"), None, "@task"), Ok(vec!["lead1".into()]), "falls back to the parent");
    }

    #[test]
    fn lookup_finds_exited_agents_too() {
        let p = peers();
        assert_eq!(lookup(&p, None, "old"), Some("old1".into()));
        assert_eq!(lookup(&p, Some("lead2"), "reviewer"), Some("rev2".into()));
        assert_eq!(lookup(&p, None, "reviewer"), None, "two of them");
        assert_eq!(lookup(&p, None, "bld1"), Some("bld1".into()));
        assert_eq!(lookup(&p, None, "nobody"), None);
    }

    fn msg(id: &str, from: &str, to: &str, kind: MessageKind) -> Message {
        Message {
            id: id.into(),
            from: from.into(),
            from_name: from.into(),
            to: to.into(),
            to_name: to.into(),
            kind,
            body: String::new(),
            reply_to: None,
            re: None,
            verdict: None,
            files: vec![],
            pr: None,
            more: None,
            more_lines: None,
            sent: 0,
        }
    }

    fn broker() -> Broker {
        Broker { record: false, ..Broker::default() }
    }

    #[test]
    fn tasks_are_tracked_until_reported() {
        let b = broker();
        assert_eq!(b.post(msg("m1", "lead", "rev", MessageKind::Task)), Posted::Type);
        assert_eq!(b.task("rev"), Some(Task { id: "m1".into(), from: "lead".into(), reported: false }));
        b.post(msg("m2", "rev", "lead", MessageKind::Done));
        assert!(b.task("rev").unwrap().reported);
        b.post(msg("m3", "lead", "rev", MessageKind::Task));
        assert!(!b.task("rev").unwrap().reported, "a new task starts over");
        assert_eq!(b.find("m2").map(|m| m.kind), Some(MessageKind::Done));
    }

    #[test]
    fn recv_takes_each_matching_message_once() {
        let b = broker();
        b.post(msg("m1", "a", "me", MessageKind::Note));
        b.post(msg("m2", "b", "me", MessageKind::Done));
        let from_b = Filter { from: Some("b".into()), ..Filter::default() };
        assert_eq!(b.take("me", &from_b, Duration::ZERO).map(|m| m.id), Some("m2".into()));
        assert_eq!(b.take("me", &from_b, Duration::ZERO), None, "already taken");
        assert_eq!(b.take("me", &Filter::default(), Duration::ZERO).map(|m| m.id), Some("m1".into()));
        assert_eq!(b.take("other", &Filter::default(), Duration::ZERO), None);
    }

    #[test]
    fn a_waiting_recv_gets_the_message_instead_of_the_terminal() {
        let b = Arc::new(broker());
        let waiter = b.clone();
        let t = std::thread::spawn(move || waiter.take("me", &Filter { kind: Some(MessageKind::Done), ..Filter::default() }, Duration::from_secs(5)));
        while b.mail.locked().waiting.is_empty() {
            std::thread::yield_now();
        }
        assert_eq!(b.post(msg("m1", "x", "me", MessageKind::Note)), Posted::Type, "not what it waits for");
        assert_eq!(b.post(msg("m2", "x", "me", MessageKind::Done)), Posted::Taken);
        assert_eq!(t.join().unwrap().map(|m| m.id), Some("m2".into()));
        assert!(b.mail.locked().waiting.is_empty());
    }
}
