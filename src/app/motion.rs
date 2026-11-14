//! Pane layout transitions: a split slides the new pane out of the seam, a closed pane
//! collapses toward its outer edge and fades while its neighbor takes the space, and focus
//! dimming fades instead of switching.
//!
//! Only the cards move. Terminals are laid out (and their PTYs sized) at the target rect
//! once; while a card is in motion its cached shapes are just translated and clipped.

use std::collections::HashMap;

use egui::{Pos2, Rect, Shape};

use crate::app::layout::{Axis, PaneId};
use crate::ui::glide::{self, Glide, ease_out};

/// Length of a split / close transition, in seconds.
const PANE_ANIM: f64 = 0.18;
/// Length of the focus dim / ring fade, in seconds.
const FOCUS_FADE: f64 = 0.12;

/// Where a pane is drawn this frame.
pub(super) struct Placed {
    pub(super) id: PaneId,
    /// The (possibly animated) card rect to paint.
    pub(super) card: Rect,
    /// The card rect the layout gives it: terminals are sized and hit-tested here.
    pub(super) target: Rect,
}

/// A closed pane still fading out: what it looked like when it closed.
pub(super) struct Ghost {
    card: Rect,
    /// Where it collapses to, resolved on the first frame the new layout is known.
    to: Option<Rect>,
    /// The card position its shapes were built for.
    origin: Pos2,
    start: f64,
    pub(super) title: String,
    pub(super) process: String,
    pub(super) shapes: Vec<Shape>,
}

impl Ghost {
    /// The ghost's card this frame, how far its shapes moved, and its opacity.
    pub(super) fn at(&self, now: f64) -> (Rect, egui::Vec2, f32) {
        let e = ease_out(((now - self.start) / PANE_ANIM) as f32);
        let card = self.to.map_or(self.card, |to| self.card.lerp_towards(&to, e));
        (card, card.min - self.origin, 1.0 - e)
    }
}

#[derive(Default)]
pub(super) struct PaneMotion {
    /// `(drawn, target)` card rects from the last frame.
    last: HashMap<PaneId, (Rect, Rect)>,
    /// Start rects of the running transition.
    from: HashMap<PaneId, Rect>,
    start: f64,
    ghosts: Vec<Ghost>,
    focus: HashMap<PaneId, Glide>,
}

impl PaneMotion {
    /// Begin a transition from wherever every pane is drawn right now.
    fn restart(&mut self, now: f64) {
        self.from = self.last.iter().map(|(id, (drawn, _))| (*id, *drawn)).collect();
        self.start = now;
    }

    /// `new` was split off `target`: it grows out of `target`'s far edge as `target` shrinks.
    pub(super) fn kick_split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis, new: PaneId) {
        if !glide::animations(ctx) {
            return;
        }
        let Some(&(r, _)) = self.last.get(&target) else { return };
        self.restart(ctx.input(|i| i.time));
        self.from.insert(new, seam_seed(r, axis));
        ctx.request_repaint();
    }

    /// Pane `id` was closed: leave a ghost that collapses and fades while the rest move.
    pub(super) fn kick_close(&mut self, ctx: &egui::Context, id: PaneId, title: String, process: String, shapes: Vec<Shape>) {
        if !glide::animations(ctx) {
            return;
        }
        let Some((card, target)) = self.last.remove(&id) else { return };
        let now = ctx.input(|i| i.time);
        self.restart(now);
        self.ghosts.push(Ghost { card, to: None, origin: target.min, start: now, title, process, shapes });
        ctx.request_repaint();
    }

    /// Where each pane is drawn this frame, given the rects the layout wants.
    pub(super) fn frame(&mut self, ctx: &egui::Context, targets: &[(PaneId, Rect)]) -> Vec<Placed> {
        let now = ctx.input(|i| i.time);
        let t = ((now - self.start) / PANE_ANIM) as f32;
        if t >= 1.0 || !glide::animations(ctx) {
            self.from.clear();
            self.ghosts.clear();
        }
        let e = ease_out(t);
        let placed: Vec<Placed> = targets
            .iter()
            .map(|&(id, target)| {
                let card = self.from.get(&id).map_or(target, |f| f.lerp_towards(&target, e));
                Placed { id, card, target }
            })
            .collect();

        self.last = placed.iter().map(|p| (p.id, (p.card, p.target))).collect();
        self.focus.retain(|id, _| self.last.contains_key(id));
        self.ghosts.retain(|g| now - g.start < PANE_ANIM);
        for g in self.ghosts.iter_mut().filter(|g| g.to.is_none()) {
            g.to = Some(collapse(g.card, targets));
        }
        if !self.from.is_empty() || !self.ghosts.is_empty() {
            ctx.request_repaint();
        }
        placed
    }

    /// Closed panes still fading out, oldest first (paint them under the live panes).
    pub(super) fn ghosts(&self) -> &[Ghost] {
        &self.ghosts
    }

    /// How focused pane `id` looks, 0 (dimmed) to 1 (focused), fading between the two.
    pub(super) fn focus_amount(&mut self, ctx: &egui::Context, id: PaneId, focused: bool) -> f32 {
        let target = if focused { 1.0 } else { 0.0 };
        self.focus.entry(id).or_insert_with(|| Glide::new(FOCUS_FADE)).value(ctx, target)
    }
}

