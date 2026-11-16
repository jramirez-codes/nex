//! `tab-animation = ink`: the new tab soaks through the old one from a drop under its tab,
//! like water bleeding through a sheet of paper (the math is in `ui::ink`).
//!
//! Only shapes the front is crossing are tessellated and shaded; the rest are drawn as they
//! are (or not at all), which keeps a frame of the bleed cheap.

use std::ops::Range;

use egui::epaint::{ClippedShape, Mesh, Tessellator};
use egui::layers::{PaintList, ShapeIdx};
use egui::{Painter, Rect, Shape};

use crate::ui::ink::{self, Bleed, Cover};

use super::{Switch, Transition, tessellator};

/// One shape of the old sheet, tessellated the first time the front reaches it.
struct OldShape {
    clip: Rect,
    bounds: Rect,
    shape: Shape,
    meshes: Option<Vec<Mesh>>,
}

pub(super) struct InkSwitch {
    bleed: Bleed,
    old: Vec<OldShape>,
}

impl InkSwitch {
    pub(super) fn new(s: Switch) -> Self {
        let old = s
            .old
            .into_iter()
            .map(|cs| OldShape { clip: cs.clip_rect, bounds: bounds(&cs), shape: cs.shape, meshes: None })
            .collect();
        Self { bleed: Bleed::new(s.origin, s.area, s.start), old }
    }
}

impl Transition for InkSwitch {
    fn done(&self, now: f64) -> bool {
        self.bleed.done(now)
    }

    fn paint_old(&mut self, ctx: &egui::Context, painter: &Painter, now: f64) {
        let mut tess = None;
        for o in &mut self.old {
            let shape = match self.bleed.cover(o.bounds, now, false) {
                Cover::Gone => continue,
                Cover::Whole => o.shape.clone(),
                Cover::Front => {
                    let tess = tess.get_or_insert_with(|| tessellator(ctx));
                    let meshes = o.meshes.get_or_insert_with(|| meshes(tess, o.clip, o.shape.clone()));
                    Shape::Vec(
                        meshes
                            .iter()
                            .map(|m| {
                                let mut m = m.clone();
                                self.bleed.shade(&mut m, now, false);
                                Shape::mesh(m)
                            })
                            .collect(),
                    )
                }
            };
            painter.with_clip_rect(o.clip).add(shape);
        }
    }

    fn style_new(&self, list: &mut PaintList, range: Range<usize>, tess: Option<&mut Tessellator>, now: f64) {
        let Some(tess) = tess else { return };
        for i in range {
            list.mutate_shape(ShapeIdx(i), |cs| match self.bleed.cover(bounds(cs), now, true) {
                Cover::Whole => {}
                Cover::Gone => cs.shape = Shape::Noop,
                Cover::Front => {
                    let shape = std::mem::replace(&mut cs.shape, Shape::Noop);
                    let meshes = meshes(tess, cs.clip_rect, shape);
                    cs.shape = Shape::Vec(
                        meshes
                            .into_iter()
                            .map(|mut m| {
                                self.bleed.shade(&mut m, now, true);
                                Shape::mesh(m)
                            })
                            .collect(),
                    );
                }
            });
        }
    }

    fn needs_tessellator(&self) -> bool {
        true
    }
}

/// Where a painted shape can show: its bounds within its clip rect.
fn bounds(cs: &ClippedShape) -> Rect {
    cs.shape.visual_bounding_rect().intersect(cs.clip_rect)
}

/// One painted shape as meshes, split finely enough for the front to fade across.
fn meshes(tess: &mut Tessellator, clip: Rect, shape: Shape) -> Vec<Mesh> {
    tess.set_clip_rect(clip);
    let mut out = Vec::new();
    ink::tessellate(tess, shape, &mut out);
    for m in &mut out {
        ink::subdivide(m, ink::MAX_EDGE);
    }
    out
}
