//! Laying out and painting listing items (rows or tiles), and the drag ghost.

use egui::{Align2, Color32, DragAndDrop, FontId, Id, LayerId, Order, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::ui::theme::ACCENT;
use crate::ui::theme::Theme;
use crate::ui::{icons, paint};

use super::Entry;
use super::fs_ops::{count, human_size};
use super::input::ExplorerDrag;

/// Sizes of the explorer's parts; touch screens get larger hit targets.
pub(super) struct Sizes {
    pub(super) bar: f32,
    pub(super) row: f32,
    pub(super) tile: Vec2,
    pub(super) icon: f32,
    pub(super) action_bar: f32,
    pub(super) button: f32,
    pub(super) font: f32,
    /// The zoom level, for paddings and small marks that aren't listed above.
    pub(super) scale: f32,
}

/// Sizes at zoom level `zoom` (1.0 = normal).
pub(super) fn sizes(touch: bool, zoom: f32) -> Sizes {
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
pub(super) struct Layout {
    pub(super) list: Rect,
    pub(super) cols: usize,
    pub(super) item: Vec2,
}

impl Layout {
    pub(super) fn item_rect(&self, i: usize, scroll: f32) -> Rect {
        let (row, col) = (i / self.cols, i % self.cols);
        let min = self.list.min + Vec2::new(col as f32 * self.item.x, row as f32 * self.item.y - scroll);
        Rect::from_min_size(min, self.item)
    }

    pub(super) fn item_at(&self, pos: Pos2, scroll: f32, len: usize) -> Option<usize> {
        if !self.list.contains(pos) {
            return None;
        }
        let local = pos - self.list.min + Vec2::new(0.0, scroll);
        let col = (local.x / self.item.x).floor() as usize;
        let row = (local.y / self.item.y).floor() as usize;
        let i = row * self.cols + col;
        (col < self.cols && i < len).then_some(i)
    }

    pub(super) fn rows(&self, len: usize) -> usize {
        len.div_ceil(self.cols)
    }
}

/// Draw the floating label that follows the pointer while files are dragged.
pub fn paint_drag_ghost(ctx: &egui::Context, c: &Theme) {
    let Some(drag) = DragAndDrop::payload::<ExplorerDrag>(ctx) else { return };
    let Some(pos) = ctx.input(|i| i.pointer.interact_pos()) else { return };
    let copy = ctx.input(|i| i.modifiers.ctrl);
    let p = ctx.layer_painter(LayerId::new(Order::Tooltip, Id::new("file-drag-ghost")));
    let text = match drag.paths.as_slice() {
        [one] => one.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default(),
        many => count(many.len()),
    };
    let galley = p.layout_no_wrap(text, FontId::proportional(13.0), c.text_bright);
    let size = Vec2::new(galley.size().x + 40.0, 28.0);
    let r = Rect::from_min_size(pos + Vec2::new(14.0, 10.0), size);
    p.rect_filled(r, 8, Color32::from_rgba_unmultiplied(30, 32, 40, 235));
    p.rect_stroke(r, 8, Stroke::new(1.0, ACCENT.gamma_multiply(0.6)), StrokeKind::Inside);
    let icon = Rect::from_center_size(Pos2::new(r.left() + 15.0, r.center().y), Vec2::new(13.0, 13.0));
    if drag.paths.len() == 1 && drag.paths[0].is_dir() {
        icons::icon_folder(&p, icon, ACCENT);
    } else {
        icons::icon_file(&p, icon, c.text);
    }
    p.galley(Pos2::new(r.left() + 28.0, r.center().y - galley.size().y / 2.0), galley, c.text_bright);
    if copy {
        let badge = Pos2::new(r.right(), r.top());
        p.circle_filled(badge, 8.0, ACCENT);
        icons::icon_plus(&p, Rect::from_center_size(badge, Vec2::splat(12.0)), Color32::WHITE);
    }
}

pub(super) struct ItemState {
    pub(super) marked: bool,
    pub(super) cursor: bool,
    pub(super) hovered: bool,
    pub(super) drop: bool,
    pub(super) dimmed: bool,
    pub(super) selecting: bool,
}

pub(super) fn item_background(p: &Painter, r: Rect, st: &ItemState) {
    if st.drop {
        p.rect_filled(r, 8, ACCENT.gamma_multiply(0.22));
        p.rect_stroke(r, 8, Stroke::new(1.5, ACCENT), StrokeKind::Inside);
    } else if st.marked {
        p.rect_filled(r, 8, ACCENT.gamma_multiply(0.25));
    } else if st.cursor {
        p.rect_filled(r, 8, Color32::from_white_alpha(26));
    } else if st.hovered {
        p.rect_filled(r, 8, Color32::from_white_alpha(10));
    }
}

pub(super) fn paint_row(p: &Painter, row: Rect, e: &Entry, st: &ItemState, c: &Theme, s: &Sizes) {
    let k = s.scale;
    item_background(p, row.shrink2(Vec2::new(0.0, 1.0)), st);
    let mut left = row.left() + 8.0 * k;
    if st.selecting {
        let r = Rect::from_center_size(Pos2::new(left + 8.0 * k, row.center().y), Vec2::splat(16.0 * k));
        icons::icon_check(p, r, if st.marked { ACCENT } else { c.text_dim }, st.marked);
        left += 22.0 * k;
    }
    let fade = if st.dimmed { 0.45 } else { 1.0 };
    let glyph_h = (s.row * 0.55).min(18.0 * k);
    let glyph = Rect::from_center_size(Pos2::new(left + 8.0 * k, row.center().y), Vec2::new(glyph_h * 1.15, glyph_h));
    if e.is_dir {
        icons::icon_folder(p, glyph, ACCENT.gamma_multiply(fade));
    } else {
        icons::icon_file(p, glyph, c.text_dim.gamma_multiply(fade));
    }
    let size = e.size.map(human_size).unwrap_or_default();
    let size_galley = p.layout_no_wrap(size, FontId::proportional(s.font - 1.5), c.text_dim);
    let size_x = row.right() - 10.0 * k - size_galley.size().x;
    let name_left = glyph.right() + 10.0 * k;
    let name_color = if st.marked || st.cursor { c.text_bright } else { c.text };
    paint::faded_text(p, Pos2::new(name_left, row.center().y), &e.name, FontId::proportional(s.font), name_color.gamma_multiply(fade), size_x - 12.0 * k - name_left, 20.0 * k);
    p.galley(Pos2::new(size_x, row.center().y - size_galley.size().y / 2.0), size_galley, c.text_dim);
}

pub(super) fn paint_tile(p: &Painter, cell: Rect, e: &Entry, st: &ItemState, c: &Theme, s: &Sizes) {
    let k = s.scale;
    let tile = cell.shrink(4.0 * k);
    item_background(p, tile, st);
    let fade = if st.dimmed { 0.45 } else { 1.0 };
    let icon_center = Pos2::new(tile.center().x, tile.top() + 8.0 * k + s.icon / 2.0);
    if e.is_dir {
        let r = Rect::from_center_size(icon_center, Vec2::new(s.icon * 1.12, s.icon * 0.86));
        icons::icon_folder(p, r, ACCENT.gamma_multiply(fade));
    } else {
        let r = Rect::from_center_size(icon_center, Vec2::new(s.icon, s.icon));
        icons::icon_file(p, r, c.text_dim.gamma_multiply(fade));
        // The extension, printed on the page.
        let ext = e.path.extension().map(|x| x.to_string_lossy().to_uppercase()).filter(|x| x.chars().count() <= 4);
        if let Some(ext) = ext {
            p.text(icon_center + Vec2::new(0.0, s.icon * 0.18), Align2::CENTER_CENTER, ext, paint::bold(s.icon * 0.22), c.text_dim.gamma_multiply(fade));
        }
    }
    if st.selecting {
        let r = Rect::from_center_size(tile.left_top() + Vec2::splat(13.0 * k), Vec2::splat(16.0 * k));
        icons::icon_check(p, r, if st.marked { ACCENT } else { c.text_dim }, st.marked);
    }
    // Name, centered under the icon (faded at the edge when too long).
    let font = FontId::proportional(s.font - 1.0);
    let name_color = (if st.marked || st.cursor { c.text_bright } else { c.text }).gamma_multiply(fade);
    let y = tile.bottom() - 6.0 * k - s.font * 0.6;
    let max_w = tile.width() - 10.0 * k;
    let w = p.layout_no_wrap(e.name.clone(), font.clone(), name_color).size().x;
    let left = if w <= max_w { tile.center().x - w / 2.0 } else { tile.left() + 5.0 * k };
    paint::faded_text(p, Pos2::new(left, y), &e.name, font, name_color, max_w, 16.0 * k);
}
