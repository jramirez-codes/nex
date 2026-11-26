//! An ink bleed: one sheet soaking through another from a single drop, like water on paper.
//!
//! egui has no stencil or offscreen pass, so the effect works on tessellated vertices: shapes
//! become meshes, big triangles are split until the soft front can run smoothly across them,
//! and every vertex is faded by how far the front has reached it.

use egui::epaint::{Mesh, Tessellator, Vertex};
use egui::{Color32, Pos2, Rect, Shape};

use crate::ui::glide::smoothstep;

/// Length of a bleed, in seconds.
pub const INK_ANIM: f64 = 0.5;
/// Width of the soft front, in points.
const FEATHER: f32 = 90.0;
/// How uneven the front is at the start (fraction of its radius); it settles to a circle.
const WOBBLE: f32 = 0.07;
/// How much darker the new sheet is along the wet front.
const RIM: f32 = 0.08;
/// Triangles longer than this are split, so the front can fade across them.
pub const MAX_EDGE: f32 = 48.0;

/// How the front meets one shape of a sheet this frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Cover {
    /// Not reached (new sheet) or not yet fading (old sheet): draw it as it is.
    Whole,
    /// Soaked away (old sheet) or not yet in (new sheet): draw nothing.
    Gone,
    /// The front runs across it: tessellate and shade it.
    Front,
}

/// One drop: where it landed, how far it must spread, and when.
#[derive(Clone, Debug)]
pub struct Bleed {
    origin: Pos2,
    /// Radius at the end: past the farthest corner of the area by a whole feather.
    reach: f32,
    start: f64,
    /// Phases of the edge's wobble, different for every drop.
    phases: [f32; 3],
}

impl Bleed {
    /// A drop at `origin` that will soak through all of `area`, starting at `start`.
    pub fn new(origin: Pos2, area: Rect, start: f64) -> Self {
        let far =
            [area.left_top(), area.right_top(), area.left_bottom(), area.right_bottom()].into_iter().map(|c| c.distance(origin)).fold(0.0, f32::max);
        // Any well-spread numbers will do: derive them from the start time.
        let seed = (start * 1000.0).fract() as f32 + start as f32;
        let phases = [seed * 7.31, seed * 13.7 + 1.9, seed * 3.17 + 4.2].map(|p| p.rem_euclid(std::f32::consts::TAU));
        Self { origin, reach: far + FEATHER, start, phases }
    }

    /// How far the front has spread, 0 to 1. The drop blooms, spreads, then settles: never
    /// faster than 1.5× its average speed, so every part of the page sees it pass.
    fn eased(&self, now: f64) -> f32 {
        smoothstep((((now - self.start) / INK_ANIM) as f32).clamp(0.0, 1.0))
    }

    pub fn done(&self, now: f64) -> bool {
        now - self.start >= INK_ANIM
    }

    /// Wobble of the edge at angle `theta`, in -1..=1.
    fn noise(&self, theta: f32) -> f32 {
        let [a, b, c] = self.phases;
        ((3.0 * theta + a).sin() + 0.6 * (5.0 * theta + b).sin() + 0.35 * (9.0 * theta + c).sin()) / 1.95
    }

    /// How far into the front `p` is: 0 not yet wet, 1 soaked through.
    fn depth(&self, p: Pos2, e: f32) -> f32 {
        let v = p - self.origin;
        let edge = e * self.reach * (1.0 + WOBBLE * (1.0 - e) * self.noise(v.y.atan2(v.x)));
        ((edge - v.length()) / FEATHER).clamp(0.0, 1.0)
    }

    /// How much of the new sheet shows at `p`: 0 to 1, soft across the front.
    #[cfg(test)]
    fn mask(&self, p: Pos2, now: f64) -> f32 {
        smoothstep(self.depth(p, self.eased(now)))
    }

    /// How the front meets a shape of the new sheet (`new`) or the old one inside `r`.
    pub fn cover(&self, r: Rect, now: f64, new: bool) -> Cover {
        let e = self.eased(now);
        let near = r.distance_to_pos(self.origin);
        let far = [r.left_top(), r.right_top(), r.left_bottom(), r.right_bottom()].into_iter().map(|c| c.distance(self.origin)).fold(0.0, f32::max);
        let wobble = WOBBLE * (1.0 - e);
        let (lo, hi) = (e * self.reach * (1.0 - wobble), e * self.reach * (1.0 + wobble));
        // Depth range over the rect.
        let (least, most) = ((lo - far) / FEATHER, (hi - near) / FEATHER);
        // The old sheet only starts fading halfway into the front (see `shade`).
        let (whole, gone) = if new { (least >= 1.0, most <= 0.0) } else { (most <= 0.5, least >= 1.0) };
        if whole {
            Cover::Whole
        } else if gone {
            Cover::Gone
        } else {
            Cover::Front
        }
    }

