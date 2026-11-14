//! Small vector icons and glossy per-process app tiles (tab icons, pane header glyphs).

use egui::epaint::{PathShape, Shadow};
use egui::{Align2, Color32, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

use crate::layout::Axis;

use super::paint::{bold, top_radius};

/// Pseudo process name for file-explorer panes, so they get a folder tile and header glyph.
pub const EXPLORER_PROCESS: &str = "files";

#[derive(Clone, Copy)]
enum Kind {
    Shell,
    Node,
    Folder,
    Other,
}

fn kind(process: &str) -> Kind {
    match process {
        "bash" | "zsh" | "fish" | "sh" | "dash" | "nu" | "ksh" | "tcsh" | "elvish" | "xonsh" | "" => Kind::Shell,
        "node" | "nodejs" | "npm" | "npx" | "pnpm" | "yarn" | "deno" => Kind::Node,
        EXPLORER_PROCESS => Kind::Folder,
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
///
/// `r` may be narrower than it is tall (a card mid-flip); the glyph then shrinks and fades
/// with the squash instead of spilling past the tile's edges.
pub fn app_tile(p: &Painter, r: Rect, process: &str) {
    let radius = (r.width().min(r.height()) * 0.26) as u8;
    let k = kind(process);
    let base = match k {
        Kind::Shell => Color32::from_rgb(0x22, 0x22, 0x25),
        Kind::Folder => Color32::from_rgb(0x2f, 0x7c, 0xf6),
        _ => tile_color(process),
    };
    let shadow = Shadow { offset: [0, 1], blur: 3, spread: 0, color: Color32::from_black_alpha(90) };
    p.add(shadow.as_shape(r, radius));
    p.rect_filled(r, radius, base);
    // Gloss: lighter upper half, darker lower edge.
    let upper = Rect::from_min_max(r.min, Pos2::new(r.right(), r.center().y));
    p.rect_filled(upper, top_radius(radius), Color32::from_white_alpha(if is_light(base) { 40 } else { 22 }));
    p.rect_stroke(r, radius, Stroke::new(1.0, Color32::from_white_alpha(38)), StrokeKind::Inside);

    // The glyph lives in a centered square; a squashed tile shrinks and fades it.
    let squash = (r.width() / r.height()).clamp(0.0, 1.0);
    if squash < 0.05 {
        return;
    }
    let mut p = p.clone();
    p.multiply_opacity(squash * squash);
    let r = Rect::from_center_size(r.center(), Vec2::splat(r.width().min(r.height())));

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
        Kind::Node => hexagon(&p, r.center(), r.width() * 0.3, Stroke::new((r.width() * 0.08).max(1.2), fg), None),
        Kind::Folder => icon_folder(&p, Rect::from_center_size(r.center(), Vec2::new(r.width() * 0.6, r.height() * 0.48)), fg),
        Kind::Other => {
            let letter: String = process.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default();
            p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, letter, bold(r.height() * 0.58), fg);
        }
    }
}

/// Visible depth of the pane-icon deck; cards further back are hidden.
pub const STACK_DEPTH: f32 = 3.0;
/// How far each card behind the top one peeks out (right and up), in the title bar.
pub const STACK_STEP: Vec2 = Vec2::new(3.0, -1.5);
/// The same in the mobile sidebar: fanned out wider, so the deck's shuffle reads at a glance.
pub const SIDEBAR_STACK_STEP: Vec2 = Vec2::new(9.0, -6.0);

/// Placement of a resting card at fractional depth `t` (0 = top): (offset, scale, opacity).
fn stack_rest(t: f32, step: Vec2) -> (Vec2, f32, f32) {
    let mut alpha = 1.0 - 0.3 * t.min(2.0);
    if t > 2.0 {
        // Fade out entirely on the way to depth 3, so at most 3 cards ever show.
        alpha *= (STACK_DEPTH - t).max(0.0);
    }
    (step * t, 1.0 - 0.12 * t, alpha)
}

/// A deck of app tiles, one per pane, with the focused pane on top.
///
/// `cards` holds `(process, depth)` for every pane of the tab, where depth is a continuous
/// position in `[0, n)` (0 = top). A card between `n - 1` and `n` is mid-flip: travelling
/// between the top of the deck and the back, squashing like a card turned edge-on. Each card
/// behind the top one is offset by another `step`.
pub fn icon_stack(p: &Painter, r: Rect, cards: &[(&str, f32)], step: Vec2) {
    use std::f32::consts::PI;
    let n = cards.len() as f32;
    // (z-order key, rect, opacity, process); larger keys are painted first (further back).
    let mut draws: Vec<(f32, Rect, f32, &str)> = Vec::with_capacity(cards.len());
    for &(process, depth) in cards {
        let (offset, scale, alpha, width, z) = if n > 1.0 && depth > n - 1.0 {
            // u: 0 = on top, 1 = resting at the back.
            let u = (n - depth).clamp(0.0, 1.0);
            let (back_off, back_scale, back_alpha) = stack_rest(n - 1.0, step);
            let lerp = |a: f32, b: f32| a + (b - a) * u;
            // Swings out to the left as it flips, as far as the deck fans out to the right.
            let slide = Vec2::new(-1.33 * step.x * (PI * u).sin(), 0.0);
            let flip = (PI * u).cos().abs();
            // In front while turning away, behind once it has turned over.
            let z = if u < 0.5 { -1.0 } else { n };
            (back_off * u + slide, lerp(1.0, back_scale), lerp(1.0, back_alpha), flip, z)
        } else if depth < STACK_DEPTH {
            let (o, s, a) = stack_rest(depth, step);
            (o, s, a, 1.0, depth)
        } else {
            continue;
        };
        if alpha <= 0.01 {
            continue;
        }
        let size = r.size() * scale;
        let rect = Rect::from_center_size(r.center() + offset, Vec2::new(size.x * width, size.y));
        draws.push((z, rect, alpha, process));
    }
    draws.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, rect, alpha, process) in draws {
        let mut p = p.clone();
        p.multiply_opacity(alpha);
        app_tile(&p, rect, process);
    }
}

