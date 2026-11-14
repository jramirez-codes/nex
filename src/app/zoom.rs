//! Pinch zoom (touchscreen, touchpad) and Ctrl+wheel, for terminals and file explorers.
//! egui folds all three into `InputState::zoom_delta`.

use egui::Pos2;

use crate::app::layout::PaneId;

use super::App;

/// How much one Ctrl+= / Ctrl+- zooms a file explorer.
pub(super) const EXPLORER_ZOOM_STEP: f32 = 1.1;

impl App {
    /// Zoom the pane under the pinch (or pointer): explorers smoothly, terminals in whole
    /// font points.
    pub(super) fn handle_pinch(&mut self, ctx: &egui::Context) {
        let (delta, at, pinching) = ctx.input(|i| {
            let multi = i.multi_touch();
            (i.zoom_delta(), multi.map(|m| m.center_pos).or(i.pointer.hover_pos()), multi.is_some())
        });
        if delta == 1.0 {
            if !pinching {
                self.pinch_accum = 1.0;
            }
            return;
        }
        let Some(id) = self.pane_at(at).or_else(|| self.focused_pane()) else { return };
        if let Some(explorer) = self.explorers.get_mut(&id) {
            explorer.zoom_by(delta, at);
            ctx.request_repaint();
        } else if self.panes.contains_key(&id) {
            self.pinch_terminal(ctx, id, delta);
        }
    }

    /// Collect the pinch until it adds up to at least a point of font size, then apply it.
    fn pinch_terminal(&mut self, ctx: &egui::Context, id: PaneId, delta: f32) {
        self.pinch_accum *= delta;
        let size = self.pane_font(id);
        let target = (size * self.pinch_accum).round();
        if (target - size).abs() >= 1.0 {
            self.set_pane_font(ctx, id, Some(target));
            self.pinch_accum = 1.0;
        }
    }

    /// The pane whose card contains `pos`.
    fn pane_at(&self, pos: Option<Pos2>) -> Option<PaneId> {
        let pos = pos?;
        self.card_rects.iter().find(|(_, r)| r.contains(pos)).map(|(id, _)| *id)
    }
}