/// A zero-thickness strip on `r`'s right (split right) or bottom (split down) edge.
fn seam_seed(r: Rect, axis: Axis) -> Rect {
    match axis {
        Axis::Horizontal => Rect::from_min_max(Pos2::new(r.right(), r.top()), r.max),
        Axis::Vertical => Rect::from_min_max(Pos2::new(r.left(), r.bottom()), r.max),
    }
}

/// Where a closed pane's ghost collapses to: the edge of it farthest from the pane that
/// grows into its space (the target overlapping it most), or its center if there is none.
fn collapse(ghost: Rect, targets: &[(PaneId, Rect)]) -> Rect {
    let area = |r: Rect| r.intersect(ghost).area().max(0.0);
    let survivor = targets.iter().map(|(_, r)| *r).filter(|r| area(*r) > 0.0).max_by(|a, b| area(*a).total_cmp(&area(*b)));
    let Some(s) = survivor else {
        return Rect::from_center_size(ghost.center(), egui::Vec2::ZERO);
    };
    let (l, t, r, b) = (ghost.left(), ghost.top(), ghost.right(), ghost.bottom());
    if ghost.width() < s.width() - 1.0 {
        if l > s.left() + 1.0 {
            return Rect::from_min_max(Pos2::new(r, t), Pos2::new(r, b));
        }
        return Rect::from_min_max(Pos2::new(l, t), Pos2::new(l, b));
    }
    if t > s.top() + 1.0 {
        return Rect::from_min_max(Pos2::new(l, b), Pos2::new(r, b));
    }
    Rect::from_min_max(Pos2::new(l, t), Pos2::new(r, t))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1))
    }

    #[test]
    fn split_seed_is_the_far_edge() {
        let r = rect(0.0, 0.0, 100.0, 50.0);
        assert_eq!(seam_seed(r, Axis::Horizontal), rect(100.0, 0.0, 100.0, 50.0));
        assert_eq!(seam_seed(r, Axis::Vertical), rect(0.0, 50.0, 100.0, 50.0));
    }

    #[test]
    fn ghost_collapses_away_from_the_survivor() {
        // Survivor on the left grows over a ghost on the right: collapse to the right edge.
        let ghost = rect(54.0, 0.0, 100.0, 50.0);
        let to = collapse(ghost, &[(1, rect(0.0, 0.0, 100.0, 50.0))]);
        assert_eq!(to, rect(100.0, 0.0, 100.0, 50.0));
        // Survivor below grows over a ghost on top: collapse to the top edge.
        let ghost = rect(0.0, 0.0, 100.0, 20.0);
        let to = collapse(ghost, &[(1, rect(0.0, 0.0, 100.0, 50.0))]);
        assert_eq!(to, rect(0.0, 0.0, 100.0, 0.0));
    }

    #[test]
    fn frame_eases_from_start_to_target() {
        let ctx = egui::Context::default();
        let mut m = PaneMotion::default();
        let target = rect(0.0, 0.0, 50.0, 50.0);
        m.from.insert(1, rect(0.0, 0.0, 100.0, 50.0));
        m.start = ctx.input(|i| i.time);
        assert_eq!(m.frame(&ctx, &[(1, target)])[0].card, rect(0.0, 0.0, 100.0, 50.0));
        m.start -= PANE_ANIM;
        assert_eq!(m.frame(&ctx, &[(1, target)])[0].card, target);
        assert!(m.from.is_empty());
    }
}
