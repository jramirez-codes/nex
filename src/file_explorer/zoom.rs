//! Zooming an explorer pane (Ctrl+= / Ctrl+- / Ctrl+0, Ctrl+wheel, touchpad or touchscreen
//! pinch). Everything in the pane scales; the row under the pinch stays where it is.

use egui::Pos2;

use super::FileExplorer;
use super::items::Layout;

const MIN_ZOOM: f32 = 0.6;
const MAX_ZOOM: f32 = 2.5;

/// A point of the listing to keep on screen at the same height across a zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct Anchor {
    /// First item of the row under the point.
    item: usize,
    /// Where the point is inside that row (0 = top, 1 = bottom).
    frac: f32,
    /// The point's distance from the top of the list, in points.
    y: f32,
}

impl FileExplorer {
    /// Zoom by `factor`, keeping the row under `at` (or, without a point, the top row) in
    /// place.
    pub fn zoom_by(&mut self, factor: f32, at: Option<Pos2>) {
        self.set_zoom(self.zoom * factor, at);
    }

    /// Back to the normal size.
    pub fn reset_zoom(&mut self) {
        self.set_zoom(1.0, None);
    }

    fn set_zoom(&mut self, zoom: f32, at: Option<Pos2>) {
        let zoom = zoom.clamp(MIN_ZOOM, MAX_ZOOM);
        if zoom == self.zoom {
            return;
        }
        self.zoom = zoom;
        // Keep an anchor from earlier in the same frame (several zoom events in one frame).
        if self.zoom_anchor.is_none() {
            self.zoom_anchor = self.layout.map(|l| anchor(&l, self.scroll, at));
        }
        if at.is_none() {
            // Keyboard zoom: keep the cursor visible.
            self.reveal = true;
        }
    }

    /// Scroll so the anchored row is back under its point, now that items have a new size
    /// (and, in grid view, maybe a new column count).
    pub(super) fn apply_zoom_anchor(&mut self, layout: &Layout) {
        let Some(a) = self.zoom_anchor.take() else { return };
        let row = (a.item / layout.cols) as f32;
        self.scroll = (row + a.frac) * layout.item.y - a.y;
    }
}

/// The anchor for a zoom at `at` (the top of the list when `None` or outside it).
fn anchor(layout: &Layout, scroll: f32, at: Option<Pos2>) -> Anchor {
    let list = layout.list;
    let y = at.filter(|p| list.contains(*p)).map_or(0.0, |p| p.y - list.top());
    let content_y = (scroll + y) / layout.item.y;
    Anchor { item: content_y.floor().max(0.0) as usize * layout.cols, frac: content_y.fract(), y }
}

#[cfg(test)]
mod tests {
    use egui::{Rect, Vec2};

    use super::super::items::sizes;
    use super::super::tests::fixture;
    use super::*;

    fn list_layout(row: f32) -> Layout {
        let list = Rect::from_min_size(Pos2::new(0.0, 100.0), Vec2::new(400.0, 300.0));
        Layout { list, cols: 1, item: Vec2::new(400.0, row) }
    }

    #[test]
    fn zoom_clamps_and_resets() {
        let dir = fixture("zoom");
        let mut b = FileExplorer::new(dir.clone());
        b.zoom_by(100.0, None);
        assert_eq!(b.zoom, MAX_ZOOM);
        b.zoom_by(0.0001, None);
        assert_eq!(b.zoom, MIN_ZOOM);
        b.reset_zoom();
        assert_eq!(b.zoom, 1.0);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn sizes_scale_with_zoom() {
        let (one, two) = (sizes(false, 1.0), sizes(false, 2.0));
        assert_eq!(two.row, one.row * 2.0);
        assert_eq!(two.tile, one.tile * 2.0);
        assert_eq!(two.font, one.font * 2.0);
        assert_eq!(two.scale, 2.0);
    }

    #[test]
    fn anchored_row_stays_under_the_pinch() {
        let dir = fixture("zoom-anchor");
        let mut b = FileExplorer::new(dir.clone());
        let before = list_layout(24.0);
        b.layout = Some(before);
        b.scroll = 30.0;
        // Pinch at 150: content y 80 → row 3, a third of the way down.
        let at = Pos2::new(10.0, 150.0);
        b.zoom_by(2.0, Some(at));
        let after = list_layout(48.0);
        b.apply_zoom_anchor(&after);
        // The same content point is still at y 150.
        let content = (150.0 - after.list.top() + b.scroll) / after.item.y;
        assert!((content - 80.0 / 24.0).abs() < 1e-4, "{content}");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
