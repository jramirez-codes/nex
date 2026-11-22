//! The Changes pane's diff column: the selected file's header and its hunks, syntax
//! colored.

use super::*;

impl GitDiff {
    /// The selected file's header and diff.
    pub(super) fn diff_column(&mut self, f: &Frame, p: &Painter, area: Rect, snap: &Snapshot, back: bool) {
        let c = f.c;
        let ui = f.ui;
        let ctx = ui.ctx().clone();
        let Some(file) = self.selected.as_ref().and_then(|s| crate::git_diff::find(snap, s)).cloned() else {
            message(p, area, c, Glyph::File, "No File Selected", "");
            return;
        };
        let header = Rect::from_min_size(area.min, Vec2::new(area.width(), FILE_HEADER_H));
        let view = Rect::from_min_max(Pos2::new(area.left(), header.bottom()), area.max);

        let diff = self.diff.as_ref().filter(|(s, _)| *s == Sel::of(&file)).map(|(_, d)| Arc::clone(d));
        if let Some(arc) = &diff
            && let Ok(d) = arc.as_ref()
        {
            self.diff_body(f, p, view, d, Arc::as_ptr(arc) as usize);
        } else {
            self.view.hunk_tops.clear();
            match diff.as_deref() {
                Some(Err(e)) => message(p, view, c, Glyph::Warning, "Couldn't Load the Diff", e),
                _ => {
                    // Usually only a frame or two; a spinner only if it takes a while.
                    if self.diff_job.is_some() {
                        ctx.request_repaint();
                    }
                }
            }
        }

        // The sticky header goes on top, casting a shadow once the diff scrolls under it.
        p.rect_filled(header, 0, opaque(c.card));
        let scrolled = (self.diff_scroll.pos / 24.0).clamp(0.0, 1.0);
        if scrolled > 0.0 {
            for k in 0..6 {
                let y = header.bottom() + k as f32;
                let a = (28.0 * scrolled * (1.0 - k as f32 / 6.0)) as u8;
                p.line_segment([Pos2::new(header.left(), y), Pos2::new(header.right(), y)], Stroke::new(1.0_f32, Color32::from_black_alpha(a)));
            }
        }
        p.line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0_f32, HAIRLINE));
        let mut x = header.left() + SIDE;
        let cy = header.center().y;
        if back {
            let r = Rect::from_center_size(Pos2::new(x + 10.0, cy), Vec2::splat(28.0));
            if IconButton::new(r, f.id("back"), c.text_dim, c.text_bright)
                .icon_rect(r.shrink(8.0))
                .hover(HoverShape::Rounded(8), HOVER)
                .tooltip("Back to Changes  (Esc)")
                .show(ui, p, icons::icon_chevron_left)
            {
                self.show_detail = false;
                self.focus = Column::List;
            }
            x = r.right() + 6.0;
        }

        // Right side: the change kind and the line counts with their share bar.
        let mut right = header.right() - SIDE;
        if let Some((added, removed)) = file.counts {
            let bar = Rect::from_min_size(Pos2::new(right - 44.0, cy - 2.5), Vec2::new(44.0, 5.0));
            share_bar(p, bar, added, removed, c);
            right = bar.left() - 10.0;
            right = counts_text(p, Pos2::new(right, cy), added, removed, 11.5, c, false) - 10.0;
        }
        let color = change_color(file.change, c);
        let kind = p.layout_no_wrap(file.change.describe().to_string(), ui_font(11.0), color);
        let pill = Rect::from_min_size(Pos2::new(right - kind.size().x - 16.0, cy - 10.0), Vec2::new(kind.size().x + 16.0, 20.0));
        if pill.left() > x + 120.0 {
            p.rect_filled(pill, 10, color.gamma_multiply(0.14));
            p.galley(Pos2::new(pill.left() + 8.0, cy - kind.size().y / 2.0), kind, color);
            right = pill.left() - 12.0;
        }

        // Left side: icon, name, folder.
        let (glyph, glyph_color) = file_glyph(file.name(), false).unwrap_or(('\u{f15b}', c.text_dim));
        paint_glyph(p, Pos2::new(x + 8.0, cy), 15.0, glyph, glyph_color);
        x += 26.0;
        let name_font = paint::bold(13.5);
        let name = match &file.orig {
            Some(orig) => format!("{} ← {}", file.name(), orig.rsplit('/').next().unwrap_or(orig)),
            None => file.name().to_string(),
        };
        let name_w = p.layout_no_wrap(name.clone(), name_font.clone(), c.text_bright).size().x;
        let room = (right - x).max(0.0);
        paint::faded_text(p, Pos2::new(x, cy), &name, name_font, c.text_bright, room, 20.0);
        let folder = file.folder();
        if !folder.is_empty() && room - name_w > 40.0 {
            let fx = x + name_w + 10.0;
            paint::faded_text(p, Pos2::new(fx, cy + 0.5), folder, ui_font(12.0), c.text_dim, right - fx, 20.0);
        }
    }

    /// Hunk cards, only the visible lines laid out.
    /// `key` identifies the diff, to remember its widest line.
    fn diff_body(&mut self, f: &Frame, p: &Painter, view: Rect, d: &FileDiff, key: usize) {
        let c = f.c;
        let ui = f.ui;
        let ctx = ui.ctx().clone();
        let fade = self.view.fade.value(&ctx, 1.0);
        let mut p = p.with_clip_rect(view.intersect(f.clip));
        p.multiply_opacity(fade);

        if d.binary {
            message(&p, view, c, Glyph::File, "Binary File", "Its contents can't be shown as text.");
            return;
        }
        if d.hunks.is_empty() {
            message(&p, view, c, Glyph::File, "No Content Changes", "Only the file's mode or name changed.");
            return;
        }

        let size = f.code;
        let code = mono(size);
        let small = mono((size - 1.5).max(7.0));
        let line_h = (size * 1.6).round();
        let char_w = p.layout_no_wrap("0".into(), code.clone(), c.text).size().x;
        let num_w = p.layout_no_wrap("0".into(), small.clone(), c.text).size().x;
        self.view.line_height = line_h;
        self.view.diff_view_height = view.height();

        // Gutter: old and new line numbers, wide enough for the biggest.
        let biggest = d.hunks.iter().map(|h| (h.old_start + h.old_len).max(h.new_start + h.new_len)).max().unwrap_or(1);
        let digits = biggest.to_string().len().max(3) as f32;
        let col_w = digits * num_w + 12.0;
        let card_x0 = view.left() + SIDE;
        let card_x1 = view.right() - SIDE;
        let gutter = 6.0 + col_w * 2.0;
        let code_x = card_x0 + gutter + 12.0;

        // Positions of every hunk card, in content coordinates.
        let mut tops = Vec::with_capacity(d.hunks.len());
        let mut y = 0.0;
        for h in &d.hunks {
            y += HUNK_GAP;
            tops.push(y);
            y += HUNK_CAPTION_H + h.lines.len() as f32 * line_h + HUNK_PAD_BOTTOM;
        }
        let content_h = y + HUNK_GAP + if d.truncated { 40.0 } else { 0.0 };
        self.view.hunk_tops = tops.iter().map(|t| t - HUNK_GAP).collect();

        // Scrolling: the wheel, keys (eased), the scrollbar.
        let hovered = ui.rect_contains_pointer(view);
        if hovered && !ui.input(|i| i.modifiers.command) {
            let d = ui.input(|i| i.smooth_scroll_delta);
            if d != Vec2::ZERO {
                self.diff_scroll.by(-d.y);
                self.diff_scroll_x -= d.x;
                self.view.diff_active = ui.input(|i| i.time);
            }
        }
        let pan = self.view.diff_pan.delta(ui, view);
        if pan != 0.0 {
            self.diff_scroll.by(pan);
            self.view.diff_active = ui.input(|i| i.time);
        }
        let max = (content_h - view.height()).max(0.0);
        if self.diff_scroll.target < 0.0 || self.diff_scroll.target > max {
            self.view.diff_pan.stop();
        }
        self.diff_scroll.clamp(max);
        if self.diff_scroll.step(&ctx) {
            self.view.diff_active = ui.input(|i| i.time);
        }
        let widest = match self.view.widest {
            Some((k, w)) if k == key => w,
            _ => {
                let w = d.hunks.iter().flat_map(|h| &h.lines).map(|l| l.text.chars().count()).max().unwrap_or(0);
                self.view.widest = Some((key, w));
                w
            }
        };
        let max_x = (widest as f32 * char_w + 40.0 - (card_x1 - code_x)).max(0.0);
        self.diff_scroll_x = self.diff_scroll_x.clamp(0.0, max_x);
        let scroll = self.diff_scroll.pos;
        let sx = self.diff_scroll_x;
        let origin = view.top() - scroll;
        // The background colorer works from the topmost hunk on screen down.
        let first_visible = tops.iter().zip(&d.hunks).position(|(&t, h)| origin + t + HUNK_CAPTION_H + h.lines.len() as f32 * line_h + HUNK_PAD_BOTTOM >= view.top());
        self.color_focus.store(first_visible.unwrap_or(0), std::sync::atomic::Ordering::Relaxed);

        for (h, &top) in d.hunks.iter().zip(&tops) {
            let card_top = origin + top;
            let card_h = HUNK_CAPTION_H + h.lines.len() as f32 * line_h + HUNK_PAD_BOTTOM;
            let card = Rect::from_min_max(Pos2::new(card_x0, card_top), Pos2::new(card_x1, card_top + card_h));
            if card.bottom() < view.top() || card.top() > view.bottom() {
                continue;
            }
            hunk_card(&p, card, h, c);
            let inner = p.with_clip_rect(card.shrink(1.0).intersect(view));
            let lines_top = card.top() + HUNK_CAPTION_H;
            let first = (((view.top() - lines_top) / line_h).floor().max(0.0)) as usize;
            let last = ((((view.bottom() - lines_top) / line_h).ceil()) as usize).min(h.lines.len());
            let code_clip = inner.with_clip_rect(Rect::from_min_max(Pos2::new(card_x0 + gutter + 4.0, view.top()), Pos2::new(card_x1 - 1.0, view.bottom())).intersect(card));
            for (k, line) in h.lines.iter().enumerate().take(last).skip(first) {
                let ly = lines_top + k as f32 * line_h;
                let row = Rect::from_min_size(Pos2::new(card.left(), ly), Vec2::new(card.width(), line_h));
                let (tint, mark) = match line.kind {
                    LineKind::Added => (Some(c.green), "+"),
                    LineKind::Removed => (Some(c.red), "−"),
                    LineKind::Context => (None, ""),
                };
                if let Some(t) = tint {
                    inner.rect_filled(row, 0, t.gamma_multiply(0.11));
                    let gut = Rect::from_min_size(row.min, Vec2::new(gutter, line_h));
                    inner.rect_filled(gut, 0, t.gamma_multiply(0.08));
                    inner.rect_filled(Rect::from_min_size(row.min, Vec2::new(3.0, line_h)), 0, t.gamma_multiply(0.9));
                }
                let cy = row.center().y;
                let num_color = match tint {
                    Some(t) => t.gamma_multiply(0.75),
                    None => c.text_dim.gamma_multiply(0.65),
                };
                for (n, col) in [(line.old_no, 0.0), (line.new_no, 1.0)] {
                    if let Some(n) = n {
                        let right = card.left() + 6.0 + col_w * (col + 1.0) - 6.0;
                        inner.text(Pos2::new(right, cy), Align2::RIGHT_CENTER, n.to_string(), small.clone(), num_color);
                    }
                }
                if let Some(t) = tint {
                    inner.text(Pos2::new(code_x - 9.0, cy), Align2::CENTER_CENTER, mark, small.clone(), t.gamma_multiply(0.9));
                }
                let job = code_job(line, code.clone(), c, tint, line.kind == LineKind::Removed);
                let galley = code_clip.layout_job(job);
                let gy = cy - galley.size().y / 2.0;
                let end_x = code_x - sx + galley.size().x;
                code_clip.galley(Pos2::new(code_x - sx, gy), galley, c.text);
                if line.no_newline {
                    code_clip.text(Pos2::new(end_x + 10.0, cy), Align2::LEFT_CENTER, "No newline at end of file", ui_font(10.5), c.text_dim.gamma_multiply(0.7));
                }
            }
        }
        if d.truncated {
            let y = origin + content_h - 30.0;
            p.text(Pos2::new(view.center().x, y), Align2::CENTER_CENTER, format!("Diff cut off after {} lines", d.line_count()), ui_font(12.0), c.text_dim);
        }
        if let Some(to) = overlay_scrollbar(ui, &p, view, f.id("diff-bar"), ScrollSpan { offset: scroll, content: content_h, view: view.height() }, &mut self.view.diff_active) {
            self.diff_scroll.set(to);
        }
    }
}

