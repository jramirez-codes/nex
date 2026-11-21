//! The nex collie from `assets/nex-animated.svg`, without its tile, painted with egui shapes
//! (nex has no SVG renderer). It marks the Files tab. [`paint`] takes the time since the
//! intro started: the dog pops in, opens its eyes, sniffs, looks left and right, blinks twice
//! and bleps, with the same keyframes as the SVG, then rests on the pose of `assets/nex.svg`.

use egui::epaint::{Mesh, PathShape, PathStroke};
use egui::{Color32, Painter, Pos2, Rect, Shape, Vec2};

/// Length of the wake-up intro, in seconds (the SVG's longest animation).
pub const INTRO: f32 = 3.6;
/// Where the intro's tongue blep begins (80.56% in the SVG). The dog is in its resting pose
/// there, so playing from here to [`INTRO`] is the blep on its own.
pub const BLEP_START: f32 = 2.9;

/// The SVG area the dog covers (strokes included), as center and side, in viewBox units.
const CENTER: Pos2 = Pos2::new(64.0, 67.0);
const SIDE: f32 = 86.0;

const COAT_FROM: Pos2 = Pos2::new(26.0, 24.0);
const COAT_TO: Pos2 = Pos2::new(102.0, 108.0);
const COAT_A: Color32 = Color32::from_rgb(0xc7, 0x92, 0xea);
const COAT_B: Color32 = Color32::from_rgb(0x6c, 0xb6, 0xff);
const FOLD: Color32 = Color32::from_rgb(0x5b, 0x6f, 0xc4);
const BLAZE: Color32 = Color32::from_rgb(0xf4, 0xf1, 0xf7);
const INK: Color32 = Color32::from_rgb(0x16, 0x14, 0x1a);
const TONGUE: Color32 = Color32::from_rgb(0xff, 0x7a, 0xa2);
const TONGUE_LINE: Color32 = Color32::from_rgb(0xd9, 0x4f, 0x7c);
/// The coat's stroke in the SVG (round joins), which rounds every corner of the silhouette.
const COAT_STROKE: f32 = 5.0;

