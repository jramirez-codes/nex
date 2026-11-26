//! Laying out and painting listing items (rows or tiles) and their icons, and the drag ghost.

use egui::{Align2, Color32, DragAndDrop, FontId, Id, LayerId, Order, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::ui::file_icons::{self, LINK_MARK};
use crate::ui::theme::ACCENT;
use crate::ui::theme::Theme;
use crate::ui::{icons, paint};

use crate::file_explorer::fs::ops::human_size;
use crate::file_explorer::input::{ExplorerDrag, Ghost};
use crate::file_explorer::{Entry, View};

/// Sizes of the explorer's parts; touch screens get larger hit targets.
pub(in crate::file_explorer) struct Sizes {
    pub(in crate::file_explorer) bar: f32,
    pub(in crate::file_explorer) row: f32,
    pub(in crate::file_explorer) tile: Vec2,
    pub(in crate::file_explorer) icon: f32,
    pub(in crate::file_explorer) action_bar: f32,
    pub(in crate::file_explorer) button: f32,
    pub(in crate::file_explorer) font: f32,
    /// The zoom level, for paddings and small marks that aren't listed above.
    pub(in crate::file_explorer) scale: f32,
}

/// Sizes at zoom level `zoom` (1.0 = normal).
pub(in crate::file_explorer) fn sizes(touch: bool, zoom: f32) -> Sizes {
    let (bar, row, tile, icon, action_bar, button, font) = if touch {
        (40.0, 40.0, Vec2::new(112.0, 104.0), 44.0, 52.0, 38.0, 14.0)
    } else {
        (30.0, 24.0, Vec2::new(96.0, 86.0), 36.0, 38.0, 26.0, 13.0)
    };
    let z = zoom;
    Sizes { bar: bar * z, row: row * z, tile: tile * z, icon: icon * z, action_bar: action_bar * z, button: button * z, font: font * z, scale: z }
}

/// Where items are laid out in the listing.
#[derive(Clone, Copy)]
pub(in crate::file_explorer) struct Layout {
    pub(in crate::file_explorer) list: Rect,
    pub(in crate::file_explorer) cols: usize,
    pub(in crate::file_explorer) item: Vec2,
}

impl Layout {
    pub(in crate::file_explorer) fn item_rect(&self, i: usize, scroll: f32) -> Rect {
        let (row, col) = (i / self.cols, i % self.cols);
        let min = self.list.min + Vec2::new(col as f32 * self.item.x, row as f32 * self.item.y - scroll);
        Rect::from_min_size(min, self.item)
    }

    pub(in crate::file_explorer) fn item_at(&self, pos: Pos2, scroll: f32, len: usize) -> Option<usize> {
        if !self.list.contains(pos) {
            return None;
        }
        let local = pos - self.list.min + Vec2::new(0.0, scroll);
        let col = (local.x / self.item.x).floor() as usize;
        let row = (local.y / self.item.y).floor() as usize;
        let i = row * self.cols + col;
        (col < self.cols && i < len).then_some(i)
    }

    pub(in crate::file_explorer) fn rows(&self, len: usize) -> usize {
        len.div_ceil(self.cols)
    }
}

/// Draw the dragged files' icon under the pointer.
pub fn paint_drag_ghost(ctx: &egui::Context, c: &Theme) {
    let Some(drag) = DragAndDrop::payload::<ExplorerDrag>(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let copy = ctx.input(|i| crate::platform::SHORTCUTS.copy_drag(&i.modifiers));
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("file-drag-ghost")));
    let g = &drag.ghost;
    // A finger would cover an icon held under it: on touch screens it floats above instead.
    let origin = if ctx.input(|i| i.any_touches()) { pos - Vec2::new(g.size.x / 2.0, g.size.y + 36.0) } else { pos - g.grab };
    paint_ghost(&p, origin, g, c, copy);
}

