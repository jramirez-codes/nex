//! Drawing the Changes pane, macOS style: a quiet toolbar (branch, a segmented filter,
//! refresh), a source-list sidebar of changed files, and the selected file's diff as rounded
//! hunk cards under a sticky file header. Mouse handling lives here too.

use std::sync::Arc;

use egui::epaint::{CornerRadius, Shadow};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

use crate::render::FONT_REGULAR;
use crate::ui::file_icons::{file_glyph, paint_glyph};
use crate::ui::glide::Glide;
use crate::ui::theme::{ACCENT, HAIRLINE, HOVER, Theme, white_alpha};
use crate::ui::widgets::{HoverShape, IconButton, ScrollSpan, overlay_scrollbar};
use crate::ui::{icons, paint};

use super::git::{Change, FileChange, FileDiff, Hunk, Line, LineKind, Snapshot, Status};
use super::syntax::Token;
use super::{Column, Filter, GitDiff, Sel};

const TOOLBAR_H: f32 = 42.0;
const SECTION_H: f32 = 30.0;
const ROW_H: f32 = 28.0;
const LIST_PAD: f32 = 6.0;
const FILE_HEADER_H: f32 = 48.0;
const HUNK_GAP: f32 = 12.0;
const HUNK_CAPTION_H: f32 = 30.0;
const HUNK_PAD_BOTTOM: f32 = 6.0;
const HUNK_RADIUS: u8 = 10;
const SIDE: f32 = 14.0;
/// Below this width the pane shows the list or the diff, not both.
const NARROW: f32 = 560.0;
const BRANCH_GLYPH: char = '\u{e725}';
const SEGMENT_TRACK: Color32 = Color32::from_black_alpha(55);
const SEGMENT_ON: Color32 = white_alpha(34);

/// What the view keeps between frames.
pub(super) struct ViewState {
    /// The selection highlight's top, in list content coordinates.
    sel: Glide,
    /// The segmented control's highlighted segment.
    segment: Glide,
    /// The diff fading in after a new file was picked.
    fade: Glide,
    /// Narrow panes: 0 shows the list, 1 the diff.
    detail: Glide,
    /// Scroll positions (diff content coordinates) at which each hunk card sits at the top.
    pub hunk_tops: Vec<f32>,
    pub line_height: f32,
    pub diff_view_height: f32,
    pub narrow: bool,
    list_active: f64,
    diff_active: f64,
    /// Longest line of the diff on screen, in chars, keyed by the diff's address.
    widest: Option<(usize, usize)>,
}

impl ViewState {
    pub(super) fn new() -> ViewState {
        ViewState {
            sel: Glide::new(0.16),
            segment: Glide::new(0.2),
            fade: Glide::new(0.18),
            detail: Glide::new(0.26),
            hunk_tops: Vec::new(),
            line_height: 20.0,
            diff_view_height: 400.0,
            narrow: false,
            list_active: -10.0,
            diff_active: -10.0,
            widest: None,
        }
    }

    pub(super) fn fade_in(&mut self, ctx: &egui::Context) {
        self.fade.snap(ctx, 0.0);
    }
}

/// Everything a frame's drawing shares.
struct Frame<'a> {
    ui: &'a Ui,
    c: &'a Theme,
    /// The pane is focused (selection is drawn in the accent color only then).
    focused: bool,
    code: f32,
    pane: u64,
    /// Where interaction is allowed (the pane's visible content).
    clip: Rect,
}

impl Frame<'_> {
    fn id(&self, what: impl std::hash::Hash) -> Id {
        Id::new(("git-diff", self.pane, what))
    }

    /// A click target, cut to the visible area so off-screen rows never take clicks.
    fn interact(&self, r: Rect, what: impl std::hash::Hash) -> Option<egui::Response> {
        let r = r.intersect(self.clip);
        r.is_positive().then(|| self.ui.interact(r, self.id(what), Sense::click()))
    }
}

fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(FONT_REGULAR.into()))
}

