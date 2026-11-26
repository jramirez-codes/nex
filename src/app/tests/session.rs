//! A session snapshotted, restored into a fresh window, and snapshotted again comes back the same.

use egui::{Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::layout::Axis;
use crate::session::{PaneKind, PaneNode, PaneState, Session, TabState};

fn frames(ctx: &egui::Context, app: &mut App, n: usize) {
    for i in 0..n {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(i as f64 * 0.016),
            ..Default::default()
        };
        let _ = ctx.run(input, |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()));
    }
}

/// A window that never touches the real session file, restoring `s` on its first frame.
fn restored(s: Session) -> (egui::Context, App) {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let args = crate::cli::Args { no_restore: true, ..Default::default() };
    let mut app = App::new(&cc, crate::config::Config::default(), args, None, Some(s), None);
    frames(&ctx, &mut app, 3);
    (ctx, app)
}

fn leaf(kind: PaneKind, cwd: &std::path::Path, font_size: Option<f32>) -> PaneNode {
    PaneNode::Leaf(PaneState { kind, cwd: Some(cwd.to_path_buf()), font_size, command: None, sticky: false })
}

#[test]
fn snapshot_restore_round_trip() {
    let dir = std::env::temp_dir().join(format!("nex-session-{}", std::process::id()));
    let (a, b) = (dir.join("a"), dir.join("b"));
    std::fs::create_dir_all(&a).unwrap();
    std::fs::create_dir_all(&b).unwrap();
    let (a, b) = (a.canonicalize().unwrap(), b.canonicalize().unwrap());
    let split = |axis, ratio, x, y| PaneNode::Split { axis, ratio, a: Box::new(x), b: Box::new(y) };
    let s = Session {
        tabs: vec![
            TabState { root: leaf(PaneKind::Terminal, &a, None), focused: 0, zoomed: false },
            TabState {
                root: split(
                    Axis::Horizontal,
                    0.3,
                    leaf(PaneKind::Terminal, &b, Some(18.0)),
                    split(Axis::Vertical, 0.6, leaf(PaneKind::Explorer, &a, None), leaf(PaneKind::Terminal, &a, None)),
                ),
                focused: 2,
                zoomed: true,
            },
        ],
        active: 1,
        explorer: Some(TabState {
            root: split(Axis::Vertical, 0.5, leaf(PaneKind::Explorer, &b, None), leaf(PaneKind::Explorer, &a, Some(20.0))),
            focused: 1,
            zoomed: false,
        }),
        explorer_active: false,
    };
    let (_ctx, app) = restored(s.clone());
    assert_eq!(app.tabs.len(), 2);
    assert_eq!(app.snapshot_session(), Some(s.clone()));

    // Missing folders fall back to home; a session that can't open anything opens nothing.
    let gone = dir.join("gone");
    let lost = Session {
        tabs: vec![],
        active: 0,
        explorer: Some(TabState { root: leaf(PaneKind::Explorer, &gone, None), focused: 0, zoomed: false }),
        explorer_active: true,
    };
    let (_ctx, app) = restored(lost);
    let snap = app.snapshot_session().unwrap();
    assert!(snap.explorer_active);
    let PaneNode::Leaf(p) = &snap.explorer.unwrap().root else { panic!("not a leaf") };
    assert_eq!(p.cwd, crate::platform::home_dir());
    std::fs::remove_dir_all(dir).unwrap();
}

/// One saved terminal tab in `dir`.
fn one_shell(dir: &std::path::Path) -> Session {
    Session {
        tabs: vec![TabState { root: leaf(PaneKind::Terminal, dir, None), focused: 0, zoomed: false }],
        active: 0,
        explorer: None,
        explorer_active: false,
    }
}

// The checks below flip `save_session` on only after the last frame, so no test touches the
// real session file.

#[test]
fn a_window_that_never_opened_a_tab_keeps_the_session() {
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let cfg = crate::config::Config { command: Some("/nonexistent/nex-test-shell".into()), ..Default::default() };
    let args = crate::cli::Args { no_restore: true, ..Default::default() };
    let mut app = App::new(&cc, cfg, args, None, Some(one_shell(&std::env::temp_dir())), None);
    frames(&ctx, &mut app, 3);
    assert!(app.tabs.is_empty(), "the shell was expected to fail to spawn");
    app.session.save_session = true;
    app.session.closed_by_user = true;
    assert!(app.keeps_session());
    assert!(!app.forgets_session_when_empty());
}

#[test]
fn closing_the_last_tab_forgets_the_session_unless_restore_is_off() {
    let (_ctx, mut app) = restored(one_shell(&std::env::temp_dir()));
    assert!(app.session.had_tabs);
    app.session.save_session = true;
    app.session.closed_by_user = true;
    assert!(app.forgets_session_when_empty());
    // Shells dying on their own (a logout) don't count as the user closing them.
    app.session.closed_by_user = false;
    assert!(!app.forgets_session_when_empty());
    // restore-session = false: the file is neither written nor removed.
    app.session.closed_by_user = true;
    app.cfg.restore_session = false;
    assert!(!app.keeps_session());
    assert!(!app.forgets_session_when_empty());
}

/// A frame with `events` (and the usual screen rect).
fn frame_with(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))), events, ..Default::default() };
    let _ = ctx.run(input, |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()));
}

/// Pane `id`'s shell ends, `typed` whether the user had just typed into it (`exit`).
fn shell_exits(ctx: &egui::Context, app: &mut App, id: crate::layout::PaneId, typed: bool) {
    let long_ago = std::time::Instant::now() - std::time::Duration::from_secs(60);
    let t = app.panes.terminal_mut(id).unwrap();
    t.last_input = if typed { std::time::Instant::now() } else { long_ago };
    app.session.last_user_input = Some(if typed { std::time::Instant::now() } else { long_ago });
    t.exited = true;
    frames(ctx, app, 1);
}

