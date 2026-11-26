//! A thin client (Android, `--remote`) coming back to a layout whose only pane showed an
//! agent that was stopped from another connection meanwhile: the pane's session is gone, so
//! the pane ends. The window must not close (an Android app can't open it again without a
//! restart); it opens a new shell on the host in its place.

use std::sync::Arc;

use egui::{Pos2, RawInput, Rect, Vec2, ViewportCommand, ViewportId};

use crate::app::App;
use crate::app::tests::new_app;
use crate::config::Config;
use crate::remote::ssh::Server;
use crate::remote::ssh::host::RemoteHost;

/// One frame; whether it asked to close the window.
fn frame(app: &mut App, ctx: &egui::Context) -> bool {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(420.0, 800.0))), ..Default::default() };
    let out = ctx.run(input, |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()));
    out.viewport_output.get(&ViewportId::ROOT).is_some_and(|v| v.commands.contains(&ViewportCommand::Close))
}

/// A window connected to a host (one nothing answers on: its panes keep trying to reach it,
/// so none ends unless the test says so), showing one tab attached to agent `agent`, as when
/// it was picked in the Agents pane before the phone slept.
fn connected_to_agent(ctx: &egui::Context, agent: &str) -> App {
    let mut app = new_app(ctx, Config::default());
    app.connect.active = true;
    let server = Server { host: "127.0.0.1".into(), port: 1, user: "me".into() };
    app.connect.host = Some(Arc::new(RemoteHost::offline(server)));
    // The connect screen's frame, then the tab opened as the Agents pane opens it.
    assert!(!frame(&mut app, ctx));
    app.launch.attach_next = Some(agent.into());
    let _ = ctx.run(RawInput::default(), |ctx| app.new_tab(ctx, None, None));
    app.launch.attach_next = None;
    assert!(!frame(&mut app, ctx));
    assert_eq!(app.tabs.len(), 1);
    app
}

/// The only pane, and the session it shows (`None`: a new shell, not started yet).
fn only_pane(app: &App) -> (crate::layout::PaneId, Option<String>, bool) {
    let id = app.tabs[0].focused;
    let remote = app.panes.terminal(id).and_then(|t| t.remote.as_ref()).expect("a pane on the host");
    let session = remote.session.lock().unwrap().clone();
    (id, session, remote.keep)
}

/// What the pane's pump does when the host says the session isn't there any more.
fn session_gone(app: &mut App, id: crate::layout::PaneId) {
    app.panes.terminal_mut(id).unwrap().exited = true;
}

#[test]
fn a_layout_whose_only_agent_is_gone_opens_a_new_shell() {
    let ctx = egui::Context::default();
    let mut app = connected_to_agent(&ctx, "gone1");
    let (agent_pane, session, keep) = only_pane(&app);
    assert_eq!((session.as_deref(), keep), (Some("gone1"), true));

    session_gone(&mut app, agent_pane);
    assert!(!frame(&mut app, &ctx), "the window closed when its agent was gone");
    assert_eq!(app.tabs.len(), 1, "no tab took the agent's place");
    let (pane, session, keep) = only_pane(&app);
    assert_ne!(pane, agent_pane);
    assert_eq!((session, keep), (None, false), "the new pane should be a new shell");
    assert!(!frame(&mut app, &ctx));

    // The new shell ending right away too: back to the connect screen, not around again.
    session_gone(&mut app, pane);
    assert!(!frame(&mut app, &ctx), "the window closed when the new shell ended");
    assert!(app.tabs.is_empty());
    for _ in 0..3 {
        assert!(!frame(&mut app, &ctx), "the connect screen closed the window");
    }
    assert!(app.tabs.is_empty(), "it opened another shell");
}

#[test]
fn exiting_the_last_shell_still_closes_a_desktop_remote_window() {
    let ctx = egui::Context::default();
    let mut app = connected_to_agent(&ctx, "a1");
    let (pane, _, _) = only_pane(&app);
    // The user typed `exit` a moment ago.
    let now = std::time::Instant::now();
    app.session.last_user_input = Some(now);
    let t = app.panes.terminal_mut(pane).unwrap();
    t.last_input = now;
    t.exited = true;
    assert!(frame(&mut app, &ctx), "the user's own exit should close the window");
}