/// Paint the dog filling the square centered in `r`. `t` is seconds since the intro started
/// (anything past [`INTRO`] is the resting pose). `dim` blends every color toward a color,
/// by an amount, which keeps the dog opaque (so overlapping shapes don't show seams).
pub fn paint(p: &Painter, r: Rect, t: f32, dim: (Color32, f32)) {
    let scale = r.width().min(r.height()) / SIDE;
    let base = Affine::map(CENTER, r.center(), scale);
    let pose = Pose::at(t);
    let shade = Shade { inverse: base.inverse(), dim };
    let tilt = base.then(&Affine::about(Pos2::new(64.0, 100.0), Vec2::ZERO, pose.tilt, Vec2::splat(1.0)));
    let stroke = COAT_STROKE * scale;

    let layer = |opacity: f32| {
        let mut p = p.clone();
        p.multiply_opacity(opacity);
        p
    };

    // Left ear, pricked.
    let ear_l = tilt.then(&Affine::about(Pos2::new(46.0, 54.0), Vec2::ZERO, pose.ear_l_rot, Vec2::splat(pose.ear_l_scale)));
    let ear_l_p = layer(pose.ear_l_op);
    rounded(&ear_l_p, &ear_l, &[[35.0, 61.0], [31.0, 27.0], [56.0, 45.0]].map(pt), stroke * pose.ear_l_scale, Fill::Coat, shade);

    // The head, then the right ear above it so its folded tip overlaps the head.
    let head = tilt.then(&Affine::about(Pos2::new(64.0, 75.0), Vec2::ZERO, 0.0, Vec2::splat(pose.head_scale)));
    let head_p = layer(pose.head_op);
    let outline = head_outline();
    rounded(&head_p, &head, &outline, stroke * pose.head_scale, Fill::Coat, shade);

    let ear_r = tilt.then(&Affine::about(Pos2::new(83.0, 53.0), Vec2::ZERO, pose.ear_r_rot, Vec2::splat(pose.ear_r_scale)));
    let ear_r_p = layer(pose.ear_r_op);
    let ear_stroke = stroke * pose.ear_r_scale;
    rounded(&ear_r_p, &ear_r, &[[93.0, 61.0], [97.0, 37.0], [80.0, 31.0], [72.0, 45.0]].map(pt), ear_stroke, Fill::Coat, shade);
    rounded(&ear_r_p, &ear_r, &[[80.0, 31.0], [97.0, 37.0], [90.0, 51.0]].map(pt), ear_stroke, Fill::Solid(FOLD), shade);

    // Blaze and muzzle, clipped to the head.
    let blaze = clip_convex(&blaze_outline(), &convex_hull(&outline));
    let blaze_screen: Vec<Pos2> = blaze.iter().map(|&q| head.apply(q)).collect();
    fill(&head_p, &blaze_screen, &triangulate(&blaze), Fill::Solid(BLAZE), shade);
    // A hairline of the same white antialiases the mesh's edge.
    head_p.add(PathShape::closed_line(blaze_screen, PathStroke::new(0.8, shade.solid(BLAZE))));

    // Eyes.
    let eyes = head.then(&Affine::about(Pos2::new(64.0, 67.0), Vec2::new(pose.eyes_x, 0.0), 0.0, Vec2::new(1.0, pose.eyes_sy)));
    let eye_r = 4.5 * scale * pose.head_scale;
    for x in [47.0, 81.0] {
        let c = eyes.apply(Pos2::new(x, 67.0));
        head_p.add(Shape::ellipse_filled(c, Vec2::new(eye_r, eye_r * pose.eyes_sy), shade.solid(INK)));
    }

    // Tongue, behind the nose.
    if pose.tongue_op > 0.0 {
        let tongue = head.then(&Affine::about(Pos2::new(64.0, 91.0), Vec2::ZERO, 0.0, Vec2::new(1.0, pose.tongue_sy)));
        let mut pts = vec![Pos2::new(60.5, 91.0)];
        quad(&mut pts, Pos2::new(60.0, 101.5), Pos2::new(64.0, 101.5));
        quad(&mut pts, Pos2::new(68.0, 101.5), Pos2::new(67.5, 91.0));
        let pts: Vec<Pos2> = pts.into_iter().map(|q| tongue.apply(q)).collect();
        let tongue_p = {
            let mut p = head_p.clone();
            p.multiply_opacity(pose.tongue_op);
            p
        };
        tongue_p.add(PathShape::convex_polygon(pts, shade.solid(TONGUE), PathStroke::NONE));
        let line = [tongue.apply(Pos2::new(64.0, 94.5)), tongue.apply(Pos2::new(64.0, 99.0))];
        tongue_p.line_segment(line, (1.0 * scale * pose.head_scale, shade.solid(TONGUE_LINE)));
    }

    // Nose.
    let nose = head.then(&Affine::about(Pos2::new(64.0, 87.0), pose.nose_t, 0.0, pose.nose_s));
    let mut pts = vec![Pos2::new(57.0, 84.0)];
    quad(&mut pts, Pos2::new(64.0, 80.5), Pos2::new(71.0, 84.0));
    quad(&mut pts, Pos2::new(71.0, 89.0), Pos2::new(64.0, 94.0));
    quad(&mut pts, Pos2::new(57.0, 89.0), Pos2::new(57.0, 84.0));
    pts.pop();
    rounded(&head_p, &nose, &pts, 2.0 * scale * pose.head_scale, Fill::Solid(INK), shade);
}

fn pt([x, y]: [f32; 2]) -> Pos2 {
    Pos2::new(x, y)
}

