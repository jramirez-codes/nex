//! What counts as an agent, how its status is read, and how the list is ordered.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use egui::{Key, Modifiers};

use super::AgentsPane;
use super::model::{AgentKind, Counts, Probe, Source, Status, Team, Tracker, elapsed, is_remote_key, is_sub_key, remote_key, sub_key};

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
        team: Team::default(),
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
fn the_live_title_is_kept_as_the_task_beside_the_name() {
    let now = Instant::now();
    let mut p = probe(1, 0, AgentKind::Claude, now);
    p.session_name = Some("build-2".into());
    let mut t = Tracker::default();
    let mut seen = |p: &Probe| {
        t.update(vec![p.clone()], now);
        let a = &t.agents()[0];
        (a.name.clone(), a.task.clone(), a.session.clone())
    };
    assert_eq!(seen(&p), ("build-2".into(), None, Some("build-2".into())), "no title: no task yet");
    p.title = "claude".into();
    assert_eq!(seen(&p).1, None, "the program's own name isn't a task");
    p.title = "Fix the login redirect".into();
    assert_eq!(seen(&p), ("Fix the login redirect".into(), Some("Fix the login redirect".into()), Some("build-2".into())));
    p.title = "Write its tests".into();
    assert_eq!(seen(&p).1.as_deref(), Some("Write its tests"), "it follows the title");

    // A pipeline's title is how its run stands: its card keeps its name.
    p.kind = AgentKind::Pipeline;
    p.session_name = Some("feature".into());
    p.title = "2 building · 1 merged — ✓ build approved".into();
    assert_eq!(seen(&p), ("feature".into(), Some("2 building · 1 merged — ✓ build approved".into()), Some("feature".into())));
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
    assert_eq!(t.counts(), Counts { needs_you: 1, done: 0, working: 1, idle: 2 });
    assert_eq!(t.counts().summary(), "1 needs you · 1 working · 2 idle");

    // Still idle a minute later: its clock keeps running. Working → idle restarts it.
    let later = start + Duration::from_secs(60);
    working.session_status = Some("idle".into());
    t.update(vec![idle, working, also_idle], later);
    let since = |pane| t.agents().iter().find(|a| a.pane == pane).unwrap().since;
    assert_eq!(since(5), start);
    assert_eq!(since(3), later);
    assert_eq!(t.counts().summary(), "1 done · 2 idle", "it stopped working while nobody looked");
}

#[test]
fn summaries_and_elapsed_times() {
    assert_eq!(Counts::default().summary(), "No agents running");
    assert_eq!(Counts { needs_you: 2, working: 1, ..Counts::default() }.summary(), "2 need you · 1 working");
    assert_eq!(Counts { working: 3, idle: 1, ..Counts::default() }.headline(), "3 working");
    assert_eq!(Counts { done: 1, working: 3, ..Counts::default() }.headline(), "1 done");
    assert_eq!(Counts::default().headline(), "None running");
    assert_eq!(elapsed(Duration::from_secs(2)), "now");
    assert_eq!(elapsed(Duration::from_secs(42)), "42s");
    assert_eq!(elapsed(Duration::from_secs(600)), "10m");
    assert_eq!(elapsed(Duration::from_secs(7300)), "2h");
    assert_eq!(elapsed(Duration::from_secs(200_000)), "2d");
}

fn at(x: f32, y: f32, w: f32, h: f32) -> egui::Rect {
    egui::Rect::from_min_size(egui::pos2(x, y), egui::vec2(w, h))
}

#[test]
fn keys_move_through_cards_and_jump() {
    let mut a = AgentsPane::new();
    // Two to a row: 10 11 / 12 13 / 14.
    a.targets = vec![
        (10, at(0.0, 0.0, 300.0, 74.0)),
        (11, at(310.0, 0.0, 300.0, 74.0)),
        (12, at(0.0, 84.0, 300.0, 74.0)),
        (13, at(310.0, 84.0, 300.0, 74.0)),
        (14, at(0.0, 168.0, 300.0, 74.0)),
    ];
    a.selected = Some(10);
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(12));
    assert!(a.key(Key::L, Modifiers::NONE));
    assert_eq!(a.selected, Some(13));
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(14), "the nearest one in the row below");
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(14), "nothing below the last");
    assert!(a.key(Key::Home, Modifiers::NONE));
    assert_eq!(a.selected, Some(10));
    assert!(a.key(Key::Enter, Modifiers::NONE));
    assert_eq!(a.requests, vec![super::Request::Focus(10)]);
    // Shortcuts with modifiers aren't the pane's.
    assert!(!a.key(Key::J, Modifiers::CTRL));
    assert!(!a.key(Key::Q, Modifiers::NONE));
}