fn ui_font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// The badge color for a change.
fn change_color(change: Change, c: &Theme) -> Color32 {
    match change {
        Change::Modified => c.yellow,
        Change::Added | Change::Untracked | Change::Copied => c.green,
        Change::Deleted | Change::Conflicted => c.red,
        Change::Renamed => c.blue,
        Change::TypeChanged => c.text_dim,
    }
}

impl GitDiff {
    /// Draw the pane into `rect` (below the pane header, edge to edge of the card; `corner`
    /// rounds the bottom corners). `code_size` is the diff's font size (the pane's zoom).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: u64, focused: bool, corner: u8, c: &Theme, code_size: f32) {
        self.poll();
        let ctx = ui.ctx().clone();
        let f = Frame { ui, c, focused, code: code_size, pane, clip: rect.intersect(painter.clip_rect()) };
        let painter = &painter.with_clip_rect(f.clip);

        let bar = Rect::from_min_size(rect.min, Vec2::new(rect.width(), TOOLBAR_H));
        self.toolbar(&f, painter, bar);
        let body = Rect::from_min_max(Pos2::new(rect.left(), bar.bottom()), rect.max);
        if !body.is_positive() {
            return;
        }
        let status = self.status.clone();
        let snap = match status.as_deref() {
            None => {
                message(painter, body, c, Glyph::Spinner(ctx.input(|i| i.time)), "Loading Changes", "");
                ctx.request_repaint();
                return;
            }
            Some(Status::NotRepo) => {
                message(painter, body, c, Glyph::Branch, "Not a Git Repository", &crate::terminal::tilde(&self.dir));
                return;
            }
            Some(Status::NoGit) => {
                message(painter, body, c, Glyph::Branch, "Git Not Found", "Install git to see your changes here.");
                return;
            }
            Some(Status::Failed(e)) => {
                message(painter, body, c, Glyph::Warning, "Couldn't Read Changes", e);
                return;
            }
            Some(Status::Ready(snap)) => snap,
        };
        if snap.staged.is_empty() && snap.unstaged.is_empty() {
            message(painter, body, c, Glyph::Check, "No Changes", "Working tree is clean");
            return;
        }

