//! Painting primitives for the macOS / iOS-style chrome: the segmented tab track, glass
//! pills, pane cards, bold UI font and text that fades out instead of clipping.

use egui::epaint::Shadow;
use egui::text::{LayoutJob, TextFormat};
use egui::{Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

pub const UI_BOLD: &str = "ui-bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI_BOLD.into()))
}

/// Only the top corners rounded (for glass highlights on the upper half of a shape).
pub(super) fn top_radius(r: u8) -> CornerRadius {
    CornerRadius { nw: r, ne: r, sw: 0, se: 0 }
}

/// The recessed track that holds the tabs (like an iOS segmented control).
pub fn tab_track(p: &Painter, r: Rect) {
    p.rect_filled(r, 11, Color32::from_black_alpha(70));
    p.rect_stroke(r, 11, Stroke::new(1.0, Color32::from_white_alpha(12)), StrokeKind::Inside);
    // Inner shadow along the top edge sells the "recessed" look.
    p.line_segment(
        [Pos2::new(r.left() + 10.0, r.top() + 1.5), Pos2::new(r.right() - 10.0, r.top() + 1.5)],
        Stroke::new(1.0, Color32::from_black_alpha(60)),
    );
}

/// A raised glass pill: soft shadow, translucent fill, lighter upper half, hairline border and
/// a bright highlight along the top edge.
pub fn glass_pill(p: &Painter, r: Rect, radius: u8) {
    let shadow = Shadow { offset: [0, 1], blur: 6, spread: 0, color: Color32::from_black_alpha(110) };
    p.add(shadow.as_shape(r, radius));
    p.rect_filled(r, radius, Color32::from_rgba_unmultiplied(0x5a, 0x55, 0x5e, 150));
    let upper = Rect::from_min_max(r.min, Pos2::new(r.right(), r.center().y));
    p.rect_filled(upper, top_radius(radius), Color32::from_white_alpha(9));
    p.rect_stroke(r, radius, Stroke::new(1.0, Color32::from_white_alpha(34)), StrokeKind::Inside);
    let inset = radius as f32;
    p.line_segment(
        [Pos2::new(r.left() + inset, r.top() + 1.0), Pos2::new(r.right() - inset, r.top() + 1.0)],
        Stroke::new(1.0, Color32::from_white_alpha(46)),
    );
}

/// A pane card: drop shadow, translucent fill, hairline border and a faint glass edge on top.
/// `focus` (0 to 1) brightens the border and glass edge, so focus changes can fade.
pub fn card(p: &Painter, r: Rect, radius: u8, fill: Color32, focus: f32, accent: Option<Color32>) {
    let alpha = |from: f32, to: f32| (from + (to - from) * focus.clamp(0.0, 1.0)).round() as u8;
    let shadow = Shadow { offset: [0, 3], blur: 16, spread: 0, color: Color32::from_black_alpha(70) };
    p.add(shadow.as_shape(r, radius));
    p.rect_filled(r, radius, fill);
    let border = accent.unwrap_or(Color32::from_white_alpha(alpha(20.0, 30.0)));
    p.rect_stroke(r, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    let inset = radius as f32 + 2.0;
    // Skipped while a card is too narrow for it (mid split / close animation).
    if r.width() > inset * 2.0 {
        p.line_segment(
            [Pos2::new(r.left() + inset, r.top() + 1.5), Pos2::new(r.right() - inset, r.top() + 1.5)],
            Stroke::new(1.0, Color32::from_white_alpha(alpha(10.0, 16.0))),
        );
    }
}

/// Draw single-line text vertically centered at `left_center`, fading the characters out over
/// the last `fade` points before `max_width` instead of hard-clipping them.
pub fn faded_text(p: &Painter, left_center: Pos2, text: &str, font: FontId, color: Color32, max_width: f32, fade: f32) {
    let galley = p.layout_no_wrap(text.to_owned(), font.clone(), color);
    let h = galley.size().y;
    let pos = left_center - Vec2::new(0.0, h / 2.0);
    if galley.size().x <= max_width {
        p.galley(pos, galley, color);
        return;
    }
    let mut job = LayoutJob::default();
    if let Some(row) = galley.rows.first() {
        for g in &row.row.glyphs {
            let center = g.pos.x + g.advance_width / 2.0;
            if center > max_width {
                break;
            }
            let alpha = ((max_width - center) / fade).clamp(0.0, 1.0);
            job.append(
                &g.chr.to_string(),
                0.0,
                TextFormat { font_id: font.clone(), color: color.gamma_multiply(alpha * alpha), ..Default::default() },
            );
        }
    }
    let faded = p.layout_job(job);
    p.galley(pos, faded, color);
}
