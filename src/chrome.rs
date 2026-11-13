//! Visual building blocks for the macOS / iOS-style window chrome: the segmented tab track,
//! glass pills, glossy app tiles, pane cards and small vector glyphs.

use egui::epaint::{PathShape, Shadow};
use egui::text::{LayoutJob, TextFormat};
use egui::{Align2, Color32, CornerRadius, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

use crate::layout::Axis;

pub const UI_BOLD: &str = "ui-bold";

pub fn bold(size: f32) -> FontId {
    FontId::new(size, FontFamily::Name(UI_BOLD.into()))
}

/// Only the top corners rounded (for glass highlights on the upper half of a shape).
fn top_radius(r: u8) -> CornerRadius {
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
pub fn card(p: &Painter, r: Rect, radius: u8, fill: Color32, focused: bool, accent: Option<Color32>) {
    let shadow = Shadow { offset: [0, 3], blur: 16, spread: 0, color: Color32::from_black_alpha(70) };
    p.add(shadow.as_shape(r, radius));
    p.rect_filled(r, radius, fill);
    let border = accent.unwrap_or(Color32::from_white_alpha(if focused { 30 } else { 20 }));
    p.rect_stroke(r, radius, Stroke::new(1.0, border), StrokeKind::Inside);
    let inset = radius as f32 + 2.0;
    p.line_segment(
        [Pos2::new(r.left() + inset, r.top() + 1.5), Pos2::new(r.right() - inset, r.top() + 1.5)],
        Stroke::new(1.0, Color32::from_white_alpha(if focused { 16 } else { 10 })),
    );
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

#[derive(Clone, Copy)]
enum Kind {
    Shell,
    Node,
    Other,
}

fn kind(process: &str) -> Kind {
    match process {
        "bash" | "zsh" | "fish" | "sh" | "dash" | "nu" | "ksh" | "tcsh" | "elvish" | "xonsh" | "" => Kind::Shell,
        "node" | "nodejs" | "npm" | "npx" | "pnpm" | "yarn" | "deno" => Kind::Node,
        _ => Kind::Other,
    }
}

/// Base color for a process' tile.
fn tile_color(process: &str) -> Color32 {
    match process {
        "node" | "nodejs" | "npm" | "npx" | "pnpm" | "yarn" => Color32::from_rgb(0x3c, 0x87, 0x3a),
        "deno" => Color32::from_rgb(0x30, 0x30, 0x34),
        "bun" => Color32::from_rgb(0xf4, 0xe6, 0xc9),
        "python" | "python3" | "ipython" => Color32::from_rgb(0x35, 0x72, 0xa5),
        "nvim" | "vim" | "vi" => Color32::from_rgb(0x4e, 0x9a, 0x3e),
        "git" | "lazygit" | "tig" | "gitui" => Color32::from_rgb(0xe8, 0x4e, 0x31),
        "claude" => Color32::from_rgb(0xd9, 0x77, 0x57),
        "codex" => Color32::from_rgb(0xf2, 0xf2, 0xf2),
        "cargo" | "rustc" => Color32::from_rgb(0xb7, 0x41, 0x0e),
        "go" => Color32::from_rgb(0x00, 0xad, 0xd8),
        "htop" | "btop" | "top" => Color32::from_rgb(0x6d, 0x4a, 0xd6),
        "ssh" | "mosh" => Color32::from_rgb(0x25, 0x63, 0xeb),
        "docker" => Color32::from_rgb(0x1d, 0x63, 0xed),
        _ => {
            let hash = process.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
            const PALETTE: [Color32; 6] = [
                Color32::from_rgb(0x3b, 0x82, 0xf6),
                Color32::from_rgb(0x10, 0xb9, 0x81),
                Color32::from_rgb(0xf5, 0x9e, 0x0b),
                Color32::from_rgb(0xec, 0x48, 0x99),
                Color32::from_rgb(0x8b, 0x5c, 0xf6),
                Color32::from_rgb(0x06, 0xb6, 0xd4),
            ];
            PALETTE[hash as usize % PALETTE.len()]
        }
    }
}

fn is_light(c: Color32) -> bool {
    (c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114) / 1000 > 170
}

/// A glossy, iOS-style app tile representing the process running in a tab.
pub fn app_tile(p: &Painter, r: Rect, process: &str) {
    let radius = (r.width() * 0.26) as u8;
    let k = kind(process);
    let base = match k {
        Kind::Shell => Color32::from_rgb(0x22, 0x22, 0x25),
        _ => tile_color(process),
    };
    let shadow = Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_black_alpha(90) };
    p.add(shadow.as_shape(r, radius));
    p.rect_filled(r, radius, base);
    // Gloss: lighter upper half, darker lower edge.
    let upper = Rect::from_min_max(r.min, Pos2::new(r.right(), r.center().y));
    p.rect_filled(upper, top_radius(radius), Color32::from_white_alpha(if is_light(base) { 40 } else { 22 }));
    p.rect_stroke(r, radius, Stroke::new(1.0, Color32::from_white_alpha(38)), StrokeKind::Inside);

    let fg = if is_light(base) { Color32::from_rgb(0x1c, 0x1c, 0x1e) } else { Color32::WHITE };
    match k {
        Kind::Shell => {
            // ">_" prompt glyph.
            let s = r.width();
            let o = r.left_top();
            let stroke = Stroke::new((s * 0.09).max(1.3), Color32::from_rgb(0x6e, 0xe0, 0x8a));
            p.add(Shape::line(
                vec![o + Vec2::new(s * 0.24, s * 0.32), o + Vec2::new(s * 0.44, s * 0.5), o + Vec2::new(s * 0.24, s * 0.68)],
                stroke,
            ));
            p.line_segment(
                [o + Vec2::new(s * 0.5, s * 0.7), o + Vec2::new(s * 0.76, s * 0.7)],
                Stroke::new((s * 0.09).max(1.3), Color32::from_white_alpha(230)),
            );
        }
        Kind::Node => hexagon(p, r.center(), r.width() * 0.3, Stroke::new((r.width() * 0.08).max(1.2), fg), None),
        Kind::Other => {
            let letter: String = process.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, letter, bold(r.height() * 0.58), fg);
        }
    }
}

/// Small monochrome glyph used in pane headers (like SF Symbols in the reference).
pub fn header_glyph(p: &Painter, r: Rect, process: &str, color: Color32) {
    match kind(process) {
        Kind::Node => hexagon(p, r.center(), r.width() * 0.48, Stroke::NONE, Some(color)),
        Kind::Shell => {
            let rr = Rect::from_center_size(r.center(), Vec2::new(r.width(), r.height() * 0.82));
            p.rect_filled(rr, 3, color);
            let s = r.width();
            let o = rr.left_top();
            let bg = Color32::from_rgb(0x24, 0x20, 0x24);
            let stroke = Stroke::new(1.4, bg);
            p.add(Shape::line(
                vec![o + Vec2::new(s * 0.22, s * 0.22), o + Vec2::new(s * 0.4, s * 0.4), o + Vec2::new(s * 0.22, s * 0.58)],
                stroke,
            ));
            p.line_segment([o + Vec2::new(s * 0.48, s * 0.6), o + Vec2::new(s * 0.75, s * 0.6)], stroke);
        }
        Kind::Other => {
            p.circle_filled(r.center(), r.width() * 0.5, color);
            let letter: String = process.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, letter, bold(r.height() * 0.62), Color32::from_rgb(0x24, 0x20, 0x24));
        }
    }
}