/// The head: `M45 46 Q64 38 83 46 L96 64 L103 78 L92 81 Q88 97 73 105 Q64 109 55 105 Q40 97
/// 36 81 L25 78 L32 64 Z`.
fn head_outline() -> Vec<Pos2> {
    let mut v = vec![Pos2::new(45.0, 46.0)];
    quad(&mut v, Pos2::new(64.0, 38.0), Pos2::new(83.0, 46.0));
    v.extend([[96.0, 64.0], [103.0, 78.0], [92.0, 81.0]].map(pt));
    quad(&mut v, Pos2::new(88.0, 97.0), Pos2::new(73.0, 105.0));
    quad(&mut v, Pos2::new(64.0, 109.0), Pos2::new(55.0, 105.0));
    quad(&mut v, Pos2::new(40.0, 97.0), Pos2::new(36.0, 81.0));
    v.extend([[25.0, 78.0], [32.0, 64.0]].map(pt));
    v
}

/// The blaze running down the face into the muzzle (before clipping to the head).
fn blaze_outline() -> Vec<Pos2> {
    let mut v = vec![Pos2::new(61.0, 36.0), Pos2::new(67.0, 36.0)];
    quad(&mut v, Pos2::new(69.0, 50.0), Pos2::new(69.5, 61.0));
    quad(&mut v, Pos2::new(70.0, 71.0), Pos2::new(80.0, 77.0));
    quad(&mut v, Pos2::new(91.0, 84.0), Pos2::new(88.0, 96.0));
    v.extend([[80.0, 113.0], [48.0, 113.0], [40.0, 96.0]].map(pt));
    quad(&mut v, Pos2::new(37.0, 84.0), Pos2::new(48.0, 77.0));
    quad(&mut v, Pos2::new(58.0, 71.0), Pos2::new(58.5, 61.0));
    quad(&mut v, Pos2::new(59.0, 50.0), Pos2::new(61.0, 36.0));
    v.pop(); // back at the start
    v
}

/// Append a quadratic Bézier from the last point through control `c` to `to`.
fn quad(v: &mut Vec<Pos2>, c: Pos2, to: Pos2) {
    const STEPS: usize = 8;
    let from = *v.last().expect("a curve starts from a point");
    for i in 1..=STEPS {
        let s = i as f32 / STEPS as f32;
        let a = from.to_vec2() * (1.0 - s) * (1.0 - s);
        let b = c.to_vec2() * 2.0 * s * (1.0 - s);
        let d = to.to_vec2() * s * s;
        v.push((a + b + d).to_pos2());
    }
}

#[derive(Clone, Copy)]
enum Fill {
    Coat,
    Solid(Color32),
}

/// Turns SVG colors into painted ones: the coat gradient (in the dog's own space, so fill and
/// outline agree), and the dim.
#[derive(Clone, Copy)]
struct Shade {
    inverse: Affine,
    dim: (Color32, f32),
}

impl Shade {
    fn solid(&self, c: Color32) -> Color32 {
        lerp(c, self.dim.0, self.dim.1)
    }

    fn at(&self, fill: Fill, screen: Pos2) -> Color32 {
        match fill {
            Fill::Solid(c) => self.solid(c),
            Fill::Coat => {
                let q = self.inverse.apply(screen);
                let axis = COAT_TO - COAT_FROM;
                let s = ((q - COAT_FROM).dot(axis) / axis.length_sq()).clamp(0.0, 1.0);
                self.solid(lerp(COAT_A, COAT_B, s))
            }
        }
    }
}

fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    Color32::from_rgba_premultiplied(mix(a.r(), b.r()), mix(a.g(), b.g()), mix(a.b(), b.b()), mix(a.a(), b.a()))
}

/// A filled polygon with a round-joined outline of `width` (like SVG's `stroke-linejoin:
/// round`): a fill mesh, then each edge as a segment and a dot on every corner, so sharp
/// corners (the ear tips) round off instead of spiking into miters.
fn rounded(p: &Painter, m: &Affine, pts: &[Pos2], width: f32, color: Fill, shade: Shade) {
    let screen: Vec<Pos2> = pts.iter().map(|&q| m.apply(q)).collect();
    fill(p, &screen, &triangulate(pts), color, shade);
    if width <= 0.0 {
        return;
    }
    for (i, &a) in screen.iter().enumerate() {
        let b = screen[(i + 1) % screen.len()];
        let stroke = PathStroke::new_uv(width, move |_, at| shade.at(color, at));
        p.add(PathShape::line(vec![a, b], stroke));
        p.circle_filled(a, width / 2.0, shade.at(color, a));
    }
}

