//! Pinch zoom (touchscreen, touchpad) and Ctrl+wheel, for terminals and file explorers.
//! egui folds all three into `InputState::zoom_delta`.

use egui::Pos2;

use crate::layout::PaneId;

use crate::app::workspace::panes::Pane;
use crate::app::App;

/// How much one Ctrl+= / Ctrl+- zooms a file explorer.
pub(in crate::app) const EXPLORER_ZOOM_STEP: f32 = 1.1;

impl App {
    /// Zoom the pane under the pinch (or pointer): explorers smoothly, terminals in whole
    /// font points.
    pub(in crate::app) fn handle_pinch(&mut self, ctx: &egui::Context) {
        let (delta, at, pinching) = ctx.input(|i| {
            let multi = i.multi_touch();
            (i.zoom_delta(), multi.map(|m| m.center_pos).or(i.pointer.hover_pos()), multi.is_some())
        });
        if delta == 1.0 {
            if !pinching {
                self.input.pinch_accum = 1.0;
            }
            return;
        }
        let Some(id) = self.pane_at(at).or_else(|| self.focused_pane()) else { return };
        match self.panes.get_mut(id) {
            Some(Pane::Explorer(b)) => {
                b.zoom_by(delta, at);
                ctx.request_repaint();
            }
            Some(Pane::Diff(_)) => {
                // Smooth, like the explorer: the diff is laid out every frame anyway.
                self.set_pane_font(ctx, id, Some(self.pane_font(id) * delta));
                ctx.request_repaint();
            }
            Some(Pane::Terminal(_)) => self.pinch_terminal(ctx, id, delta),
            Some(Pane::Agents(_)) | None => {}
        }
    }

    /// Collect the pinch until it adds up to at least a point of font size, then apply it.
    fn pinch_terminal(&mut self, ctx: &egui::Context, id: PaneId, delta: f32) {
        self.input.pinch_accum *= delta;
        let size = self.pane_font(id);
        let target = (size * self.input.pinch_accum).round();
        if (target - size).abs() >= 1.0 {
            self.set_pane_font(ctx, id, Some(target));
            self.input.pinch_accum = 1.0;
        }
    }

    /// The pane whose card contains `pos`.
    fn pane_at(&self, pos: Option<Pos2>) -> Option<PaneId> {
        let pos = pos?;
        self.card_rects.iter().find(|(_, r)| r.contains(pos)).map(|(id, _)| *id)
    }
}