/// What the window would do with the session file now, as if it held the lock:
/// `(keeps writing it, removes it on emptying)`. `save_session` is only on while asking, so
/// no frame ever touches the real file.
fn file_policy(app: &mut App) -> (bool, bool) {
    app.session.save_session = true;
    let policy = (app.keeps_session(), app.forgets_session_when_empty());
    app.session.save_session = false;
    policy
}

fn terminals(app: &App) -> Vec<crate::layout::PaneId> {
    app.tabs.iter().flat_map(|t| t.root.panes()).filter(|id| app.panes.is_terminal(*id)).collect()
}

#[test]
fn a_lone_shell_killed_keeps_the_session() {
    let (ctx, mut app) = restored(one_shell(&std::env::temp_dir()));
    let id = terminals(&app)[0];
    shell_exits(&ctx, &mut app, id, false);
    assert!(app.tabs.is_empty());
    assert_eq!(file_policy(&mut app), (false, false));
}

#[test]
fn staggered_shell_deaths_never_shrink_the_session() {
    let dir = std::env::temp_dir();
    let files = TabState { root: leaf(PaneKind::Explorer, &dir, None), focused: 0, zoomed: false };
    let mut s = one_shell(&dir);
    s.tabs.push(s.tabs[0].clone());
    s.explorer = Some(files);
    let (ctx, mut app) = restored(s);
    let ids = terminals(&app);
    assert_eq!(ids.len(), 2);
    shell_exits(&ctx, &mut app, ids[0], false);
    // Polls go by: the smaller layout is never written.
    app.timers.last_proc_poll = std::time::Instant::now() - std::time::Duration::from_secs(2);
    frames(&ctx, &mut app, 3);
    assert_eq!(file_policy(&mut app), (false, false));
    shell_exits(&ctx, &mut app, ids[1], false);
    assert!(app.tabs.is_empty() && app.home_tab.is_some());
    assert_eq!(file_policy(&mut app), (false, false));
    // The user is back (a click): the session follows the window again.
    let click = egui::Event::PointerButton {
        pos: Pos2::new(600.0, 400.0),
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: Default::default(),
    };
    frame_with(&ctx, &mut app, vec![click]);
    assert!(file_policy(&mut app).0);
}

#[test]
fn exit_typed_in_the_last_shell_starts_fresh() {
    let (ctx, mut app) = restored(one_shell(&std::env::temp_dir()));
    let id = terminals(&app)[0];
    shell_exits(&ctx, &mut app, id, true);
    assert!(app.tabs.is_empty());
    assert_eq!(file_policy(&mut app), (true, true));
}

#[test]
fn only_close_actions_count_as_the_user_closing() {
    let (ctx, mut app) = restored(one_shell(&std::env::temp_dir()));
    app.session.session_suspended = true;
    app.pending.push(crate::app::actions::Action::ReloadConfig);
    frames(&ctx, &mut app, 1);
    assert!(!app.session.closed_by_user && app.session.session_suspended);
    app.pending.push(crate::app::actions::Action::CloseTab(0));
    frames(&ctx, &mut app, 1);
    assert!(app.tabs.is_empty());
    assert_eq!(file_policy(&mut app), (true, true));
}

/// Run frames until `done` holds (the shell and its programs run in real time), or panic.
#[cfg(unix)]
fn frames_until(ctx: &egui::Context, app: &mut App, what: &str, mut done: impl FnMut(&mut App) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while !done(app) {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(std::time::Duration::from_millis(50));
        frames(ctx, app, 1);
    }
}

#[cfg(unix)]
#[test]
fn a_running_command_is_saved_and_run_again() {
    let dir = std::env::temp_dir().join(format!("nex-session-cmd-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let marker = dir.join("ran");
    let argv = |a: &[&str]| a.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    // Leaves a mark, then keeps running as the shell's foreground job.
    let cmd = argv(&["sh", "-c", &format!("echo it\\'s > '{}' && exec sleep 30", marker.display())]);
    let mut s = one_shell(&dir);
    let PaneNode::Leaf(p) = &mut s.tabs[0].root else { unreachable!() };
    p.command = Some(cmd);

    let (ctx, mut app) = restored(s);
    let id = terminals(&app)[0];
    frames_until(&ctx, &mut app, "the restored command to run", |_| marker.exists());
    assert_eq!(std::fs::read_to_string(&marker).unwrap().trim(), "it's");

    // What the shell is running now is what the next session saves.
    frames_until(&ctx, &mut app, "sleep in the foreground", |app| {
        let t = app.panes.terminal_mut(id).unwrap();
        t.refresh_process_info();
        t.running_command().is_some_and(|c| c == argv(&["sleep", "30"]))
    });
    let snap = app.snapshot_session().unwrap();
    assert_eq!(snap.tabs[0].root.leaves()[0].command, Some(argv(&["sleep", "30"])));

    // Interrupted, the pane is back at its prompt and saves no command.
    app.panes.terminal(id).unwrap().write(&b"\x03"[..]);
    frames_until(&ctx, &mut app, "the prompt", |app| {
        let t = app.panes.terminal_mut(id).unwrap();
        t.refresh_process_info();
        t.running_command().is_none()
    });
    assert_eq!(app.snapshot_session().unwrap().tabs[0].root.leaves()[0].command, None);
    std::fs::remove_dir_all(dir).unwrap();
}
