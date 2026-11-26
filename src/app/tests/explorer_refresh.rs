//! Stale file listings are re-read only for the explorers on screen, and off the UI thread.

use std::time::{Duration, Instant};

use crate::config::Config;
use crate::layout::PaneKind;

use super::{new_app, run};

/// The names the Files tab's explorer lists.
fn names(app: &crate::app::App) -> Vec<String> {
    app.panes.explorers().next().unwrap().entries.iter().map(|e| e.name.clone()).collect()
}

/// A background poll, as if the interval had passed.
fn poll(app: &mut crate::app::App) {
    app.last_proc_poll = Instant::now() - Duration::from_secs(60);
    app.poll_background();
}

#[test]
fn only_explorers_on_screen_are_reread() {
    let dir = std::env::temp_dir().join(format!("nex-refresh-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("one.txt"), "1").unwrap();

    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 400.0);
    app.toggle_explorer(&ctx);
    let id = app.panes.ids(PaneKind::Explorer).next().unwrap();
    app.panes.explorer_mut(id).unwrap().navigate(dir.clone());
    assert_eq!(names(&app), ["one.txt"]);

    // Back on the terminal tab: files changing don't make the hidden explorer read its folder.
    app.toggle_explorer(&ctx);
    assert!(!app.home_active);
    std::fs::write(dir.join("two.txt"), "2").unwrap();
    crate::file_explorer::files_changed();
    poll(&mut app);
    std::thread::sleep(Duration::from_millis(100));
    poll(&mut app);
    assert_eq!(names(&app), ["one.txt"]);

    // Shown again, it is re-read: in the background, so not by the time the poll returns.
    app.toggle_explorer(&ctx);
    assert!(app.home_active);
    poll(&mut app);
    assert_eq!(names(&app), ["one.txt"]);
    let deadline = Instant::now() + Duration::from_secs(10);
    while names(&app) != ["one.txt", "two.txt"] {
        assert!(Instant::now() < deadline, "the explorer on screen wasn't re-read");
        std::thread::sleep(Duration::from_millis(5));
        poll(&mut app);
    }
    std::fs::remove_dir_all(dir).unwrap();
}