#[test]
fn space_opens_and_closes_a_team_and_keys_go_through_its_helpers() {
    let mut a = AgentsPane::new();
    // A team (lead 1, helpers 2 and 3) across the row, with a card under it.
    a.team_of = std::collections::HashMap::from([(1, 1), (2, 1), (3, 1)]);
    a.targets = vec![(1, at(0.0, 0.0, 620.0, 74.0)), (9, at(0.0, 200.0, 300.0, 74.0))];
    a.selected = Some(1);
    assert!(a.key(Key::Space, Modifiers::NONE));
    assert_eq!(a.expanded.get(&1), Some(&false), "open");
    assert!(a.requests.is_empty(), "Space on a team opens it rather than jumping");
    // Drawn open: its rows are targets after it.
    a.targets = vec![(1, at(0.0, 0.0, 620.0, 74.0)), (2, at(12.0, 76.0, 596.0, 46.0)), (3, at(12.0, 122.0, 596.0, 46.0)), (9, at(0.0, 200.0, 300.0, 74.0))];
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(2));
    assert!(a.key(Key::ArrowDown, Modifiers::NONE));
    assert_eq!(a.selected, Some(3));
    assert!(a.key(Key::Enter, Modifiers::NONE));
    assert_eq!(a.requests, vec![super::Request::Focus(3)], "Enter jumps to the helper");
    // Closing it from a helper takes the highlight back up to the team.
    assert!(a.key(Key::Space, Modifiers::NONE));
    assert!(a.expanded.is_empty());
    assert_eq!(a.selected, Some(1));
    // Space on a card that isn't a team jumps to it, as before.
    a.selected = Some(9);
    a.requests.clear();
    assert!(a.key(Key::Space, Modifiers::NONE));
    assert_eq!(a.requests, vec![super::Request::Focus(9)]);
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
fn what_nex_host_reports_is_the_status_and_exited_agents_are_not_listed() {
    let at = Instant::now();
    let mut needs = remote("devbox", "a", Status::Working, at);
    needs.attention = true;
    assert_eq!(needs.status(at), Status::NeedsYou, "the pane showing it heard the bell");
    assert_eq!(remote("devbox", "b", Status::Idle, at).status(at), Status::Idle);

    use nex_host::proto::Status as Host;
    assert_eq!(Status::from_host(Host::Working), Some(Status::Working));
    assert_eq!(Status::from_host(Host::Exited), None);
}

fn claude(pane: u64, status: &str, at: Instant) -> Probe {
    Probe { session_status: Some(status.into()), ..probe(pane, 0, AgentKind::Claude, at) }
}

fn status_of(t: &Tracker, pane: u64) -> Status {
    t.agents().iter().find(|a| a.pane == pane).unwrap().status
}

#[test]
fn an_agent_that_stops_working_unseen_is_done_until_its_pane_is_looked_at() {
    let at = Instant::now();
    let mut t = Tracker::default();
    // Idle from the start: nothing was finished.
    t.update(vec![claude(1, "idle", at)], at);
    assert_eq!(status_of(&t, 1), Status::Idle);
    assert!(!t.wants_you(1));

    t.update(vec![claude(1, "busy", at)], at);
    t.update(vec![claude(1, "idle", at)], at);
    assert_eq!(status_of(&t, 1), Status::Done);
    assert!(t.wants_you(1), "its tab gets the dot");
    assert!(t.take_alerted() && !t.take_alerted(), "the window is flagged once");
    // Still done on the next poll, and the dot doesn't flag the window again.
    t.update(vec![claude(1, "idle", at)], at);
    assert_eq!(status_of(&t, 1), Status::Done);
    assert!(!t.take_alerted());

    // Looking at another pane changes nothing; looking at its pane makes it idle.
    t.mark_seen(Some(2), at);
    assert_eq!(status_of(&t, 1), Status::Done);
    let later = at + Duration::from_secs(5);
    t.mark_seen(Some(1), later);
    assert_eq!(status_of(&t, 1), Status::Idle);
    assert_eq!(t.agents()[0].since, later);
    assert!(!t.wants_you(1));
    t.mark_seen(None, later);
    t.update(vec![claude(1, "idle", at)], later);
    assert_eq!(status_of(&t, 1), Status::Idle, "and stays idle");

    // Working again, then done again.
    t.update(vec![claude(1, "busy", at)], later);
    assert_eq!(status_of(&t, 1), Status::Working);
    t.update(vec![claude(1, "idle", at)], later);
    assert_eq!(status_of(&t, 1), Status::Done);
}

