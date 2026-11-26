//! Small vector icons and glossy per-process app tiles (tab icons, pane header glyphs).

use egui::epaint::{PathShape, Shadow};
use std::sync::OnceLock;

use egui::{Align2, Color32, FontFamily, Painter, Pos2, Rect, Shape, Stroke, StrokeKind, Vec2};

use crate::layout::Axis;
use crate::render::FONT_ICONS;

use super::file_icons::{paint_glyph, paint_glyph_ink};
use super::glide::Lerp;
use super::paint::{bold, top_radius};

/// Pseudo process name for file-explorer panes, so they get a folder tile and header glyph.
pub const EXPLORER_PROCESS: &str = "files";
/// Pseudo process name for Changes (git diff) panes.
pub const DIFF_PROCESS: &str = "changes";
/// Pseudo process name for Agents panes.
pub const AGENTS_PROCESS: &str = "agents";
/// Pseudo process name for a pipeline (`nex-host pipe`) in the Agents pane.
pub const PIPELINE_PROCESS: &str = "pipeline";
/// The Git logo (Devicons' "git"), for Changes panes.
const GIT_LOGO: char = '\u{e702}';

#[derive(Clone, Copy)]
enum Kind {
    Shell,
    Node,
    Folder,
    Diff,
    Agents,
    Pipeline,
    Other,
}

fn kind(process: &str) -> Kind {
    match process {
        "bash" | "zsh" | "fish" | "sh" | "dash" | "nu" | "ksh" | "tcsh" | "elvish" | "xonsh" | "" => Kind::Shell,
        "node" | "nodejs" | "npm" | "npx" | "pnpm" | "yarn" | "deno" => Kind::Node,
        EXPLORER_PROCESS => Kind::Folder,
        DIFF_PROCESS => Kind::Diff,
        AGENTS_PROCESS => Kind::Agents,
        PIPELINE_PROCESS => Kind::Pipeline,
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
        "git" | "lazygit" | "tig" | "gitui" | DIFF_PROCESS => Color32::from_rgb(0xe8, 0x4e, 0x31),
        "claude" => Color32::from_rgb(0xd9, 0x77, 0x57),
        "codex" => Color32::from_rgb(0xf2, 0xf2, 0xf2),
        AGENTS_PROCESS => Color32::from_rgb(0x5e, 0x5c, 0xe6),
        PIPELINE_PROCESS => Color32::from_rgb(0x30, 0xa4, 0x8c),
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

/// Whether `c` reads as an icon rather than text: any non-ASCII symbol (Nerd Font and other
/// private-use glyphs, emoji, dingbats like Claude Code's ✳ spinner, braille spinners), or the
/// ASCII `*` some agents use as a spinner frame. Letters of any script are text.
fn is_icon(c: char) -> bool {
    c == '*' || (!c.is_ascii() && !c.is_alphanumeric() && !c.is_whitespace())
}

/// Code points that extend the icon before them: variation selectors, zero-width joiners,
/// skin tones and other combining marks (so `⚡️` or `👩‍💻` stay one icon).
fn extends_icon(c: char) -> bool {
    matches!(c, '\u{200d}' | '\u{fe00}'..='\u{fe0f}' | '\u{1f3fb}'..='\u{1f3ff}' | '\u{20d0}'..='\u{20ff}' | '\u{e0020}'..='\u{e007f}')
}

/// The icon leading a pane's title (Claude Code's `✳ Claude Code` or the spinner frame it
/// cycles through while working, a Nerd Font glyph, an emoji), and the title without it. The
/// icon takes the place of the tile's own glyph, animating as the program rewrites its title;
/// a title without one leaves the tile as it was. File explorers keep their folder.
pub fn title_mark<'a>(process: &str, title: &'a str) -> (Option<&'a str>, &'a str) {
    if matches!(kind(process), Kind::Folder | Kind::Diff | Kind::Agents | Kind::Pipeline) {
        return (None, title);
    }
    let mut it = title.char_indices();
    let Some((_, first)) = it.next().filter(|&(_, c)| is_icon(c)) else { return (None, title) };
    let mut end = first.len_utf8();
    let mut after_joiner = false;
    for (i, c) in it {
        if extends_icon(c) || (after_joiner && is_icon(c)) {
            after_joiner = c == '\u{200d}';
            end = i + c.len_utf8();
        } else {
            break;
        }
    }
    let rest = &title[end..];
    // An icon is set apart from the title by a space; `*bold*` or `★★★` are just text.
    if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
        return (None, title);
    }
    (Some(&title[..end]), rest.trim_start())
}

fn is_light(c: Color32) -> bool {
    (c.r() as u32 * 299 + c.g() as u32 * 587 + c.b() as u32 * 114) / 1000 > 170
}

/// A glossy, iOS-style app tile representing the process running in a tab.
///
/// `r` may be narrower than it is tall (a card mid-flip); the glyph then shrinks and fades
/// with the squash instead of spilling past the tile's edges.
/// `mark` (see [`title_mark`]) replaces the tile's own glyph. `phase` (see [`pipeline_phase`])
/// is where a running pipeline's glyph is in its cycle; `None` draws it at rest.
pub fn app_tile(p: &Painter, r: Rect, process: &str, mark: Option<&str>, phase: Option<f32>) {
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
    p.rect_stroke(r, radius, Stroke::new(1.0_f32, Color32::from_white_alpha(38)), StrokeKind::Inside);

    // The glyph lives in a centered square; a squashed tile shrinks and fades it.
    let squash = (r.width() / r.height()).clamp(0.0, 1.0);
    if squash < 0.05 {
        return;
    }
    let mut p = p.clone();
    p.multiply_opacity(squash * squash);
    let r = Rect::from_center_size(r.center(), Vec2::splat(r.width().min(r.height())));

    let fg = if is_light(base) { Color32::from_rgb(0x1c, 0x1c, 0x1e) } else { Color32::WHITE };
    if let Some(mark) = mark {
        p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, mark, bold(r.height() * 0.58), fg);
        return;
    }
    match k {
        Kind::Shell => {
            // ">_" prompt glyph.
            let s = r.width();
            let o = r.left_top();
            let stroke = Stroke::new((s * 0.09).max(1.3), Color32::from_rgb(0x6e, 0xe0, 0x8a));
            p.add(Shape::line(vec![o + Vec2::new(s * 0.24, s * 0.32), o + Vec2::new(s * 0.44, s * 0.5), o + Vec2::new(s * 0.24, s * 0.68)], stroke));
            p.line_segment(
                [o + Vec2::new(s * 0.5, s * 0.7), o + Vec2::new(s * 0.76, s * 0.7)],
                Stroke::new((s * 0.09).max(1.3), Color32::from_white_alpha(230)),
            );
        }
        Kind::Node => hexagon(&p, r.center(), r.width() * 0.3, Stroke::new((r.width() * 0.08).max(1.2), fg), None),
        Kind::Folder => icon_folder(&p, Rect::from_center_size(r.center(), Vec2::new(r.width() * 0.6, r.height() * 0.48)), fg),
        Kind::Diff => paint_glyph(&p, r.center(), r.width() * 0.56, GIT_LOGO, fg),
        Kind::Agents => sparkles(&p, r, fg),
        Kind::Pipeline => steps(&p, r, fg, phase),
        Kind::Other => {
            p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, initial(process), bold(r.height() * 0.58), fg);
        }
    }
}

