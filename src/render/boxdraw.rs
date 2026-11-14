//! Procedurally drawn box-drawing, block and Powerline glyphs.
//!
//! Font glyphs for these rarely line up across cells (especially with extra line spacing),
//! leaving gaps in TUI borders. Like Ghostty, we draw them as geometry filling the cell exactly.

use egui::epaint::{PathShape, QuadraticBezierShape};
use egui::{Color32, Pos2, Rect, Shape, Stroke, Vec2};

/// Line weight per arm: 0 none, 1 light, 2 heavy, 3 double.
type Arms = [u8; 4]; // left, right, up, down

fn arms(c: char) -> Option<(Arms, bool /* dashed */)> {
    let a = match c {
        '─' => [1, 1, 0, 0],
        '━' => [2, 2, 0, 0],
        '│' => [0, 0, 1, 1],
        '┃' => [0, 0, 2, 2],
        '┌' => [0, 1, 0, 1],
        '┐' => [1, 0, 0, 1],
        '└' => [0, 1, 1, 0],
        '┘' => [1, 0, 1, 0],
        '├' => [0, 1, 1, 1],
        '┤' => [1, 0, 1, 1],
        '┬' => [1, 1, 0, 1],
        '┴' => [1, 1, 1, 0],
        '┼' => [1, 1, 1, 1],
        '┏' => [0, 2, 0, 2],
        '┓' => [2, 0, 0, 2],
        '┗' => [0, 2, 2, 0],
        '┛' => [2, 0, 2, 0],
        '┣' => [0, 2, 2, 2],
        '┫' => [2, 0, 2, 2],
        '┳' => [2, 2, 0, 2],
        '┻' => [2, 2, 2, 0],
        '╋' => [2, 2, 2, 2],
        '═' => [3, 3, 0, 0],
        '║' => [0, 0, 3, 3],
        '╔' => [0, 3, 0, 3],
        '╗' => [3, 0, 0, 3],
        '╚' => [0, 3, 3, 0],
        '╝' => [3, 0, 3, 0],
        '╠' => [0, 3, 3, 3],
        '╣' => [3, 0, 3, 3],
        '╦' => [3, 3, 0, 3],
        '╩' => [3, 3, 3, 0],
        '╬' => [3, 3, 3, 3],
        '╴' => [1, 0, 0, 0],
        '╵' => [0, 0, 1, 0],
        '╶' => [0, 1, 0, 0],
        '╷' => [0, 0, 0, 1],
        '╸' => [2, 0, 0, 0],
        '╹' => [0, 0, 2, 0],
        '╺' => [0, 2, 0, 0],
        '╻' => [0, 0, 0, 2],
        '┄' | '┈' | '╌' => return Some(([1, 1, 0, 0], true)),
        '┅' | '┉' | '╍' => return Some(([2, 2, 0, 0], true)),
        '┆' | '┊' | '╎' => return Some(([0, 0, 1, 1], true)),
        '┇' | '┋' | '╏' => return Some(([0, 0, 2, 2], true)),
        _ => return None,
    };
    Some((a, false))
}

