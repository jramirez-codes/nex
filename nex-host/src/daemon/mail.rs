//! Messages: writing one, putting it in its recipients' mailboxes and typing it in (now, or
//! once the recipient can take it), and reminding agents that went quiet on a task.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto::{self, Message, MessageKind, Payload, Status, USER};

use crate::daemon::agent::Agent;
use crate::daemon::broker::{self, Posted};
use crate::daemon::state::State;
use crate::daemon::deliver;
use crate::util::{first_line, now_ms};

/// How often messages waiting to be typed in are looked at.
const DELIVER_TICK: Duration = Duration::from_millis(400);
/// A program this new may not be reading its input yet: messages wait until it's older.
const STARTUP_MS: u64 = 2500;
/// How long the screen of a CLI with start-up dialogs (Claude Code) must have been still
/// before a message is typed in.
const SETTLE_MS: u64 = 500;
/// How long an agent may sit idle after working on a task without reporting before it's
/// reminded to (once per task).
const NUDGE_AFTER: Duration = Duration::from_secs(45);
/// Who reminders come from.
const NEX: &str = "nex";

/// A message about to be sent.
pub(super) struct Draft {
    pub(super) from: Option<String>,
    pub(super) to: String,
    pub(super) kind: MessageKind,
    pub(super) payload: Payload,
}

/// A message from `from` to `to`, its body cut short (and the whole of it saved) if long.
pub(super) fn letter(state: &State, from: &str, from_name: &str, to: &str, to_name: &str, kind: MessageKind, payload: Payload) -> Message {
    let id = state.broker().next_id();
    let body = payload.body.as_str();
    let (short, cut) = deliver::clip(body);
    let more = cut.and_then(|_| {
        let dir = paths::mail_dir()?;
        std::fs::create_dir_all(&dir).ok()?;
        let path = dir.join(format!("{id}.md"));
        std::fs::write(&path, body).ok()?;
        Some(paths::tilde(&path))
    });
    let (body, more_lines) = if more.is_some() { (short, cut) } else { (body.to_string(), None) };
    let re = payload.reply_to.as_deref().and_then(|r| state.broker().find(r)).map(|m| first_line(&m.payload.body));
    Message {
        id,
        from: from.into(),
        from_name: from_name.into(),
        to: to.into(),
        to_name: to_name.into(),
        kind,
        payload: Payload { body, ..payload },
        re,
        more,
        more_lines,
        sent: now_ms(),
    }
}

/// `send`: put the message in each recipient's mailbox and type it in. Returns the
/// message ids, and anything the sender should know (it's waiting to be typed in).
pub(super) fn post(state: &Arc<State>, d: Draft, force: bool) -> Result<(Vec<String>, Vec<String>), String> {
    let sender = d.from.as_deref().and_then(|f| state.get(f));
    let (from, from_name) = match &sender {
        Some(a) => (a.id.clone(), a.name.clone()),
        None if d.from.as_deref() == Some(NEX) => (NEX.to_string(), NEX.to_string()),
        None => (USER.to_string(), USER.to_string()),
    };
    let me = sender.as_ref().map(|a| a.id.clone());
    let task = me.as_deref().and_then(|m| state.broker().task(m));
    let to = broker::resolve(&state.peers(), me.as_deref(), task.as_ref().map(|t| t.from.as_str()), &d.to)?;
    // A report answers the task it's about.
    let reply_to = d.payload.reply_to.clone().or_else(|| (d.kind == MessageKind::Done).then(|| task.map(|t| t.id)).flatten());
    let payload = Payload { reply_to, ..d.payload };
    let mut ids = Vec::new();
    let mut notes = Vec::new();
    for to in to {
        let recipient = state.get(&to);
        let to_name = recipient.as_ref().map_or_else(|| to.clone(), |a| a.name.clone());
        let m = letter(state, &from, &from_name, &to, &to_name, d.kind, payload.clone());
        ids.push(m.id.clone());
        let rendered = deliver::render(&m);
        if state.broker().post(m) == Posted::Taken {
            continue;
        }
        let Some(agent) = recipient else { continue };
        if agent.kind == proto::PIPELINE_KIND {
            continue;
        }
        if force || can_take(&agent) {
            agent.type_in(rendered);
        } else {
            notes.push(format!("{to_name} is waiting for the user to answer something: it gets this as soon as it's free (--force to type it in now)."));
            state.queue(&agent.id, rendered);
        }
    }
    Ok((ids, notes))
}

