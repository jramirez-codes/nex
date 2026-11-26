//! Pane layout transitions: a split slides the new pane out of the seam, a closed pane
//! collapses toward its outer edge and fades while its neighbor takes the space, and focus
//! dimming fades instead of switching.
//!
//! Only the cards move. Terminals are laid out (and their PTYs sized) at the target rect
//! once; while a card is in motion its cached shapes are just translated and clipped.

use std::collections::{HashMap, HashSet};

use egui::{Pos2, Rect, Shape};

use crate::layout::{Axis, PaneId};
use crate::ui::glide::{self, Glide, ease_out};

/// Length of a split / close transition, in seconds.
const PANE_ANIM: f64 = 0.18;
/// Length of the focus dim / ring fade, in seconds.
const FOCUS_FADE: f64 = 0.12;
/// A pane coming back on screen mid-transition (unzoom) grows in from this scale.
const APPEAR_SCALE: f32 = 0.97;

/// Where a pane is drawn this frame.
pub(in crate::app) struct Placed {
    pub(in crate::app) id: PaneId,
    /// The (possibly animated) card rect to paint.
    pub(in crate::app) card: Rect,
    /// The card rect the layout gives it: terminals are sized and hit-tested here.
    pub(in crate::app) target: Rect,
    /// Opacity: below 1 while a pane fades in.
    pub(in crate::app) alpha: f32,
    /// Drawn above the other panes (a zoomed pane shrinking back into its slot).
    pub(in crate::app) on_top: bool,
    /// Its content scales with the card instead of being clipped (zoom).
    pub(in crate::app) scale_content: bool,
}

/// A closed pane still fading out: what it looked like when it closed.
pub(in crate::app) struct Ghost {
    card: Rect,
    /// Where it collapses to, resolved on the first frame the new layout is known.
    to: Option<Rect>,
    /// The card position its shapes were built for.
    origin: Pos2,
    start: f64,
    pub(in crate::app) title: String,
    pub(in crate::app) process: String,
    pub(in crate::app) shapes: Vec<Shape>,
}

impl Ghost {
    /// The ghost's card this frame, how far its shapes moved, and its opacity.
    pub(in crate::app) fn at(&self, now: f64) -> (Rect, egui::Vec2, f32) {
        let e = ease_out(((now - self.start) / PANE_ANIM) as f32);
        let card = self.to.map_or(self.card, |to| self.card.lerp_towards(&to, e));
        (card, card.min - self.origin, 1.0 - e)
    }
}

#[derive(Default)]
pub(in crate::app) struct PaneMotion {
    /// `(drawn, target)` card rects from the last frame.
    last: HashMap<PaneId, (Rect, Rect)>,
    /// Start rects of the running transition.
    from: HashMap<PaneId, Rect>,
    start: f64,
    ghosts: Vec<Ghost>,
    focus: HashMap<PaneId, Glide>,
    /// Panes that came on screen during the running transition: they fade in.
    appearing: HashSet<PaneId>,
    /// Drawn on top until the transition ends.
    top: Option<PaneId>,
    /// Its content scales with its card until the transition ends.
    scaling: Option<PaneId>,
}

impl PaneMotion {
    /// Begin a transition from wherever every pane is drawn right now.
    fn restart(&mut self, now: f64) {
        self.from = self.last.iter().map(|(id, (drawn, _))| (*id, *drawn)).collect();
        self.start = now;
    }

