//! Messages: writing one, putting it in its recipients' mailboxes and typing it in (now, or
//! once the recipient can take it), and reminding agents that went quiet on a task.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto::{self, Message, MessageKind, Payload, Status, USER};

use crate::daemon::agent::Agent;
use crate::daemon::broker::{self, Posted};
use crate::daemon::State;
use crate::daemon::deliver;
use crate::util::{Locked, first_line, now_ms};

/// How often messages waiting to be typed in are looked at.
const DELIVER_TICK: Duration = Duration::from_millis(400);
/// A program this new may not be reading its input yet: messages wait until it's older.
const STARTUP_MS: u64 = 2500;
/// How long Claude Code's screen must have been still before a message is typed in.
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
    let id = state.broker.next_id();
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
    let re = payload.reply_to.as_deref().and_then(|r| state.broker.find(r)).map(|m| first_line(&m.payload.body));
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
    let task = me.as_deref().and_then(|m| state.broker.task(m));
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
        if state.broker.post(m) == Posted::Taken {
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
            state.pending.locked().push((agent.id.clone(), rendered));
        }
    }
    Ok((ids, notes))
}

/// Whether a message can be typed in now: it has started up and isn't asking the user
/// something (typing into a yes/no prompt would answer it).
fn can_take(a: &Agent) -> bool {
    let now = now_ms();
    let s = a.session(now);
    // Claude Code's first screen may be a dialog (see `Agent::session`): wait until it has
    // drawn one and stopped, so a slow start can't have the message answer it unseen.
    let settled = s.kind != "claude" || (s.last_output != 0 && now.saturating_sub(s.last_output) >= SETTLE_MS);
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
        let waiting = std::mem::take(&mut *state.pending.locked());
        let mut still = Vec::new();
        for (id, text) in waiting {
            match state.get(&id) {
                Some(a) if a.exited() => {}
                Some(a) if can_take(&a) => a.type_in(text),
                Some(_) => still.push((id, text)),
                None => {}
            }
        }
        state.pending.locked().splice(0..0, still);

        let agents: Vec<Arc<Agent>> = state.agents.locked().values().filter(|a| matches!(a.kind.as_str(), "claude" | "codex") && !a.exited()).cloned().collect();
        quiet.retain(|id, _| agents.iter().any(|a| a.id == *id));
        for a in agents {
            let Some(task) = state.broker.task(&a.id).filter(|t| !t.reported) else {
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