/// The corner radius [`app_tile`] gives a tile `r`.
fn tile_radius(r: Rect) -> f32 {
    ((r.width().min(r.height()) * 0.26) as u8) as f32
}

/// The first `t` (0 to 1) of the polyline through `points`, by length.
fn polyline_prefix(points: &[Pos2], t: f32) -> Vec<Pos2> {
    let total: f32 = points.windows(2).map(|w| w[0].distance(w[1])).sum();
    let mut left = total * t.clamp(0.0, 1.0);
    let mut out = Vec::with_capacity(points.len());
    let Some(&first) = points.first() else { return out };
    out.push(first);
    for w in points.windows(2) {
        let d = w[0].distance(w[1]);
        if left >= d {
            out.push(w[1]);
            left -= d;
        } else {
            if left > 0.0 {
                out.push(w[0] + (w[1] - w[0]) * (left / d));
            }
            break;
        }
    }
    out
}

/// The outline of rounded rect `r` (corner radius `rad`), clockwise from the middle of its top
/// edge and back.
fn rounded_outline(r: Rect, rad: f32) -> Vec<Pos2> {
    use std::f32::consts::{FRAC_PI_2, PI};
    const STEPS: usize = 8;
    let rad = rad.min(r.width() / 2.0).min(r.height() / 2.0);
    let corners = [
        (Pos2::new(r.right() - rad, r.top() + rad), -FRAC_PI_2),
        (Pos2::new(r.right() - rad, r.bottom() - rad), 0.0),
        (Pos2::new(r.left() + rad, r.bottom() - rad), FRAC_PI_2),
        (Pos2::new(r.left() + rad, r.top() + rad), PI),
    ];
    let mut pts = vec![Pos2::new(r.center().x, r.top())];
    for (c, from) in corners {
        for i in 0..=STEPS {
            let a = from + FRAC_PI_2 * i as f32 / STEPS as f32;
            pts.push(c + Vec2::new(a.cos(), a.sin()) * rad);
        }
    }
    pts.push(Pos2::new(r.center().x, r.top()));
    pts
}

/// A thin progress ring just outside app tile `tile`, following its rounded corners: a faint
/// track, and `progress` (0 to 1) of it in `color`, clockwise from the top. `glow` (0 to 1)
/// lights its leading end softly, for a stage at work.
pub fn tile_ring(p: &Painter, tile: Rect, progress: f32, color: Color32, glow: f32) {
    let gap = (tile.width() * 0.09).max(2.5);
    let outline = rounded_outline(tile.expand(gap), tile_radius(tile) + gap);
    let width = (tile.width() * 0.05).clamp(1.5, 2.0);
    p.add(PathShape::line(outline.clone(), Stroke::new(width, Color32::from_white_alpha(16))));
    let arc = polyline_prefix(&outline, progress);
    if progress > 0.001 && arc.len() > 1 {
        p.add(PathShape::line(arc.clone(), Stroke::new(width, color)));
    }
    if glow > 0.01
        && let Some(&end) = arc.last()
    {
        for (r, a) in [(width * 3.2, 0.10), (width * 2.0, 0.22), (width * 1.0, 0.6)] {
            p.circle_filled(end, r, color.gamma_multiply(a * glow));
        }
    }
}

/// A check mark centred on `at`, `scale` times the 1-point design (about 8 points wide), its
/// stroke drawn in from the left `drawn` (0 to 1) of the way.
pub fn check(p: &Painter, at: Pos2, scale: f32, stroke: Stroke, drawn: f32) {
    let pts = [at + Vec2::new(-3.5, 0.2) * scale, at + Vec2::new(-1.0, 2.8) * scale, at + Vec2::new(3.8, -2.6) * scale];
    let shown = polyline_prefix(&pts, drawn);
    if shown.len() > 1 {
        p.add(Shape::line(shown, stroke));
    }
}

/// Where a tile's corner badge sits, and how big it is: on its top-left corner, like the
/// attention dot on a tab's icon.
fn badge_spot(tile: Rect) -> (Pos2, f32) {
    let r = (tile.width() * 0.2).max(5.5);
    (Pos2::new(tile.left() + r * 0.25, tile.top() + r * 0.25), r)
}

/// An iOS-style badge on app tile `tile`'s top-left corner: a disc of `fill` `scale` times its
/// size, `opacity` (0 to 1) in, with `breath` (0 to 1) swelling a soft halo round it. Returns
/// its centre and radius, for its glyph.
fn badge_disc(p: &Painter, tile: Rect, fill: Color32, scale: f32, opacity: f32, breath: f32) -> Option<(Pos2, f32)> {
    let (at, r) = badge_spot(tile);
    let r = r * scale * (1.0 + 0.04 * breath);
    if r <= 0.2 || opacity <= 0.01 {
        return None;
    }
    if breath > 0.0 {
        p.circle_filled(at, r * (1.25 + 0.25 * breath), fill.gamma_multiply(0.16 * breath * opacity));
    }
    p.circle_filled(at + Vec2::new(0.0, 0.5), r + 0.5, Color32::from_black_alpha((90.0 * opacity) as u8));
    p.circle_filled(at, r, fill.gamma_multiply(opacity));
    Some((at, r))
}