/// A slim capsule split between additions (green) and removals (red).
fn share_bar(p: &Painter, r: Rect, added: u32, removed: u32, c: &Theme) {
    p.rect_filled(r, 3, white_alpha(16));
    let total = (added + removed) as f32;
    if total <= 0.0 {
        return;
    }
    let split = r.left() + r.width() * added as f32 / total;
    let clip = p.with_clip_rect(r);
    if added > 0 {
        clip.rect_filled(Rect::from_min_max(r.min, Pos2::new(split, r.bottom())), CornerRadius { nw: 3, sw: 3, ne: 0, se: 0 }, c.green);
    }
    if removed > 0 {
        clip.rect_filled(Rect::from_min_max(Pos2::new(split, r.top()), r.max), CornerRadius { nw: 0, sw: 0, ne: 3, se: 3 }, c.red);
    }
    // Keep a hairline gap between the two, like a segmented gauge.
    if added > 0 && removed > 0 {
        clip.line_segment([Pos2::new(split, r.top()), Pos2::new(split, r.bottom())], Stroke::new(1.5_f32, opaque(c.card)));
    }
}

/// A hunk's rounded card and its caption: the enclosing function, and the line ranges.
fn hunk_card(p: &Painter, card: Rect, h: &Hunk, c: &Theme) {
    p.rect_filled(card, HUNK_RADIUS, white_alpha(5));
    p.rect_stroke(card, HUNK_RADIUS, Stroke::new(1.0_f32, white_alpha(13)), StrokeKind::Inside);
    let cap = Rect::from_min_size(card.min, Vec2::new(card.width(), HUNK_CAPTION_H));
    p.rect_filled(cap, CornerRadius { nw: HUNK_RADIUS, ne: HUNK_RADIUS, sw: 0, se: 0 }, white_alpha(4));
    p.line_segment([cap.left_bottom() + Vec2::new(1.0, 0.0), cap.right_bottom() - Vec2::new(1.0, 0.0)], Stroke::new(1.0_f32, white_alpha(10)));
    let cy = cap.center().y;
    let ranges = format!("−{},{}  +{},{}", h.old_start, h.old_len, h.new_start, h.new_len);
    let rg = p.layout_no_wrap(ranges, mono(10.5), c.text_dim.gamma_multiply(0.7));
    let rx = cap.right() - 12.0 - rg.size().x;
    let rgh = rg.size().y;
    p.galley(Pos2::new(rx, cy - rgh / 2.0), rg, c.text_dim);
    let label = if h.context.is_empty() {
        let end = h.new_start + h.new_len.saturating_sub(1);
        if h.new_len <= 1 { format!("Line {}", h.new_start) } else { format!("Lines {}–{end}", h.new_start) }
    } else {
        h.context.clone()
    };
    let font = if h.context.is_empty() { ui_font(12.0) } else { mono(11.5) };
    paint::faded_text(p, Pos2::new(cap.left() + 12.0, cy), &label, font, c.text_dim, (rx - cap.left() - 24.0).max(0.0), 20.0);
}

