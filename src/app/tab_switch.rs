//! Switching tabs: noticing the switch, keeping what the old tab looked like, and handing
//! both to the transition the `tab-animation` config key picks (see `transition/`).
//!
//! Every frame keeps a copy of what the panes painted (meshes and galleys are shared, so
//! this is cheap). When the shown tab changes, that copy is the old sheet; the transition
//! paints it before the new tab's panes and restyles the new tab's shapes after them.

use egui::epaint::ClippedShape;
use egui::layers::ShapeIdx;
use egui::{Pos2, Rect, Shape, Ui, Vec2};

use crate::config::TabAnimation;
use crate::ui::glide;

use super::transition::{self, Switch, Transition, tessellator};

/// The tab being drawn this frame, and how a switch to it should look.
pub(super) struct Shown {
    pub(super) uid: u64,
    /// Its place in the tab strip: -1 for the Files tab, else its index.
    pub(super) pos: isize,
    /// The pane area.
    pub(super) area: Rect,
    /// Where its tab is, brought onto the pane area's edge.
    pub(super) origin: Pos2,
    /// The tab strip runs top to bottom (the mobile sidebar).
    pub(super) vertical: bool,
    pub(super) style: TabAnimation,
}

#[derive(Default)]
pub(super) struct TabSwitch {
    /// The tab shown last frame: its uid and place in the strip.
    shown: Option<(u64, isize)>,
    /// What its panes painted, before any restyling.
    last: Vec<ClippedShape>,
    running: Option<Box<dyn Transition>>,
}

impl TabSwitch {
    /// Call before painting the panes of `tab`. If it is not the tab shown last frame, start
    /// a transition. Paints the old sheet while one runs, and returns where the new sheet's
    /// shapes start.
    pub(super) fn begin(&mut self, ui: &Ui, tab: &Shown) -> ShapeIdx {
        let ctx = ui.ctx();
        let now = ctx.input(|i| i.time);
        let previous = self.shown.replace((tab.uid, tab.pos));
        if !glide::animations(ctx) {
            self.running = None;
        } else if let Some((_, pos)) = previous.filter(|&(uid, _)| uid != tab.uid) {
            // Content moves away from the new tab's side: a tab further along comes in
            // from the right (or below), pushing the old one left (or up).
            let axis = if tab.vertical { Vec2::Y } else { Vec2::X };
            let travel = if tab.pos >= pos { -axis } else { axis };
            let old = std::mem::take(&mut self.last);
            self.running = transition::start(tab.style, Switch { old, area: tab.area, origin: tab.origin, travel, start: now });
        }
        if self.running.as_ref().is_some_and(|r| r.done(now)) {
            self.running = None;
        }
        let painter = ui.painter();
        if let Some(r) = &mut self.running {
            r.paint_old(ctx, painter, now);
            ctx.request_repaint();
        }
        painter.add(Shape::Noop)
    }

    /// Call after painting the panes: remember them (the next switch starts from them) and,
    /// while a transition runs, restyle them.
    pub(super) fn end(&mut self, ui: &Ui, start: ShapeIdx) {
        let ctx = ui.ctx();
        let now = ctx.input(|i| i.time);
        // Made outside `graphics_mut`: it reads the fonts, which that lock would deadlock on.
        let mut tess = self.running.as_ref().filter(|r| r.needs_tessellator()).map(|_| tessellator(ctx));
        ctx.graphics_mut(|g| {
            let list = g.entry(ui.layer_id());
            let end = list.next_idx().0;
            self.last.clear();
            self.last.extend(list.all_entries().skip(start.0).cloned());
            if let Some(r) = &self.running {
                r.style_new(list, start.0..end, tess.as_mut(), now);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Run one frame that paints a rect for tab `uid` at strip position `pos`; returns the
    /// panes' shapes after it.
    fn frame(ctx: &egui::Context, switch: &mut TabSwitch, uid: u64, pos: isize, style: TabAnimation) -> Vec<ClippedShape> {
        let mut out = Vec::new();
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            egui::CentralPanel::default().show(ctx, |ui| {
                let area = Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 300.0));
                let tab = Shown { uid, pos, area, origin: Pos2::new(200.0, 0.0), vertical: false, style };
                let start = switch.begin(ui, &tab);
                ui.painter().rect_filled(area, 0.0, egui::Color32::WHITE);
                switch.end(ui, start);
                ctx.graphics(|g| {
                    // Past the marker `begin` returns.
                    out = g.get(ui.layer_id()).map(|l| l.all_entries().skip(start.0 + 1).cloned().collect()).unwrap_or_default();
                });
            });
        });
        out
    }

    #[test]
    fn switching_tabs_starts_a_transition_and_reordering_does_not() {
        let ctx = egui::Context::default();
        let mut s = TabSwitch::default();
        frame(&ctx, &mut s, 1, 0, TabAnimation::Ink);
        assert!(s.running.is_none(), "the first tab shown just appears");
        frame(&ctx, &mut s, 1, 1, TabAnimation::Ink);
        assert!(s.running.is_none(), "same tab (reordered or not) never animates");
        let shapes = frame(&ctx, &mut s, 2, 0, TabAnimation::Ink);
        assert!(s.running.is_some());
        // Nothing of the new sheet shows yet.
        assert!(matches!(shapes[0].shape, Shape::Noop));
        // Its unrestyled shapes are kept for the next switch.
        assert!(matches!(s.last[1].shape, Shape::Rect(_)));
    }

    #[test]
    fn glide_moves_the_new_sheet_in_from_the_new_tabs_side() {
        let ctx = egui::Context::default();
        let mut s = TabSwitch::default();
        frame(&ctx, &mut s, 1, 0, TabAnimation::Glide);
        frame(&ctx, &mut s, 2, 1, TabAnimation::Glide);
        assert!(s.running.is_some());
        // The next frame shows it to the right of where it settles, fading in.
        let shapes = frame(&ctx, &mut s, 2, 1, TabAnimation::Glide);
        assert!(shapes.iter().all(|cs| match &cs.shape {
            Shape::Rect(r) => r.rect.left() > 0.0,
            Shape::Noop => true,
            _ => false,
        }));
    }

    #[test]
    fn none_and_animations_off_switch_instantly() {
        let ctx = egui::Context::default();
        let mut s = TabSwitch::default();
        frame(&ctx, &mut s, 1, 0, TabAnimation::None);
        let shapes = frame(&ctx, &mut s, 2, 1, TabAnimation::None);
        assert!(s.running.is_none());
        assert!(matches!(shapes[0].shape, Shape::Rect(_)));

        glide::set_animations(&ctx, false);
        frame(&ctx, &mut s, 3, 2, TabAnimation::Ink);
        let shapes = frame(&ctx, &mut s, 1, 0, TabAnimation::Ink);
        assert!(s.running.is_none());
        assert!(matches!(shapes[0].shape, Shape::Rect(_)));
    }
}
