//! Tab-switch transitions: how the tab being left (the old sheet) gives way to the one being
//! shown (the new sheet). `TabSwitch` (see `tab_switch.rs`) notices the switch, keeps the old
//! sheet and calls into one of these; the `tab-animation` config key picks which.
//!
//! Adding a style: a `TabAnimation` variant (and its config value), a module here
//! implementing `Transition`, and an arm in `start`.

mod glide;
mod ink;

use std::ops::Range;

use egui::epaint::{ClippedShape, Tessellator};
use egui::layers::PaintList;
use egui::{Painter, Pos2, Rect, Vec2};

use crate::config::TabAnimation;

/// A tab switch that just happened.
pub(super) struct Switch {
    /// What the old tab's panes painted on its last frame.
    pub(super) old: Vec<ClippedShape>,
    /// The pane area.
    pub(super) area: Rect,
    /// Where the new tab is in the tab strip, brought onto the pane area's edge.
    pub(super) origin: Pos2,
    /// The way content moves: the way the pill moves along the strip (unit length).
    pub(super) travel: Vec2,
    pub(super) start: f64,
}

/// One running tab-switch transition.
pub(super) trait Transition {
    fn done(&self, now: f64) -> bool;

    /// Paint the old sheet. Called before the new tab's panes are drawn, so it lies under them.
    fn paint_old(&mut self, ctx: &egui::Context, painter: &Painter, now: f64);

    /// Restyle the new tab's shapes, `range` of `list`, in place. `tess` is there if
    /// `needs_tessellator` says so.
    fn style_new(&self, list: &mut PaintList, range: Range<usize>, tess: Option<&mut Tessellator>, now: f64);

    /// Whether `style_new` needs a tessellator (it can't make one: the paint list is locked).
    fn needs_tessellator(&self) -> bool {
        false
    }
}

/// Start the transition `style` asks for, if any.
pub(super) fn start(style: TabAnimation, switch: Switch) -> Option<Box<dyn Transition>> {
    match style {
        TabAnimation::Ink => Some(Box::new(ink::InkSwitch::new(switch))),
        TabAnimation::Glide => Some(Box::new(glide::GlideSwitch::new(switch))),
        TabAnimation::None => None,
    }
}

/// A tessellator like the one egui paints with at the end of the frame.
pub(super) fn tessellator(ctx: &egui::Context) -> Tessellator {
    let options = ctx.tessellation_options(|o| *o);
    let size = ctx.fonts(|f| f.font_image_size());
    // Prepared discs only speed up tiny circles; panes have none worth it.
    Tessellator::new(ctx.pixels_per_point(), options, size, Vec::new())
}
