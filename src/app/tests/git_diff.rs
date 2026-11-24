//! Ctrl+G opens a Changes pane beside the focused pane and closes it again; it is drawn
//! (wide and narrow) without trouble, and comes back with the session.

use std::process::Command;
use std::time::{Duration, Instant};

use crate::app::actions::Action;
use crate::config::Config;
use crate::session::PaneKind;

use super::{new_app, run};

/// A repository with a modified, a staged and an untracked file, or `None` without git.
fn repo() -> Option<std::path::PathBuf> {
    let dir = std::env::temp_dir().join(format!("nex-app-diff-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let git = |args: &[&str]| Command::new("git").arg("-C").arg(&dir).args(["-c", "user.name=nex", "-c", "user.email=nex@example.com", "-c", "commit.gpgsign=false"]).args(args).output();
    git(&["init", "-q"]).ok().filter(|o| o.status.success())?;
    std::fs::write(dir.join("main.rs"), "fn main() {\n    let x = 1;\n}\n").unwrap();
    git(&["add", "."]).unwrap();
    git(&["commit", "-q", "-m", "first"]).unwrap();
    std::fs::write(dir.join("main.rs"), "fn main() {\n    let x = 2;\n    println!(\"{x}\");\n}\n").unwrap();
    std::fs::write(dir.join("lib.rs"), "pub fn f() {}\n").unwrap();
    git(&["add", "lib.rs"]).unwrap();
    std::fs::write(dir.join("notes.md"), "# notes\n").unwrap();
    Some(dir)
}

#[test]
fn ctrl_g_toggles_a_changes_pane_that_draws_and_restores() {
    let Some(dir) = repo() else { return };
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let shell = app.focused_pane().unwrap();
    app.split_diff(&ctx, shell, crate::layout::Axis::Horizontal);
    let id = *app.diffs.keys().next().unwrap();
    assert_eq!(app.focused_pane(), Some(id));
    // Point it at the repository (the shell's own cwd is wherever the tests run).
    app.diffs.get_mut(&id).unwrap().follow(None, dir.clone());

    // Draw until git has answered, wide (list beside diff) and narrow (one at a time).
    let deadline = Instant::now() + Duration::from_secs(10);
    while app.diffs[&id].loading() && Instant::now() < deadline {
        run(&mut app, &ctx, 1400.0);
        std::thread::sleep(Duration::from_millis(10));
    }
    for width in [1400.0, 700.0, 500.0] {
        for _ in 0..3 {
            run(&mut app, &ctx, width);
        }
    }
    assert!(app.pane_label(id).0.ends_with("Changes"), "{}", app.pane_label(id).0);
    let d = app.diffs.get_mut(&id).unwrap();
    assert_eq!(d.copy_text().as_deref(), Some("lib.rs"), "the staged file comes first");
    assert!(d.key(egui::Key::ArrowDown, egui::Modifiers::NONE));
    for _ in 0..3 {
        run(&mut app, &ctx, 1400.0);
    }

    let s = app.snapshot_session().unwrap();
    let kinds: Vec<PaneKind> = s.tabs[0].root.leaves().iter().map(|p| p.kind).collect();
    assert_eq!(kinds, vec![PaneKind::Terminal, PaneKind::Diff]);

    // Ctrl+G again closes it, and focus goes back to the shell.
    app.pending.push(Action::ToggleGitDiff);
    run(&mut app, &ctx, 1400.0);
    assert!(app.diffs.is_empty());
    assert_eq!(app.focused_pane(), Some(shell));
    // And once more opens it.
    app.pending.push(Action::ToggleGitDiff);
    run(&mut app, &ctx, 1400.0);
    assert_eq!(app.diffs.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Zooming a Changes pane goes through `set_pane_font`: clamped to the font limits, with no
/// entry of its own at the config size, and cell metrics ready for the size it lands on.
#[test]
fn a_changes_pane_zooms_within_the_font_limits() {
    use crate::config::{FONT_MAX, FONT_MIN};
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, Config::default());
    run(&mut app, &ctx, 1400.0);
    let shell = app.focused_pane().unwrap();
    app.split_diff(&ctx, shell, crate::layout::Axis::Horizontal);
    let id = app.focused_pane().unwrap();
    assert!(app.diffs.contains_key(&id));
    let base = app.cfg.font_size;

    app.apply(&ctx, Action::FontBigger);
    assert_eq!(app.pane_font(id), base + 1.0);
    assert!(app.metrics.contains_key(&(base + 1.0).to_bits()));
    assert_eq!(app.pane_font(shell), base, "only the Changes pane zooms");
    app.apply(&ctx, Action::FontSmaller);
    assert_eq!(app.pane_font(id), base);
    assert!(!app.pane_fonts.contains_key(&id), "back at the config size, it follows the config again");
    for _ in 0..200 {
        app.apply(&ctx, Action::FontBigger);
    }
    assert_eq!(app.pane_font(id), FONT_MAX);
    for _ in 0..200 {
        app.apply(&ctx, Action::FontSmaller);
    }
    assert_eq!(app.pane_font(id), FONT_MIN);
    app.apply(&ctx, Action::FontReset);
    assert_eq!(app.pane_font(id), base);
    assert!(!app.pane_fonts.contains_key(&id));
}
