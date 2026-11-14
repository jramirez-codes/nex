//! A per-character glyph cache over egui's font atlas, and the per-pane mesh built from it.
//!
//! Laying out text is the slow part of drawing a terminal with egui: whenever a row's text
//! changes, every run on it is shaped from scratch. A terminal doesn't need shaping — every
//! glyph sits in a known cell — so each character is laid out once to learn where its
//! bitmap lives in egui's font atlas, and from then on drawing it is one textured quad.
//! Backgrounds and glyphs for a whole pane go into a single [`Mesh`], which egui uploads
//! as-is (no tessellation).

use std::collections::HashMap;

use alacritty_terminal::term::cell::Flags;
use egui::epaint::WHITE_UV;
use egui::{Color32, Context, Mesh, Painter, Pos2, Rect, TextureId, Vec2};

use crate::render::{CellMetrics, font_for};

/// Where a glyph's bitmap is drawn relative to its cell, and where it sits in the atlas.
#[derive(Clone, Copy)]
struct Glyph {
    /// Top-left of the bitmap relative to the cell's top-left, before centering.
    offset: Vec2,
    size: Vec2,
    /// Texel rectangle in the font atlas.
    uv_min: [u16; 2],
    uv_max: [u16; 2],
    /// Laid-out width, used to center non-ASCII glyphs in their cells.
    advance: f32,
}

/// A looked-up glyph: `None` for characters with nothing to draw.
type Slot = Option<Glyph>;

/// The glyphs of one font size. ASCII (nearly every cell) is a plain table lookup; anything
/// else goes through a map.
struct SizeGlyphs {
    /// Indexed by style (bold = 1, italic = 2) then character; `None` = not looked up yet.
    ascii: [[Option<Slot>; 128]; 4],
    other: HashMap<(char, usize), Slot>,
}

impl SizeGlyphs {
    fn new() -> Box<Self> {
        Box::new(Self { ascii: [[None; 128]; 4], other: HashMap::new() })
    }
}

fn style_index(style: Flags) -> usize {
    usize::from(style.contains(Flags::BOLD)) | usize::from(style.contains(Flags::ITALIC)) << 1
}

#[derive(Default)]
pub struct GlyphCache {
    /// Keyed by font size bits (panes can be zoomed to different sizes).
    sizes: HashMap<u32, Box<SizeGlyphs>>,
    ppp: f32,
    /// Atlas fill ratio last seen. egui throws the atlas away when it gets full (or the
    /// texture limits change), and a drop in the fill ratio is how that shows up here.
    atlas_fill: f32,
    /// Atlas size the current frame's meshes were built against.
    atlas_size: [usize; 2],
    /// Bumped whenever cached glyphs are thrown away.
    epoch: u64,
}

impl GlyphCache {
    /// Call once per frame before painting terminals.
    pub fn begin_frame(&mut self, ctx: &Context) {
        let ppp = ctx.pixels_per_point();
        let (fill, size) = ctx.fonts(|f| (f.font_atlas_fill_ratio(), f.font_image_size()));
        if ppp != self.ppp || fill < self.atlas_fill {
            self.sizes.clear();
            self.epoch += 1;
        }
        self.ppp = ppp;
        self.atlas_fill = fill;
        self.atlas_size = size;
    }

    /// Identifies the atlas layout that texture coordinates built now refer to. Meshes built
    /// under a different value must be rebuilt.
    pub fn epoch(&self) -> (u64, [usize; 2]) {
        (self.epoch, self.atlas_size)
    }

    /// Call once per frame after painting. If laying out new text grew the atlas after some
    /// meshes were built, their texture coordinates are off for this frame: draw another.
    pub fn end_frame(&mut self, ctx: &Context) {
        if ctx.fonts(|f| f.font_image_size()) != self.atlas_size {
            ctx.request_repaint();
        }
    }
}

/// Lay out `c` alone to find its bitmap in the atlas (rasterizing it there if needed).
fn look_up(painter: &Painter, c: char, style: Flags, font_size: f32) -> Slot {
    let galley = painter.layout_no_wrap(c.to_string(), font_for(style, font_size), Color32::WHITE);
    let row = galley.rows.first()?;
    let g = row.glyphs.first()?;
    (!g.uv_rect.is_nothing()).then(|| Glyph {
        offset: row.pos.to_vec2() + g.pos.to_vec2() + g.uv_rect.offset,
        size: g.uv_rect.size,
        uv_min: g.uv_rect.min,
        uv_max: g.uv_rect.max,
        advance: galley.size().x,
    })
}

