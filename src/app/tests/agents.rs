//! Ctrl+Shift+A opens an Agents pane beside the focused pane and closes it again; it draws
//! (empty, and full of agents at every width), jumps to an agent, and comes back with the
//! session. The title bar's label opens the Agents tab, while `--explorer` still gets Files.

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::agents::model::{AgentKind, Probe, Source, Status, remote_key};
use crate::app::actions::Action;
use crate::config::Config;
use crate::session::PaneKind;

use super::{new_app, run};

fn probe(pane: u64, tab: Option<usize>, kind: AgentKind, status: &str) -> Probe {
    let now = Instant::now();
    Probe {
        pane,
        tab,
        kind,
        title: "Refactor the session reader".into(),
        mark: Some("✳".into()),
        cwd: Some(PathBuf::from("/home/u/code/nex")),
        session_name: None,
        session_status: Some(status.into()),
        attention: false,
        last_output: now - Duration::from_secs(30),
        last_input: now - Duration::from_secs(30),
        source: Source::Local,
        reported: None,
        team: Default::default(),
    }
}

#[test]
fn ctrl_shift_a_toggles_an_agents_pane_that_draws_jumps_and_restores() {
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let shell = app.focused_pane().unwrap();
    app.pending.push(Action::NewTab);
    run(&mut app, &ctx, 1400.0);
    let other = app.focused_pane().unwrap();
    assert_ne!(shell, other);

    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);
    let id = *app.agents.keys().next().unwrap();
    assert_eq!(app.focused_pane(), Some(id));
    assert_eq!(app.pane_label(id).0, "Agents");
    // Empty, then with agents (pretend both shells run one), wide to narrow.
    for width in [1400.0, 500.0] {
        run(&mut app, &ctx, width);
    }
    // Hold off the process poll, which would find no agents in these shells.
    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    let agents = vec![probe(shell, Some(0), AgentKind::Claude, "busy"), probe(other, Some(1), AgentKind::Codex, "waiting")];
    app.agent_tracker.update(agents, Instant::now());
    for width in [1400.0, 900.0, 700.0, 420.0, 260.0] {
        for _ in 0..2 {
            run(&mut app, &ctx, width);
        }
    }
    // The one that needs you comes first; Enter jumps to it, in this tab.
    assert_eq!(app.agents[&id].selected, Some(other));
    app.agents.get_mut(&id).unwrap().key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    app.agents.get_mut(&id).unwrap().key(egui::Key::Enter, egui::Modifiers::NONE);
    run(&mut app, &ctx, 1400.0);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.focused_pane(), Some(shell), "jumped to the first tab's agent");
    assert_eq!(app.active, 0);

    // Back on the second tab, the pane is saved with the session.
    app.show_tab(1);
    let s = app.snapshot_session().unwrap();
    let kinds: Vec<PaneKind> = s.tabs[1].root.leaves().iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PaneKind::Terminal, PaneKind::Agents]);

    // Ctrl+Shift+A again closes it, and focus goes back to the shell it came from.
    if let Some(t) = app.cur_tab_mut() {
        t.focused = id;
    }
    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);
    assert!(app.agents.is_empty());
    assert_eq!(app.focused_pane(), Some(other));
}

#[test]
fn the_title_bar_label_opens_the_agents_tab() {
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1200.0);
    assert!(app.shows_agents_label());
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1200.0);
    assert!(app.home_active);
    let id = app.focused_pane().unwrap();
    assert!(app.agents.contains_key(&id), "the Agents tab holds an Agents pane");
    assert!(app.explorers.is_empty());
    for width in [1200.0, 600.0] {
        run(&mut app, &ctx, width);
    }
    // Clicking it again goes back to the terminal.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1200.0);
    assert!(!app.home_active);
    assert!(app.panes.contains_key(&app.focused_pane().unwrap()));
    // An Agents tab on its own doesn't keep the window open.
    assert!(!app.home_keeps_window());
}

