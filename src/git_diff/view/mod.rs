//! Drawing the Changes pane, macOS style: a quiet toolbar (branch, a segmented filter,
//! refresh), a source-list sidebar of changed files, and the selected file's diff as rounded
//! hunk cards under a sticky file header. Mouse handling lives here too.
//!
//! Here: the layout (`paint`), the frame and helpers every part shares, and the empty and
//! error messages. `toolbar`, `list` (the sidebar of files) and `diff` (the selected file)
//! draw the three parts.

use std::sync::Arc;

use egui::epaint::{CornerRadius, Shadow};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, FontFamily, FontId, Id, Painter, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2};

use crate::render::FONT_REGULAR;
use crate::ui::file_icons::{file_glyph, paint_glyph};
use crate::ui::glide::Glide;
use crate::ui::theme::{ACCENT, HAIRLINE, HOVER, Theme, white_alpha};
use crate::ui::touch_pan::TouchPan;
use crate::ui::widgets::{HoverShape, IconButton, ScrollSpan, overlay_scrollbar};
use crate::ui::{icons, paint};

use super::git::{Change, FileChange, FileDiff, Hunk, Line, LineKind, Snapshot, Status};
use super::syntax::Token;
use super::{Column, Filter, GitDiff, Sel};

mod diff;
mod list;
mod toolbar;

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
    /// Scrolling the file list, and the diff, with a finger.
    list_pan: TouchPan,
    diff_pan: TouchPan,
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
            list_pan: TouchPan::default(),
            diff_pan: TouchPan::default(),
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
            Some(Status::NeedsLogin) => {
                let host = self.remote.as_ref().map_or_else(|| "the host".into(), |t| t.label());
                message(painter, body, c, Glyph::Warning, "Couldn't Log In", &format!("nex can't log in to {host} without a password."));
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
            painter.line_segment([list.right_top(), list.right_bottom()], Stroke::new(1.0_f32, HAIRLINE));
            self.diff_column(&f, painter, diff, snap, false);
        }
    }

}

/// The list's width beside the diff.
fn sidebar_width(total: f32) -> f32 {
    (total * 0.3).clamp(210.0, 320.0)
}

fn pressed_in(ui: &Ui, r: Rect) -> bool {
    crate::ui::widgets::pressed_in(ui, r)
}

/// The card color without its transparency (sticky headers hide what scrolls beneath).
fn opaque(c: Color32) -> Color32 {
    let [r, g, b, _] = c.to_srgba_unmultiplied();
    Color32::from_rgb(r, g, b)
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
            let s = Stroke::new(2.4_f32, c.green);
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
    p.add(Shape::line(pts, Stroke::new(1.6_f32, color)));
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