/// The amber `!` badge: something needs you. `breath` (0 to 1) is where its slow breath is.
pub fn badge_alert(p: &Painter, tile: Rect, amber: Color32, scale: f32, opacity: f32, breath: f32) {
    let Some((at, r)) = badge_disc(p, tile, amber, scale, opacity, breath) else { return };
    let ink = Color32::from_rgb(0x1c, 0x1c, 0x1e).gamma_multiply(opacity);
    let w = (r * 0.26).max(1.2);
    p.line_segment([at + Vec2::new(0.0, -r * 0.5), at + Vec2::new(0.0, r * 0.12)], Stroke::new(w, ink));
    p.circle_filled(at + Vec2::new(0.0, r * 0.45), w * 0.6, ink);
}

/// The green tick badge: it's done.
pub fn badge_done(p: &Painter, tile: Rect, green: Color32, scale: f32, opacity: f32) {
    let Some((at, r)) = badge_disc(p, tile, green, scale, opacity, 0.0) else { return };
    check(p, at + Vec2::new(0.0, -r * 0.05), r / 6.5, Stroke::new((r * 0.24).max(1.2), Color32::WHITE.gamma_multiply(opacity)), 1.0);
}

/// Visible depth of the pane-icon deck; cards further back are hidden.
pub const STACK_DEPTH: f32 = 3.0;
/// How far each card behind the top one peeks out (right and up), in the title bar.
pub const STACK_STEP: Vec2 = Vec2::new(3.0, -1.5);
/// The same in the sidebar: fanned out wider, so the deck's shuffle reads at a glance.
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
/// `cards` holds `(process, mark, phase, depth)` for every pane of the tab, where depth is a continuous
/// position in `[0, n)` (0 = top). A card between `n - 1` and `n` is mid-flip: travelling
/// between the top of the deck and the back, squashing like a card turned edge-on. Each card
/// behind the top one is offset by another `step`.
pub fn icon_stack(p: &Painter, r: Rect, cards: &[(&str, Option<&str>, Option<f32>, f32)], step: Vec2) {
    use std::f32::consts::PI;
    let n = cards.len() as f32;
    // (z-order key, rect, opacity, card); larger keys are painted first (further back).
    let mut draws: Vec<(f32, Rect, f32, usize)> = Vec::with_capacity(cards.len());
    for (i, &(_, _, _, depth)) in cards.iter().enumerate() {
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
        draws.push((z, rect, alpha, i));
    }
    draws.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, rect, alpha, i) in draws {
        let (process, mark, phase, _) = cards[i];
        let mut p = p.clone();
        p.multiply_opacity(alpha);
        app_tile(&p, rect, process, mark, phase);
    }
}

/// The process' initial, for tiles without a glyph of their own.
fn initial(process: &str) -> String {
    process.chars().next().map(|c| c.to_uppercase().collect()).unwrap_or_default()
}

/// Small monochrome glyph used in pane headers (like SF Symbols in the reference).
pub fn header_glyph(p: &Painter, r: Rect, process: &str, mark: Option<&str>, color: Color32) {
    if let Some(mark) = mark {
        p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, mark, bold(r.height() * 1.05), color);
        return;
    }
    match kind(process) {
        Kind::Node => hexagon(p, r.center(), r.width() * 0.48, Stroke::NONE, Some(color)),
        Kind::Folder => icon_folder(p, Rect::from_center_size(r.center(), Vec2::new(r.width(), r.height() * 0.82)), color),
        Kind::Diff => paint_glyph(p, r.center(), r.width() * 1.05, GIT_LOGO, color),
        Kind::Agents => sparkles(p, r.expand(r.width() * 0.2), color),
        Kind::Pipeline => steps(p, r.expand(r.width() * 0.15), color, None),
        Kind::Shell => {
            let rr = Rect::from_center_size(r.center(), Vec2::new(r.width(), r.height() * 0.82));
            p.rect_filled(rr, 3, color);
            let s = r.width();
            let o = rr.left_top();
            let bg = Color32::from_rgb(0x24, 0x20, 0x24);
            let stroke = Stroke::new(1.4_f32, bg);
            p.add(Shape::line(vec![o + Vec2::new(s * 0.22, s * 0.22), o + Vec2::new(s * 0.4, s * 0.4), o + Vec2::new(s * 0.22, s * 0.58)], stroke));
            p.line_segment([o + Vec2::new(s * 0.48, s * 0.6), o + Vec2::new(s * 0.75, s * 0.6)], stroke);
        }
        Kind::Other => {
            p.circle_filled(r.center(), r.width() * 0.5, color);
            let letter = initial(process);
            p.text(r.center() + Vec2::new(0.0, 0.5), Align2::CENTER_CENTER, letter, bold(r.height() * 0.62), Color32::from_rgb(0x24, 0x20, 0x24));
        }
    }
}

/// A four-pointed sparkle centered at `c`, `radius` to its tips: four slim kites meeting in
/// the middle (each one convex, so it fills with clean anti-aliased edges).
pub fn sparkle(p: &Painter, c: Pos2, radius: f32, color: Color32) {
    let waist = radius * 0.2;
    for i in 0..4 {
        let a = std::f32::consts::FRAC_PI_2 * i as f32;
        let dir = |a: f32, len: f32| c + Vec2::new(a.cos(), a.sin()) * len;
        let q = std::f32::consts::FRAC_PI_4;
        p.add(PathShape::convex_polygon(vec![c, dir(a - q, waist), dir(a, radius), dir(a + q, waist)], color, Stroke::NONE));
    }
}

/// The Agents glyph: a large sparkle with a small one at its upper right.
fn sparkles(p: &Painter, r: Rect, color: Color32) {
    let s = r.width().min(r.height());
    sparkle(p, r.center() + Vec2::new(-s * 0.06, s * 0.06), s * 0.3, color);
    sparkle(p, r.center() + Vec2::new(s * 0.22, -s * 0.22), s * 0.12, color);
}

/// How long a running pipeline's highlight takes to travel along its steps and back.
pub const PIPELINE_CYCLE: f64 = 2.2;

/// Where a running pipeline's glyph is in its cycle at `time` (seconds): 0 to 1.
pub fn pipeline_phase(time: f64) -> f32 {
    (time / PIPELINE_CYCLE).rem_euclid(1.0) as f32
}

