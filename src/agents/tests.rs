//! What counts as an agent, how its status is read, and how the list is ordered.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use super::AgentsPane;
use super::model::{AgentKind, Counts, Probe, Source, Status, Tracker, elapsed, is_remote_key, remote_key};

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

fn probe(pane: u64, tab: usize, kind: AgentKind, at: Instant) -> Probe {
    Probe {
        pane,
        tab: Some(tab),
        kind,
        title: String::new(),
        mark: None,
        cwd: Some(PathBuf::from("/home/u/code/nex")),
        session_name: None,
        session_status: None,
        attention: false,
        last_output: at - Duration::from_secs(60),
        last_input: at - Duration::from_secs(60),
        source: Source::Local,
        reported: None,
    }
}

#[test]
fn recognizes_agents() {
    assert_eq!(AgentKind::detect("claude", &v(&["claude"])), Some(AgentKind::Claude));
    assert_eq!(AgentKind::detect("node", &v(&["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])), Some(AgentKind::Claude));
    assert_eq!(AgentKind::detect("codex", &v(&["codex", "--full-auto"])), Some(AgentKind::Codex));
    assert_eq!(AgentKind::detect("node", &v(&["node", "/home/u/.npm/bin/codex"])), Some(AgentKind::Codex));
    assert_eq!(AgentKind::detect("node", &v(&["node", "server.js"])), None);
    assert_eq!(AgentKind::detect("fish", &[]), None);
    assert_eq!(AgentKind::detect("codex-helper", &v(&["codex-helper"])), None);
}

#[test]
fn reads_claude_codes_own_word() {
    assert_eq!(Status::from_claude("idle"), Some(Status::Idle));
    assert_eq!(Status::from_claude("busy"), Some(Status::Working));
    assert_eq!(Status::from_claude("waiting_for_permission"), Some(Status::NeedsYou));
    assert_eq!(Status::from_claude("Awaiting input"), Some(Status::NeedsYou));
    assert_eq!(Status::from_claude("something-new"), None);
}

#[test]
fn status_falls_back_through_bell_title_and_output() {
    let now = Instant::now();
    let mut p = probe(1, 0, AgentKind::Claude, now);
    // Claude Code's session file wins over its title.
    p.session_status = Some("busy".into());
    p.mark = Some("✳".into());
    assert_eq!(p.status(now), Status::Working);
    // An unseen bell means it wants you, unless the file says so already anyway.
    p.attention = true;
    assert_eq!(p.status(now), Status::NeedsYou);
    p.attention = false;
    // No file: the title's ✳ is idle, a spinner frame is working.
    p.session_status = None;
    assert_eq!(p.status(now), Status::Idle);
    p.mark = Some("⠂".into());
    assert_eq!(p.status(now), Status::Working);
    // An unknown word in the file falls through to the title too.
    p.session_status = Some("pondering-the-orb".into());
    assert_eq!(p.status(now), Status::Working);

    // Codex has no session file or idle glyph: output on its own means working, but the
    // echo of the user typing doesn't.
    let mut c = probe(2, 0, AgentKind::Codex, now);
    assert_eq!(c.status(now), Status::Idle);
    c.last_output = now - Duration::from_millis(200);
    assert_eq!(c.status(now), Status::Working);
    c.last_input = now - Duration::from_millis(300);
    assert_eq!(c.status(now), Status::Idle);
}

#[test]
fn names_by_task_then_session_then_folder() {
    let now = Instant::now();
    let mut p = probe(1, 0, AgentKind::Claude, now);
    p.session_status = Some("idle".into());
    let mut t = Tracker::default();
    let name = |t: &mut Tracker, p: &Probe| {
        t.update(vec![p.clone()], now);
        t.agents()[0].name.clone()
    };
    assert_eq!(name(&mut t, &p), "nex");
    p.session_name = Some("nex-2e".into());
    assert_eq!(name(&mut t, &p), "nex-2e");
    p.title = "Claude Code".into();
    assert_eq!(name(&mut t, &p), "nex-2e", "the generic title isn't a task");
    p.title = "Fix the login redirect".into();
    assert_eq!(name(&mut t, &p), "Fix the login redirect");
}

#[test]
fn sorts_by_need_then_tab_and_keeps_time_in_state() {
    let start = Instant::now();
    let mut idle = probe(5, 0, AgentKind::Claude, start);
    idle.session_status = Some("idle".into());
    let mut working = probe(3, 1, AgentKind::Claude, start);
    working.session_status = Some("busy".into());
    let mut waiting = probe(9, 2, AgentKind::Codex, start);
    waiting.attention = true;
    let mut also_idle = probe(2, 0, AgentKind::Claude, start);
    also_idle.session_status = Some("idle".into());

    let mut t = Tracker::default();
    t.update(vec![idle.clone(), working.clone(), waiting.clone(), also_idle.clone()], start);
    let order: Vec<u64> = t.agents().iter().map(|a| a.pane).collect();
    assert_eq!(order, vec![9, 3, 2, 5]);
    assert_eq!(t.counts(), Counts { needs_you: 1, working: 1, idle: 2, exited: 0 });
    assert_eq!(t.counts().summary(), "1 needs you · 1 working · 2 idle");

    // Still idle a minute later: its clock keeps running. Working → idle restarts it.
    let later = start + Duration::from_secs(60);
    working.session_status = Some("idle".into());
    t.update(vec![idle, working, also_idle], later);
    let since = |pane| t.agents().iter().find(|a| a.pane == pane).unwrap().since;
    assert_eq!(since(5), start);
    assert_eq!(since(3), later);
    assert_eq!(t.counts().summary(), "3 idle");
}

#[test]
fn summaries_and_elapsed_times() {
    assert_eq!(Counts::default().summary(), "No agents running");
    assert_eq!(Counts { needs_you: 2, working: 1, idle: 0, exited: 0 }.summary(), "2 need you · 1 working");
    assert_eq!(Counts { needs_you: 0, working: 3, idle: 1, exited: 0 }.headline(), "3 working");
    assert_eq!(Counts::default().headline(), "None running");
    assert_eq!(elapsed(Duration::from_secs(2)), "now");
    assert_eq!(elapsed(Duration::from_secs(42)), "42s");
    assert_eq!(elapsed(Duration::from_secs(600)), "10m");
    assert_eq!(elapsed(Duration::from_secs(7300)), "2h");
    assert_eq!(elapsed(Duration::from_secs(200_000)), "2d");
}

#[test]
fn keys_move_through_cards_and_jump() {
    let mut a = AgentsPane::new();
    a.order = vec![10, 11, 12, 13, 14];
    a.columns = 2;
    a.selected = Some(10);
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(12));
    assert!(a.key(Key::L, Modifiers::NONE));
    assert_eq!(a.selected, Some(13));
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(14), "clamped at the last card");
    assert!(a.key(Key::Home, Modifiers::NONE));
    assert_eq!(a.selected, Some(10));
    assert!(a.key(Key::Enter, Modifiers::NONE));
    assert_eq!(a.requests, vec![super::Request::Focus(10)]);
    // Shortcuts with modifiers aren't the pane's.
    assert!(!a.key(Key::J, Modifiers::CTRL));
    assert!(!a.key(Key::Q, Modifiers::NONE));
}

fn remote(host: &str, session: &str, status: Status, at: Instant) -> Probe {
    let target = crate::remote::SshTarget { opts: vec![], destination: host.into() };
    Probe {
        pane: remote_key(host, session),
        tab: None,
        reported: Some(status),
        source: Source::Remote { host: host.into(), target, session: session.into(), pane: None },
        ..probe(0, 0, AgentKind::Claude, at)
    }
}

#[test]
fn agents_on_ssh_hosts_have_keys_no_pane_has() {
    let k = remote_key("devbox", "3fa91c0e");
    assert_eq!(k, remote_key("devbox", "3fa91c0e"), "the same card every poll");
    assert_ne!(k, remote_key("gpu", "3fa91c0e"));
    assert!(is_remote_key(k));
    assert!(!is_remote_key(1) && !is_remote_key(4096));
}

#[test]
fn what_nex_host_reports_is_the_status_and_exited_agents_go_last() {
    let at = Instant::now();
    let mut needs = remote("devbox", "a", Status::Working, at);
    needs.attention = true;
    assert_eq!(needs.status(at), Status::NeedsYou, "the pane showing it heard the bell");
    let mut exited = remote("devbox", "b", Status::Exited, at);
    exited.attention = true;
    assert_eq!(exited.status(at), Status::Exited);

    let mut t = Tracker::default();
    t.update(vec![remote("devbox", "b", Status::Exited, at), remote("devbox", "c", Status::Idle, at), probe(1, 0, AgentKind::Codex, at)], at);
    let statuses: Vec<Status> = t.agents().iter().map(|a| a.status).collect();
    assert_eq!(statuses, vec![Status::Idle, Status::Idle, Status::Exited]);
    assert!(matches!(&t.agents()[2].source, Source::Remote { session, .. } if session == "b"));
    assert_eq!(t.counts(), Counts { needs_you: 0, working: 0, idle: 2, exited: 1 });
    assert_eq!(t.counts().summary(), "2 idle · 1 exited");
}