    /// Fade the sheet being covered (`new == false`) or the one soaking through. The new
    /// sheet comes in across the front, a little darker where it is wet; the old one stays
    /// until the new one is half in, then goes, so the two never thin out together.
    pub fn shade(&self, mesh: &mut Mesh, now: f64, new: bool) {
        let e = self.eased(now);
        for v in &mut mesh.vertices {
            let x = self.depth(v.pos, e);
            let (alpha, dark) = if new { (smoothstep(x), RIM * 4.0 * x * (1.0 - x)) } else { (1.0 - smoothstep((2.0 * x - 1.0).max(0.0)), 0.0) };
            v.color = fade(v.color, alpha, dark);
        }
    }
}

/// Premultiplied `c` at `alpha` opacity, its color darkened by `dark`.
fn fade(c: Color32, alpha: f32, dark: f32) -> Color32 {
    let rgb = alpha * (1.0 - dark);
    let ch = |v: u8, k: f32| (v as f32 * k).round() as u8;
    Color32::from_rgba_premultiplied(ch(c.r(), rgb), ch(c.g(), rgb), ch(c.b(), rgb), ch(c.a(), alpha))
}

/// Tessellate `shape` into meshes (one per run of shapes sharing a texture, in paint order),
/// appended to `out`.
pub fn tessellate(tess: &mut Tessellator, shape: Shape, out: &mut Vec<Mesh>) {
    match shape {
        Shape::Noop | Shape::Callback(_) => {}
        Shape::Vec(shapes) => {
            for s in shapes {
                tessellate(tess, s, out);
            }
        }
        s => {
            let mut m = Mesh::default();
            tess.tessellate_shape(s, &mut m);
            if m.is_empty() {
                return;
            }
            match out.last_mut() {
                Some(last) if last.texture_id == m.texture_id => last.append(m),
                _ => out.push(m),
            }
        }
    }
}

/// Split every triangle longer than `max_edge` at the middle of its longest edge, until none
/// is. Positions, texture coordinates and colors interpolate linearly, so it looks the same.
pub fn subdivide(mesh: &mut Mesh, max_edge: f32) {
    let max2 = max_edge * max_edge;
    let long = |m: &Mesh, i: u32, j: u32| m.vertices[i as usize].pos.distance_sq(m.vertices[j as usize].pos);
    if mesh.indices.as_chunks::<3>().0.iter().all(|t| long(mesh, t[0], t[1]).max(long(mesh, t[1], t[2])).max(long(mesh, t[2], t[0])) <= max2) {
        return;
    }
    let mut todo: Vec<[u32; 3]> = mesh.indices.as_chunks::<3>().0.iter().map(|t| [t[0], t[1], t[2]]).collect();
    let mut indices = Vec::with_capacity(mesh.indices.len());
    while let Some([a, b, c]) = todo.pop() {
        let (ab, bc, ca) = (long(mesh, a, b), long(mesh, b, c), long(mesh, c, a));
        if ab.max(bc).max(ca) <= max2 {
            indices.extend([a, b, c]);
            continue;
        }
        // Rotate so the longest edge is (a, b), keeping the winding.
        let [a, b, c] = if ab >= bc && ab >= ca {
            [a, b, c]
        } else if bc >= ca {
            [b, c, a]
        } else {
            [c, a, b]
        };
        let mid = midpoint(&mesh.vertices[a as usize], &mesh.vertices[b as usize]);
        let m = mesh.vertices.len() as u32;
        mesh.vertices.push(mid);
        todo.push([a, m, c]);
        todo.push([m, b, c]);
    }
    mesh.indices = indices;
}

