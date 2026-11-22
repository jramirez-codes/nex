//! `explorer-view` picks the layout of every new file explorer, in mobile mode too.

use crate::layout::Axis;
use crate::config::Config;
use crate::file_explorer::View;

use super::{new_app, run};

/// Open the Files tab and split another explorer off it; the layouts of both.
fn explorer_views(mobile: bool, view: View) -> Vec<View> {
    let cfg = Config { mobile, explorer_view: view, ..Default::default() };
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, cfg);
    run(&mut app, &ctx, 400.0);
    app.toggle_explorer();
    let first = *app.explorers.keys().next().unwrap();
    app.split_explorer(&ctx, first, Axis::Vertical);
    run(&mut app, &ctx, 400.0);
    assert_eq!(app.explorers.len(), 2);
    app.explorers.values().map(|b| b.view).collect()
}

#[test]
fn mobile_keeps_the_configured_explorer_view() {
    for mobile in [false, true] {
        for view in [View::List, View::Grid] {
            assert_eq!(explorer_views(mobile, view), vec![view, view], "mobile = {mobile}, explorer-view = {view:?}");
        }
    }
}