/// A mesh over `screen` from triangle `indices`, colored per vertex.
fn fill(p: &Painter, screen: &[Pos2], indices: &[[u32; 3]], color: Fill, shade: Shade) {
    let mut mesh = Mesh::default();
    for &q in screen {
        mesh.colored_vertex(q, shade.at(color, q));
    }
    for &[a, b, c] in indices {
        mesh.add_triangle(a, b, c);
    }
    p.add(Shape::mesh(mesh));
}

fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
    (a - o).x * (b - o).y - (a - o).y * (b - o).x
}

/// Triangles covering a simple polygon (ear clipping). epaint only fills convex paths, and the
/// head and blaze aren't.
fn triangulate(pts: &[Pos2]) -> Vec<[u32; 3]> {
    let area: f32 = (0..pts.len()).map(|i| cross(Pos2::ZERO, pts[i], pts[(i + 1) % pts.len()])).sum();
    let ccw = area > 0.0;
    let mut left: Vec<usize> = (0..pts.len()).collect();
    let mut out = Vec::with_capacity(pts.len().saturating_sub(2));
    while left.len() > 3 {
        let n = left.len();
        let ear = (0..n).find(|&i| {
            let (a, b, c) = (pts[left[(i + n - 1) % n]], pts[left[i]], pts[left[(i + 1) % n]]);
            let turn = cross(a, b, c);
            if (turn > 0.0) != ccw || turn.abs() < 1e-4 {
                return false;
            }
            // No other corner inside the ear.
            left.iter().all(|&j| {
                let q = pts[j];
                if q == a || q == b || q == c {
                    return true;
                }
                let (d1, d2, d3) = (cross(a, b, q), cross(b, c, q), cross(c, a, q));
                let inside = if ccw { d1 > 0.0 && d2 > 0.0 && d3 > 0.0 } else { d1 < 0.0 && d2 < 0.0 && d3 < 0.0 };
                !inside
            })
        });
        // A degenerate remainder (collinear points): drop a corner and carry on.
        let i = ear.unwrap_or(0);
        out.push([left[(i + n - 1) % n] as u32, left[i] as u32, left[(i + 1) % n] as u32]);
        left.remove(i);
    }
    if left.len() == 3 {
        out.push([left[0] as u32, left[1] as u32, left[2] as u32]);
    }
    out
}

/// The convex hull, counter-clockwise in math terms (monotone chain).
fn convex_hull(pts: &[Pos2]) -> Vec<Pos2> {
    let mut v = pts.to_vec();
    v.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    let mut hull: Vec<Pos2> = Vec::with_capacity(v.len() + 1);
    for pass in 0..2 {
        let start = hull.len();
        for &q in v.iter() {
            while hull.len() >= start + 2 && cross(hull[hull.len() - 2], hull[hull.len() - 1], q) <= 0.0 {
                hull.pop();
            }
            hull.push(q);
        }
        hull.pop();
        if pass == 0 {
            v.reverse();
        }
    }
    hull
}

/// `subject` clipped to the convex polygon `clip` (Sutherland–Hodgman). The blaze is clipped
/// to the head's hull, which matches the head wherever the blaze reaches its edge.
fn clip_convex(subject: &[Pos2], clip: &[Pos2]) -> Vec<Pos2> {
    let mut out = subject.to_vec();
    for i in 0..clip.len() {
        let (a, b) = (clip[i], clip[(i + 1) % clip.len()]);
        let input = std::mem::take(&mut out);
        let inside = |q: Pos2| cross(a, b, q) >= 0.0;
        for j in 0..input.len() {
            let (cur, prev) = (input[j], input[(j + input.len() - 1) % input.len()]);
            let cut = || {
                let (d1, d2) = (cross(a, b, prev), cross(a, b, cur));
                prev + (cur - prev) * (d1 / (d1 - d2))
            };
            match (inside(prev), inside(cur)) {
                (true, true) => out.push(cur),
                (true, false) => out.push(cut()),
                (false, true) => {
                    out.push(cut());
                    out.push(cur);
                }
                (false, false) => {}
            }
        }
    }
    out
}