        let narrow = body.width() < NARROW;
        self.view.narrow = narrow;
        if pressed_in(ui, body) && !narrow {
            let split = body.left() + sidebar_width(body.width());
            let at = ui.input(|i| i.pointer.interact_pos()).unwrap_or_default();
            self.focus = if at.x < split { Column::List } else { Column::Diff };
        }
        if narrow {
            let t = self.view.detail.value(&ctx, if self.show_detail { 1.0 } else { 0.0 });
            let w = body.width();
            if t < 0.999 {
                let list = body.translate(Vec2::new(-t * w * 0.3, 0.0));
                let mut p = painter.clone();
                p.multiply_opacity(1.0 - t * 0.7);
                self.list(&f, &p, list, snap, corner, corner);
            }
            if t > 0.001 {
                let detail = body.translate(Vec2::new((1.0 - t) * w, 0.0));
                let shadow = Shadow { offset: [-2, 0], blur: 18, spread: 0, color: Color32::from_black_alpha((90.0 * t) as u8) };
                painter.add(shadow.as_shape(detail, 0));
                painter.rect_filled(detail, CornerRadius { nw: 0, ne: 0, sw: corner, se: corner }, opaque(c.card));
                self.diff_column(&f, painter, detail, snap, true);
            }
        } else {
            let w = sidebar_width(body.width());
            let list = Rect::from_min_size(body.min, Vec2::new(w, body.height()));
            let diff = Rect::from_min_max(Pos2::new(list.right(), body.top()), body.max);
            self.list(&f, painter, list, snap, corner, 0);
            painter.line_segment([list.right_top(), list.right_bottom()], Stroke::new(1.0, HAIRLINE));
            self.diff_column(&f, painter, diff, snap, false);
        }
    }

    /// Branch and ahead / behind on the left; the filter and refresh on the right.
    fn toolbar(&mut self, f: &Frame, p: &Painter, bar: Rect) {
        let c = f.c;
        let ctx = f.ui.ctx();
        p.line_segment([bar.left_bottom(), bar.right_bottom()], Stroke::new(1.0, HAIRLINE));
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
            p.rect_stroke(track, 8, Stroke::new(1.0, white_alpha(8)), StrokeKind::Inside);
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
            p.rect_stroke(pill, 6, Stroke::new(1.0, white_alpha(16)), StrokeKind::Inside);
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

    /// The sidebar: Staged and Changes sections, one row per file.
    fn list(&mut self, f: &Frame, p: &Painter, area: Rect, snap: &Snapshot, corner_sw: u8, corner_se: u8) {
        let c = f.c;
        let ui = f.ui;
        let ctx = ui.ctx();
        p.rect_filled(area, CornerRadius { nw: 0, ne: 0, sw: corner_sw, se: corner_se }, Color32::from_black_alpha(34));

        // Lay the rows out in content coordinates first, to keep the selection in view.
        enum Row<'a> {
            Header(bool, usize),
            File(&'a FileChange),
        }
        let mut rows: Vec<(Row, f32, f32)> = Vec::new();
        let mut y = LIST_PAD;
        for (staged, files) in sections(snap, self.filter) {
            rows.push((Row::Header(staged, files.len()), y, SECTION_H));
            y += SECTION_H;
            if !self.collapsed[if staged { 0 } else { 1 }] {
                for file in files {
                    rows.push((Row::File(file), y, ROW_H));
                    y += ROW_H;
                }
            }
            y += 8.0;
        }
        let content_h = y + LIST_PAD;
        let sel_y = rows.iter().find_map(|(r, y, _)| match r {
            Row::File(file) if self.selected.as_ref() == Some(&Sel::of(file)) => Some(*y),
            _ => None,
        });

        if ui.rect_contains_pointer(area) && !ui.input(|i| i.modifiers.command) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if d != 0.0 {
                self.list_scroll -= d;
                self.view.list_active = ui.input(|i| i.time);
            }
        }
        if std::mem::take(&mut self.reveal)
            && let Some(sy) = sel_y
        {
            if sy < self.list_scroll {
                self.list_scroll = sy - LIST_PAD;
            } else if sy + ROW_H > self.list_scroll + area.height() {
                self.list_scroll = sy + ROW_H + LIST_PAD - area.height();
            }
        }
        self.list_scroll = self.list_scroll.clamp(0.0, (content_h - area.height()).max(0.0));
        let top = area.top() - self.list_scroll;
        let clip = p.with_clip_rect(area.intersect(f.clip));

        // The selection highlight, gliding from row to row.
        let active = f.focused && self.focus == Column::List;
        if let Some(sy) = sel_y {
            let gy = self.view.sel.value(ctx, sy);
            let r = Rect::from_min_size(Pos2::new(area.left() + 6.0, top + gy + 1.0), Vec2::new(area.width() - 12.0, ROW_H - 2.0));
            let fill = if active { ACCENT.gamma_multiply(0.85) } else { white_alpha(22) };
            clip.rect_filled(r, 6, fill);
        }

        let mut clicked: Option<Sel> = None;
        let mut reveal = false;
        let mut toggle: Option<bool> = None;
        for (i, (row, ry, h)) in rows.iter().enumerate() {
            let r = Rect::from_min_size(Pos2::new(area.left(), top + ry), Vec2::new(area.width(), *h));
            if r.bottom() < area.top() || r.top() > area.bottom() {
                continue;
            }
            let hit = Frame { clip: area.intersect(f.clip), ..*f };
            match row {
                Row::Header(staged, count) => {
                    let resp = hit.interact(r, ("section", *staged));
                    let hovered = resp.as_ref().is_some_and(|r| r.hovered());
                    if resp.is_some_and(|r| r.clicked()) {
                        toggle = Some(*staged);
                    }
                    let collapsed = self.collapsed[if *staged { 0 } else { 1 }];
                    section_header(&clip, r, if *staged { "STAGED" } else { "CHANGES" }, *count, collapsed, hovered, c);
                }
                Row::File(file) => {
                    let sel = Sel::of(file);
                    let is_sel = self.selected.as_ref() == Some(&sel);
                    let resp = hit.interact(r, ("row", i, &file.path, file.staged));
                    let hovered = resp.as_ref().is_some_and(|r| r.hovered());
                    if let Some(resp) = resp {
                        if resp.double_clicked() {
                            reveal = true;
                        }
                        if resp.clicked() {
                            clicked = Some(sel.clone());
                        }
                        resp.on_hover_text_at_pointer(format!("{} · {}", file.change.describe(), file.path));
                    }
                    if hovered && !is_sel {
                        clip.rect_filled(r.shrink2(Vec2::new(6.0, 1.0)), 6, white_alpha(10));
                    }
                    file_row(&clip, r, file, is_sel && active, c);
                }
            }
        }
        if let Some(staged) = toggle {
            self.toggle_section(staged);
        }
        if let Some(sel) = clicked {
            self.focus = Column::List;
            self.select(sel);
            if self.view.narrow {
                self.show_detail = true;
                self.focus = Column::Diff;
            }
        }
        if reveal {
            self.reveal_selected();
        }
        if let Some(to) = overlay_scrollbar(ui, p, area, f.id("list-bar"), ScrollSpan { offset: self.list_scroll, content: content_h, view: area.height() }, &mut self.view.list_active) {
            self.list_scroll = to;
        }
    }

    /// The selected file's header and diff.
    fn diff_column(&mut self, f: &Frame, p: &Painter, area: Rect, snap: &Snapshot, back: bool) {
        let c = f.c;
        let ui = f.ui;
        let ctx = ui.ctx().clone();
        let Some(file) = self.selected.as_ref().and_then(|s| super::find(snap, s)).cloned() else {
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
                p.line_segment([Pos2::new(header.left(), y), Pos2::new(header.right(), y)], Stroke::new(1.0, Color32::from_black_alpha(a)));
            }
        }
        p.line_segment([header.left_bottom(), header.right_bottom()], Stroke::new(1.0, HAIRLINE));
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
        let max = (content_h - view.height()).max(0.0);
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

