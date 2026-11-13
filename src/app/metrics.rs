//! Font sizes (per-pane zoom) and the terminal cell geometry they produce.


use egui::{
    FontFamily, FontId, Pos2, Rect,
};

use crate::layout::PaneId;
use crate::render::{CellMetrics, FONT_REGULAR};
use crate::terminal::GridSize;

use super::{App, HEADER_HEIGHT};

impl App {
    /// The terminal content rect inside a pane card (below the header, inside the padding).
    pub(super) fn term_rect_for(&self, card: Rect, ppp: f32) -> Rect {
        let pad = self.cfg.window_padding;
        let top = if self.cfg.pane_headers { card.top() + HEADER_HEIGHT } else { card.top() + pad * 0.5 };
        let r = Rect::from_min_max(
            Pos2::new(card.left() + pad, top + 2.0),
            Pos2::new(card.right() - pad, card.bottom() - pad * 0.6),
        );
        // Snap the origin to the pixel grid so glyphs stay crisp.
        Rect::from_min_size(Pos2::new((r.left() * ppp).round() / ppp, (r.top() * ppp).round() / ppp), r.size())
    }

    pub(super) fn grid_for(&self, card: Rect, ppp: f32, m: CellMetrics) -> GridSize {
        if !card.is_positive() {
            return GridSize { cols: 80, lines: 24 };
        }
        let r = self.term_rect_for(card, ppp);
        GridSize {
            cols: ((r.width() / m.width).floor() as usize).max(2),
            lines: ((r.height() / m.height).floor() as usize).max(1),
        }
    }

    /// Font size of one pane (its own zoom level, or the config default).
    pub(super) fn pane_font(&self, id: PaneId) -> f32 {
        self.pane_fonts.get(&id).copied().unwrap_or(self.cfg.font_size)
    }

    /// Zoom only pane `id`; `None` resets it to the config size.
    pub(super) fn set_pane_font(&mut self, ctx: &egui::Context, id: PaneId, size: Option<f32>) {
        match size.map(|s| s.clamp(6.0, 72.0)) {
            Some(s) if s != self.cfg.font_size => {
                self.pane_fonts.insert(id, s);
            }
            _ => {
                self.pane_fonts.remove(&id);
            }
        }
        self.update_metrics(ctx);
    }

    pub(super) fn compute_metrics(&self, ctx: &egui::Context, font_size: f32) -> CellMetrics {
        let fid = FontId::new(font_size, FontFamily::Name(FONT_REGULAR.into()));
        let (w, h) = ctx.fonts_mut(|f| (f.glyph_width(&fid, 'M'), f.row_height(&fid)));
        let ppp = ctx.pixels_per_point();
        let height = (h * (1.0 + self.cfg.cell_height_adjust) * ppp).round() / ppp;
        let width = ((w * ppp).round().max(1.0)) / ppp;
        CellMetrics { width, advance: w, height, baseline_offset: (((height - h) / 2.0) * ppp).round() / ppp, font_size, ppp }
    }

    /// Metrics for `font_size`, from this frame's cache when available.
    pub(super) fn metrics_for(&self, ctx: &egui::Context, font_size: f32) -> CellMetrics {
        match self.metrics.get(&font_size.to_bits()) {
            Some(m) => *m,
            None => self.compute_metrics(ctx, font_size),
        }
    }

    /// Cell metrics of pane `id` (filled by `update_metrics` each frame).
    pub(super) fn pane_metrics(&self, id: PaneId) -> CellMetrics {
        let size = self.pane_font(id);
        self.metrics
            .get(&size.to_bits())
            .or_else(|| self.metrics.get(&self.cfg.font_size.to_bits()))
            .copied()
            .unwrap_or(CellMetrics { width: 8.0, advance: 8.0, height: 17.0, baseline_offset: 0.0, font_size: size, ppp: 1.0 })
    }

    /// Recompute metrics for the default size and every per-pane size in use.
    pub(super) fn update_metrics(&mut self, ctx: &egui::Context) {
        let mut sizes: Vec<f32> = self.pane_fonts.values().copied().collect();
        sizes.push(self.cfg.font_size);
        self.metrics.clear();
        for s in sizes {
            if !self.metrics.contains_key(&s.to_bits()) {
                let m = self.compute_metrics(ctx, s);
                self.metrics.insert(s.to_bits(), m);
            }
        }
    }
}

/// Cell size in physical pixels (reported to the pty for image protocols).
pub(super) fn cell_px(ctx: &egui::Context, m: CellMetrics) -> (u16, u16) {
    let ppp = ctx.pixels_per_point();
    ((m.width * ppp) as u16, (m.height * ppp) as u16)
}