#[test]
fn explorer_mode_still_opens_files() {
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1200.0);
    app.toggle_explorer(&ctx);
    run(&mut app, &ctx, 1200.0);
    let id = app.focused_pane().unwrap();
    assert!(app.explorers.contains_key(&id));
    assert!(app.home_is_files() && !app.shows_agents_label());
    assert!(app.home_keeps_window());
}

#[test]
fn an_agent_on_an_ssh_host_opens_in_a_new_tab_attached_to_it() {
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);
    let id = *app.agents.keys().next().unwrap();
    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);

    // A detached agent on a host no pane is connected to. Its ssh never gets anywhere (a
    // proxy that just waits), so the tab it opens stays open for the test.
    let opts = vec!["-o".to_string(), "ProxyCommand=sleep 5".to_string()];
    let target = crate::remote::SshTarget { opts, destination: "nex-test.invalid".into() };
    let key = remote_key("nex-test.invalid", "3fa91c0e");
    let detached = Probe {
        pane: key,
        tab: None,
        reported: Some(Status::Working),
        source: Source::Remote { host: "nex-test.invalid".into(), target: target.clone(), session: "3fa91c0e".into(), pane: None },
        ..probe(0, None, AgentKind::Claude, "")
    };
    app.agent_tracker.update(vec![detached], Instant::now());
    for width in [1400.0, 420.0] {
        run(&mut app, &ctx, width);
    }
    assert_eq!(app.agents[&id].selected, Some(key));

    let tabs = app.tabs.len();
    app.agents.get_mut(&id).unwrap().key(egui::Key::Enter, egui::Modifiers::NONE);
    run(&mut app, &ctx, 1400.0);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs.len(), tabs + 1, "opened a tab on it");
    for _ in 0..3 {
        run(&mut app, &ctx, 1400.0);
    }
    assert_eq!(app.active, tabs, "and shows it");
    assert!(!app.home_active);
}

#[test]
fn delete_on_a_running_agent_on_an_ssh_host_asks_before_stopping_it() {
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    let target = crate::remote::SshTarget { opts: vec![], destination: "nex-test.invalid".into() };
    let key = remote_key("nex-test.invalid", "3fa91c0e");
    let remote = |status| Probe {
        pane: key,
        tab: None,
        reported: Some(status),
        source: Source::Remote { host: "nex-test.invalid".into(), target: target.clone(), session: "3fa91c0e".into(), pane: None },
        title: "Fix the tests".into(),
        ..probe(0, None, AgentKind::Claude, "")
    };

    app.agent_tracker.update(vec![remote(Status::Working)], Instant::now());
    app.dismiss_agent(key);
    match &app.ssh.agent_prompt {
        Some(crate::app::ssh::hosts::AgentPrompt::Stop { session, what, .. }) => {
            assert_eq!(session, "3fa91c0e");
            assert!(what.contains("Fix the tests"), "{what}");
        }
        other => panic!("expected the stop alert, got {other:?}"),
    }
    assert!(app.ssh_modal_open(), "the alert takes the keyboard");
    run(&mut app, &ctx, 1400.0);
    assert!(app.ssh.agent_prompt.is_some(), "and stays up until answered");
}

#[test]
fn a_tab_keeps_its_terminals_name_while_its_agents_pane_has_focus() {
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let shell = app.focused_pane().unwrap();
    app.pending.push(Action::NewTab);
    run(&mut app, &ctx, 1400.0);
    app.show_tab(0);
    run(&mut app, &ctx, 1400.0);

    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);
    let id = *app.agents.keys().next().unwrap();
    assert_eq!(app.focused_pane(), Some(id));
    assert_eq!(app.tab_header_pane(&app.tabs[0]), shell, "the header still names the shell");

    // Away and back, the Agents pane still focused: still the shell.
    app.show_tab(1);
    run(&mut app, &ctx, 1400.0);
    app.show_tab(0);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.focused_pane(), Some(id));
    assert_eq!(app.tab_header_pane(&app.tabs[0]), shell);

    // With the shell gone, the tab is named after its Agents pane.
    app.close_pane(&ctx, shell);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.tab_header_pane(&app.tabs[0]), id);
    assert_eq!(app.pane_label(id).0, "Agents");
}