/// A picture of the dragged files' icon, for the compositor to show once the drag leaves the
/// window, with the pointer where it holds the icon in the window.
pub fn drag_image(ctx: &egui::Context, c: &Theme, drag: &ExplorerDrag) -> Option<crate::platform::DragImage> {
    let layer = LayerId::new(Order::Background, Id::new("file-drag-image"));
    // Painted around (0, 0) = the pointer, then taken back out before the frame is drawn.
    paint_ghost(&Painter::new(ctx.clone(), layer, Rect::EVERYTHING), Pos2::ZERO - drag.ghost.grab, &drag.ghost, c, false);
    let shapes: Vec<_> = ctx.graphics_mut(|g| std::mem::take(g.entry(layer))).all_entries().cloned().collect();
    let bounds = shapes.iter().map(|s| s.shape.visual_bounding_rect()).fold(Rect::NOTHING, |a, b| a.union(b));
    if !bounds.is_positive() {
        return None;
    }
    // Points → the window's logical pixels (zoom) → buffer pixels (the output's scale).
    let zoom = ctx.zoom_factor();
    let native = ctx.input(|i| i.viewport().native_pixels_per_point).unwrap_or(ctx.pixels_per_point() / zoom);
    let scale = native.ceil().max(1.0);
    let min = (bounds.min.to_vec2() * zoom).floor() - Vec2::splat(1.0);
    let max = (bounds.max.to_vec2() * zoom).ceil() + Vec2::splat(1.0);
    let size = [((max.x - min.x) * scale) as usize, ((max.y - min.y) * scale) as usize];
    let pixels = crate::ui::raster::render(ctx, shapes, (min / zoom).to_pos2(), zoom * scale, size);
    Some(crate::platform::DragImage { pixels, size, scale: scale as i32, hotspot: [-min.x as i32, -min.y as i32] })
}

/// The dragged icon with its top-left at `origin`: a second one peeking out behind it and a
/// count when several files go, and a plus when they'll be copied.
fn paint_ghost(p: &Painter, origin: Pos2, g: &Ghost, c: &Theme, copy: bool) {
    let k = g.scale;
    let r = Rect::from_min_size(origin, g.size);
    if g.count > 1 {
        let behind = (g.size.y * 0.12).max(3.0 * k);
        paint_icon(p, r.translate(Vec2::new(behind, -behind)), &g.lead, 0.55, c, g.large, g.in_home);
    }
    paint_icon(p, r, &g.lead, 1.0, c, g.large, g.in_home);
    let rad = (g.size.y * 0.2).clamp(7.0 * k, 10.0 * k);
    if g.count > 1 {
        let text = if g.count > 99 { "99+".to_string() } else { g.count.to_string() };
        let galley = p.layout_no_wrap(text, paint::bold(rad * 1.15), Color32::WHITE);
        let w = (galley.size().x + rad).max(rad * 2.0);
        let badge = Rect::from_center_size(r.right_top(), Vec2::new(w, rad * 2.0));
        p.rect_filled(badge, rad, ACCENT);
        p.galley(badge.center() - galley.size() / 2.0, galley, Color32::WHITE);
    }
    if copy {
        let at = r.right_bottom();
        p.circle_filled(at, rad, ACCENT);
        icons::icon_plus(p, Rect::from_center_size(at, Vec2::splat(rad * 1.5)), Color32::WHITE);
    }
}

pub(in crate::file_explorer) struct ItemState {
    pub(in crate::file_explorer) marked: bool,
    pub(in crate::file_explorer) cursor: bool,
    pub(in crate::file_explorer) hovered: bool,
    pub(in crate::file_explorer) drop: bool,
    pub(in crate::file_explorer) dimmed: bool,
    pub(in crate::file_explorer) selecting: bool,
    /// The listing is the home folder (Desktop, Downloads… get their marks).
    pub(in crate::file_explorer) in_home: bool,
    /// The name is being edited: the rename field is drawn there instead.
    pub(in crate::file_explorer) hide_name: bool,
}