/// The share of a cycle the highlight rests at each end (on the last dot at the turn, and on
/// the first as the cycle closes and the next opens).
const STEPS_HOLD: f32 = 0.05;

/// Where the highlight is along the steps (0 = first dot, 2 = last) at `phase`: out to the
/// last dot over the first half of the cycle and back over the second, eased each way, so one
/// cycle ends where the next begins.
fn steps_highlight(phase: f32) -> f32 {
    let phase = phase.clamp(0.0, 1.0);
    let half = (phase.min(1.0 - phase) * 2.0 - STEPS_HOLD * 2.0) / (1.0 - STEPS_HOLD * 4.0);
    2.0 * super::glide::ease_in_out(half)
}

/// How lit dot `i` is with the highlight at `pos`: 1 under it, easing to 0 a dot away.
fn steps_glow(pos: f32, i: usize) -> f32 {
    super::glide::smoothstep(1.0 - (pos - i as f32).abs())
}

/// A pipeline: three steps joined by a line, the first two done (filled), the last to come.
/// With a `phase` (it's running), a highlight travels along them and back: each dot in turn
/// swells and brightens, and the line is lit from the first dot up to it.
fn steps(p: &Painter, r: Rect, color: Color32, phase: Option<f32>) {
    let s = r.width().min(r.height());
    let y = r.center().y;
    let xs = [-0.27, 0.0, 0.27].map(|f| r.center().x + s * f);
    let radius = s * 0.085;
    let stroke = Stroke::new((s * 0.055).max(1.0), color);
    let Some(phase) = phase else {
        p.line_segment([Pos2::new(xs[0] + radius, y), Pos2::new(xs[2] - radius, y)], stroke);
        p.circle_filled(Pos2::new(xs[0], y), radius, color);
        p.circle_filled(Pos2::new(xs[1], y), radius, color);
        p.circle_stroke(Pos2::new(xs[2], y), radius, stroke);
        return;
    };
    let pos = steps_highlight(phase);
    let dim = color.gamma_multiply(0.55);
    // The line: dim, then lit from the first dot up to the highlight.
    p.line_segment([Pos2::new(xs[0] + radius, y), Pos2::new(xs[2] - radius, y)], Stroke::new(stroke.width, dim));
    let head = xs[0] + (xs[2] - xs[0]) * pos / 2.0;
    if head > xs[0] + radius {
        p.line_segment([Pos2::new(xs[0] + radius, y), Pos2::new(head.min(xs[2] - radius), y)], Stroke::new(stroke.width, color));
    }
    for (i, &x) in xs.iter().enumerate() {
        let glow = steps_glow(pos, i);
        let at = Pos2::new(x, y);
        let rad = radius * (1.0 + 0.3 * glow);
        let c = Color32::lerp(dim, color, glow.max(if i < 2 { 0.3 } else { 0.0 }));
        if i < 2 {
            p.circle_filled(at, rad, c);
        } else {
            // The step to come fills in while the highlight is on it.
            p.circle_stroke(at, rad, Stroke::new(stroke.width, c));
            if glow > 0.0 {
                p.circle_filled(at, rad, color.gamma_multiply(glow));
            }
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

/// Codicons (VS Code's icon set), from the embedded Nerd Font so the window chrome looks the
/// same whatever font the grid uses, on desktop and Android alike.
/// Each is drawn by its ink: the larger side of every one is this share of the icon rect, and
/// the ink is centered on it, so the header's buttons share one height and one center line.
const GLYPH_INK: f32 = 0.64;
const COD_ADD: char = '\u{ea60}';
const COD_CLOSE: char = '\u{ea76}';
const COD_MIC: char = '\u{ec12}';
const COD_MIC_FILLED: char = '\u{ec1c}';
/// An upright pin (the sticky pane). Codicons' plain `pin` (U+EB2B) is tilted.
const COD_PINNED: char = '\u{eba0}';
const COD_SCREEN_FULL: char = '\u{eb4c}';
const COD_SCREEN_NORMAL: char = '\u{eb4d}';
/// Two panes side by side (a vertical divider).
const COD_SPLIT_HORIZONTAL: char = '\u{eb56}';
/// Two panes stacked (a horizontal divider).
const COD_SPLIT_VERTICAL: char = '\u{eb57}';

/// The font the codicons come from. Made once: naming a family allocates, cloning one doesn't.
fn icons_family() -> FontFamily {
    static FAMILY: OnceLock<FontFamily> = OnceLock::new();
    FAMILY.get_or_init(|| FontFamily::Name(FONT_ICONS.into())).clone()
}

/// The ink size codicons are drawn at in icon rect `r`.
fn codicon_side(r: Rect) -> f32 {
    r.width().min(r.height()) * GLYPH_INK
}

fn codicon(p: &Painter, r: Rect, c: Color32, ch: char) {
    paint_glyph_ink(p, icons_family(), r.center(), codicon_side(r), ch, c);
}

pub fn icon_close(p: &Painter, r: Rect, c: Color32) {
    codicon(p, r, c, COD_CLOSE);
}

/// A microphone: filled while it's recording.
pub fn icon_mic(p: &Painter, r: Rect, c: Color32, filled: bool) {
    codicon(p, r, c, if filled { COD_MIC_FILLED } else { COD_MIC });
}

/// An upright pin. Pinned or not, it's the same pin: the button's colour tells them apart.
pub fn icon_pin(p: &Painter, r: Rect, c: Color32) {
    codicon(p, r, c, COD_PINNED);
}

pub fn icon_split(p: &Painter, r: Rect, c: Color32, axis: Axis) {
    let ch = match axis {
        Axis::Horizontal => COD_SPLIT_HORIZONTAL,
        Axis::Vertical => COD_SPLIT_VERTICAL,
    };
    codicon(p, r, c, ch);
}

pub fn icon_zoom(p: &Painter, r: Rect, c: Color32, zoomed: bool) {
    codicon(p, r, c, if zoomed { COD_SCREEN_NORMAL } else { COD_SCREEN_FULL });
}

pub fn icon_plus(p: &Painter, r: Rect, c: Color32) {
    codicon(p, r, c, COD_ADD);
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
    let s = Stroke::new(1.2_f32, c);
    let pts = vec![r.left_top(), Pos2::new(r.right() - fold, r.top()), Pos2::new(r.right(), r.top() + fold), r.right_bottom(), r.left_bottom()];
    p.add(Shape::closed_line(pts, s));
    p.add(Shape::line(
        vec![Pos2::new(r.right() - fold, r.top()), Pos2::new(r.right() - fold, r.top() + fold), Pos2::new(r.right(), r.top() + fold)],
        s,
    ));
}

/// An upward chevron (parent folder).
pub fn icon_chevron_up(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.5_f32, c);
    let k = r.width() * 0.4;
    let ctr = r.center() + Vec2::new(0.0, k * 0.3);
    p.add(Shape::line(vec![ctr + Vec2::new(-k, 0.0), ctr + Vec2::new(0.0, -k), ctr + Vec2::new(k, 0.0)], s));
}

/// A left-pointing chevron (Back).
pub fn icon_chevron_left(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.5_f32, c);
    let k = r.width() * 0.4;
    let ctr = r.center() + Vec2::new(-k * 0.3, 0.0);
    p.add(Shape::line(vec![ctr + Vec2::new(k * 0.75, -k), ctr + Vec2::new(-k * 0.25, 0.0), ctr + Vec2::new(k * 0.75, k)], s));
}

/// A right-pointing chevron (Forward).
pub fn icon_chevron_right(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.5_f32, c);
    let k = r.width() * 0.4;
    let ctr = r.center() + Vec2::new(k * 0.3, 0.0);
    p.add(Shape::line(vec![ctr + Vec2::new(-k * 0.75, -k), ctr + Vec2::new(k * 0.25, 0.0), ctr + Vec2::new(-k * 0.75, k)], s));
}

