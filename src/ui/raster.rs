//! A tiny software rasterizer for egui shapes, for pictures that leave the GPU's frame: the
//! drag icon handed to the compositor when files are dragged out of the window.

use egui::epaint::{ClippedShape, Primitive};
use egui::{ColorImage, Context, Pos2, TextureId};

/// Draw `shapes` (in points) into a `size` pixel image whose top-left corner is `origin`, at
/// `px_per_point` pixels per point. Returns premultiplied RGBA, row by row.
pub fn render(ctx: &Context, shapes: Vec<ClippedShape>, origin: Pos2, px_per_point: f32, size: [usize; 2]) -> Vec<[u8; 4]> {
    let [w, h] = size;
    let mut out = vec![[0u8; 4]; w * h];
    let atlas = ctx.fonts(|f| f.image());
    let to_px = |p: Pos2| ((p - origin) * px_per_point).to_pos2();
    for prim in ctx.tessellate(shapes, px_per_point) {
        let Primitive::Mesh(mesh) = prim.primitive else { continue };
        let textured = mesh.texture_id == TextureId::default();
        let clip_min = to_px(prim.clip_rect.min);
        let clip_max = to_px(prim.clip_rect.max);
        let (cx0, cy0) = (clip_min.x.max(0.0), clip_min.y.max(0.0));
        let (cx1, cy1) = (clip_max.x.min(w as f32), clip_max.y.min(h as f32));
        for tri in mesh.indices.as_chunks::<3>().0 {
            let v = [&mesh.vertices[tri[0] as usize], &mesh.vertices[tri[1] as usize], &mesh.vertices[tri[2] as usize]];
            let p = v.map(|v| to_px(v.pos));
            let area = edge(p[0], p[1], p[2]);
            if area.abs() < 1e-6 {
                continue;
            }
            let x0 = p.iter().map(|p| p.x).fold(f32::MAX, f32::min).max(cx0).floor() as usize;
            let y0 = p.iter().map(|p| p.y).fold(f32::MAX, f32::min).max(cy0).floor() as usize;
            let x1 = p.iter().map(|p| p.x).fold(f32::MIN, f32::max).min(cx1).ceil() as usize;
            let y1 = p.iter().map(|p| p.y).fold(f32::MIN, f32::max).min(cy1).ceil() as usize;
            for y in y0..y1.min(h) {
                for x in x0..x1.min(w) {
                    let c = Pos2::new(x as f32 + 0.5, y as f32 + 0.5);
                    let b = [edge(p[1], p[2], c) / area, edge(p[2], p[0], c) / area, edge(p[0], p[1], c) / area];
                    if b.iter().any(|&b| b < 0.0) {
                        continue;
                    }
                    let mut src = [0.0f32; 4];
                    for (k, vert) in v.iter().enumerate() {
                        let col = vert.color.to_array();
                        for ch in 0..4 {
                            src[ch] += b[k] * col[ch] as f32;
                        }
                    }
                    if textured {
                        let uv =
                            Pos2::new(b[0] * v[0].uv.x + b[1] * v[1].uv.x + b[2] * v[2].uv.x, b[0] * v[0].uv.y + b[1] * v[1].uv.y + b[2] * v[2].uv.y);
                        let t = sample(&atlas, uv);
                        for ch in 0..4 {
                            src[ch] *= t[ch] / 255.0;
                        }
                    }
                    let dst = &mut out[y * w + x];
                    let keep = 1.0 - src[3] / 255.0;
                    for ch in 0..4 {
                        dst[ch] = (src[ch] + dst[ch] as f32 * keep).round().clamp(0.0, 255.0) as u8;
                    }
                }
            }
        }
    }
    out
}

/// Twice the signed area of the triangle `a b c`.
fn edge(a: Pos2, b: Pos2, c: Pos2) -> f32 {
    (b.x - a.x) * (c.y - a.y) - (b.y - a.y) * (c.x - a.x)
}

/// Bilinear sample of `img` at normalized `uv`.
fn sample(img: &ColorImage, uv: Pos2) -> [f32; 4] {
    let [w, h] = img.size;
    let x = (uv.x * w as f32 - 0.5).clamp(0.0, (w - 1) as f32);
    let y = (uv.y * h as f32 - 0.5).clamp(0.0, (h - 1) as f32);
    let (x0, y0) = (x.floor() as usize, y.floor() as usize);
    let (x1, y1) = ((x0 + 1).min(w - 1), (y0 + 1).min(h - 1));
    let (fx, fy) = (x - x0 as f32, y - y0 as f32);
    let px = |x: usize, y: usize| -> [u8; 4] { img.pixels[y * w + x].to_array() };
    let (a, b, c, d) = (px(x0, y0), px(x1, y0), px(x0, y1), px(x1, y1));
    std::array::from_fn(|ch| {
        let top = a[ch] as f32 * (1.0 - fx) + b[ch] as f32 * fx;
        let bottom = c[ch] as f32 * (1.0 - fx) + d[ch] as f32 * fx;
        top * (1.0 - fy) + bottom * fy
    })
}