#[test]
fn the_agents_on_screen_are_marked_viewing_both_halves_of_a_split_and_where_a_card_jumped_not_from_the_agents_tab() {
    use crate::agents::model::{Seen, Viewing};
    use crate::layout::Axis;
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let left = app.focused_pane().unwrap();
    app.pending.push(Action::Split(Axis::Horizontal));
    run(&mut app, &ctx, 1400.0);
    let right = app.focused_pane().unwrap();
    assert_ne!(left, right);
    app.pending.push(Action::NewTab);
    run(&mut app, &ctx, 1400.0);
    let other = app.focused_pane().unwrap();

    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    // Pretend the three shells run agents (opening the Agents tab polls, finding none).
    let pretend = |app: &mut crate::app::App| {
        let agents = vec![probe(left, Some(0), AgentKind::Claude, "busy"), probe(right, Some(0), AgentKind::Codex, "busy"), probe(other, Some(1), AgentKind::Claude, "idle")];
        app.agent_tracker.update(agents, Instant::now());
    };
    let seen = |app: &crate::app::App, v: &Viewing, pane| v.of(app.agent_tracker.agents().iter().find(|a| a.pane == pane).unwrap());

    // On the Agents tab, no terminal tab is on screen: nothing is viewed.
    app.show_tab(0);
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    let id = app.focused_pane().unwrap();
    assert!(app.agents.contains_key(&id));
    pretend(&mut app);
    run(&mut app, &ctx, 1400.0);
    let v = app.viewing(id);
    assert_eq!(v, Viewing::default());
    assert_eq!((seen(&app, &v, left), seen(&app, &v, right), seen(&app, &v, other)), (None, None, None));

    // Back in the first tab, its split: both its agents, the focused one first.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(!app.home_active);
    let v = app.viewing(id);
    assert_eq!(seen(&app, &v, right), Some(Seen::Primary));
    assert_eq!(seen(&app, &v, left), Some(Seen::Shared));
    assert_eq!(seen(&app, &v, other), None);

    // A card jumps to the second tab's agent: in that tab, it's the one viewed; back on the
    // Agents tab, nothing is.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    app.agents.get_mut(&id).unwrap().selected = Some(other);
    app.agents.get_mut(&id).unwrap().key(egui::Key::Enter, egui::Modifiers::NONE);
    run(&mut app, &ctx, 1400.0);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.focused_pane(), Some(other));
    assert!(!app.home_active);
    pretend(&mut app);
    let v = app.viewing(id);
    assert_eq!(seen(&app, &v, other), Some(Seen::Primary));
    assert_eq!((seen(&app, &v, left), seen(&app, &v, right)), (None, None));
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(app.home_active);
    pretend(&mut app);
    assert_eq!(app.viewing(id), Viewing::default());
    for width in [1400.0, 420.0] {
        run(&mut app, &ctx, width);
    }
}