/// The list's width beside the diff.
fn sidebar_width(total: f32) -> f32 {
    (total * 0.3).clamp(210.0, 320.0)
}

fn sections(snap: &Snapshot, filter: Filter) -> Vec<(bool, &[FileChange])> {
    let mut out = Vec::new();
    if filter != Filter::Unstaged && !snap.staged.is_empty() {
        out.push((true, snap.staged.as_slice()));
    }
    if filter != Filter::Staged && !snap.unstaged.is_empty() {
        out.push((false, snap.unstaged.as_slice()));
    }
    out
}

fn pressed_in(ui: &Ui, r: Rect) -> bool {
    crate::ui::widgets::pressed_in(ui, r)
}

/// The card color without its transparency (sticky headers hide what scrolls beneath).
fn opaque(c: Color32) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgb(r, g, b)
}

/// "STAGED  3" with a disclosure chevron.
fn section_header(p: &Painter, r: Rect, label: &str, count: usize, collapsed: bool, hovered: bool, c: &Theme) {
    let cy = r.center().y + 2.0;
    let color = if hovered { c.text } else { c.text_dim };
    let x = r.left() + SIDE;
    let chev = Rect::from_center_size(Pos2::new(x + 4.0, cy), Vec2::splat(8.0));
    disclosure(p, chev, color, collapsed);
    let mut job = LayoutJob::default();
    job.append(label, 0.0, TextFormat { font_id: paint::bold(10.5), color, extra_letter_spacing: 0.8, ..Default::default() });
    let g = p.layout_job(job);
    p.galley(Pos2::new(x + 14.0, cy - g.size().y / 2.0), g, color);
    p.text(Pos2::new(r.right() - SIDE, cy), Align2::RIGHT_CENTER, count.to_string(), ui_font(11.0), c.text_dim.gamma_multiply(0.8));
}

/// A small triangle-ish chevron: pointing down when open, right when collapsed.
fn disclosure(p: &Painter, r: Rect, color: Color32, collapsed: bool) {
    let s = Stroke::new(1.4, color);
    let k = r.width() * 0.42;
    let m = r.center();
    let pts = if collapsed {
        vec![m + Vec2::new(-k * 0.45, -k), m + Vec2::new(k * 0.55, 0.0), m + Vec2::new(-k * 0.45, k)]
    } else {
        vec![m + Vec2::new(-k, -k * 0.45), m + Vec2::new(0.0, k * 0.55), m + Vec2::new(k, -k * 0.45)]
    };
    p.add(Shape::line(pts, s));
}