/// Three horizontal lines with leading dots (list view).
pub fn icon_list(p: &Painter, r: Rect, c: Color32) {
    let s = Stroke::new(1.3_f32, c);
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
        p.rect_stroke(Rect::from_min_size(min, Vec2::splat(cell)), 1.5, Stroke::new(1.2_f32, c), StrokeKind::Inside);
    }
}

/// An eye (an agent being viewed): an almond outline around its pupil, a filled one when
/// `on`, a ring when not.
pub fn icon_eye(p: &Painter, r: Rect, c: Color32, on: bool) {
    let ctr = r.center();
    let w = r.width() / 2.0;
    let h = r.height() * 0.36;
    const N: usize = 10;
    // Two arcs meeting at the corners: the upper lid left to right, the lower one back.
    let lid = |t: f32, sign: f32| {
        let x = -w + 2.0 * w * t;
        ctr + Vec2::new(x, sign * h * (1.0 - (x / w).powi(2)))
    };
    let mut pts: Vec<Pos2> = (0..=N).map(|i| lid(i as f32 / N as f32, -1.0)).collect();
    pts.extend((1..N).map(|i| lid(1.0 - i as f32 / N as f32, 1.0)));
    p.add(Shape::closed_line(pts, Stroke::new(1.3_f32, c)));
    let pupil = h * 0.62;
    if on {
        p.circle_filled(ctr, pupil, c);
    } else {
        p.circle_stroke(ctr, pupil - 0.4, Stroke::new(1.2_f32, c));
    }
}

/// A tree connector from a parent down to the row in `r` (an outline view's `└` when
/// `last`, else its `├`): a line down `r`'s left edge from the top, turning with a rounded
/// elbow into an arm out to `r`'s right edge at its middle; when not `last` the line also
/// carries on to the bottom, to the next row. `r` spans the row's height, so stacked rows'
/// connectors join into one line. Drawn on the device's pixel grid ([`branch_geometry`]).
pub fn icon_branch(p: &Painter, r: Rect, c: Color32, last: bool) {
    let b = branch_geometry(r, last, p.pixels_per_point());
    p.rect_filled(b.stem, 0, c);
    let clip = p.clip_rect().intersect(b.elbow_clip);
    p.with_clip_rect(clip).add(Shape::line(b.elbow, Stroke::new(b.width, c)));
}

/// A connector's line running on down `x` from `top` to `bottom`, past rows it doesn't
/// branch to: the same column, on the same pixels, as [`icon_branch`]'s at that `x`.
pub fn icon_branch_run(p: &Painter, x: f32, top: f32, bottom: f32, c: Color32) {
    p.rect_filled(branch_stem(x, top, bottom, p.pixels_per_point()), 0, c);
}

/// How thick a connector's line is: about 1.2 points, in whole device pixels.
pub fn branch_width(ppp: f32) -> f32 {
    (1.2 * ppp).round().max(1.0) / ppp
}

/// A connector's vertical line at `x` from `top` to `bottom`, as a rect whose every edge
/// is on a pixel boundary: the same `x` always lands on the same pixel column, and a row's
/// bottom and the next one's top (the same number) on the same boundary, so stacked pieces
/// meet with no gap, overlap or sideways step.
pub fn branch_stem(x: f32, top: f32, bottom: f32, ppp: f32) -> Rect {
    let w = branch_width(ppp);
    let left = snap(x - w / 2.0, ppp);
    Rect::from_min_max(Pos2::new(left, snap(top, ppp)), Pos2::new(left + w, snap(bottom, ppp)))
}

/// `v` on the nearest pixel boundary.
fn snap(v: f32, ppp: f32) -> f32 {
    (v * ppp).round() / ppp
}

/// [`icon_branch`]'s shapes, on the pixel grid at `ppp` pixels per point.
#[derive(Debug, Clone, PartialEq)]
pub struct BranchGeometry {
    /// The vertical line: down to where the elbow starts, or (a tee) on to the bottom.
    pub stem: Rect,
    /// The elbow and the arm out to the right, a stroke `width` thick.
    pub elbow: Vec<Pos2>,
    /// Where the elbow is drawn: off the stem's pixels, so the two never paint a pixel
    /// twice (the line is translucent, and a doubled pixel shows as a bright spot).
    pub elbow_clip: Rect,
    pub width: f32,
}