/// Draw `e`'s icon in `r`: a folder (with a mark on well-known ones), a colored glyph for a
/// known kind of file, or a plain page. `large` (grid tiles) draws glyphs on a page and
/// prints unknown files' extensions; symlinks get a small link badge either way.
pub(in crate::file_explorer) fn paint_icon(p: &Painter, r: Rect, e: &Entry, fade: f32, c: &Theme, large: bool, in_home: bool) {
    let glyph = (!e.is_dir).then(|| file_icons::file_glyph(&e.name, e.executable)).flatten();
    if e.is_dir {
        icons::icon_folder(p, r, ACCENT.gamma_multiply(fade));
        if let Some(mark) = file_icons::folder_mark(&e.name, in_home).filter(|_| r.height() >= 20.0) {
            let body = Pos2::new(r.center().x, r.top() + r.height() * 0.6);
            file_icons::paint_glyph(p, body, r.height() * 0.42, mark, Color32::from_black_alpha(150).gamma_multiply(fade));
        }
    } else if let Some((ch, color)) = glyph {
        if large {
            icons::icon_file(p, r, c.text_dim.gamma_multiply(0.6 * fade));
            file_icons::paint_glyph(p, r.center() + Vec2::new(0.0, r.height() * 0.08), r.height() * 0.5, ch, color.gamma_multiply(fade));
        } else {
            file_icons::paint_glyph(p, r.center(), r.height() * 1.15, ch, color.gamma_multiply(fade));
        }
    } else {
        icons::icon_file(p, r, c.text_dim.gamma_multiply(fade));
        // The extension, printed on the page.
        let ext = e.path.extension().map(|x| x.to_string_lossy().to_uppercase()).filter(|x| x.chars().count() <= 4);
        if let (true, Some(ext)) = (large, ext) {
            let at = r.center() + Vec2::new(0.0, r.height() * 0.18);
            p.text(at, Align2::CENTER_CENTER, ext, paint::bold(r.height() * 0.22), c.text_dim.gamma_multiply(fade));
        }
    }
    if e.is_symlink {
        let rad = (r.height() * 0.22).max(5.0);
        let at = Pos2::new(r.left() + rad * 0.6, r.bottom() - rad * 0.6);
        p.circle_filled(at, rad, c.card);
        p.circle_stroke(at, rad, Stroke::new(1.0_f32, c.text_dim.gamma_multiply(0.6 * fade)));
        file_icons::paint_glyph(p, at, rad * 1.5, LINK_MARK, c.text_bright.gamma_multiply(fade));
    }
}

/// Where a row's or tile's name is drawn, for the rename field on top of it.
pub(in crate::file_explorer) fn name_rect(view: View, item: Rect, s: &Sizes, selecting: bool) -> Rect {
    let k = s.scale;
    match view {
        View::List => {
            let glyph_h = (s.row * 0.55).min(18.0 * k);
            let left = item.left() + 8.0 * k + if selecting { 22.0 * k } else { 0.0 } + 8.0 * k + glyph_h * 0.575 + 10.0 * k;
            Rect::from_min_max(Pos2::new(left - 5.0 * k, item.top() + 2.0 * k), Pos2::new(item.right() - 6.0 * k, item.bottom() - 2.0 * k))
        }
        View::Grid => {
            let tile = item.shrink(4.0 * k);
            let y = tile.bottom() - 6.0 * k - s.font * 0.6;
            let w = tile.width().max(170.0 * k);
            Rect::from_center_size(Pos2::new(tile.center().x, y), Vec2::new(w, s.font * 1.7))
        }
    }
}

pub(in crate::file_explorer) fn item_background(p: &Painter, r: Rect, st: &ItemState) {
    if st.drop {
        p.rect_filled(r, 8, ACCENT.gamma_multiply(0.22));
        p.rect_stroke(r, 8, Stroke::new(1.5_f32, ACCENT), StrokeKind::Inside);
    } else if st.marked {
        p.rect_filled(r, 8, ACCENT.gamma_multiply(0.25));
    } else if st.cursor {
        p.rect_filled(r, 8, Color32::from_white_alpha(26));
    } else if st.hovered {
        p.rect_filled(r, 8, Color32::from_white_alpha(10));
    }
}