/// One file in the list: status badge, icon, name, folder, line counts.
fn file_row(p: &Painter, r: Rect, file: &FileChange, selected: bool, c: &Theme) {
    let cy = r.center().y;
    let mut x = r.left() + SIDE;
    let color = change_color(file.change, c);
    let badge = Rect::from_center_size(Pos2::new(x + 8.0, cy), Vec2::splat(16.0));
    let (fill, ink) = if selected { (white_alpha(50), Color32::WHITE) } else { (color.gamma_multiply(0.17), color) };
    p.rect_filled(badge, 5, fill);
    p.text(badge.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, file.change.letter(), paint::bold(10.0), ink);
    x += 24.0;
    let (glyph, glyph_color) = file_glyph(file.name(), false).unwrap_or(('\u{f15b}', c.text_dim));
    paint_glyph(p, Pos2::new(x + 6.0, cy), 12.5, glyph, if selected { Color32::WHITE } else { glyph_color });
    x += 20.0;

    let right = r.right() - SIDE;
    let counts_left = match file.counts {
        Some((a, d)) if a + d > 0 => counts_text(p, Pos2::new(right, cy), a, d, 10.5, c, selected),
        Some(_) => right,
        None => {
            let g = p.layout_no_wrap("BIN".into(), ui_font(9.5), c.text_dim);
            let at = Pos2::new(right - g.size().x, cy - g.size().y / 2.0);
            let w = g.size().x;
            p.galley(at, g, c.text_dim);
            right - w
        }
    };
    let room = (counts_left - 8.0 - x).max(0.0);
    let name_color = if selected { Color32::WHITE } else { c.text };
    let font = ui_font(13.0);
    let name_w = p.layout_no_wrap(file.name().to_string(), font.clone(), name_color).size().x;
    let struck = file.change == Change::Deleted;
    paint::faded_text(p, Pos2::new(x, cy), file.name(), font, if struck { name_color.gamma_multiply(0.7) } else { name_color }, room, 18.0);
    if struck {
        let w = name_w.min(room);
        p.line_segment([Pos2::new(x, cy + 0.5), Pos2::new(x + w, cy + 0.5)], Stroke::new(1.0, name_color.gamma_multiply(0.5)));
    }
    let folder = file.folder();
    if !folder.is_empty() && room - name_w > 28.0 {
        let fx = x + name_w + 7.0;
        let dim = if selected { white_alpha(150) } else { c.text_dim.gamma_multiply(0.85) };
        paint::faded_text(p, Pos2::new(fx, cy + 0.5), folder, ui_font(11.0), dim, counts_left - 8.0 - fx, 18.0);
    }
}

/// "+12 −3", right-aligned at `right_center`; returns where it starts.
fn counts_text(p: &Painter, right_center: Pos2, added: u32, removed: u32, size: f32, c: &Theme, selected: bool) -> f32 {
    let font = mono(size);
    let mut job = LayoutJob::default();
    let (g, r) = if selected { (Color32::WHITE, white_alpha(200)) } else { (c.green, c.red) };
    if added > 0 {
        job.append(&format!("+{added}"), 0.0, TextFormat { font_id: font.clone(), color: g, ..Default::default() });
    }
    if removed > 0 {
        job.append(&format!("−{removed}"), if added > 0 { 5.0 } else { 0.0 }, TextFormat { font_id: font, color: r, ..Default::default() });
    }
    let galley = p.layout_job(job);
    let at = Pos2::new(right_center.x - galley.size().x, right_center.y - galley.size().y / 2.0);
    let w = galley.size().x;
    p.galley(at, galley, c.text);
    right_center.x - w
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
        clip.line_segment([Pos2::new(split, r.top()), Pos2::new(split, r.bottom())], Stroke::new(1.5, opaque(c.card)));
    }
}

