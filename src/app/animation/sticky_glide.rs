//! A pinned (sticky) pane on a tab switch: it doesn't leave with the old tab's panes, its
//! card glides (and resizes) from where the old tab had it to where the new one does. Each
//! pinned pane has its own `StickyGlide`, so several glide at once, each to its own place;
//! one not drawn in a frame is dropped and starts afresh (in place) when drawn again.

use egui::Rect;

use crate::ui::glide::{self, ease_out};

/// Length of the glide, in seconds (about as long as the tab transitions).
const GLIDE: f64 = 0.22;

#[derive(Default)]
pub(in crate::app) struct StickyGlide {
    /// The tab it was drawn in last, and where.
    last: Option<(u64, Rect)>,
    /// The running glide: its start rect and time.
    from: Option<(Rect, f64)>,
}

impl StickyGlide {
    /// Where to draw the sticky card this frame in tab `tab`, whose own motion puts it at
    /// `card`, and whether it is still gliding (its content then scales with the card).
    pub(in crate::app) fn frame(&mut self, ctx: &egui::Context, tab: u64, card: Rect) -> (Rect, bool) {
        let now = ctx.input(|i| i.time);
        if let Some((was, at)) = self.last
            && was != tab
            && glide::animations(ctx)
        {
            self.from = Some((at, now));
        }
        let drawn = match self.from {
            Some((from, start)) if glide::animations(ctx) && now - start < GLIDE => {
                ctx.request_repaint();
                from.lerp_towards(&card, ease_out(((now - start) / GLIDE) as f32))
            }
            _ => {
                self.from = None;
                card
            }
        };
        self.last = Some((tab, drawn));
        (drawn, self.from.is_some())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    fn rect(x0: f32, x1: f32) -> Rect {
        Rect::from_min_max(Pos2::new(x0, 0.0), Pos2::new(x1, 50.0))
    }

    fn at(ctx: &egui::Context, t: f64) {
        let _ = ctx.run(egui::RawInput { time: Some(t), ..Default::default() }, |_| {});
    }

    #[test]
    fn glides_between_tabs_and_stays_put_within_one() {
        let ctx = egui::Context::default();
        let mut g = StickyGlide::default();
        at(&ctx, 1.0);
        assert_eq!(g.frame(&ctx, 1, rect(60.0, 100.0)), (rect(60.0, 100.0), false));
        // Same tab, new layout: its own motion moves it, no glide.
        assert_eq!(g.frame(&ctx, 1, rect(50.0, 100.0)), (rect(50.0, 100.0), false));
        // Another tab: it starts where it was.
        assert_eq!(g.frame(&ctx, 2, rect(0.0, 30.0)), (rect(50.0, 100.0), true));
        at(&ctx, 1.0 + GLIDE + 0.01);
        assert_eq!(g.frame(&ctx, 2, rect(0.0, 30.0)), (rect(0.0, 30.0), false));
    }

    #[test]
    fn snaps_with_animations_off_or_when_new() {
        let ctx = egui::Context::default();
        let mut g = StickyGlide::default();
        assert_eq!(g.frame(&ctx, 2, rect(0.0, 30.0)), (rect(0.0, 30.0), false));
        glide::set_animations(&ctx, false);
        assert_eq!(g.frame(&ctx, 1, rect(60.0, 100.0)), (rect(60.0, 100.0), false));
    }
}
