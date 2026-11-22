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
    app.toggle_explorer();
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
