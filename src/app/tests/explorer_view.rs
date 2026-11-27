//! `explorer-view` picks the layout of every new file explorer, with the sidebar tab bar too;
//! on a phone (the sidebar tab bar) new explorers are touch-sized from the start.

use crate::config::{Config, TabBar};
use crate::file_explorer::View;
use crate::layout::{Axis, PaneKind};

use super::{new_app, run};

/// Open the Files tab and split another explorer off it; the layouts of both.
fn explorer_views(tab_bar: TabBar, view: View) -> Vec<View> {
    let cfg = Config { tab_bar, explorer_view: view, ..Default::default() };
    let ctx = egui::Context::default();
    let mut app = new_app(&ctx, cfg);
    run(&mut app, &ctx, 400.0);
    app.toggle_explorer(&ctx);
    let first = app.panes.ids(PaneKind::Explorer).next().unwrap();
    app.split_explorer(&ctx, first, Axis::Vertical);
    run(&mut app, &ctx, 400.0);
    assert_eq!(app.panes.ids(PaneKind::Explorer).count(), 2);
    app.panes.explorers().map(|b| b.view).collect()
}

#[test]
fn the_sidebar_keeps_the_configured_explorer_view() {
    for tab_bar in [TabBar::Top, TabBar::Sidebar] {
        for view in [View::List, View::Grid] {
            assert_eq!(explorer_views(tab_bar, view), vec![view, view], "tab-bar = {tab_bar:?}, explorer-view = {view:?}");
        }
    }
}

/// Explorers made on a phone are touch-sized from their first frame, not after the first tap.
#[test]
fn phone_explorers_start_touch_sized() {
    for (tab_bar, touch) in [(TabBar::Top, false), (TabBar::Sidebar, true)] {
        let ctx = egui::Context::default();
        let mut app = new_app(&ctx, Config { tab_bar, ..Default::default() });
        run(&mut app, &ctx, 400.0);
        app.toggle_explorer(&ctx);
        let first = app.panes.ids(PaneKind::Explorer).next().unwrap();
        app.split_explorer(&ctx, first, Axis::Vertical);
        assert_eq!(app.panes.explorers().map(|b| b.touch).collect::<Vec<_>>(), vec![touch, touch], "tab-bar = {tab_bar:?}");
    }
}
