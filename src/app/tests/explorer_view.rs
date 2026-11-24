//! `explorer-view` picks the layout of every new file explorer, with the sidebar tab bar too.

use crate::layout::Axis;
use crate::config::{Config, TabBar};
use crate::file_explorer::View;

use super::{new_app, run};

/// Open the Files tab and split another explorer off it; the layouts of both.
fn explorer_views(tab_bar: TabBar, view: View) -> Vec<View> {
    let cfg = Config { tab_bar, explorer_view: view, ..Default::default() };
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, cfg);
    run(&mut app, &ctx, 400.0);
    app.toggle_explorer(&ctx);
    let first = *app.explorers.keys().next().unwrap();
    app.split_explorer(&ctx, first, Axis::Vertical);
    run(&mut app, &ctx, 400.0);
    assert_eq!(app.explorers.len(), 2);
    app.explorers.values().map(|b| b.view).collect()
}

#[test]
fn the_sidebar_keeps_the_configured_explorer_view() {
    for tab_bar in [TabBar::Top, TabBar::Sidebar] {
        for view in [View::List, View::Grid] {
            assert_eq!(explorer_views(tab_bar, view), vec![view, view], "tab-bar = {tab_bar:?}, explorer-view = {view:?}");
        }
    }
}