/// Works out [`icon_branch`] for `r`. The line's edges sit on pixel boundaries (its
/// centre snapped by its whole-pixel width, as [`branch_stem`]), the arm's too, and the
/// elbow's radius is whole pixels, so the curve leaves the line and meets the arm exactly.
/// (egui rounds a lone axis-aligned segment to pixels, but not a polyline, so the two
/// halves of a tee drawn that way stepped sideways by up to half a pixel.)
pub fn branch_geometry(r: Rect, last: bool, ppp: f32) -> BranchGeometry {
    let w = branch_width(ppp);
    let line = branch_stem(r.left(), r.top(), r.bottom(), ppp);
    let x = line.center().x;
    // The arm's centre: its edges on boundaries, as the line's are.
    let y = snap(r.center().y - w / 2.0, ppp) + w / 2.0;
    let rad = snap(5.0_f32.min(r.width()).min(r.height() / 2.0), ppp).max(1.0 / ppp);
    const N: usize = 6;
    // A quarter circle centred `rad` right of the line and `rad` above the arm.
    let arc = (0..=N).map(|i| {
        let a = std::f32::consts::FRAC_PI_2 * i as f32 / N as f32;
        Pos2::new(x + rad * (1.0 - a.cos()), y - rad * (1.0 - a.sin()))
    });
    let wide = r.expand(w);
    if last {
        // The line stops on the boundary at (or just above) where the curve starts, and the
        // curve picks up from there, cut off above it.
        let end = snap(y - rad - w / 2.0, ppp) + ((w * ppp).round() / 2.0).floor() / ppp;
        let stem = Rect::from_min_max(line.min, Pos2::new(line.right(), end.max(line.top())));
        let mut elbow = vec![Pos2::new(x, stem.bottom())];
        elbow.extend(arc.skip_while(|p| p.y <= stem.bottom() + 1e-3));
        elbow.push(Pos2::new(r.right(), y));
        BranchGeometry { stem, elbow, elbow_clip: Rect::from_min_max(Pos2::new(wide.left(), stem.bottom()), wide.max), width: w }
    } else {
        // The line runs the whole height; the curve is cut off where it overlaps it.
        let mut elbow: Vec<Pos2> = arc.collect();
        elbow.push(Pos2::new(r.right(), y));
        BranchGeometry { stem: line, elbow, elbow_clip: Rect::from_min_max(Pos2::new(line.right(), wide.top()), wide.max), width: w }
    }
}

/// A round selection check: an outline when off, a filled disc with a tick when on.
pub fn icon_check(p: &Painter, r: Rect, c: Color32, on: bool) {
    let ctr = r.center();
    let rad = r.width().min(r.height()) / 2.0;
    if on {
        p.circle_filled(ctr, rad, c);
        let k = rad * 0.5;
        let s = Stroke::new(1.6_f32, Color32::WHITE);
        p.add(Shape::line(vec![ctr + Vec2::new(-k, 0.0), ctr + Vec2::new(-k * 0.25, k * 0.7), ctr + Vec2::new(k, -k * 0.6)], s));
    } else {
        p.circle_filled(ctr, rad, Color32::from_black_alpha(60));
        p.circle_stroke(ctr, rad - 0.6, Stroke::new(1.2_f32, c));
    }
}

/// A keyboard: a rounded outline with two rows of keys and a space bar.
pub fn icon_keyboard(p: &Painter, r: Rect, c: Color32) {
    let w = r.width();
    let body = Rect::from_center_size(r.center(), Vec2::new(w, w * 0.64));
    p.rect_stroke(body, (w * 0.14) as u8, Stroke::new(1.5_f32, c), StrokeKind::Middle);

    // Keys: two rows of five, then a space bar, laid out inside the outline's padding.
    let pad = w * 0.16;
    let inner = body.shrink2(Vec2::new(pad, pad * 0.85));
    let key = w * 0.085;
    let row_gap = (inner.height() - key * 3.0) / 2.0;
    for row in 0..2 {
        let y = inner.top() + key / 2.0 + row as f32 * (key + row_gap);
        let step = (inner.width() - key) / 4.0;
        for col in 0..5 {
            let x = inner.left() + key / 2.0 + col as f32 * step;
            p.rect_filled(Rect::from_center_size(Pos2::new(x, y), Vec2::splat(key)), 1, c);
        }
    }
    let space_w = inner.width() * 0.56;
    let space = Rect::from_center_size(Pos2::new(inner.center().x, inner.bottom() - key / 2.0), Vec2::new(space_w, key));
    p.rect_filled(space, 1, c);
}

/// A keyboard with a small down chevron under it: put the keyboard away (as iOS's dismiss key).
pub fn icon_keyboard_hide(p: &Painter, r: Rect, c: Color32) {
    let w = r.width();
    let kb = Rect::from_center_size(r.center() - Vec2::new(0.0, w * 0.12), Vec2::splat(w * 0.92));
    icon_keyboard(p, kb, c);
    let k = w * 0.11;
    let tip = Pos2::new(r.center().x, r.center().y + w * 0.44);
    p.add(Shape::line(vec![tip + Vec2::new(-k * 1.3, -k), tip, tip + Vec2::new(k * 1.3, -k)], Stroke::new(1.5_f32, c)));
}