#[test]
fn from_the_agents_tab_a_pinned_agent_is_not_marked_viewing_either() {
    use crate::agents::model::{Seen, Viewing};
    use crate::layout::Axis;
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let left = app.focused_pane().unwrap();
    app.pending.push(Action::Split(Axis::Horizontal));
    run(&mut app, &ctx, 1400.0);
    let pinned = app.focused_pane().unwrap();
    app.pending.push(Action::ToggleSticky(pinned));
    run(&mut app, &ctx, 1400.0);

    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    let pretend = |app: &mut crate::app::App| {
        let agents = vec![probe(left, Some(0), AgentKind::Claude, "busy"), probe(pinned, Some(0), AgentKind::Codex, "busy")];
        app.agent_tracker.update(agents, Instant::now());
    };
    let seen = |app: &crate::app::App, v: &Viewing, pane| v.of(app.agent_tracker.agents().iter().find(|a| a.pane == pane).unwrap());

    // In the terminal tab, the pinned pane is one of its panes like any other.
    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);
    let split = *app.agents.keys().next().unwrap();
    pretend(&mut app);
    let v = app.viewing(split);
    assert_eq!(seen(&app, &v, pinned), Some(Seen::Primary));
    assert_eq!(seen(&app, &v, left), Some(Seen::Shared));
    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);

    // The Agents tab holds the pinned pane too: neither its card nor the other agent's is
    // ringed there.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(app.home_active);
    assert!(app.home_tab.as_ref().unwrap().contains(pinned));
    let id = app.home_tab.as_ref().unwrap().root.iter().find(|p| app.agents.contains_key(p)).unwrap();
    pretend(&mut app);
    let v = app.viewing(id);
    assert_eq!(seen(&app, &v, pinned), None);
    assert_eq!(seen(&app, &v, left), None);
    run(&mut app, &ctx, 1400.0);

    // Back in the terminal tab, it's marked again.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(!app.home_active);
    let v = app.viewing(id);
    assert_eq!(seen(&app, &v, pinned), Some(Seen::Primary));
}

#[test]
fn switching_to_the_agents_tab_clears_every_viewing_mark() {
    use crate::agents::model::{Seen, Viewing};
    use crate::layout::Axis;
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let left = app.focused_pane().unwrap();
    app.pending.push(Action::Split(Axis::Horizontal));
    run(&mut app, &ctx, 1400.0);
    let right = app.focused_pane().unwrap();

    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    let pretend = |app: &mut crate::app::App| {
        let agents = vec![probe(left, Some(0), AgentKind::Claude, "busy"), probe(right, Some(0), AgentKind::Codex, "busy")];
        app.agent_tracker.update(agents, Instant::now());
    };
    let seen = |app: &crate::app::App, v: &Viewing, pane| v.of(app.agent_tracker.agents().iter().find(|a| a.pane == pane).unwrap());

    // An Agents pane split beside the terminals marks both of them.
    app.pending.push(Action::ToggleAgents);
    run(&mut app, &ctx, 1400.0);
    let split = *app.agents.keys().next().unwrap();
    pretend(&mut app);
    let v = app.viewing(split);
    assert_eq!(seen(&app, &v, right), Some(Seen::Primary));
    assert_eq!(seen(&app, &v, left), Some(Seen::Shared));

    // On the Agents tab, no Agents pane marks anything.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(app.home_active);
    pretend(&mut app);
    run(&mut app, &ctx, 1400.0);
    for id in app.agents.keys().copied().collect::<Vec<_>>() {
        let v = app.viewing(id);
        assert_eq!(v, Viewing::default());
        assert_eq!((seen(&app, &v, left), seen(&app, &v, right)), (None, None));
    }
}

