//! Closing a tab: a tab in the background shrinks, whole (tint, icon and title), into its ×,
//! which lingers a moment and then goes too, while the tabs beside it slide in to fill the
//! gap. Closing the *active* tab folds the glass pill into a rounded square around its ×,
//! and the pill then grows back out onto the tab now showing.
//!
//! The tab is gone from `App::tabs` at once; a [`TabGhost`] keeps how it looked so it can
//! play out over the next few frames.

use egui::{Rect, Vec2};

use crate::ui::glide::{Lerp, ease_in_out, ease_out};

/// The active tab's pill folds into the square around the ×, in seconds.
const FOLD: f64 = 0.14;
/// The active tab's title and icon are gone by now.
const CONTENT_FADE: f64 = 0.06;
/// A background tab shrinks into its × over this long...
const SHRINK: f64 = 0.24;
/// ...its title and icon fading out between these, and its tint between these...
const SHRINK_CONTENT_FADE: (f64, f64) = (0.04, 0.18);
const SHRINK_TINT_FADE: (f64, f64) = (0.10, SHRINK);
/// ...and then the ×, which it shrank into, follows it out.
const SHRINK_GLYPH_FADE: (f64, f64) = (0.16, VANISH_END);
const VANISH_END: f64 = 0.28;
/// The pill grows back out of the square onto the new active tab, from `FOLD` to here.
const RETURN_END: f64 = 0.34;
/// The × inside the folded pill fades out as the pill leaves, from `FOLD` to here.
const GLYPH_FADE_END: f64 = 0.22;
/// The tab's empty slot closes up between these.
const GAP_START: f64 = 0.08;
const GAP_END: f64 = 0.26;

/// The square the tab folds into: the close button's hover circle, squared off.
const SQUARE: f32 = 18.0;
const TAB_RADIUS: f32 = 8.0;
const SQUARE_RADIUS: f32 = 6.0;
/// The × glyph's size at rest (the close button's icon rect).
pub(in crate::app) const GLYPH: f32 = 12.0;

/// Where a tab was drawn last frame, so a close can start from there.
#[derive(Clone, Copy)]
pub(in crate::app) struct TabDrawn {
    pub(in crate::app) rect: Rect,
    pub(in crate::app) close: Rect,
    pub(in crate::app) icon: Rect,
    pub(in crate::app) text_left: f32,
    pub(in crate::app) active: bool,
}

/// A closed tab still playing its close animation.
pub(in crate::app) struct TabGhost {
    pub(in crate::app) drawn: TabDrawn,
    /// The slot it left, in the strip as it is now (later closes shift it left).
    pub(in crate::app) index: usize,
    /// Set on the first frame it is drawn (closing a tab doesn't have the frame time).
    pub(in crate::app) start: Option<f64>,
    pub(in crate::app) title: String,
    pub(in crate::app) process: String,
    pub(in crate::app) mark: Option<String>,
}

/// How a ghost looks this frame.
pub(in crate::app) struct GhostFrame {
    /// Size of the whole tab (tint, icon, title), shrinking toward the × center: 1 to 0.
    pub(in crate::app) scale: f32,
    /// Opacity of the title and icon.
    pub(in crate::app) content: f32,
    /// Opacity of the tab's tint (only a background tab has one; the active tab's is the pill).
    pub(in crate::app) tint: f32,
    /// Opacity and scale of the ×.
    pub(in crate::app) glyph: f32,
    pub(in crate::app) glyph_scale: f32,
    /// How much of a slot the closed tab still takes up in the strip, 1 to 0.
    pub(in crate::app) gap: f32,
}

/// Progress of `t` through `[a, b]`, 0 to 1.
fn phase(t: f64, a: f64, b: f64) -> f32 {
    ((t - a) / (b - a)).clamp(0.0, 1.0) as f32
}

fn scaled(r: Rect, s: f32) -> Rect {
    Rect::from_center_size(r.center(), r.size() * s)
}

impl TabGhost {
    fn elapsed(&self, now: f64) -> f64 {
        self.start.map_or(0.0, |s| now - s)
    }

    pub(in crate::app) fn done(&self, now: f64) -> bool {
        self.elapsed(now) >= if self.drawn.active { RETURN_END } else { VANISH_END }
    }

    /// The × center, which a background tab shrinks into.
    pub(in crate::app) fn pivot(&self) -> egui::Pos2 {
        self.drawn.close.center()
    }

    fn square(&self) -> Rect {
        Rect::from_center_size(self.drawn.close.center(), Vec2::splat(SQUARE))
    }

    /// The active tab's pill folding into the square, and its corner radius.
    fn folded(&self, t: f64) -> (Rect, f32) {
        let e = ease_out(phase(t, 0.0, FOLD));
        (self.drawn.rect.lerp_towards(&self.square(), e), f32::lerp(TAB_RADIUS, SQUARE_RADIUS, e))
    }