#[test]
fn an_agent_that_finishes_while_watched_goes_straight_to_idle() {
    let at = Instant::now();
    let mut t = Tracker::default();
    t.update(vec![claude(1, "busy", at)], at);
    t.mark_seen(Some(1), at);
    t.update(vec![claude(1, "idle", at)], at);
    assert_eq!(status_of(&t, 1), Status::Idle);
    assert!(!t.wants_you(1) && !t.take_alerted());
}

#[test]
fn sorts_done_between_needs_you_and_working() {
    let at = Instant::now();
    let mut t = Tracker::default();
    t.update(vec![claude(1, "busy", at), claude(2, "busy", at), claude(3, "busy", at)], at);
    t.update(vec![claude(1, "busy", at), claude(2, "idle", at), claude(3, "waiting_for_permission", at)], at);
    let order: Vec<(u64, Status)> = t.agents().iter().map(|a| (a.pane, a.status)).collect();
    assert_eq!(order, vec![(3, Status::NeedsYou), (2, Status::Done), (1, Status::Working)]);
    assert_eq!(t.counts().summary(), "1 needs you · 1 done · 1 working");
}

#[test]
fn an_agent_over_ssh_lights_the_tab_of_the_pane_showing_it() {
    let at = Instant::now();
    let shown = |status| {
        let mut p = remote("devbox", "a", status, at);
        if let Source::Remote { pane, .. } = &mut p.source {
            *pane = Some(7);
        }
        p
    };
    let key = remote_key("devbox", "a");
    let mut t = Tracker::default();
    t.update(vec![shown(Status::Working)], at);
    assert!(!t.wants_you(7));
    // nex-host says it wants input; no bell reached the pane.
    t.update(vec![shown(Status::NeedsYou)], at);
    assert!(t.wants_you(7));
    assert!(!t.wants_you(key), "the dot goes on the pane, not the card's key");
    t.mark_seen(Some(7), at);
    assert!(!t.wants_you(7));
    t.mark_seen(None, at);
    assert_eq!(status_of(&t, key), Status::NeedsYou, "it still needs you; only the dot is answered");

    // Done while nobody looks, then the pane showing it is looked at.
    t.update(vec![shown(Status::Working)], at);
    t.update(vec![shown(Status::Idle)], at);
    assert_eq!(status_of(&t, key), Status::Done);
    assert!(t.wants_you(7));
    t.mark_seen(Some(7), at);
    assert_eq!(status_of(&t, key), Status::Idle);

    // An agent no pane shows lights no tab, but stays done until it's opened.
    let mut t = Tracker::default();
    t.update(vec![remote("devbox", "b", Status::Working, at)], at);
    t.update(vec![remote("devbox", "b", Status::Idle, at)], at);
    assert_eq!(status_of(&t, remote_key("devbox", "b")), Status::Done);
}

#[test]
fn sub_agents_are_not_counted_and_never_ask_for_you() {
    let now = Instant::now();
    let lead = probe(1, 0, AgentKind::Claude, now);
    let sub = |id: &str, status| Probe {
        pane: sub_key(1, id),
        title: format!("task {id}"),
        source: Source::Sub { lead: 1, pane: Some(1) },
        reported: Some(status),
        team: Team { parent: Some(1), role: Some("Explore".into()), started: 0, sub: true },
        ..lead.clone()
    };
    let mut t = Tracker::default();
    t.update(vec![lead.clone(), sub("a", Status::Working)], now);
    assert_eq!(t.counts().total(), 1, "the agent, not its sub-agent");
    t.update(vec![lead.clone(), sub("a", Status::Done)], now + Duration::from_secs(5));
    assert!(!t.take_alerted() && !t.wants_you(1), "its agent's pane says when it wants you");
    assert_eq!(t.agents().iter().find(|a| a.team.sub).map(|a| a.status), Some(Status::Done));

    let k = sub_key(remote_key("h", "s"), "a");
    assert!(is_sub_key(k) && !is_remote_key(k) && !is_sub_key(remote_key("h", "s")) && !is_sub_key(7));
    assert_eq!(k, sub_key(remote_key("h", "s"), "a"));
    assert_ne!(k, sub_key(remote_key("h", "s"), "b"));
}