#[test]
fn a_card_jumped_to_from_the_agents_tab_is_not_left_highlighted() {
    use crate::agents::model::Viewing;
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let first = app.focused_pane().unwrap();
    app.pending.push(Action::NewTab);
    run(&mut app, &ctx, 1400.0);
    let second = app.focused_pane().unwrap();
    app.show_tab(0);
    run(&mut app, &ctx, 1400.0);

    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    let pretend = |app: &mut crate::app::App| {
        let agents = vec![probe(first, Some(0), AgentKind::Claude, "busy"), probe(second, Some(1), AgentKind::Codex, "busy")];
        app.agent_tracker.update(agents, Instant::now());
    };

    // On the Agents tab, the keyboard's highlight is on a card.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    let id = app.home_tab.as_ref().unwrap().root.iter().find(|p| app.agents.contains_key(p)).unwrap();
    pretend(&mut app);
    run(&mut app, &ctx, 1400.0);
    assert!(app.agents[&id].highlighted().is_some());

    // Enter on the second tab's card swaps to that tab.
    app.agents.get_mut(&id).unwrap().selected = Some(second);
    app.agents.get_mut(&id).unwrap().key(egui::Key::Enter, egui::Modifiers::NONE);
    run(&mut app, &ctx, 1400.0);
    run(&mut app, &ctx, 1400.0);
    assert!(!app.home_active);
    assert_eq!(app.focused_pane(), Some(second));
    assert_eq!(app.agents[&id].highlighted(), None, "the card it jumped from isn't left ringed");

    // Back on the Agents tab, no card is ringed, neither as viewed nor as the highlight.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(app.home_active);
    pretend(&mut app);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.viewing(id), Viewing::default());
    assert_eq!(app.agents[&id].highlighted(), None);
    assert_eq!(app.agents[&id].selected, Some(second), "Enter still goes where it went");

    // A key brings the highlight back.
    app.agents.get_mut(&id).unwrap().key(egui::Key::ArrowLeft, egui::Modifiers::NONE);
    assert!(app.agents[&id].highlighted().is_some());
}

#[test]
fn on_the_agents_tab_a_terminal_opened_there_marks_its_ssh_agent_viewing_and_only_it() {
    use crate::agents::model::{Seen, Viewing};
    use crate::layout::Axis;
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let shell = app.focused_pane().unwrap();
    app.pending.push(Action::Split(Axis::Horizontal));
    run(&mut app, &ctx, 1400.0);
    let pinned = app.focused_pane().unwrap();
    app.pending.push(Action::ToggleSticky(pinned));
    run(&mut app, &ctx, 1400.0);

    // On the Agents tab, open a terminal beside its Agents pane.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(app.home_active);
    let id = app.focused_pane().unwrap();
    assert!(app.agents.contains_key(&id));
    app.pending.push(Action::Split(Axis::Horizontal));
    run(&mut app, &ctx, 1400.0);
    let term = app.focused_pane().unwrap();
    assert!(app.panes.contains_key(&term), "a terminal");
    assert!(app.home_tab.as_ref().unwrap().contains(term));

    // In it, Claude on an ssh host (shown by that pane); agents in the terminal tab's shell and
    // the pinned pane too.
    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);
    let target = crate::remote::SshTarget { opts: vec![], destination: "nex-test.invalid".into() };
    let key = remote_key("nex-test.invalid", "3fa91c0e");
    let pretend = |app: &mut crate::app::App| {
        let remote = Probe {
            pane: key,
            tab: None,
            reported: Some(Status::Working),
            source: Source::Remote { host: "nex-test.invalid".into(), target: target.clone(), session: "3fa91c0e".into(), pane: Some(term) },
            ..probe(0, None, AgentKind::Claude, "")
        };
        let agents = vec![probe(shell, Some(0), AgentKind::Codex, "busy"), probe(pinned, Some(0), AgentKind::Claude, "busy"), remote];
        app.agent_tracker.update(agents, Instant::now());
    };
    let seen = |app: &crate::app::App, v: &Viewing, key| v.of(app.agent_tracker.agents().iter().find(|a| a.pane == key).unwrap());
    let marks = |app: &crate::app::App, v: &Viewing| (seen(app, v, key), seen(app, v, shell), seen(app, v, pinned));
    pretend(&mut app);
    run(&mut app, &ctx, 1400.0);

    // The terminal has focus: its agent is the one viewed, and the only one.
    assert_eq!(marks(&app, &app.viewing(id)), (Some(Seen::Primary), None, None));

    // Focus back on the Agents pane: still that terminal's agent, the one beside it.
    app.home_tab.as_mut().unwrap().focused = id;
    run(&mut app, &ctx, 1400.0);
    assert_eq!(marks(&app, &app.viewing(id)), (Some(Seen::Primary), None, None));
    for width in [1400.0, 420.0] {
        run(&mut app, &ctx, width);
    }

    // Back in the terminal tab, its own panes are viewed, and the Agents tab's terminal isn't.
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    assert!(!app.home_active);
    pretend(&mut app);
    let v = app.viewing(id);
    assert_eq!(seen(&app, &v, key), None);
    assert!(seen(&app, &v, shell).is_some() && seen(&app, &v, pinned).is_some());
}