/// A 2D affine map: `p ↦ (a·x + c·y + e, b·x + d·y + f)`.
#[derive(Clone, Copy, Debug)]
struct Affine {
    a: f32,
    b: f32,
    c: f32,
    d: f32,
    e: f32,
    f: f32,
}

impl Affine {
    /// SVG units around `from` to points around `to`, `scale` points per unit.
    fn map(from: Pos2, to: Pos2, scale: f32) -> Self {
        Affine { a: scale, b: 0.0, c: 0.0, d: scale, e: to.x - from.x * scale, f: to.y - from.y * scale }
    }

    /// CSS `translate(t) rotate(deg) scale(s)` with `transform-origin: o`.
    fn about(o: Pos2, t: Vec2, deg: f32, s: Vec2) -> Self {
        let (sin, cos) = deg.to_radians().sin_cos();
        let (a, b, c, d) = (cos * s.x, sin * s.x, -sin * s.y, cos * s.y);
        Affine { a, b, c, d, e: o.x + t.x - (a * o.x + c * o.y), f: o.y + t.y - (b * o.x + d * o.y) }
    }

    /// `self` after `inner` (`inner` is applied first).
    fn then(&self, inner: &Affine) -> Self {
        Affine {
            a: self.a * inner.a + self.c * inner.b,
            b: self.b * inner.a + self.d * inner.b,
            c: self.a * inner.c + self.c * inner.d,
            d: self.b * inner.c + self.d * inner.d,
            e: self.a * inner.e + self.c * inner.f + self.e,
            f: self.b * inner.e + self.d * inner.f + self.f,
        }
    }

    fn inverse(&self) -> Self {
        let det = self.a * self.d - self.b * self.c;
        let (a, b, c, d) = (self.d / det, -self.b / det, -self.c / det, self.a / det);
        Affine { a, b, c, d, e: -(a * self.e + c * self.f), f: -(b * self.e + d * self.f) }
    }

    fn apply(&self, q: Pos2) -> Pos2 {
        Pos2::new(self.a * q.x + self.c * q.y + self.e, self.b * q.x + self.d * q.y + self.f)
    }
}

/// CSS `cubic-bezier(x1, y1, x2, y2)`.
#[derive(Clone, Copy)]
struct Ease(f32, f32, f32, f32);

const EASE_IN_OUT: Ease = Ease(0.42, 0.0, 0.58, 1.0);
const EASE_OUT: Ease = Ease(0.0, 0.0, 0.58, 1.0);

impl Ease {
    fn apply(self, u: f32) -> f32 {
        let Ease(x1, y1, x2, y2) = self;
        let bez = |s: f32, p1: f32, p2: f32| 3.0 * (1.0 - s) * (1.0 - s) * s * p1 + 3.0 * (1.0 - s) * s * s * p2 + s * s * s;
        // x(s) is monotonic for x1, x2 in [0, 1]: bisect for s, then read y.
        let (mut lo, mut hi) = (0.0f32, 1.0f32);
        for _ in 0..24 {
            let mid = (lo + hi) / 2.0;
            if bez(mid, x1, x2) < u { lo = mid } else { hi = mid }
        }
        bez((lo + hi) / 2.0, y1, y2)
    }
}

/// One CSS animation: `duration`, `delay`, timing function and `fill-mode: both`.
struct Track<const N: usize> {
    delay: f32,
    duration: f32,
    ease: Ease,
    /// `(percent, values)`, from 0 to 100.
    keys: &'static [(f32, [f32; N])],
}