    pub(in crate::app) fn frame(&self, now: f64) -> GhostFrame {
        let t = self.elapsed(now);
        let gap = 1.0 - ease_out(phase(t, GAP_START, GAP_END));
        if self.drawn.active {
            return GhostFrame {
                scale: 1.0,
                content: 1.0 - phase(t, 0.0, CONTENT_FADE),
                tint: 0.0,
                glyph: 1.0 - phase(t, FOLD, GLYPH_FADE_END),
                glyph_scale: 1.0,
                gap,
            };
        }
        let fade = |(a, b): (f64, f64)| 1.0 - phase(t, a, b);
        let glyph = fade(SHRINK_GLYPH_FADE);
        GhostFrame {
            // Ease in and out: it gathers itself, then is drawn in quickly and settles.
            scale: 1.0 - ease_in_out(phase(t, 0.0, SHRINK)),
            content: fade(SHRINK_CONTENT_FADE),
            tint: fade(SHRINK_TINT_FADE),
            glyph,
            // The × gives a little as the tab lands in it, then goes.
            glyph_scale: 0.6 + 0.4 * glyph,
            gap,
        }
    }

    /// The glass pill while the active tab closes: it folds into the square, then grows back
    /// out onto `target` (where the pill belongs this frame). With nowhere to go it shrinks
    /// away instead. `None` once there is nothing left to draw.
    pub(in crate::app) fn pill(&self, now: f64, target: Option<Rect>) -> Option<(Rect, f32)> {
        let t = self.elapsed(now);
        if t < FOLD {
            let (r, radius) = self.folded(t);
            return Some((r, radius));
        }
        let e = ease_out(phase(t, FOLD, RETURN_END));
        match target {
            Some(to) => Some((self.square().lerp_towards(&to, e), f32::lerp(SQUARE_RADIUS, TAB_RADIUS, e))),
            None if e < 1.0 => Some((scaled(self.square(), 1.0 - e), SQUARE_RADIUS * (1.0 - e))),
            None => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    fn ghost(active: bool) -> TabGhost {
        let rect = Rect::from_min_size(Pos2::new(100.0, 10.0), Vec2::new(200.0, 30.0));
        let close = Rect::from_center_size(Pos2::new(rect.right() - 15.0, rect.center().y), Vec2::splat(18.0));
        let icon = Rect::from_center_size(Pos2::new(rect.left() + 19.0, rect.center().y), Vec2::splat(20.0));
        TabGhost {
            drawn: TabDrawn { rect, close, icon, text_left: icon.right() + 9.0, active },
            index: 1,
            start: Some(0.0),
            title: "zsh".into(),
            process: "zsh".into(),
            mark: None,
        }
    }

    #[test]
    fn background_tab_shrinks_into_its_close_button() {
        let g = ghost(false);
        let f = g.frame(0.0);
        assert_eq!((f.scale, f.content, f.tint, f.glyph, f.gap), (1.0, 1.0, 1.0, 1.0, 1.0));
        // Its title is gone before it is small, and the × outlasts the tab.
        let f = g.frame(SHRINK_CONTENT_FADE.1);
        assert!(f.content == 0.0 && f.scale > 0.0 && f.glyph > 0.5);
        let f = g.frame(SHRINK);
        assert!(f.scale == 0.0 && f.tint == 0.0 && f.glyph > 0.0);
        let f = g.frame(VANISH_END);
        assert_eq!((f.glyph, f.gap), (0.0, 0.0));
        assert!(g.done(VANISH_END) && !g.done(VANISH_END - 0.01));
    }

    #[test]
    fn active_pill_folds_then_lands_on_the_new_tab() {
        let g = ghost(true);
        let target = Rect::from_min_size(Pos2::new(0.0, 10.0), Vec2::new(100.0, 30.0));
        assert_eq!(g.pill(0.0, Some(target)).unwrap().0, g.drawn.rect);
        assert_eq!(g.pill(FOLD, Some(target)).unwrap().0, g.square());
        assert_eq!(g.pill(RETURN_END, Some(target)).unwrap(), (target, TAB_RADIUS));
        assert!(g.done(RETURN_END) && !g.done(VANISH_END));
        // With no tab to land on it shrinks away.
        assert!(g.pill(RETURN_END, None).is_none());
        // The × stays put while the pill folds around it.
        assert_eq!((g.frame(FOLD / 2.0).glyph, g.frame(GLYPH_FADE_END).glyph), (1.0, 0.0));
    }

    #[test]
    fn waits_for_its_first_frame() {
        let mut g = ghost(false);
        g.start = None;
        assert_eq!(g.frame(1000.0).scale, 1.0);
        assert!(!g.done(1000.0));
    }
}