/// A hunk's rounded card and its caption: the enclosing function, and the line ranges.
fn hunk_card(p: &Painter, card: Rect, h: &Hunk, c: &Theme) {
    p.rect_filled(card, HUNK_RADIUS, white_alpha(5));
    p.rect_stroke(card, HUNK_RADIUS, Stroke::new(1.0, white_alpha(13)), StrokeKind::Inside);
    let cap = Rect::from_min_size(card.min, Vec2::new(card.width(), HUNK_CAPTION_H));
    p.rect_filled(cap, CornerRadius { nw: HUNK_RADIUS, ne: HUNK_RADIUS, sw: 0, se: 0 }, white_alpha(4));
    p.line_segment([cap.left_bottom() + Vec2::new(1.0, 0.0), cap.right_bottom() - Vec2::new(1.0, 0.0)], Stroke::new(1.0, white_alpha(10)));
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

enum Glyph {
    Spinner(f64),
    Branch,
    Check,
    Warning,
    File,
}

/// A centered empty state: a large soft glyph, a title and a line of detail.
fn message(p: &Painter, area: Rect, c: &Theme, glyph: Glyph, title: &str, detail: &str) {
    let center = area.center() - Vec2::new(0.0, (area.height() * 0.06).min(30.0));
    let g = Rect::from_center_size(center - Vec2::new(0.0, 34.0), Vec2::splat(44.0));
    match glyph {
        Glyph::Spinner(t) => refresh_icon(p, g.shrink(10.0), c.text_dim, t as f32 * 5.0, true),
        Glyph::Branch => {
            p.circle_filled(g.center(), 22.0, white_alpha(10));
            paint_glyph(p, g.center(), 20.0, BRANCH_GLYPH, c.text_dim);
        }
        Glyph::Check => {
            p.circle_filled(g.center(), 22.0, c.green.gamma_multiply(0.16));
            let s = Stroke::new(2.4, c.green);
            let m = g.center();
            p.add(Shape::line(vec![m + Vec2::new(-8.0, 0.5), m + Vec2::new(-2.5, 6.0), m + Vec2::new(8.5, -6.0)], s));
        }
        Glyph::Warning => {
            p.circle_filled(g.center(), 22.0, c.yellow.gamma_multiply(0.14));
            p.text(g.center(), Align2::CENTER_CENTER, "!", paint::bold(22.0), c.yellow);
        }
        Glyph::File => {
            p.circle_filled(g.center(), 22.0, white_alpha(10));
            icons::icon_file(p, Rect::from_center_size(g.center(), Vec2::new(14.0, 18.0)), c.text_dim);
        }
    }
    p.text(center + Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, title, paint::bold(15.0), c.text_bright);
    if !detail.is_empty() {
        paint::caption(p, Rect::from_center_size(center + Vec2::new(0.0, 28.0), Vec2::new((area.width() - 40.0).max(0.0), 18.0)), detail, ui_font(12.5), c.text_dim);
    }
}

/// A circular arrow (refresh); with `spinning`, an open arc turning by `turn` radians.
fn refresh_icon(p: &Painter, r: Rect, color: Color32, turn: f32, spinning: bool) {
    use std::f32::consts::{PI, TAU};
    let m = r.center();
    let radius = r.width().min(r.height()) / 2.0;
    let (start, sweep) = if spinning { (turn, TAU * 0.72) } else { (-PI * 0.35, TAU * 0.8) };
    let n = 28;
    let pts: Vec<Pos2> = (0..=n).map(|i| {
        let a = start + sweep * i as f32 / n as f32;
        m + Vec2::new(a.cos(), a.sin()) * radius
    }).collect();
    let end = *pts.last().unwrap_or(&m);
    p.add(Shape::line(pts, Stroke::new(1.6, color)));
    if !spinning {
        // The arrowhead, at the end of the arc, pointing along it.
        let a = start + sweep;
        let tangent = Vec2::new(-a.sin(), a.cos());
        let normal = Vec2::new(a.cos(), a.sin());
        let k = radius * 0.55;
        let tip = end + tangent * k * 0.6;
        p.add(Shape::convex_polygon(vec![tip, end - tangent * k * 0.35 + normal * k * 0.6, end - tangent * k * 0.35 - normal * k * 0.6], color, Stroke::NONE));
    }
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