/// The quads for one pane: cell backgrounds first, then glyphs on top.
pub struct GridMesh<'a> {
    glyphs_of_size: &'a mut SizeGlyphs,
    atlas_fill: &'a mut f32,
    painter: &'a Painter,
    ppp: f32,
    /// 1 / atlas size, to turn texel coordinates into UVs.
    uv_scale: Vec2,
    backgrounds: Mesh,
    glyphs: Mesh,
}

impl<'a> GridMesh<'a> {
    /// A mesh for one pane at `font_size`, with room for about `cells` glyphs.
    pub fn new(cache: &'a mut GlyphCache, painter: &'a Painter, font_size: f32, cells: usize) -> Self {
        let GlyphCache { sizes, ppp, atlas_fill, atlas_size, .. } = cache;
        let mesh = |quads: usize| {
            let mut m = Mesh::with_texture(TextureId::default());
            m.vertices.reserve(quads * 4);
            m.indices.reserve(quads * 6);
            m
        };
        Self {
            glyphs_of_size: sizes.entry(font_size.to_bits()).or_insert_with(SizeGlyphs::new),
            atlas_fill,
            painter,
            ppp: *ppp,
            uv_scale: Vec2::new(1.0 / atlas_size[0] as f32, 1.0 / atlas_size[1] as f32),
            backgrounds: mesh(64),
            glyphs: mesh(cells),
        }
    }

    pub fn background(&mut self, rect: Rect, color: Color32) {
        self.backgrounds.add_rect_with_uv(rect, Rect::from_min_max(WHITE_UV, WHITE_UV), color);
    }

    fn glyph_for(&mut self, c: char, style: Flags, font_size: f32) -> Slot {
        let s = style_index(style);
        let table = &mut *self.glyphs_of_size;
        let cached = if c.is_ascii() { table.ascii[s][c as usize] } else { table.other.get(&(c, s)).copied() };
        if let Some(glyph) = cached {
            return glyph;
        }
        let glyph = look_up(self.painter, c, style, font_size);
        if c.is_ascii() {
            table.ascii[s][c as usize] = Some(glyph);
        } else {
            table.other.insert((c, s), glyph);
        }
        // Count this glyph's own atlas space, so it isn't mistaken for a reset next frame.
        *self.atlas_fill = self.painter.ctx().fonts(|f| f.font_atlas_fill_ratio());
        glyph
    }

    /// Draw `c` in the cell `cell` (one or two columns wide), if it has a bitmap.
    pub fn glyph(&mut self, cell: Rect, m: CellMetrics, c: char, style: Flags, fg: Color32) {
        let Some(g) = self.glyph_for(c, style, m.font_size) else { return };
        // ASCII sits at the cell's left edge like the font intends; other glyphs (symbols,
        // CJK, icons) are centered in their cells, but never shifted by more than half a cell.
        let shift = if c.is_ascii() { 0.0 } else { ((cell.width() - g.advance) / 2.0).clamp(0.0, m.width * 0.5) };
        let ppp = self.ppp;
        let snap = |v: f32| (v * ppp).round() / ppp;
        let min = cell.min + Vec2::new(shift, m.baseline_offset) + g.offset;
        let min = Pos2::new(snap(min.x), snap(min.y));
        let texel = |[x, y]: [u16; 2]| Pos2::new(x as f32 * self.uv_scale.x, y as f32 * self.uv_scale.y);
        let uv = Rect::from_min_max(texel(g.uv_min), texel(g.uv_max));
        self.glyphs.add_rect_with_uv(Rect::from_min_size(min, g.size), uv, fg);
    }

    /// The two meshes as shapes, in paint order. Background alpha follows window opacity.
    pub fn into_shapes(mut self, bg_alpha: f32) -> [egui::Shape; 2] {
        let alpha = bg_alpha.max(0.85);
        for v in &mut self.backgrounds.vertices {
            v.color = v.color.gamma_multiply(alpha);
        }
        [egui::Shape::mesh(self.backgrounds), egui::Shape::mesh(self.glyphs)]
    }
}
