//! `tab-animation = glide`: the old tab drifts a little the way the tab pill is moving and
//! fades out, while the new one drifts in behind it, from the other side, and fades in.
//!
//! Whole sheets only move and fade, so nothing is tessellated: shapes are translated and
//! their colors scaled, as egui's own `Painter::multiply_opacity` does.

use std::ops::Range;

use egui::emath::TSTransform;
use egui::epaint::shape_transform::adjust_colors;
use egui::epaint::{ClippedShape, Tessellator};
use egui::layers::{PaintList, ShapeIdx};
use egui::{Color32, Painter, Rect, Shape, Vec2};

use crate::ui::glide::{ease_out, smoothstep};

use super::{Switch, Transition};

/// Length of the glide, in seconds.
const GLIDE_ANIM: f64 = 0.32;
/// How far the sheets drift, as a fraction of the pane area along the way they move...
const DRIFT: f32 = 0.2;
/// ...but never more than this, in points, so wide windows stay subtle.
const MAX_DRIFT: f32 = 160.0;
/// The old sheet is gone by this far into the glide.
const OUT_BY: f32 = 0.6;
/// The new sheet starts showing this far in, so the two cross over instead of flashing.
const IN_FROM: f32 = 0.15;

pub(super) struct GlideSwitch {
    old: Vec<ClippedShape>,
    area: Rect,
    /// The full drift: where the old sheet ends up, and where the new one starts from (negated).
    drift: Vec2,
    start: f64,
}

/// Where each sheet is this frame: `(offset, alpha)` for the old and the new one.
struct Frame {
    old: (Vec2, f32),
    new: (Vec2, f32),
}

impl GlideSwitch {
    pub(super) fn new(s: Switch) -> Self {
        let extent = (s.travel.x.abs() * s.area.width() + s.travel.y.abs() * s.area.height()).max(0.0);
        let drift = s.travel * (DRIFT * extent).min(MAX_DRIFT);
        Self { old: s.old, area: s.area, drift, start: s.start }
    }

    fn at(&self, now: f64) -> Frame {
        let t = (((now - self.start) / GLIDE_ANIM) as f32).clamp(0.0, 1.0);
        let e = ease_out(t);
        Frame { old: (self.drift * e, 1.0 - smoothstep(t / OUT_BY)), new: (-self.drift * (1.0 - e), smoothstep((t - IN_FROM) / (1.0 - IN_FROM))) }
    }
}

impl Transition for GlideSwitch {
    fn done(&self, now: f64) -> bool {
        now - self.start >= GLIDE_ANIM
    }

    fn paint_old(&mut self, _ctx: &egui::Context, painter: &Painter, now: f64) {
        let (offset, alpha) = self.at(now).old;
        if alpha <= 0.0 {
            return;
        }
        for cs in &self.old {
            let mut shape = cs.shape.clone();
            shape.translate(offset);
            fade(&mut shape, alpha);
            painter.with_clip_rect(cs.clip_rect.translate(offset).intersect(self.area)).add(shape);
        }
    }

    fn style_new(&self, list: &mut PaintList, range: Range<usize>, _tess: Option<&mut Tessellator>, now: f64) {
        let (offset, alpha) = self.at(now).new;
        if range.is_empty() || (offset == Vec2::ZERO && alpha >= 1.0) {
            return;
        }
        list.transform_range(ShapeIdx(range.start), ShapeIdx(range.end), TSTransform::from_translation(offset));
        for i in range {
            list.mutate_shape(ShapeIdx(i), |cs| {
                if alpha <= 0.0 {
                    cs.shape = Shape::Noop;
                    return;
                }
                fade(&mut cs.shape, alpha);
                cs.clip_rect = cs.clip_rect.intersect(self.area);
            });
        }
    }
}

/// Scale every color of `shape` to `alpha` opacity.
///
/// Text laid out in the placeholder color keeps it: the tessellator swaps in the text's
/// fallback color (faded here like any other) only where it finds the placeholder untouched,
/// and would draw a scaled one as it is, a green.
fn fade(shape: &mut Shape, alpha: f32) {
    if alpha < 1.0 {
        adjust_colors(shape, move |c| {
            if *c != Color32::PLACEHOLDER {
                *c = c.gamma_multiply(alpha);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    fn glide(travel: Vec2) -> GlideSwitch {
        let area = Rect::from_min_max(Pos2::ZERO, Pos2::new(400.0, 1000.0));
        GlideSwitch::new(Switch { old: Vec::new(), area, origin: Pos2::ZERO, travel, start: 0.0 })
    }

    #[test]
    fn the_old_sheet_leaves_as_the_new_one_arrives() {
        let g = glide(Vec2::new(-1.0, 0.0));
        let f = g.at(0.0);
        assert_eq!(f.old, (Vec2::ZERO, 1.0));
        assert_eq!(f.new, (Vec2::new(80.0, 0.0), 0.0));
        let f = g.at(GLIDE_ANIM);
        assert_eq!(f.old, (Vec2::new(-80.0, 0.0), 0.0));
        assert_eq!(f.new, (Vec2::ZERO, 1.0));
        assert!(g.done(GLIDE_ANIM));
    }

    #[test]
    fn the_drift_follows_the_strip_and_stays_small() {
        // Vertical strip (the sidebar): along y, capped on a tall area.
        let g = glide(Vec2::new(0.0, 1.0));
        assert_eq!(g.drift, Vec2::new(0.0, MAX_DRIFT));
    }

    #[test]
    fn the_new_sheet_moves_and_fades_in_place() {
        let g = glide(Vec2::new(-1.0, 0.0));
        let mut list = PaintList::default();
        let r = Rect::from_min_max(Pos2::new(10.0, 10.0), Pos2::new(50.0, 50.0));
        list.add(g.area, Shape::rect_filled(r, 0.0, egui::Color32::WHITE));
        g.style_new(&mut list, 0..1, None, GLIDE_ANIM * 0.5);
        let Frame { new: (offset, alpha), .. } = g.at(GLIDE_ANIM * 0.5);
        let Shape::Rect(rect) = &list.all_entries().next().unwrap().shape else { panic!("still a rect") };
        assert_eq!(rect.rect, r.translate(offset));
        assert!(offset.x > 0.0 && alpha > 0.0 && alpha < 1.0);
        assert_eq!(rect.fill, egui::Color32::WHITE.gamma_multiply(alpha));
    }

    #[test]
    fn text_in_the_placeholder_color_fades_in_its_own_color() {
        // Pane header icons are laid out in the placeholder color and painted with a fallback.
        let ctx = egui::Context::default();
        let mut text = None;
        let _ = ctx.run(egui::RawInput::default(), |ctx| {
            let galley = ctx.fonts_mut(|f| f.layout_no_wrap("x".into(), egui::FontId::monospace(14.0), Color32::PLACEHOLDER));
            text = Some(egui::epaint::TextShape::new(Pos2::new(10.0, 10.0), galley, Color32::RED));
        });
        let mut shape = Shape::Text(text.unwrap());
        fade(&mut shape, 0.5);
        let Shape::Text(text) = &shape else { panic!("still text") };
        assert_eq!(text.fallback_color, Color32::RED.gamma_multiply(0.5));
        let glyph_colors = text.galley.rows.iter().flat_map(|r| r.visuals.mesh.vertices.iter().map(|v| v.color));
        let section_colors = text.galley.job.sections.iter().map(|s| s.format.color);
        for c in glyph_colors.chain(section_colors) {
            assert_eq!(c, Color32::PLACEHOLDER);
        }
    }
}
