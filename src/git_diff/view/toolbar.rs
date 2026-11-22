//! The Changes pane's toolbar: branch and ahead / behind, the filter and refresh.

use super::*;

const SEGMENT_TRACK: Color32 = Color32::from_black_alpha(55);
const SEGMENT_ON: Color32 = white_alpha(34);

impl GitDiff {
    /// Branch and ahead / behind on the left; the filter and refresh on the right.
    pub(super) fn toolbar(&mut self, f: &Frame, p: &Painter, bar: Rect) {
        let c = f.c;
        let ctx = f.ui.ctx();
        p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0_f32, HAIRLINE));
        let cy = bar.center().y;

        // Refresh, at the far right; it spins while git runs.
        let r = Rect::from_center_size(Pos2::new(bar.right() - 24.0, cy), Vec2::splat(28.0));
        let loading = self.loading();
        if IconButton::new(r, f.id("refresh"), c.text_dim, c.text_bright)
            .hover(HoverShape::Rounded(8), HOVER)
            .tooltip("Refresh  (R)")
            .show(f.ui, p, |p, r, col| {
                let turn = if loading { ctx.input(|i| i.time) as f32 * 5.0 } else { 0.0 };
                refresh_icon(p, r.shrink(7.0), col, turn, loading);
            })
        {
            self.refresh();
        }
        if loading {
            ctx.request_repaint();
        }
        let mut right = r.left() - 6.0;

        // The segmented filter, when there's room for it beside the branch.
        let counts = self.snapshot().map(|s| (s.unstaged.len(), s.staged.len()));
        if let Some((unstaged, staged)) = counts
            && bar.width() >= 420.0
        {
            let labels = [(Filter::All, "All".to_string()), (Filter::Unstaged, format!("Changes {unstaged}")), (Filter::Staged, format!("Staged {staged}"))];
            let font = ui_font(12.0);
            let widths: Vec<f32> = labels.iter().map(|(_, l)| p.layout_no_wrap(l.clone(), font.clone(), c.text).size().x + 22.0).collect();
            let total: f32 = widths.iter().sum::<f32>() + 4.0;
            let track = Rect::from_min_size(Pos2::new(right - total, cy - 13.0), Vec2::new(total, 26.0));
            right = track.left() - 10.0;
            p.rect_filled(track, 8, SEGMENT_TRACK);
            p.rect_stroke(track, 8, Stroke::new(1.0_f32, white_alpha(8)), StrokeKind::Inside);
            let mut x = track.left() + 2.0;
            let mut slots = Vec::new();
            for w in &widths {
                slots.push(Rect::from_min_size(Pos2::new(x, track.top() + 2.0), Vec2::new(*w, track.height() - 4.0)));
                x += w;
            }
            let on = labels.iter().position(|(k, _)| *k == self.filter).unwrap_or(0);
            let at = self.view.segment.value(ctx, on as f32);
            // The highlight glides between segments, stretching to their widths.
            let (i, frac) = (at.floor() as usize, at.fract());
            let a = slots[i.min(slots.len() - 1)];
            let b = slots[(i + 1).min(slots.len() - 1)];
            let pill = Rect::from_min_max(a.min.lerp(b.min, frac), a.max.lerp(b.max, frac));
            let shadow = Shadow { offset: [0, 1], blur: 4, spread: 0, color: Color32::from_black_alpha(70) };
            p.add(shadow.as_shape(pill, 6));
            p.rect_filled(pill, 6, SEGMENT_ON);
            p.rect_stroke(pill, 6, Stroke::new(1.0_f32, white_alpha(16)), StrokeKind::Inside);
            for (k, ((filter, label), slot)) in labels.iter().zip(&slots).enumerate() {
                let resp = f.interact(*slot, ("segment", k));
                let hovered = resp.as_ref().is_some_and(|r| r.hovered());
                let color = if k == on { c.text_bright } else if hovered { c.text } else { c.text_dim };
                p.text(slot.center(), Align2::CENTER_CENTER, label, font.clone(), color);
                if resp.is_some_and(|r| r.clicked()) {
                    self.set_filter(*filter);
                }
            }
        }

        // The branch.
        let mut x = bar.left() + SIDE;
        let (label, ahead, behind) = match self.snapshot() {
            Some(s) => (s.branch.clone(), s.ahead, s.behind),
            None => (crate::terminal::tilde(&self.dir), 0, 0),
        };
        paint_glyph(p, Pos2::new(x + 6.0, cy), 12.0, BRANCH_GLYPH, c.text_dim);
        x += 20.0;
        let mut room = right - x;
        let ab = match (ahead, behind) {
            (0, 0) => None,
            (a, 0) => Some(format!("↑{a}")),
            (0, b) => Some(format!("↓{b}")),
            (a, b) => Some(format!("↑{a} ↓{b}")),
        };
        let ab = ab.map(|t| p.layout_no_wrap(t, ui_font(11.0), c.text_dim));
        if let Some(g) = &ab {
            room -= g.size().x + 22.0;
        }
        if room > 16.0 {
            let font = paint::bold(13.0);
            let w = p.layout_no_wrap(label.clone(), font.clone(), c.text_bright).size().x.min(room);
            paint::faded_text(p, Pos2::new(x, cy), &label, font, c.text_bright, room, 20.0);
            x += w + 10.0;
            if let Some(g) = ab {
                let pill = Rect::from_min_size(Pos2::new(x, cy - 9.0), Vec2::new(g.size().x + 14.0, 18.0));
                p.rect_filled(pill, 9, white_alpha(12));
                p.galley(Pos2::new(pill.left() + 7.0, cy - g.size().y / 2.0), g, c.text_dim);
            }
        }
    }
}