/// A token's color, from the terminal palette.
fn token_color(t: Token, c: &Theme) -> Color32 {
    match t {
        Token::Keyword => c.magenta,
        Token::String => c.green,
        Token::Comment => c.text_dim.gamma_multiply(0.85),
        Token::Number | Token::Constant => c.yellow,
        Token::Function => c.blue,
        Token::Type => c.cyan,
        Token::Attribute => c.yellow.gamma_multiply(0.8),
        Token::Operator | Token::Punct => c.text.gamma_multiply(0.75),
    }
}

/// `color` lifted toward white (changed words stand out on their tint).
fn brighten(color: Color32) -> Color32 {
    crate::ui::theme::lighten(color, 0.35)
}

/// One line of code in its syntax colors, with its changed words on a stronger tint.
/// `dim` softens the colors (removed lines, so the new code reads first).
fn code_job(line: &Line, font: FontId, c: &Theme, tint: Option<Color32>, dim: bool) -> LayoutJob {
    use std::ops::Range;
    let text = line.text.as_str();
    let ok = |r: &Range<usize>| r.start < r.end && r.end <= text.len() && text.is_char_boundary(r.start) && text.is_char_boundary(r.end);
    let emphasis: Vec<&Range<usize>> = line.emphasis.iter().filter(|r| ok(r)).collect();
    let syntax: Vec<&(Range<usize>, Token)> = line.syntax.iter().filter(|(r, _)| ok(r)).collect();

    // Cut the line wherever a token or a changed word starts or ends.
    let mut cuts: Vec<usize> = vec![0, text.len()];
    cuts.extend(emphasis.iter().flat_map(|r| [r.start, r.end]));
    cuts.extend(syntax.iter().flat_map(|(r, _)| [r.start, r.end]));
    cuts.sort_unstable();
    cuts.dedup();

    let background = tint.map_or(Color32::TRANSPARENT, |t| t.gamma_multiply(0.32));
    let mut job = LayoutJob::default();
    let (mut si, mut ei) = (0, 0);
    for w in cuts.windows(2) {
        let (a, b) = (w[0], w[1]);
        while si < syntax.len() && syntax[si].0.end <= a {
            si += 1;
        }
        while ei < emphasis.len() && emphasis[ei].end <= a {
            ei += 1;
        }
        let token = syntax.get(si).filter(|(r, _)| r.start <= a).map(|(_, t)| *t);
        let strong = emphasis.get(ei).is_some_and(|r| r.start <= a);
        let mut color = token.map_or(c.text, |t| token_color(t, c));
        if dim {
            color = color.gamma_multiply(0.8);
        }
        let format = if strong {
            TextFormat { font_id: font.clone(), color: brighten(color), background, ..Default::default() }
        } else {
            TextFormat { font_id: font.clone(), color, ..Default::default() }
        };
        job.append(&text[a..b], 0.0, format);
    }
    if job.sections.is_empty() {
        job.append(" ", 0.0, TextFormat { color: Color32::TRANSPARENT, ..TextFormat::simple(FontId::proportional(10.0), c.text) });
    }
    job
}