fn midpoint(a: &Vertex, b: &Vertex) -> Vertex {
    let avg = |x: u8, y: u8| ((x as u16 + y as u16).div_ceil(2)) as u8;
    let (ca, cb) = (a.color, b.color);
    Vertex {
        pos: a.pos.lerp(b.pos, 0.5),
        uv: a.uv.lerp(b.uv, 0.5),
        color: Color32::from_rgba_premultiplied(avg(ca.r(), cb.r()), avg(ca.g(), cb.g()), avg(ca.b(), cb.b()), avg(ca.a(), cb.a())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area() -> Rect {
        Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 500.0))
    }

    #[test]
    fn nothing_shows_at_the_start_and_everything_at_the_end() {
        let b = Bleed::new(Pos2::new(400.0, 0.0), area(), 10.0);
        for p in [Pos2::new(400.0, 0.0), Pos2::new(0.0, 500.0), Pos2::new(800.0, 250.0)] {
            assert_eq!(b.mask(p, 10.0), 0.0);
            assert_eq!(b.mask(p, 10.0 + INK_ANIM), 1.0);
        }
    }

    #[test]
    fn the_front_spreads_out_from_the_drop() {
        let b = Bleed::new(Pos2::new(400.0, 0.0), area(), 0.0);
        let now = INK_ANIM * 0.3;
        let near = b.mask(Pos2::new(400.0, 20.0), now);
        let mid = b.mask(Pos2::new(400.0, 200.0), now);
        let far = b.mask(Pos2::new(400.0, 500.0), now);
        assert!(near >= mid && mid >= far, "{near} {mid} {far}");
        assert_eq!(near, 1.0);
        assert_eq!(far, 0.0);
    }

    #[test]
    fn the_edge_settles_round() {
        let b = Bleed::new(Pos2::new(400.0, 250.0), area(), 0.0);
        // At e = 1 the wobble is gone: every direction is at the same depth.
        let at = |theta: f32| b.depth(Pos2::new(400.0, 250.0) + 300.0 * egui::Vec2::angled(theta), 1.0);
        let d = at(0.0);
        for k in 1..12 {
            assert!((at(k as f32 * 0.5) - d).abs() < 1e-4);
        }
    }

    #[test]
    fn the_sheets_never_thin_out_together() {
        let b = Bleed::new(Pos2::new(100.0, 0.0), area(), 0.0);
        let quad = || {
            let mut m = Mesh::default();
            m.add_colored_rect(Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 500.0)), Color32::WHITE);
            subdivide(&mut m, 10.0);
            m
        };
        for k in 1..10 {
            let now = INK_ANIM * k as f64 / 10.0;
            let (mut old, mut new) = (quad(), quad());
            b.shade(&mut old, now, false);
            b.shade(&mut new, now, true);
            for (o, n) in old.vertices.iter().zip(&new.vertices) {
                // New painted over old.
                let (o, n) = (o.color.a() as f32 / 255.0, n.color.a() as f32 / 255.0);
                assert!(n + o * (1.0 - n) > 0.9, "{o} {n}");
            }
        }
    }

    #[test]
    fn cover_agrees_with_the_mask() {
        let b = Bleed::new(Pos2::new(400.0, 0.0), area(), 0.0);
        let now = INK_ANIM * 0.5;
        let r = |y0: f32, y1: f32| Rect::from_min_max(Pos2::new(380.0, y0), Pos2::new(420.0, y1));
        assert_eq!(b.cover(r(0.0, 10.0), now, true), Cover::Whole);
        assert_eq!(b.cover(r(0.0, 10.0), now, false), Cover::Gone);
        assert_eq!(b.cover(r(480.0, 500.0), now, true), Cover::Gone);
        assert_eq!(b.cover(r(480.0, 500.0), now, false), Cover::Whole);
        assert_eq!(b.cover(r(0.0, 500.0), now, true), Cover::Front);
        // Every point of a rect outside the front really has the mask it claims.
        for (rect, want) in [(r(0.0, 10.0), 1.0), (r(480.0, 500.0), 0.0)] {
            for p in [rect.left_top(), rect.right_bottom(), rect.center()] {
                assert_eq!(b.mask(p, now), want);
            }
        }
    }

    #[test]
    fn subdivide_splits_long_triangles_and_keeps_their_area() {
        let mut m = Mesh::default();
        m.add_rect_with_uv(
            Rect::from_min_max(Pos2::ZERO, Pos2::new(200.0, 90.0)),
            Rect::from_min_max(Pos2::ZERO, Pos2::new(1.0, 1.0)),
            Color32::WHITE,
        );
        subdivide(&mut m, MAX_EDGE);
        let mut area = 0.0;
        for t in m.indices.as_chunks::<3>().0 {
            let [a, b, c] = [t[0], t[1], t[2]].map(|i| m.vertices[i as usize]);
            for (p, q) in [(a, b), (b, c), (c, a)] {
                assert!(p.pos.distance(q.pos) <= MAX_EDGE + 1e-3);
                // Texture coordinates stay affine in position.
                assert!((p.uv.x - p.pos.x / 200.0).abs() < 1e-5 && (p.uv.y - p.pos.y / 90.0).abs() < 1e-5);
            }
            area += ((b.pos - a.pos).x * (c.pos - a.pos).y - (b.pos - a.pos).y * (c.pos - a.pos).x).abs() / 2.0;
        }
        assert!((area - 200.0 * 90.0).abs() < 0.5, "{area}");
    }
}