/// The Control key's mark (⌃): a wide, shallow caret.
pub fn icon_control(p: &Painter, r: Rect, c: Color32) {
    let w = r.width();
    let ctr = r.center() + Vec2::new(0.0, w * 0.06);
    let (dx, dy) = (w * 0.34, w * 0.26);
    let s = Stroke::new(1.8_f32, c);
    p.add(Shape::line(vec![ctr + Vec2::new(-dx, dy / 2.0), ctr + Vec2::new(0.0, -dy / 2.0), ctr + Vec2::new(dx, dy / 2.0)], s));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_polyline_is_cut_by_length() {
        let pts = [Pos2::new(0.0, 0.0), Pos2::new(10.0, 0.0), Pos2::new(10.0, 30.0)];
        assert_eq!(polyline_prefix(&pts, 0.0), vec![Pos2::ZERO]);
        assert_eq!(polyline_prefix(&pts, 0.125), vec![Pos2::ZERO, Pos2::new(5.0, 0.0)]);
        assert_eq!(polyline_prefix(&pts, 0.5), vec![Pos2::ZERO, Pos2::new(10.0, 0.0), Pos2::new(10.0, 10.0)]);
        assert_eq!(polyline_prefix(&pts, 1.0), pts.to_vec());
    }

    /// A tile's badge sits on its top-left corner, like a tab's attention dot, overhanging it a
    /// little and staying clear of the right side (where a card's status pill is).
    #[test]
    fn a_tiles_badge_sits_on_its_top_left_corner() {
        let tile = Rect::from_min_size(Pos2::new(14.0, 17.0), Vec2::splat(40.0));
        let (at, r) = badge_spot(tile);
        assert!(at.x - r < tile.left() && at.x + r < tile.center().x, "{at:?} r {r}");
        assert!(at.y - r < tile.top() && at.y < tile.center().y, "{at:?} r {r}");
        assert!(at.x - r > 0.0 && at.y - r > 0.0, "inside the card's padding");
    }

    /// The ring starts at the top's middle, goes clockwise (right first), and closes.
    #[test]
    fn a_tiles_ring_runs_clockwise_from_the_top() {
        let r = Rect::from_min_size(Pos2::ZERO, Vec2::splat(40.0));
        let o = rounded_outline(r, 10.0);
        assert_eq!((o[0], *o.last().unwrap()), (Pos2::new(20.0, 0.0), Pos2::new(20.0, 0.0)));
        assert!(o[1].x > 20.0 && o[1].y.abs() < 1e-4);
        // A quarter of the way round is the middle of the right edge.
        let q = *polyline_prefix(&o, 0.25).last().unwrap();
        assert!((q.x - 40.0).abs() < 1e-3 && (q.y - 20.0).abs() < 0.2, "{q:?}");
        assert!(o.iter().all(|p| r.expand(1e-3).contains(*p)));
    }

    /// The L leaves its line at the left edge and meets the right edge at the middle; a tee's
    /// line also runs on to the bottom.
    #[test]
    fn a_branch_is_an_l_and_a_tee_runs_on_down() {
        let r = Rect::from_min_max(Pos2::new(10.0, 0.0), Pos2::new(16.0, 46.0));
        let l = branch_geometry(r, true, 1.0);
        assert_eq!(l.stem, Rect::from_min_max(Pos2::new(10.0, 0.0), Pos2::new(11.0, 18.0)));
        assert_eq!((l.elbow[0], *l.elbow.last().unwrap()), (Pos2::new(10.5, 18.0), Pos2::new(16.0, 23.5)));
        assert!(l.elbow.windows(2).all(|w| w[1].x >= w[0].x && w[1].y >= w[0].y), "down, round, then out");
        let t = branch_geometry(r, false, 1.0);
        assert_eq!(t.stem, Rect::from_min_max(Pos2::new(10.0, 0.0), Pos2::new(11.0, 46.0)), "a tee runs on");
        assert_eq!(t.elbow_clip.left(), t.stem.right(), "its curve is cut off the line");
    }

    /// At any scale and any fractional place, a connector's edges are on pixel boundaries
    /// and its line whole pixels wide; stacked rows' lines (├ over ├ over └, with runs
    /// between) share one column and meet with no gap or overlap; the arm is at the row's
    /// middle to within half a pixel and the elbow lands on it exactly.
    #[test]
    fn connectors_sit_on_whole_pixels_and_stack_into_one_line() {
        let on_grid = |v: f32, ppp: f32| ((v * ppp) - (v * ppp).round()).abs() < 1e-3;
        for ppp in [1.0, 1.25, 1.5, 1.75, 2.0, 2.625, 2.75, 3.0, 3.5] {
            for (x0, y0) in [(10.0, 0.0), (37.3, 12.7), (121.45, 3.333), (0.19, 0.61)] {
                let rows = [(0.0, false), (1.0, false), (3.0, true)];
                let slot = |i: f32| Rect::from_min_max(Pos2::new(x0, y0 + i * 46.0), Pos2::new(x0 + 9.0, y0 + (i + 1.0) * 46.0));
                let b: Vec<BranchGeometry> = rows.iter().map(|&(i, last)| branch_geometry(slot(i), last, ppp)).collect();
                let run = branch_stem(x0, y0 + 2.0 * 46.0, y0 + 3.0 * 46.0, ppp);
                let w = b[0].width;
                assert!(on_grid(w, ppp) && w * ppp >= 1.0, "ppp {ppp}: width {w}");
                for g in &b {
                    let s = g.stem;
                    assert!([s.left(), s.right(), s.top(), s.bottom()].iter().all(|&v| on_grid(v, ppp)), "ppp {ppp} at {x0},{y0}: {s:?}");
                    assert!((s.width() - w).abs() < 1e-4);
                    assert_eq!((s.left(), s.right()), (b[0].stem.left(), b[0].stem.right()), "one column");
                    let arm = *g.elbow.last().unwrap();
                    assert!(on_grid(arm.y - w / 2.0, ppp) && on_grid(arm.y + w / 2.0, ppp), "arm edges on pixels: {arm:?}");
                    assert!((g.elbow[g.elbow.len() - 2].y - arm.y).abs() < 1e-4, "the curve ends on the arm");
                    assert!(g.elbow.windows(2).all(|w| w[1].x >= w[0].x - 1e-4 && w[1].y >= w[0].y - 1e-4));
                    assert!((g.elbow[0].x - s.center().x).abs() < 1e-4, "the curve leaves the line's centre");
                }
                assert_eq!((run.left(), run.right()), (b[0].stem.left(), b[0].stem.right()), "a run is the same column");
                // ├ meets ├, ├ meets the run, the run meets └: edge to edge.
                assert_eq!(b[0].stem.bottom(), b[1].stem.top());
                assert_eq!(b[1].stem.bottom(), run.top());
                assert_eq!(run.bottom(), b[2].stem.top());
                // └'s line stops where its curve starts, which is drawn from there on only.
                let l = &b[2];
                assert_eq!((l.elbow[0].y, l.elbow_clip.top()), (l.stem.bottom(), l.stem.bottom()));
                for (g, &(i, _)) in b.iter().zip(&rows) {
                    let mid = slot(i).center().y;
                    assert!((g.elbow.last().unwrap().y - mid).abs() <= 0.5 / ppp + 1e-4, "arm at the middle");
                }
            }
        }
    }

    /// A context whose icon font is the embedded one, at `ppp` pixels per point.
    fn icons_ctx(ppp: f32) -> egui::Context {
        let font = include_bytes!("../../assets/fonts/JetBrainsMonoNLNerdFontMono-Regular.ttf");
        let mut defs = egui::FontDefinitions::default();
        defs.font_data.insert("nerd".into(), std::sync::Arc::new(egui::FontData::from_static(font)));
        defs.families.insert(icons_family(), vec!["nerd".into()]);
        let ctx = egui::Context::default();
        ctx.set_fonts(defs);
        ctx.set_pixels_per_point(ppp);
        let _ = ctx.run(Default::default(), |_| {});
        ctx
    }

    #[test]
    fn the_pipeline_highlight_travels_the_steps_and_back() {
        assert_eq!(pipeline_phase(0.0), 0.0);
        assert!((pipeline_phase(PIPELINE_CYCLE * 2.5) - 0.5).abs() < 1e-5);
        assert!((0.0..1.0).contains(&pipeline_phase(-0.3)));
        // On the first dot at the start, the last at the midpoint, the first again at the end.
        assert_eq!(steps_highlight(0.0), 0.0);
        assert_eq!(steps_highlight(0.5), 2.0);
        assert_eq!(steps_highlight(1.0), 0.0);
        // So the loop is seamless: one cycle ends where the next begins.
        assert_eq!(steps_highlight(0.0), steps_highlight(1.0));
        assert!((steps_highlight(0.25) - 1.0).abs() < 1e-5);
        assert!((steps_highlight(0.75) - 1.0).abs() < 1e-5);
        // Forward over the first half, back over the second.
        let at = |i: usize| steps_highlight(i as f32 / 32.0);
        assert!((0..16).all(|i| at(i + 1) >= at(i)));
        assert!((16..32).all(|i| at(i + 1) <= at(i)));
        assert!(at(4) > at(0) && at(28) < at(20));
        // Each dot in turn is the lit one.
        for i in 0..3 {
            assert_eq!(steps_glow(i as f32, i), 1.0);
            assert_eq!(steps_glow(i as f32, (i + 1) % 3), 0.0);
        }
    }

    /// Every pane header icon, drawn in the same icon rect, has the same ink height and the
    /// same ink center: in the desktop's and mobile mode's rects, at 1x and 2x.
    #[test]
    fn header_icons_share_one_height_and_center() {
        use crate::ui::file_icons::{ink_placement, ink_ratio};
        let header = [COD_CLOSE, COD_SCREEN_FULL, COD_SCREEN_NORMAL, COD_SPLIT_VERTICAL, COD_SPLIT_HORIZONTAL, COD_MIC, COD_MIC_FILLED, COD_PINNED];
        for ppp in [1.0, 2.0] {
            let ctx = icons_ctx(ppp);
            // The desktop's icon rect (a 28px button less 4.5 each side) and mobile mode's (36, 7).
            for (side, center) in [(19.0, Pos2::new(100.0, 40.5)), (22.0, Pos2::new(300.25, 41.0))] {
                let r = Rect::from_center_size(center, Vec2::splat(side));
                let target = codicon_side(r);
                let mut heights = Vec::new();
                for ch in header {
                    let (galley, origin) = ink_placement(&ctx, icons_family(), r.center(), target, ch);
                    let size = galley.job.sections[0].format.font_id.size;
                    let what = format!("U+{:x} in {side}px at {ppp}x", ch as u32);
                    // The outline's own height, and the bitmap drawn: it can be one (anti-aliased)
                    // pixel taller where the outline's edges fall between pixels.
                    let ink = ink_ratio(&ctx, &icons_family(), ch);
                    let height = ink.height() * size;
                    assert!((height - (target * ppp).round() / ppp).abs() <= 0.5, "{what}: ink {height} tall, for {target}");
                    heights.push(height);
                    let drawn = galley.mesh_bounds.translate(origin.to_vec2());
                    assert!(
                        drawn.height() >= height - 0.5 && drawn.height() <= height + 1.0 / ppp + 0.5,
                        "{what}: drawn {drawn:?}, ink {height} tall"
                    );
                    assert!((drawn.center() - r.center()).length() <= 0.5 * 2f32.sqrt(), "{what}: drawn {drawn:?}, centered on {:?}", r.center());
                    // On the pixel grid.
                    for v in [drawn.min.x, drawn.min.y] {
                        assert!(((v * ppp).round() - v * ppp).abs() < 1e-3, "{what}: {drawn:?} is between pixels");
                    }
                }
                let (lo, hi) = heights.iter().fold((f32::MAX, f32::MIN), |(lo, hi), &h| (lo.min(h), hi.max(h)));
                assert!(hi - lo <= 0.5, "in {side}px at {ppp}x, ink heights {heights:?}");
            }
        }
    }

    #[test]
    fn leading_title_icon_moves_to_the_tile() {
        // Claude Code's idle mark and spinner frames.
        assert_eq!(title_mark("claude", "✳ Claude Code"), (Some("✳"), "Claude Code"));
        assert_eq!(title_mark("claude", "✻ Fixing tests"), (Some("✻"), "Fixing tests"));
        assert_eq!(title_mark("claude", "⠋ Working"), (Some("⠋"), "Working"));
        // Any icon, from any process: Nerd Font glyphs, emoji (with selectors and joiners).
        assert_eq!(title_mark("zsh", "\u{f07c} ~/code"), (Some("\u{f07c}"), "~/code"));
        assert_eq!(title_mark("nvim", "⚡\u{fe0f} main.rs"), (Some("⚡\u{fe0f}"), "main.rs"));
        assert_eq!(title_mark("codex", "👩\u{200d}💻 codex"), (Some("👩\u{200d}💻"), "codex"));
        assert_eq!(title_mark("claude", "✳"), (Some("✳"), ""));
    }

    #[test]
    fn titles_without_an_icon_are_untouched() {
        assert_eq!(title_mark("claude", "Claude Code"), (None, "Claude Code"));
        assert_eq!(title_mark("bash", "~/code"), (None, "~/code"));
        assert_eq!(title_mark("python", "Über script"), (None, "Über script"));
        assert_eq!(title_mark("bash", "★★★ rated"), (None, "★★★ rated"));
        assert_eq!(title_mark("bash", "*bold*"), (None, "*bold*"));
        // The explorer keeps its folder.
        assert_eq!(title_mark(super::EXPLORER_PROCESS, "✳ x"), (None, "✳ x"));
    }
}