#[cfg(test)]
mod tests {
    use super::*;

    fn theme() -> Theme {
        let k = |v: u8| Color32::from_rgb(v, v, v);
        Theme { window: k(0), card: k(10), text: k(200), text_bright: k(250), text_dim: k(120), green: k(1), red: k(2), yellow: k(3), blue: k(4), magenta: k(5), cyan: k(6) }
    }

    #[test]
    fn code_job_merges_tokens_and_emphasis() {
        // "let é = 1;": keyword "let", changed word "é = 1" crossing into the number.
        let text = "let é = 1;".to_string();
        let one = text.find('1').unwrap();
        let line = Line {
            kind: LineKind::Added,
            old_no: None,
            new_no: Some(1),
            emphasis: std::iter::once(4..one + 1).collect(),
            syntax: vec![(0..3, Token::Keyword), (one..one + 1, Token::Number)],
            text,
            no_newline: false,
        };
        let c = theme();
        let job = code_job(&line, mono(12.0), &c, Some(c.green), false);
        let parts: Vec<(&str, Color32, bool)> = job
            .sections
            .iter()
            .map(|s| (&job.text[s.byte_range.clone()], s.format.color, s.format.background != Color32::TRANSPARENT))
            .collect();
        assert_eq!(
            parts,
            vec![
                ("let", c.magenta, false),
                (" ", c.text, false),
                ("é = ", brighten(c.text), true),
                ("1", brighten(c.yellow), true),
                (";", c.text, false),
            ]
        );
    }
}
