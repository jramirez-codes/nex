//! `explorer-view` picks the layout of every new file explorer, in mobile mode too.

use egui::{Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::layout::Axis;
use crate::config::Config;
use crate::file_explorer::View;

fn run(app: &mut App, ctx: &egui::Context) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 800.0))), ..Default::default() };
    let _ = ctx.run(input, |ctx| {
        let mut frame = eframe::Frame::_new_kittest();
        eframe::App::update(app, ctx, &mut frame);
    });
}

/// Open the Files tab and split another explorer off it; the layouts of both.
fn explorer_views(mobile: bool, view: View) -> Vec<View> {
    let cfg = Config { mobile, explorer_view: view, ..Default::default() };
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let mut app = App::new(&cc, cfg, crate::cli::Args { no_restore: true, ..Default::default() }, None, None, None);
    run(&mut app, &ctx);
    app.toggle_explorer();
    let first = *app.explorers.keys().next().unwrap();
    app.split_explorer(&ctx, first, Axis::Vertical);
    run(&mut app, &ctx);
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