/// Whether a message can be typed in now: it has started up and isn't asking the user
/// something (typing into a yes/no prompt would answer it).
fn can_take(a: &Agent) -> bool {
    let now = now_ms();
    let s = a.session(now);
    // A CLI's first screen may be a dialog (Claude Code's, see `Agent::session`): wait until
    // it has drawn one and stopped, so a slow start can't have the message answer it unseen.
    let dialogs = nex_host::clis::by_kind(&s.kind).is_some_and(|c| !c.dialog_marks().is_empty());
    let settled = !dialogs || (s.last_output != 0 && now.saturating_sub(s.last_output) >= SETTLE_MS);
    now.saturating_sub(a.started) >= STARTUP_MS && settled && s.status != Status::NeedsYou
}

/// Type in the messages that were waiting, once their recipients can take them; remind
/// agents that went quiet without reporting.
pub(super) fn deliver_loop(state: &Arc<State>) {
    struct Quiet {
        task: String,
        worked: bool,
        idle_since: Option<Instant>,
        nudged: bool,
    }
    let mut quiet: HashMap<String, Quiet> = HashMap::new();
    loop {
        std::thread::sleep(DELIVER_TICK);
        let waiting = state.take_pending();
        let mut still = Vec::new();
        for (id, text) in waiting {
            match state.get(&id) {
                Some(a) if a.exited() => {}
                Some(a) if can_take(&a) => a.type_in(text),
                Some(_) => still.push((id, text)),
                None => {}
            }
        }
        state.requeue(still);

        let agents = state.agents_where(|a| nex_host::clis::by_kind(&a.kind).is_some() && !a.exited());
        quiet.retain(|id, _| agents.iter().any(|a| a.id == *id));
        for a in agents {
            let Some(task) = state.broker().task(&a.id).filter(|t| !t.reported) else {
                quiet.remove(&a.id);
                continue;
            };
            let q = quiet.entry(a.id.clone()).or_insert_with(|| Quiet { task: task.id.clone(), worked: false, idle_since: None, nudged: false });
            if q.task != task.id {
                *q = Quiet { task: task.id.clone(), worked: false, idle_since: None, nudged: false };
            }
            match a.session(now_ms()).status {
                Status::Working => {
                    q.worked = true;
                    q.idle_since = None;
                }
                Status::Idle if q.worked && !q.nudged => {
                    if q.idle_since.get_or_insert_with(Instant::now).elapsed() >= NUDGE_AFTER {
                        q.nudged = true;
                        let body = format!(
                            "You've stopped without reporting on task {}. If it's finished, run nex-host done \"<what you did>\". If you're waiting for the user, ignore this.",
                            task.id
                        );
                        let draft = Draft { from: Some(NEX.into()), to: a.id.clone(), kind: MessageKind::Note, payload: Payload::text(body) };
                        let _ = post(state, draft, false);
                    }
                }
                _ => q.idle_since = None,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;
    use crate::daemon::broker::Filter;
    use crate::daemon::testing::{kill_all, start, start_kind, state, wait_for};

    fn draft(from: Option<&str>, to: &str, kind: MessageKind, body: &str) -> Draft {
        Draft { from: from.map(str::to_string), to: to.into(), kind, payload: Payload::text(body) }
    }

    #[test]
    fn a_letter_quotes_the_first_line_of_what_it_answers() {
        let state = state();
        let first = letter(&state, USER, USER, "a1", "dev", MessageKind::Note, Payload::text("\n  the question\nmore"));
        let first_id = first.id.clone();
        state.broker().post(first);
        let reply = letter(&state, "a1", "dev", USER, USER, MessageKind::Note, Payload { reply_to: Some(first_id.clone()), ..Payload::text("an answer") });
        assert_ne!(reply.id, first_id);
        assert_eq!(reply.re.as_deref(), Some("the question"));
        assert_eq!((reply.more, reply.more_lines), (None, None), "a short body is sent whole");
        assert_eq!(letter(&state, USER, USER, "a1", "dev", MessageKind::Note, Payload::text("x")).re, None);
    }

    #[test]
    fn a_message_for_the_user_waits_in_their_mailbox() {
        let state = state();
        let (ids, notes) = post(&state, draft(None, USER, MessageKind::Note, "hello"), false).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(notes.is_empty());
        let m = state.broker().take(USER, &Filter::default(), Duration::ZERO).expect("in the user's mailbox");
        assert_eq!((m.id, m.from.as_str(), m.payload.body.as_str()), (ids[0].clone(), USER, "hello"));
        assert!(state.take_pending().is_empty());
    }

    #[test]
    fn an_unknown_recipient_is_an_error() {
        let state = state();
        assert!(post(&state, draft(None, "nobody", MessageKind::Note, "hi"), false).is_err());
        assert!(post(&state, draft(None, "@me", MessageKind::Note, "hi"), false).is_err());
    }

    #[test]
    fn an_agent_just_started_gets_its_message_once_it_can_take_it() {
        let state = state();
        let a = start(&state, "a1", "dev", &["sleep", "30"], None);
        assert!(!can_take(&a), "it has only just started");
        let (_, notes) = post(&state, draft(None, "dev", MessageKind::Note, "later"), false).unwrap();
        assert_eq!(notes.len(), 1);
        assert!(notes[0].starts_with("dev is waiting"), "{}", notes[0]);
        let pending = state.take_pending();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].0, "a1");
        assert!(pending[0].1.contains("later"));
        kill_all(&state);
    }

    #[test]
    fn force_types_it_in_now() {
        let state = state();
        let a = start(&state, "a1", "dev", &["sleep", "30"], None);
        let (_, notes) = post(&state, draft(None, "a1", MessageKind::Note, "now"), true).unwrap();
        assert!(notes.is_empty());
        assert!(state.take_pending().is_empty());
        assert_ne!(a.last_message.load(std::sync::atomic::Ordering::Relaxed), 0, "typed in");
        kill_all(&state);
    }

    #[test]
    fn a_waiting_recv_takes_it_instead_of_the_terminal() {
        let state = state();
        let a = start(&state, "a1", "dev", &["sleep", "30"], None);
        let taker = state.clone();
        let recv = std::thread::spawn(move || taker.broker().take("a1", &Filter::default(), Duration::from_secs(5)));
        wait_for("the recv to wait", || state.broker().waiting());
        let (_, notes) = post(&state, draft(None, "dev", MessageKind::Note, "for recv"), false).unwrap();
        assert!(notes.is_empty());
        assert!(state.take_pending().is_empty());
        assert_eq!(recv.join().unwrap().map(|m| m.payload.body), Some("for recv".into()));
        assert_eq!(a.last_message.load(std::sync::atomic::Ordering::Relaxed), 0, "not typed in");
        kill_all(&state);
    }

    #[test]
    fn nothing_is_typed_into_a_pipeline() {
        let state = state();
        start_kind(&state, "pl", "flow", &["sleep", "30"], None, proto::PIPELINE_KIND);
        let (ids, notes) = post(&state, draft(None, "pl", MessageKind::Note, "hi"), false).unwrap();
        assert_eq!(ids.len(), 1);
        assert!(notes.is_empty());
        assert!(state.take_pending().is_empty());
        kill_all(&state);
    }

    #[test]
    fn a_report_answers_the_task_it_is_about() {
        let state = state();
        start(&state, "p1", "boss", &["sleep", "30"], None);
        start(&state, "c1", "helper", &["sleep", "30"], Some("p1"));
        let (task, _) = post(&state, draft(Some("p1"), "helper", MessageKind::Task, "do it"), false).unwrap();
        let t = state.broker().task("c1").expect("its task");
        assert_eq!((t.id.as_str(), t.from.as_str(), t.reported), (task[0].as_str(), "p1", false));
        post(&state, draft(Some("c1"), "@task", MessageKind::Done, "did it"), false).unwrap();
        assert!(state.broker().task("c1").unwrap().reported);
        let filter = Filter { kind: Some(MessageKind::Done), ..Filter::default() };
        let m = state.broker().take("p1", &filter, Duration::ZERO).expect("the report");
        assert_eq!((m.from.as_str(), m.from_name.as_str()), ("c1", "helper"));
        assert_eq!(m.payload.reply_to.as_deref(), Some(task[0].as_str()));
        assert_eq!(m.re.as_deref(), Some("do it"));
        kill_all(&state);
    }

    #[test]
    fn reminders_come_from_nex() {
        let state = state();
        start(&state, "a1", "dev", &["sleep", "30"], None);
        post(&state, draft(Some(NEX), USER, MessageKind::Note, "psst"), false).unwrap();
        let m = state.broker().take(USER, &Filter::default(), Duration::ZERO).unwrap();
        assert_eq!((m.from.as_str(), m.from_name.as_str()), (NEX, NEX));
        kill_all(&state);
    }
}