/// Draw `c` into `cell` if it's a glyph we render procedurally. Returns false otherwise.
pub fn draw(c: char, cell: Rect, color: Color32, ppp: f32, out: &mut Vec<Shape>) -> bool {
    let snap = |v: f32| (v * ppp).round() / ppp;
    let px = 1.0 / ppp;
    let light = (snap(cell.width() / 9.0)).max(px);
    let heavy = light * 2.0;
    let cx = snap(cell.center().x - light / 2.0) + light / 2.0;
    let cy = snap(cell.center().y - light / 2.0) + light / 2.0;

    if let Some((a, dashed)) = arms(c) {
        let [l, r, u, d] = a;
        let thick = |w: u8| if w == 2 { heavy } else { light };
        // Horizontal arms.
        for (w, x0, x1) in [(l, cell.left(), cx), (r, cx, cell.right())] {
            if w == 0 {
                continue;
            }
            // Extend into the center so corners join cleanly.
            let ext = thick(u.max(d).min(2)) / 2.0;
            let (x0, x1) = if x0 < cx { (x0, x1 + ext) } else { (x0 - ext, x1) };
            let offsets: &[f32] = if w == 3 { &[-light, light] } else { &[0.0] };
            for off in offsets {
                let t = thick(w);
                let rect = Rect::from_min_max(Pos2::new(x0, cy + off - t / 2.0), Pos2::new(x1, cy + off + t / 2.0));
                push_line(out, rect, color, dashed, true);
            }
        }
        // Vertical arms.
        for (w, y0, y1) in [(u, cell.top(), cy), (d, cy, cell.bottom())] {
            if w == 0 {
                continue;
            }
            let ext = thick(l.max(r).min(2)) / 2.0;
            let (y0, y1) = if y0 < cy { (y0, y1 + ext) } else { (y0 - ext, y1) };
            let offsets: &[f32] = if w == 3 { &[-light, light] } else { &[0.0] };
            for off in offsets {
                let t = thick(w);
                let rect = Rect::from_min_max(Pos2::new(cx + off - t / 2.0, y0), Pos2::new(cx + off + t / 2.0, y1));
                push_line(out, rect, color, dashed, false);
            }
        }
        return true;
    }

    // Rounded corners.
    let center = Pos2::new(cx, cy);
    let rounded = match c {
        '╭' => Some((Pos2::new(cell.right(), cy), Pos2::new(cx, cell.bottom()))),
        '╮' => Some((Pos2::new(cell.left(), cy), Pos2::new(cx, cell.bottom()))),
        '╯' => Some((Pos2::new(cell.left(), cy), Pos2::new(cx, cell.top()))),
        '╰' => Some((Pos2::new(cell.right(), cy), Pos2::new(cx, cell.top()))),
        _ => None,
    };
    if let Some((a, b)) = rounded {
        out.push(Shape::QuadraticBezier(QuadraticBezierShape::from_points_stroke(
            [a, center, b],
            false,
            Color32::TRANSPARENT,
            Stroke::new(light, color),
        )));
        return true;
    }

    // Block elements.
    let frac = |x0: f32, y0: f32, x1: f32, y1: f32| {
        Rect::from_min_max(
            Pos2::new(snap(cell.left() + cell.width() * x0), snap(cell.top() + cell.height() * y0)),
            Pos2::new(snap(cell.left() + cell.width() * x1), snap(cell.top() + cell.height() * y1)),
        )
    };
    let fill = |out: &mut Vec<Shape>, r: Rect, col: Color32| out.push(Shape::rect_filled(r, 0.0, col));
    match c {
        '█' => fill(out, frac(0.0, 0.0, 1.0, 1.0), color),
        '▀' => fill(out, frac(0.0, 0.0, 1.0, 0.5), color),
        '▔' => fill(out, frac(0.0, 0.0, 1.0, 0.125), color),
        '▕' => fill(out, frac(0.875, 0.0, 1.0, 1.0), color),
        '▐' => fill(out, frac(0.5, 0.0, 1.0, 1.0), color),
        '▁'..='▇' => {
            // Lower one-eighth .. seven-eighths.
            let n = (c as u32 - '▁' as u32 + 1) as f32 / 8.0;
            fill(out, frac(0.0, 1.0 - n, 1.0, 1.0), color);
        }
        '▉'..='▏' => {
            // Left seven-eighths .. one-eighth.
            let n = (8 - (c as u32 - '▉' as u32 + 1)) as f32 / 8.0;
            fill(out, frac(0.0, 0.0, n, 1.0), color);
        }
        '░' => fill(out, frac(0.0, 0.0, 1.0, 1.0), color.gamma_multiply(0.25)),
        '▒' => fill(out, frac(0.0, 0.0, 1.0, 1.0), color.gamma_multiply(0.5)),
        '▓' => fill(out, frac(0.0, 0.0, 1.0, 1.0), color.gamma_multiply(0.75)),
        '▖' | '▗' | '▘' | '▙' | '▚' | '▛' | '▜' | '▝' | '▞' | '▟' => {
            // Quadrants: bits = upper-left, upper-right, lower-left, lower-right.
            let bits: [bool; 4] = match c {
                '▖' => [false, false, true, false],
                '▗' => [false, false, false, true],
                '▘' => [true, false, false, false],
                '▝' => [false, true, false, false],
                '▙' => [true, false, true, true],
                '▚' => [true, false, false, true],
                '▛' => [true, true, true, false],
                '▜' => [true, true, false, true],
                '▞' => [false, true, true, false],
                _ => [false, true, true, true], // ▟
            };
            let quads = [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5)];
            for (on, (x, y)) in bits.into_iter().zip(quads) {
                if on {
                    fill(out, frac(x, y, x + 0.5, y + 0.5), color);
                }
            }
        }
        // Braille patterns (used for graphs by btop, gping, etc.): 2×4 dots.
        '\u{2801}'..='\u{28FF}' => {
            let bits = c as u32 - 0x2800;
            // Unicode dot numbering → (column, row) for bits 0..8.
            const DOTS: [(f32, f32); 8] = [(0., 0.), (0., 1.), (0., 2.), (1., 0.), (1., 1.), (1., 2.), (0., 3.), (1., 3.)];
            let size = snap((cell.width() / 4.0).min(cell.height() / 8.0) * 1.15).max(px);
            for (i, (col, row)) in DOTS.iter().enumerate() {
                if bits & (1 << i) != 0 {
                    let center = Pos2::new(
                        cell.left() + cell.width() * (0.25 + 0.5 * col),
                        cell.top() + cell.height() * (0.125 + 0.25 * row),
                    );
                    let min = Pos2::new(snap(center.x - size / 2.0), snap(center.y - size / 2.0));
                    fill(out, Rect::from_min_size(min, Vec2::splat(size)), color);
                }
            }
        }
        '\u{2800}' => {} // blank braille
        // Powerline separators.
        '\u{E0B0}' | '\u{E0B2}' => {
            let pts = if c == '\u{E0B0}' {
                vec![cell.left_top(), Pos2::new(cell.right(), cell.center().y), cell.left_bottom()]
            } else {
                vec![cell.right_top(), Pos2::new(cell.left(), cell.center().y), cell.right_bottom()]
            };
            out.push(Shape::Path(PathShape::convex_polygon(pts, color, Stroke::NONE)));
        }
        '\u{E0B1}' | '\u{E0B3}' => {
            let pts = if c == '\u{E0B1}' {
                vec![cell.left_top(), Pos2::new(cell.right(), cell.center().y), cell.left_bottom()]
            } else {
                vec![cell.right_top(), Pos2::new(cell.left(), cell.center().y), cell.right_bottom()]
            };
            out.push(Shape::line(pts, Stroke::new(light, color)));
        }
        _ => return false,
    }
    true
}

fn push_line(out: &mut Vec<Shape>, rect: Rect, color: Color32, dashed: bool, horizontal: bool) {
    if !dashed {
        out.push(Shape::rect_filled(rect, 0.0, color));
        return;
    }
    // Two dashes per cell with gaps at the ends so adjacent cells form an even pattern.
    let len = if horizontal { rect.width() } else { rect.height() };
    let dash = len / 4.0;
    for i in 0..2 {
        let start = dash * 0.5 + i as f32 * dash * 2.0;
        let r = if horizontal {
            Rect::from_min_size(rect.min + Vec2::new(start, 0.0), Vec2::new(dash, rect.height()))
        } else {
            Rect::from_min_size(rect.min + Vec2::new(0.0, start), Vec2::new(rect.width(), dash))
        };
        out.push(Shape::rect_filled(r, 0.0, color));
    }
}