/// Small monochrome glyph used in pane headers (like SF Symbols in the reference).
pub fn header_glyph(p: &Painter, r: Rect, process: &str, color: Color32) {
    match kind(process) {
        Kind::Node => hexagon(p, r.center(), r.width() * 0.48, Stroke::NONE, Some(color)),
        Kind::Folder => icon_folder(p, Rect::from_center_size(r.center(), Vec2::new(r.width(), r.height() * 0.82)), color),
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

/// A filled folder: a tab on the upper left and a body below it.
pub fn icon_folder(p: &Painter, r: Rect, c: Color32) {
    let tab_h = (r.height() * 0.22).max(2.0);
    let tab = Rect::from_min_size(r.min, Vec2::new(r.width() * 0.45, tab_h + 2.0));
    p.rect_filled(tab, 1.5, c);
    let body = Rect::from_min_max(Pos2::new(r.left(), r.top() + tab_h), r.max);
    p.rect_filled(body, 2, c);
    // A faint highlight on the front flap gives it some depth.
    let flap = Rect::from_min_max(Pos2::new(body.left(), body.top() + body.height() * 0.3), body.max);
    p.rect_filled(flap, 2, Color32::from_white_alpha(28));
}

/// An outlined page with a folded top-right corner.
pub fn icon_file(p: &Painter, r: Rect, c: Color32) {
    let w = r.height() * 0.78;
    let r = Rect::from_center_size(r.center(), Vec2::new(w, r.height()));
    let fold = w * 0.35;
    let s = Stroke::new(1.2, c);
    let pts = vec![
        r.left_top(),
        Pos2::new(r.right() - fold, r.top()),
        Pos2::new(r.right(), r.top() + fold),
        r.right_bottom(),
        r.left_bottom(),
    ];
    p.add(Shape::closed_line(pts, s));
    p.add(Shape::line(
        vec![Pos2::new(r.right() - fold, r.top()), Pos2::new(r.right() - fold, r.top() + fold), Pos2::new(r.right(), r.top() + fold)],
        s,
    ));
}

/// An upward chevron (parent folder).
pub fn icon_chevron_up(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.5, c);
    let k = r.width() * 0.4;
    let ctr = r.center() + Vec2::new(0.0, k * 0.3);
    p.add(Shape::line(vec![ctr + Vec2::new(-k, 0.0), ctr + Vec2::new(0.0, -k), ctr + Vec2::new(k, 0.0)], s));
}

/// Three horizontal lines with leading dots (list view).
pub fn icon_list(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.3, c);
    for k in 0..3 {
        let y = r.top() + r.height() * (0.2 + 0.3 * k as f32);
        p.circle_filled(Pos2::new(r.left() + 1.5, y), 1.2, c);
        p.line_segment([Pos2::new(r.left() + 5.0, y), Pos2::new(r.right(), y)], s);
    }
}

/// A 2×2 block of rounded squares (grid view).
pub fn icon_grid(p: &Painter, r: Rect, c: Color32) {
    let gap = r.width() * 0.16;
    let cell = (r.width() - gap) / 2.0;
    for (x, y) in [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (1.0, 1.0)] {
        let min = r.min + Vec2::new(x * (cell + gap), y * (cell + gap));
        p.rect_stroke(Rect::from_min_size(min, Vec2::splat(cell)), 1.5, Stroke::new(1.2, c), StrokeKind::Inside);
    }
}

/// A round selection check: an outline when off, a filled disc with a tick when on.
pub fn icon_check(p: &Painter, r: Rect, c: Color32, on: bool) {
    let ctr = r.center();
    let rad = r.width().min(r.height()) / 2.0;
    if on {
        p.circle_filled(ctr, rad, c);
        let k = rad * 0.5;
        let s = Stroke::new(1.6, Color32::WHITE);
        p.add(Shape::line(vec![ctr + Vec2::new(-k, 0.0), ctr + Vec2::new(-k * 0.25, k * 0.7), ctr + Vec2::new(k, -k * 0.6)], s));
    } else {
        p.circle_filled(ctr, rad, Color32::from_black_alpha(60));
        p.circle_stroke(ctr, rad - 0.6, Stroke::new(1.2, c));
    }
}