impl<const N: usize> Track<N> {
    /// The values at `t` seconds. The timing function eases each keyframe interval.
    fn at(&self, t: f32) -> [f32; N] {
        let pct = ((t - self.delay) / self.duration).clamp(0.0, 1.0) * 100.0;
        let keys = self.keys;
        let i = keys.iter().rposition(|k| k.0 <= pct).unwrap_or(0).min(keys.len() - 2);
        let ((p0, v0), (p1, v1)) = (keys[i], keys[i + 1]);
        let u = if p1 > p0 { ((pct - p0) / (p1 - p0)).clamp(0.0, 1.0) } else { 1.0 };
        let e = self.ease.apply(u);
        std::array::from_fn(|k| v0[k] + (v1[k] - v0[k]) * e)
    }
}

// The SVG's keyframes, plus a head wiggle during the blep (see `TILT`).
const HEAD_SCALE: Track<1> = Track { delay: 0.35, duration: 0.6, ease: Ease(0.3, 1.5, 0.5, 1.0), keys: &[(0.0, [0.12]), (100.0, [1.0])] };
const HEAD_OP: Track<1> = Track { keys: &[(0.0, [0.0]), (30.0, [1.0]), (100.0, [1.0])], ..HEAD_SCALE };
const EAR_L: Track<2> =
    Track { delay: 0.75, duration: 0.55, ease: Ease(0.3, 1.6, 0.5, 1.0), keys: &[(0.0, [-40.0, 0.4]), (100.0, [0.0, 1.0])] };
const EAR_L_OP: Track<1> = Track { delay: 0.75, duration: 0.55, ease: Ease(0.3, 1.6, 0.5, 1.0), keys: &[(0.0, [0.0]), (40.0, [1.0]), (100.0, [1.0])] };
const EAR_R: Track<3> = Track {
    delay: 0.85,
    duration: 0.9,
    ease: EASE_OUT,
    // rotation, scale, opacity
    keys: &[(0.0, [45.0, 0.4, 0.0]), (30.0, [-12.0, 1.0, 1.0]), (55.0, [7.0, 1.0, 1.0]), (78.0, [-3.0, 1.0, 1.0]), (100.0, [0.0, 1.0, 1.0])],
};
const TILT: Track<1> = Track {
    delay: 0.0,
    duration: INTRO,
    ease: EASE_IN_OUT,
    keys: &[
        (0.0, [0.0]),
        (41.67, [0.0]),
        (47.22, [-6.0]),
        (54.17, [-6.0]),
        (56.94, [6.0]),
        (65.28, [6.0]),
        (69.44, [-1.5]),
        (72.22, [0.0]),
        // Not in the SVG: a head wiggle through the blep, so it reads as moving when it
        // plays on its own (switching to Files).
        (81.11, [0.0]),
        (83.33, [-7.0]),
        (86.11, [7.0]),
        (88.89, [-4.0]),
        (91.67, [2.0]),
        (94.44, [0.0]),
        (100.0, [0.0]),
    ],
};
/// translateX, scaleY
const EYES: Track<2> = Track {
    delay: 0.0,
    duration: INTRO,
    ease: EASE_IN_OUT,
    keys: &[
        (0.0, [0.0, 0.1]),
        (26.39, [0.0, 0.1]),
        (30.56, [0.0, 1.15]),
        (33.33, [0.0, 1.0]),
        (41.67, [0.0, 1.0]),
        (47.22, [-2.5, 1.0]),
        (54.17, [-2.5, 1.0]),
        (56.94, [2.5, 1.0]),
        (65.28, [2.5, 1.0]),
        (68.06, [0.0, 1.0]),
        (70.83, [0.0, 1.0]),
        (72.22, [0.0, 0.1]),
        (73.61, [0.0, 1.0]),
        (75.56, [0.0, 0.1]),
        (77.78, [0.0, 1.0]),
        (81.94, [0.0, 1.0]),
        (84.72, [0.0, 0.55]),
        (95.83, [0.0, 0.55]),
        (100.0, [0.0, 1.0]),
    ],

};
/// translate x, y, scale x, y
const NOSE: Track<4> = Track {
    delay: 0.0,
    duration: INTRO,
    ease: EASE_IN_OUT,
    keys: &[
        (0.0, [0.0, 0.0, 1.0, 1.0]),
        (27.78, [0.0, 0.0, 1.0, 1.0]),
        (30.0, [0.0, -0.8, 1.12, 0.9]),
        (32.22, [0.0, 0.0, 1.0, 1.0]),
        (34.44, [0.0, -0.8, 1.12, 0.9]),
        (36.67, [0.0, 0.0, 1.0, 1.0]),
        (38.89, [0.0, -0.8, 1.12, 0.9]),
        (41.67, [0.0, 0.0, 1.0, 1.0]),
        (47.22, [-1.0, 0.0, 1.0, 1.0]),
        (54.17, [-1.0, 0.0, 1.0, 1.0]),
        (56.94, [1.0, 0.0, 1.0, 1.0]),
        (65.28, [1.0, 0.0, 1.0, 1.0]),
        (68.06, [0.0, 0.0, 1.0, 1.0]),
        (81.94, [0.0, 0.0, 1.0, 1.0]),
        (84.72, [0.0, -0.6, 1.06, 1.0]),
        (95.83, [0.0, -0.6, 1.06, 1.0]),
        (100.0, [0.0, 0.0, 1.0, 1.0]),
    ],

};
/// scaleY, opacity
const BLEP: Track<2> = Track {
    delay: 0.0,
    duration: INTRO,
    ease: EASE_IN_OUT,
    keys: &[
        (0.0, [0.01, 0.0]),
        (80.56, [0.01, 0.0]),
        (81.11, [0.05, 1.0]),
        (84.72, [1.1, 1.0]),
        (87.5, [1.0, 1.0]),
        (94.44, [1.0, 1.0]),
        (98.06, [0.05, 1.0]),
        (98.61, [0.01, 0.0]),
        (100.0, [0.01, 0.0]),
    ],

};

