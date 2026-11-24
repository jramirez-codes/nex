//! Tests that drive the whole `App` through egui frames. The helpers here are what most of
//! them share: an `App` with no window and no saved session, and one frame of it.

use egui::{Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::config::Config;

mod agents;
mod drop;
mod explorer_refresh;
mod explorer_view;
mod git_diff;
mod keyboard_button;
mod session;
mod sticky;
mod tab_scroll;
mod touch_scroll;

/// An `App` with config `cfg` that doesn't restore (or save over) the user's session.
fn new_app(ctx: &egui::Context, cfg: Config) -> App {
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    App::new(&cc, cfg, crate::cli::Args { no_restore: true, ..Default::default() }, None, None, None)
}

/// One frame of `app` in a window `width` wide and 800 tall.
fn run(app: &mut App, ctx: &egui::Context, width: f32) {
    let input = RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(width, 800.0))), ..Default::default() };
    let _ = ctx.run(input, |ctx| {
        let mut frame = eframe::Frame::_new_kittest();
        eframe::App::update(app, ctx, &mut frame);
    });
}
