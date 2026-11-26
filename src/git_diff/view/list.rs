//! The Changes pane's file list: Staged and Changes sections, one row per file.

use egui::epaint::CornerRadius;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::git_diff::git::status::{Change, FileChange, Snapshot};
use crate::git_diff::view::{Frame, LIST_PAD, ROW_H, SECTION_H, SIDE, change_color, counts_text};
use crate::git_diff::{Column, Filter, GitDiff, Sel};
use crate::ui::file_icons::{file_glyph, paint_glyph};
use crate::ui::paint;
use crate::ui::theme::{ACCENT, Theme, white_alpha};
use crate::ui::widgets::{ScrollSpan, overlay_scrollbar};

impl GitDiff {
    /// The sidebar: Staged and Changes sections, one row per file.
    pub(super) fn list(&mut self, f: &Frame, p: &Painter, area: Rect, snap: &Snapshot, corner_sw: u8, corner_se: u8) {
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
        let d = self.view.list_pan.delta(ui, area);
        if d != 0.0 {
            self.list_scroll += d;
            self.view.list_active = ui.input(|i| i.time);
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
        let clamped = self.list_scroll.clamp(0.0, (content_h - area.height()).max(0.0));
        if clamped != self.list_scroll {
            self.view.list_pan.stop();
        }
        self.list_scroll = clamped;
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
        if let Some(to) = overlay_scrollbar(
            ui,
            p,
            area,
            f.id("list-bar"),
            ScrollSpan { offset: self.list_scroll, content: content_h, view: area.height() },
            &mut self.view.list_active,
        ) {
            self.list_scroll = to;
        }
    }
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
    p.text(Pos2::new(r.right() - SIDE, cy), Align2::RIGHT_CENTER, count.to_string(), FontId::proportional(11.0), c.text_dim.gamma_multiply(0.8));
}

/// A small triangle-ish chevron: pointing down when open, right when collapsed.
fn disclosure(p: &Painter, r: Rect, color: Color32, collapsed: bool) {
    let s = Stroke::new(1.4_f32, color);
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
            let g = p.layout_no_wrap("BIN".into(), FontId::proportional(9.5), c.text_dim);
            let at = Pos2::new(right - g.size().x, cy - g.size().y / 2.0);
            let w = g.size().x;
            p.galley(at, g, c.text_dim);
            right - w
        }
    };
    let room = (counts_left - 8.0 - x).max(0.0);
    let name_color = if selected { Color32::WHITE } else { c.text };
    let font = FontId::proportional(13.0);
    let name_w = p.layout_no_wrap(file.name().to_string(), font.clone(), name_color).size().x;
    let struck = file.change == Change::Deleted;
    paint::faded_text(p, Pos2::new(x, cy), file.name(), font, if struck { name_color.gamma_multiply(0.7) } else { name_color }, room, 18.0);
    if struck {
        let w = name_w.min(room);
        p.line_segment([Pos2::new(x, cy + 0.5), Pos2::new(x + w, cy + 0.5)], Stroke::new(1.0_f32, name_color.gamma_multiply(0.5)));
    }
    let folder = file.folder();
    if !folder.is_empty() && room - name_w > 28.0 {
        let fx = x + name_w + 7.0;
        let dim = if selected { white_alpha(150) } else { c.text_dim.gamma_multiply(0.85) };
        paint::faded_text(p, Pos2::new(fx, cy + 0.5), folder, FontId::proportional(11.0), dim, counts_left - 8.0 - fx, 18.0);
    }
}