#[test]
fn a_sub_agent_opens_in_a_tab_of_its_own_and_a_second_click_focuses_it() {
    use crate::agents::model::{Team, sub_key};
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let claude = app.focused_pane().unwrap();
    app.pending.push(Action::NewTab);
    run(&mut app, &ctx, 1400.0);
    let codex = app.focused_pane().unwrap();
    app.pending.push(Action::NewTab);
    run(&mut app, &ctx, 1400.0);
    let stage_pane = app.focused_pane().unwrap();
    app.pending.push(Action::ToggleAgentsTab);
    run(&mut app, &ctx, 1400.0);
    let id = app.home_tab.as_ref().unwrap().root.iter().find(|p| app.agents.contains_key(p)).unwrap();
    app.last_proc_poll = Instant::now() + Duration::from_secs(3600);

    // A sub-agent of `lead` (the card it's under), as the poll makes them: shown in the
    // pane `app` has following it, if any.
    let sub = |app: &crate::app::App, lead: &Probe, sub_id: &str| {
        let key = sub_key(lead.pane, sub_id);
        let pane = app.sub_views.get(&key).copied();
        Probe {
            pane: key,
            tab: pane.and_then(|p| app.tabs.iter().position(|t| t.contains(p))),
            source: Source::Sub { lead: lead.pane, id: sub_id.into(), pane },
            reported: Some(Status::Working),
            team: Team { parent: Some(lead.pane), role: Some("Explore".into()), started: 1, sub: true },
            ..lead.clone()
        }
    };
    let lead_claude = probe(claude, Some(0), AgentKind::Claude, "busy");
    let lead_codex = probe(codex, Some(1), AgentKind::Codex, "busy");
    // A pipeline stage on an ssh host, attached to in the third tab, running one of its
    // own. Its ssh never gets anywhere (a proxy that just waits): the tab stays open.
    let opts = vec!["-o".to_string(), "ProxyCommand=sleep 5".to_string()];
    let target = crate::remote::SshTarget { opts, destination: "nex-test.invalid".into() };
    let pipe = Probe {
        pane: remote_key("devbox", "pipe"),
        kind: AgentKind::Pipeline,
        source: Source::Remote { host: "devbox".into(), target: target.clone(), session: "pipe".into(), pane: None },
        reported: Some(Status::Working),
        ..probe(0, None, AgentKind::Pipeline, "")
    };
    let stage = Probe {
        pane: remote_key("devbox", "build"),
        tab: Some(2),
        source: Source::Remote { host: "devbox".into(), target, session: "build".into(), pane: Some(stage_pane) },
        reported: Some(Status::Working),
        team: Team { parent: Some(pipe.pane), role: Some("build".into()), started: 1, sub: false },
        ..probe(0, Some(2), AgentKind::Claude, "")
    };
    let pipe_key = pipe.pane;
    let leads = [(lead_claude.clone(), "c1", claude), (lead_codex.clone(), "x1", codex), (stage.clone(), "s1", pipe_key)];
    let poll = |app: &mut crate::app::App| {
        let mut agents = vec![lead_claude.clone(), lead_codex.clone(), pipe.clone(), stage.clone()];
        agents.extend(leads.iter().map(|(lead, s, _)| sub(app, lead, s)));
        app.agent_tracker.update(agents, Instant::now());
    };
    poll(&mut app);
    run(&mut app, &ctx, 1400.0);

    let key = |app: &mut crate::app::App, k| app.agents.get_mut(&id).unwrap().key(k, egui::Modifiers::NONE);
    let to_agents = |app: &mut crate::app::App| {
        if !app.home_active {
            app.pending.push(Action::ToggleAgentsTab);
            run(app, &ctx, 1400.0);
        }
        assert!(app.home_active);
    };
    for (n, (lead, sub_id, card)) in leads.iter().enumerate() {
        let sub_card = sub_key(lead.pane, sub_id);
        to_agents(&mut app);
        // Space opens its card, the arrows go down its rows to it, then Enter (the first
        // time) or Space (the second).
        app.agents.get_mut(&id).unwrap().selected = Some(*card);
        key(&mut app, egui::Key::Space);
        run(&mut app, &ctx, 1400.0);
        for _ in 0..3 {
            if app.agents[&id].selected != Some(sub_card) {
                key(&mut app, egui::Key::ArrowDown);
            }
        }
        assert_eq!(app.agents[&id].selected, Some(sub_card));
        let tabs = app.tabs.len();
        key(&mut app, egui::Key::Enter);
        run(&mut app, &ctx, 1400.0);
        run(&mut app, &ctx, 1400.0);
        assert_eq!(app.tabs.len(), tabs + 1, "sub-agent {n} opened a tab of its own");
        assert!(!app.home_active);
        let view = app.focused_pane().unwrap();
        assert!(![claude, codex, stage_pane].contains(&view), "not its agent's pane");
        assert_eq!(app.sub_views.get(&sub_card), Some(&view));
        poll(&mut app);
        let shown = app.agent_tracker.agents().iter().find(|a| a.pane == sub_card).unwrap().shown_pane();
        assert_eq!(shown, Some(view), "its card's pane is the tab following it");

        // Again, from another tab: that tab is focused, none opened.
        app.show_tab(0);
        to_agents(&mut app);
        app.agents.get_mut(&id).unwrap().selected = Some(sub_card);
        key(&mut app, egui::Key::Space);
        run(&mut app, &ctx, 1400.0);
        run(&mut app, &ctx, 1400.0);
        assert_eq!((app.tabs.len(), app.focused_pane()), (tabs + 1, Some(view)), "sub-agent {n}'s tab, focused");
    }

    // Its agent's pane on screen beside the Agents pane makes no difference.
    app.show_tab(0);
    run(&mut app, &ctx, 1400.0);
    app.split_agents(&ctx, claude, crate::layout::Axis::Horizontal);
    run(&mut app, &ctx, 1400.0);
    let side = app.tabs[0].root.iter().find(|p| app.agents.contains_key(p)).unwrap();
    // (Tracking for real finds no agents in these shells: the poll stands in for it.)
    poll(&mut app);
    run(&mut app, &ctx, 1400.0);
    let c1 = sub_key(claude, "c1");
    let tabs = app.tabs.len();
    app.agents.get_mut(&side).unwrap().selected = Some(claude);
    app.agents.get_mut(&side).unwrap().key(egui::Key::Space, egui::Modifiers::NONE);
    run(&mut app, &ctx, 1400.0);
    app.agents.get_mut(&side).unwrap().selected = Some(c1);
    app.agents.get_mut(&side).unwrap().key(egui::Key::Enter, egui::Modifiers::NONE);
    run(&mut app, &ctx, 1400.0);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs.len(), tabs);
    assert_eq!(app.focused_pane(), app.sub_views.get(&c1).copied(), "its tab, not the agent's pane already on screen");

    // Closing the tab closes only the view: the next click opens a new one.
    let view = app.sub_views[&c1];
    app.close_pane(&ctx, view);
    run(&mut app, &ctx, 1400.0);
    app.track_agents();
    assert!(!app.sub_views.contains_key(&c1));
    assert!(app.panes.contains_key(&claude), "its agent's pane is untouched");
    poll(&mut app);
    let tabs = app.tabs.len();
    app.focus_agent(&ctx, c1);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.tabs.len(), tabs + 1);
}