/// Where a row's icon is drawn.
pub(in crate::file_explorer) fn row_icon_rect(row: Rect, s: &Sizes, selecting: bool) -> Rect {
    let k = s.scale;
    let left = row.left() + 8.0 * k + if selecting { 22.0 * k } else { 0.0 };
    let glyph_h = (s.row * 0.55).min(18.0 * k);
    Rect::from_center_size(Pos2::new(left + 8.0 * k, row.center().y), Vec2::new(glyph_h * 1.15, glyph_h))
}

/// Where a grid tile's icon is drawn.
pub(in crate::file_explorer) fn tile_icon_rect(cell: Rect, s: &Sizes, is_dir: bool) -> Rect {
    let tile = cell.shrink(4.0 * s.scale);
    let center = Pos2::new(tile.center().x, tile.top() + 8.0 * s.scale + s.icon / 2.0);
    let size = if is_dir { Vec2::new(s.icon * 1.12, s.icon * 0.86) } else { Vec2::splat(s.icon) };
    Rect::from_center_size(center, size)
}

pub(in crate::file_explorer) fn paint_row(p: &Painter, row: Rect, e: &Entry, st: &ItemState, c: &Theme, s: &Sizes) {
    let k = s.scale;
    item_background(p, row.shrink2(Vec2::new(0.0, 1.0)), st);
    if st.selecting {
        let r = Rect::from_center_size(Pos2::new(row.left() + 16.0 * k, row.center().y), Vec2::splat(16.0 * k));
        icons::icon_check(p, r, if st.marked { ACCENT } else { c.text_dim }, st.marked);
    }
    let fade = if st.dimmed { 0.45 } else { 1.0 };
    let glyph = row_icon_rect(row, s, st.selecting);
    paint_icon(p, glyph, e, fade, c, false, st.in_home);
    let size = e.size.map(human_size).unwrap_or_default();
    let size_galley = p.layout_no_wrap(size, FontId::proportional(s.font - 1.5), c.text_dim);
    let size_x = row.right() - 10.0 * k - size_galley.size().x;
    let name_left = glyph.right() + 10.0 * k;
    let name_color = if st.marked || st.cursor { c.text_bright } else { c.text };
    if st.hide_name {
        return;
    }
    paint::faded_text(
        p,
        Pos2::new(name_left, row.center().y),
        &e.name,
        FontId::proportional(s.font),
        name_color.gamma_multiply(fade),
        size_x - 12.0 * k - name_left,
        20.0 * k,
    );
    p.galley(Pos2::new(size_x, row.center().y - size_galley.size().y / 2.0), size_galley, c.text_dim);
}

pub(in crate::file_explorer) fn paint_tile(p: &Painter, cell: Rect, e: &Entry, st: &ItemState, c: &Theme, s: &Sizes) {
    let k = s.scale;
    let tile = cell.shrink(4.0 * k);
    item_background(p, tile, st);
    let fade = if st.dimmed { 0.45 } else { 1.0 };
    paint_icon(p, tile_icon_rect(cell, s, e.is_dir), e, fade, c, true, st.in_home);
    if st.selecting {
        let r = Rect::from_center_size(tile.left_top() + Vec2::splat(13.0 * k), Vec2::splat(16.0 * k));
        icons::icon_check(p, r, if st.marked { ACCENT } else { c.text_dim }, st.marked);
    }
    // Name, centered under the icon (faded at the edge when too long).
    if st.hide_name {
        return;
    }
    let font = FontId::proportional(s.font - 1.0);
    let name_color = (if st.marked || st.cursor { c.text_bright } else { c.text }).gamma_multiply(fade);
    let y = tile.bottom() - 6.0 * k - s.font * 0.6;
    let max_w = tile.width() - 10.0 * k;
    let w = p.layout_no_wrap(e.name.clone(), font.clone(), name_color).size().x;
    let left = if w <= max_w { tile.center().x - w / 2.0 } else { tile.left() + 5.0 * k };
    paint::faded_text(p, Pos2::new(left, y), &e.name, font, name_color, max_w, 16.0 * k);
}