/// Every animated value at one moment.
#[derive(Debug, PartialEq)]
struct Pose {
    tilt: f32,
    head_scale: f32,
    head_op: f32,
    ear_l_rot: f32,
    ear_l_scale: f32,
    ear_l_op: f32,
    ear_r_rot: f32,
    ear_r_scale: f32,
    ear_r_op: f32,
    eyes_x: f32,
    eyes_sy: f32,
    nose_t: Vec2,
    nose_s: Vec2,
    tongue_sy: f32,
    tongue_op: f32,
}

impl Pose {
    fn at(t: f32) -> Self {
        let [tilt] = TILT.at(t);
        let [head_scale] = HEAD_SCALE.at(t);
        let [head_op] = HEAD_OP.at(t);
        let [ear_l_rot, ear_l_scale] = EAR_L.at(t);
        let [ear_l_op] = EAR_L_OP.at(t);
        let [ear_r_rot, ear_r_scale, ear_r_op] = EAR_R.at(t);
        let [eyes_x, eyes_sy] = EYES.at(t);
        let [nx, ny, nsx, nsy] = NOSE.at(t);
        let [tongue_sy, tongue_op] = BLEP.at(t);
        Pose {
            tilt,
            head_scale,
            head_op,
            ear_l_rot,
            ear_l_scale,
            ear_l_op,
            ear_r_rot,
            ear_r_scale,
            ear_r_op,
            eyes_x,
            eyes_sy,
            nose_t: Vec2::new(nx, ny),
            nose_s: Vec2::new(nsx, nsy),
            tongue_sy,
            tongue_op,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn close(a: f32, b: f32) -> bool {
        (a - b).abs() < 1e-3
    }

    #[test]
    fn keyframes_hold_their_ends_and_ease_between() {
        // Before its delay a track holds its first keyframe, after its end the last.
        assert!(close(HEAD_SCALE.at(0.0)[0], 0.12));
        assert!(close(HEAD_SCALE.at(10.0)[0], 1.0));
        // Halfway through a symmetric ease-in-out segment is halfway between its values.
        let mid = (47.22 + 54.17) / 2.0 / 100.0 * INTRO;
        assert!(close(TILT.at(mid)[0], -6.0));
        let mid = (54.17 + 56.94) / 2.0 / 100.0 * INTRO;
        assert!(close(TILT.at(mid)[0], 0.0));
        // The spring overshoots.
        assert!((0..=100).any(|i| HEAD_SCALE.at(0.35 + 0.6 * i as f32 / 100.0)[0] > 1.0));
    }

    #[test]
    fn the_intro_ends_on_the_static_pose() {
        let rest = Pose::at(INTRO);
        assert_eq!(rest, Pose::at(INTRO + 100.0));
        assert!(close(rest.tilt, 0.0) && close(rest.head_scale, 1.0) && close(rest.head_op, 1.0));
        assert!(close(rest.ear_l_rot, 0.0) && close(rest.ear_l_scale, 1.0) && close(rest.ear_l_op, 1.0));
        assert!(close(rest.ear_r_rot, 0.0) && close(rest.ear_r_scale, 1.0) && close(rest.ear_r_op, 1.0));
        assert!(close(rest.eyes_x, 0.0) && close(rest.eyes_sy, 1.0));
        assert_eq!(rest.nose_t, Vec2::ZERO);
        assert_eq!(rest.nose_s, Vec2::splat(1.0));
        assert!(close(rest.tongue_op, 0.0));
    }

    #[test]
    fn the_blep_starts_from_the_resting_pose() {
        let (start, rest) = (Pose::at(BLEP_START), Pose::at(INTRO));
        for (a, b) in [
            (start.tilt, rest.tilt),
            (start.head_scale, rest.head_scale),
            (start.ear_l_rot, rest.ear_l_rot),
            (start.ear_r_rot, rest.ear_r_rot),
            (start.eyes_x, rest.eyes_x),
            (start.eyes_sy, rest.eyes_sy),
            (start.nose_t.y, rest.nose_t.y),
            (start.tongue_op, rest.tongue_op),
        ] {
            assert!(close(a, b), "{a} != {b}");
        }
        // And the tongue does come out, with the head wiggling both ways.
        let blep: Vec<Pose> = (0..=70).map(|i| Pose::at(BLEP_START + 0.01 * i as f32)).collect();
        assert!(blep.iter().any(|p| p.tongue_op > 0.9));
        assert!(blep.iter().any(|p| p.tilt < -5.0) && blep.iter().any(|p| p.tilt > 5.0));
    }

    #[test]
    fn transforms_compose_like_css() {
        let m = Affine::about(Pos2::new(10.0, 10.0), Vec2::new(1.0, 0.0), 90.0, Vec2::splat(2.0));
        let q = m.apply(Pos2::new(11.0, 10.0));
        assert!(close(q.x, 11.0) && close(q.y, 12.0));
        let back = m.inverse().apply(q);
        assert!(close(back.x, 11.0) && close(back.y, 10.0));
    }

    #[test]
    fn concave_outlines_triangulate_fully() {
        let area = |pts: &[Pos2], tris: &[[u32; 3]]| -> f32 {
            tris.iter().map(|t| cross(pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]).abs() / 2.0).sum()
        };
        let polygon_area = |pts: &[Pos2]| -> f32 {
            (0..pts.len()).map(|i| cross(Pos2::ZERO, pts[i], pts[(i + 1) % pts.len()])).sum::<f32>().abs() / 2.0
        };
        let head = head_outline();
        assert!(close(area(&head, &triangulate(&head)) / polygon_area(&head), 1.0));
        let blaze = clip_convex(&blaze_outline(), &convex_hull(&head));
        assert!(close(area(&blaze, &triangulate(&blaze)) / polygon_area(&blaze), 1.0));
        // Clipping trims the blaze to the head: nothing above its brow or below its chin.
        assert!(blaze.iter().all(|q| q.y >= 41.9 && q.y <= 107.1));
    }
}