fn hexagon(p: &Painter, c: Pos2, radius: f32, stroke: Stroke, fill: Option<Color32>) {
    let pts: Vec<Pos2> = (0..6)
        .map(|i| {
            let a = std::f32::consts::FRAC_PI_3 * i as f32 + std::f32::consts::FRAC_PI_6;
            c + Vec2::new(a.cos(), a.sin()) * radius
        })
        .collect();
    match fill {
        Some(f) => p.add(Shape::Path(PathShape::convex_polygon(pts, f, stroke))),
        None => p.add(Shape::closed_line(pts, stroke)),
    };
}

/// A small display/workspace glyph for the title bar label.
pub fn workspace_glyph(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.3, c);
    let screen = Rect::from_min_max(r.min, Pos2::new(r.right(), r.bottom() - 3.0));
    p.rect_stroke(screen, 2, s, StrokeKind::Inside);
    p.line_segment([Pos2::new(r.center().x - 3.0, r.bottom()), Pos2::new(r.center().x + 3.0, r.bottom())], s);
    p.circle_filled(Pos2::new(screen.right() - 3.5, screen.center().y), 1.0, c);
}

pub fn icon_close(p: &Painter, r: Rect, c: Color32) {
    let r = r.shrink(r.width() * 0.2);
    let s = Stroke::new(1.3, c);
    p.line_segment([r.left_top(), r.right_bottom()], s);
    p.line_segment([r.right_top(), r.left_bottom()], s);
}

pub fn icon_split(p: &Painter, r: Rect, c: Color32, axis: Axis) {
    let r = Rect::from_center_size(r.center(), Vec2::new(r.width(), r.height() * 0.84));
    let s = Stroke::new(1.2, c);
    p.rect_stroke(r, 2.5, s, StrokeKind::Inside);
    match axis {
        Axis::Horizontal => p.line_segment([r.center_top(), r.center_bottom()], s),
        Axis::Vertical => p.line_segment([r.left_center(), r.right_center()], s),
    };
}

pub fn icon_zoom(p: &Painter, r: Rect, c: Color32, zoomed: bool) {
    let s = Stroke::new(1.2, c);
    let k = r.width() * 0.38;
    // Two corner arrows pointing outward (or inward when already zoomed).
    for (corner, dir) in [(r.left_bottom(), Vec2::new(1.0, -1.0)), (r.right_top(), Vec2::new(-1.0, 1.0))] {
        let (tip, tail) = if zoomed { (corner + dir * k, corner) } else { (corner, corner + dir * k) };
        p.line_segment([tip, tail], s);
        let away = (tail - tip).normalized();
        let h = k * 0.75;
        p.line_segment([tip, tip + Vec2::new(away.x * h, 0.0)], s);
        p.line_segment([tip, tip + Vec2::new(0.0, away.y * h)], s);
    }
}

pub fn icon_plus(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.5, c);
    let ctr = r.center();
    let k = r.width() * 0.32;
    p.line_segment([ctr - Vec2::new(k, 0.0), ctr + Vec2::new(k, 0.0)], s);
    p.line_segment([ctr - Vec2::new(0.0, k), ctr + Vec2::new(0.0, k)], s);
}