    /// `new` was split off `target`: it grows out of `target`'s far edge as `target` shrinks.
    pub(in crate::app) fn kick_split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis, new: PaneId) {
        if !glide::animations(ctx) {
            return;
        }
        let Some(&(r, _)) = self.last.get(&target) else { return };
        self.restart(ctx.input(|i| i.time));
        self.from.insert(new, seam_seed(r, axis));
        ctx.request_repaint();
    }

    /// Ease every card from where it is drawn now to wherever the next layout puts it (after
    /// a swap, or when a dragged pane's drop preview changes).
    pub(in crate::app) fn kick_reflow(&mut self, ctx: &egui::Context) {
        if !glide::animations(ctx) {
            return;
        }
        self.restart(ctx.input(|i| i.time));
        ctx.request_repaint();
    }

    /// Pane `pane` was zoomed to fill the tab (`zooming_in`) or put back in its slot. Its card
    /// and content scale between the two. Zooming in, the `others` (id, title, process, last
    /// frame) stay where they are and fade out under it; zooming out, they fade back in
    /// under it.
    pub(in crate::app) fn kick_zoom(
        &mut self,
        ctx: &egui::Context,
        pane: PaneId,
        zooming_in: bool,
        others: Vec<(PaneId, String, String, Vec<Shape>)>,
    ) {
        if !glide::animations(ctx) {
            return;
        }
        let now = ctx.input(|i| i.time);
        self.restart(now);
        self.scaling = Some(pane);
        if zooming_in {
            for (id, title, process, shapes) in others {
                let Some(&(card, target)) = self.last.get(&id) else { continue };
                self.ghosts.push(Ghost { card, to: Some(card), origin: target.min, start: now, title, process, shapes });
            }
        } else {
            self.top = Some(pane);
        }
        ctx.request_repaint();
    }

    /// Pane `id` was closed: leave a ghost that collapses and fades while the rest move.
    pub(in crate::app) fn kick_close(&mut self, ctx: &egui::Context, id: PaneId, title: String, process: String, shapes: Vec<Shape>) {
        if !glide::animations(ctx) {
            return;
        }
        let Some((card, target)) = self.last.remove(&id) else { return };
        let now = ctx.input(|i| i.time);
        self.restart(now);
        self.ghosts.push(Ghost { card, to: None, origin: target.min, start: now, title, process, shapes });
        ctx.request_repaint();
    }

    /// Where each pane is drawn this frame, given the rects the layout wants. A pane in
    /// `goals` heads there instead of its layout rect (a drop preview: its terminal keeps the
    /// layout's size). The `held` pane (being dragged) is drawn exactly where it is held.
    pub(in crate::app) fn frame(
        &mut self,
        ctx: &egui::Context,
        targets: &[(PaneId, Rect)],
        goals: &HashMap<PaneId, Rect>,
        held: Option<(PaneId, Rect)>,
    ) -> Vec<Placed> {
        let now = ctx.input(|i| i.time);
        let t = ((now - self.start) / PANE_ANIM) as f32;
        if t >= 1.0 || !glide::animations(ctx) {
            self.from.clear();
            self.ghosts.clear();
            self.appearing.clear();
            self.top = None;
            self.scaling = None;
        }
        let running = !self.from.is_empty();
        let e = ease_out(t);
        let placed: Vec<Placed> = targets
            .iter()
            .map(|&(id, target)| {
                let goal = goals.get(&id).copied().unwrap_or(target);
                // New on screen mid-transition: grow in from a little smaller.
                if running && !self.from.contains_key(&id) && !self.last.contains_key(&id) {
                    self.from.insert(id, Rect::from_center_size(goal.center(), goal.size() * APPEAR_SCALE));
                    self.appearing.insert(id);
                }
                let card = match held {
                    Some((h, r)) if h == id => r,
                    _ => self.from.get(&id).map_or(goal, |f| f.lerp_towards(&goal, e)),
                };
                let alpha = if self.appearing.contains(&id) { e } else { 1.0 };
                Placed { id, card, target, alpha, on_top: self.top == Some(id), scale_content: self.scaling == Some(id) }
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
    pub(in crate::app) fn ghosts(&self) -> &[Ghost] {
        &self.ghosts
    }

    /// How focused pane `id` looks, 0 (dimmed) to 1 (focused), fading between the two.
    pub(in crate::app) fn focus_amount(&mut self, ctx: &egui::Context, id: PaneId, focused: bool) -> f32 {
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
        assert_eq!(m.frame(&ctx, &[(1, target)], &HashMap::new(), None)[0].card, rect(0.0, 0.0, 100.0, 50.0));
        m.start -= PANE_ANIM;
        assert_eq!(m.frame(&ctx, &[(1, target)], &HashMap::new(), None)[0].card, target);
        assert!(m.from.is_empty());
    }

    #[test]
    fn held_card_follows_the_pointer_then_glides_home() {
        let ctx = egui::Context::default();
        let mut m = PaneMotion::default();
        let target = rect(0.0, 0.0, 50.0, 50.0);
        let held = rect(30.0, 20.0, 80.0, 70.0);
        assert_eq!(m.frame(&ctx, &[(1, target)], &HashMap::new(), Some((1, held)))[0].card, held);
        // Let go: the next frames start from where it was held.
        m.restart(ctx.input(|i| i.time));
        assert_eq!(m.frame(&ctx, &[(1, target)], &HashMap::new(), None)[0].card, held);
    }

    #[test]
    fn unzoom_fades_the_others_in_under_the_shrinking_pane() {
        let ctx = egui::Context::default();
        let mut m = PaneMotion::default();
        let full = rect(0.0, 0.0, 100.0, 50.0);
        let (left, right) = (rect(0.0, 0.0, 50.0, 50.0), rect(50.0, 0.0, 100.0, 50.0));
        m.frame(&ctx, &[(1, full)], &HashMap::new(), None);
        m.kick_zoom(&ctx, 1, false, Vec::new());
        let p = m.frame(&ctx, &[(1, left), (2, right)], &HashMap::new(), None);
        assert_eq!((p[0].card, p[0].alpha, p[0].on_top, p[0].scale_content), (full, 1.0, true, true));
        assert_eq!(p[1].alpha, 0.0);
        assert_eq!(p[1].card, Rect::from_center_size(right.center(), right.size() * APPEAR_SCALE));
        m.start -= PANE_ANIM;
        let p = m.frame(&ctx, &[(1, left), (2, right)], &HashMap::new(), None);
        assert_eq!((p[0].card, p[0].on_top, p[0].scale_content), (left, false, false));
        assert_eq!((p[1].card, p[1].alpha), (right, 1.0));
    }

    #[test]
    fn zoom_in_leaves_the_others_fading_in_place() {
        let ctx = egui::Context::default();
        let mut m = PaneMotion::default();
        let full = rect(0.0, 0.0, 100.0, 50.0);
        let (left, right) = (rect(0.0, 0.0, 50.0, 50.0), rect(50.0, 0.0, 100.0, 50.0));
        m.frame(&ctx, &[(1, left), (2, right)], &HashMap::new(), None);
        m.kick_zoom(&ctx, 1, true, vec![(2, "t".into(), "p".into(), Vec::new())]);
        let p = m.frame(&ctx, &[(1, full)], &HashMap::new(), None);
        assert_eq!((p[0].card, p[0].alpha, p[0].on_top, p[0].scale_content), (left, 1.0, false, true));
        let now = ctx.input(|i| i.time);
        let (card, shift, alpha) = m.ghosts()[0].at(now);
        assert_eq!((card, shift, alpha), (right, egui::Vec2::ZERO, 1.0));
        let (card, _, alpha) = m.ghosts()[0].at(now + PANE_ANIM);
        assert_eq!((card, alpha), (right, 0.0));
    }

    #[test]
    fn goal_moves_the_card_but_not_its_target() {
        let ctx = egui::Context::default();
        let mut m = PaneMotion::default();
        let target = rect(0.0, 0.0, 50.0, 50.0);
        let goal = rect(60.0, 0.0, 110.0, 50.0);
        let p = &m.frame(&ctx, &[(1, target)], &HashMap::from([(1, goal)]), None)[0];
        assert_eq!((p.card, p.target), (goal, target));
    }
}
