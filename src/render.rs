//! Paint a terminal grid with egui's painter.
//!
//! Text is batched into runs of identically-styled ASCII/box-drawing cells so a typical row
//! becomes a handful of cached galleys instead of one per cell.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::config::Config;
use crate::terminal::Terminal;

#[derive(Clone, Copy, Debug)]
pub struct CellMetrics {
    pub width: f32,
    pub height: f32,
    /// Offset that vertically centers the font's row inside the (possibly taller) cell.
    pub baseline_offset: f32,
    pub font_size: f32,
    pub ppp: f32,
}

pub const FONT_REGULAR: &str = "term-regular";
pub const FONT_BOLD: &str = "term-bold";
pub const FONT_ITALIC: &str = "term-italic";
pub const FONT_BOLD_ITALIC: &str = "term-bold-italic";

fn font_for(flags: Flags, size: f32) -> FontId {
    let name = match (flags.contains(Flags::BOLD), flags.contains(Flags::ITALIC)) {
        (false, false) => FONT_REGULAR,
        (true, false) => FONT_BOLD,
        (false, true) => FONT_ITALIC,
        (true, true) => FONT_BOLD_ITALIC,
    };
    FontId::new(size, FontFamily::Name(name.into()))
}

/// xterm 256-color palette with the first 16 entries from the config.
pub fn default_indexed_color(index: usize, cfg: &Config) -> Color32 {
    match index {
        0..=15 => cfg.palette[index],
        16..=231 => {
            let i = index - 16;
            let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
            Color32::from_rgb(level(i / 36), level((i / 6) % 6), level(i % 6))
        }
        232..=255 => {
            let v = (8 + (index - 232) * 10) as u8;
            Color32::from_rgb(v, v, v)
        }
        256 => cfg.foreground,              // NamedColor::Foreground
        257 => cfg.background,              // NamedColor::Background
        258 => cfg.cursor_color,            // NamedColor::Cursor
        259..=266 => dim(cfg.palette[index - 259]), // DimBlack..DimWhite
        267 => cfg.foreground,              // BrightForeground
        268 => dim(cfg.foreground),         // DimForeground
        _ => cfg.foreground,
    }
}

fn dim(c: Color32) -> Color32 {
    Color32::from_rgb(
        (c.r() as f32 * 0.66) as u8,
        (c.g() as f32 * 0.66) as u8,
        (c.b() as f32 * 0.66) as u8,
    )
}

fn resolve(color: Color, colors: &Colors, cfg: &Config) -> Color32 {
    let indexed = |i: usize| {
        colors[i]
            .map(|rgb| Color32::from_rgb(rgb.r, rgb.g, rgb.b))
            .unwrap_or_else(|| default_indexed_color(i, cfg))
    };
    match color {
        Color::Spec(rgb) => Color32::from_rgb(rgb.r, rgb.g, rgb.b),
        Color::Named(n) => indexed(n as usize),
        Color::Indexed(i) => indexed(i as usize),
    }
}

/// Characters that are safe to batch into a run: they come from the primary monospace font
/// and have exactly one cell of advance.
fn batchable(c: char) -> bool {
    c.is_ascii_graphic()
}

struct Run {
    text: String,
    col: usize,
    color: Color32,
    flags: Flags,
}

pub struct PaintOptions<'a> {
    pub focused: bool,
    pub cursor_visible: bool,
    pub bg_alpha: f32,
    /// Search matches (sorted, oldest first) and the focused one.
    pub matches: &'a [Match],
    pub current_match: Option<&'a Match>,
    /// Link under the pointer while Ctrl is held; drawn underlined.
    pub link: Option<&'a Match>,
}

const MATCH_BG: Color32 = Color32::from_rgb(0x8a, 0x6d, 0x2c);
const CURRENT_MATCH_BG: Color32 = Color32::from_rgb(0xff, 0x9e, 0x3d);

pub fn paint_terminal(
    painter: &Painter,
    rect: Rect,
    terminal: &Terminal,
    cfg: &Config,
    m: CellMetrics,
    opts: PaintOptions<'_>,
) {
    let term = terminal.term.lock();
    let content = term.renderable_content();
    let colors = content.colors;
    let offset = content.display_offset;
    let selection = content.selection;
    let cursor = content.cursor;
    let screen_lines = term.screen_lines();
    let mode = content.mode;

    let default_bg = resolve(Color::Named(NamedColor::Background), colors, cfg);
    let cursor_color = colors[NamedColor::Cursor as usize]
        .map(|c| Color32::from_rgb(c.r, c.g, c.b))
        .unwrap_or(cfg.cursor_color);

    let cursor_line = cursor.point.line.0 + offset as i32;
    let cursor_shape = if opts.cursor_visible && offset == 0 && mode.contains(TermMode::SHOW_CURSOR) {
        cursor.shape
    } else {
        CursorShape::Hidden
    };
    let block_cursor = opts.focused && cursor_shape == CursorShape::Block;

    let cell_rect = |line: usize, col: usize, width: usize| {
        Rect::from_min_size(
            Pos2::new(rect.left() + col as f32 * m.width, rect.top() + line as f32 * m.height),
            Vec2::new(m.width * width as f32, m.height),
        )
    };

    let mut backgrounds: Vec<Shape> = Vec::new();
    let mut texts: Vec<Shape> = Vec::new();
    let mut decorations: Vec<Shape> = Vec::new();
    let mut cursor_cell_width = 1;

    // Matches are sorted and cells arrive in order, so walk both together.
    let viewport_top = Point::new(Line(-(offset as i32)), Column(0));
    let mut match_idx = opts.matches.partition_point(|m| *m.end() < viewport_top);

    // Background run being built on the current line: (line, start_col, end_col, color).
    let mut bg_run: Option<(usize, usize, usize, Color32)> = None;
    let mut text_run: Option<(usize, Run)> = None;

    let flush_bg = |run: &mut Option<(usize, usize, usize, Color32)>, out: &mut Vec<Shape>| {
        if let Some((line, start, end, color)) = run.take() {
            out.push(Shape::rect_filled(cell_rect(line, start, end - start), 0.0, color));
        }
    };
    let flush_text = |run: &mut Option<(usize, Run)>, out: &mut Vec<Shape>| {
        if let Some((line, run)) = run.take() {
            let trimmed = run.text.trim_end();
            if trimmed.is_empty() {
                return;
            }
            let galley = painter.layout_no_wrap(trimmed.to_string(), font_for(run.flags, m.font_size), run.color);
            let pos = cell_rect(line, run.col, 1).min + Vec2::new(0.0, m.baseline_offset);
            out.push(Shape::galley(pos, galley, run.color));
        }
    };

    for indexed in content.display_iter {
        let cell = indexed.cell;
        let point = indexed.point;
        let Ok(line) = usize::try_from(point.line.0 + offset as i32) else { continue };
        if line >= screen_lines {
            continue;
        }
        let col = point.column.0;
        if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            continue;
        }
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };

        let mut fg = resolve(cell.fg, colors, cfg);
        let mut bg = match cell.bg {
            Color::Named(NamedColor::Background) => None,
            c => Some(resolve(c, colors, cfg)),
        };
        if cell.flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        if cell.flags.contains(Flags::INVERSE) {
            let old_fg = fg;
            fg = bg.unwrap_or(default_bg);
            bg = Some(old_fg);
        }
        if selection.is_some_and(|s| s.contains(point)) {
            bg = Some(cfg.selection_background);
            if let Some(sfg) = cfg.selection_foreground {
                fg = sfg;
            }
        }
        while match_idx < opts.matches.len() && *opts.matches[match_idx].end() < point {
            match_idx += 1;
        }
        if let Some(m) = opts.matches.get(match_idx).filter(|m| *m.start() <= point) {
            let current = opts.current_match.is_some_and(|c| c == m);
            bg = Some(if current { CURRENT_MATCH_BG } else { MATCH_BG });
            if current {
                fg = default_bg;
            }
        }
        let is_cursor = line as i32 == cursor_line && col == cursor.point.column.0;
        if is_cursor {
            cursor_cell_width = width;
            if block_cursor {
                bg = Some(cursor_color);
                fg = default_bg;
            }
        }
        let c = if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };

        // Backgrounds: merge adjacent cells of the same color.
        match (&mut bg_run, bg) {
            (Some((l, _, end, color)), Some(b)) if *l == line && *end == col && *color == b => {
                *end = col + width
            }
            (_, Some(b)) => {
                flush_bg(&mut bg_run, &mut backgrounds);
                bg_run = Some((line, col, col + width, b));
            }
            (_, None) => flush_bg(&mut bg_run, &mut backgrounds),
        }

        // Decorations.
        let r = cell_rect(line, col, width);
        let in_link = opts.link.is_some_and(|l| l.contains(&point));
        if cell.flags.intersects(Flags::ALL_UNDERLINES) || in_link {
            let y = r.bottom() - (m.height * 0.12).max(1.5);
            decorations.push(Shape::hline(r.x_range(), y, Stroke::new(1.0, fg)));
            if cell.flags.contains(Flags::DOUBLE_UNDERLINE) {
                decorations.push(Shape::hline(r.x_range(), y - 2.0, Stroke::new(1.0, fg)));
            }
        }
        if cell.flags.contains(Flags::STRIKEOUT) {
            decorations.push(Shape::hline(r.x_range(), r.center().y, Stroke::new(1.0, fg)));
        }

        // Text.
        let style_flags = cell.flags & (Flags::BOLD | Flags::ITALIC);
        if c == ' ' && cell.zerowidth().is_none() {
            // Spaces extend whatever run is open (their color doesn't matter).
            if let Some((l, run)) = &mut text_run {
                if *l == line && run.col + run.text.chars().count() == col {
                    run.text.push(' ');
                }
            }
            continue;
        }
        if width == 1 && crate::boxdraw::draw(c, r, fg, m.ppp, &mut texts) {
            continue;
        }
        if batchable(c) && width == 1 && cell.zerowidth().is_none() {
            let continues = matches!(&text_run, Some((l, run))
                if *l == line
                    && run.color == fg
                    && run.flags == style_flags
                    && run.col + run.text.chars().count() == col);
            if continues {
                text_run.as_mut().unwrap().1.text.push(c);
            } else {
                flush_text(&mut text_run, &mut texts);
                text_run = Some((line, Run { text: c.to_string(), col, color: fg, flags: style_flags }));
            }
        } else {
            // Wide / non-ASCII / combining: draw on its own, centered in its cell(s).
            flush_text(&mut text_run, &mut texts);
            let mut s = String::from(c);
            if let Some(extra) = cell.zerowidth() {
                s.extend(extra);
            }
            let galley = painter.layout_no_wrap(s, font_for(style_flags, m.font_size), fg);
            let x = r.left() + ((r.width() - galley.size().x) / 2.0).max(0.0).min(m.width * 0.5);
            texts.push(Shape::galley(Pos2::new(x, r.top() + m.baseline_offset), galley, fg));
        }
    }
    flush_bg(&mut bg_run, &mut backgrounds);
    flush_text(&mut text_run, &mut texts);

    // Cursor shapes other than the (already painted) focused block.
    if cursor_shape != CursorShape::Hidden && cursor_line >= 0 && (cursor_line as usize) < screen_lines {
        let r = cell_rect(cursor_line as usize, cursor.point.column.0, cursor_cell_width);
        let thickness = (m.width * 0.12).clamp(1.5, 3.0);
        match cursor_shape {
            CursorShape::Block if opts.focused => {}
            CursorShape::Block | CursorShape::HollowBlock => decorations.push(Shape::rect_stroke(
                r.shrink(0.5),
                1.0,
                Stroke::new(1.0, cursor_color),
                egui::StrokeKind::Inside,
            )),
            CursorShape::Beam => decorations.push(Shape::rect_filled(
                Rect::from_min_size(r.min, Vec2::new(thickness, r.height())),
                0.0,
                cursor_color,
            )),
            CursorShape::Underline => decorations.push(Shape::rect_filled(
                Rect::from_min_max(Pos2::new(r.left(), r.bottom() - thickness), r.max),
                0.0,
                cursor_color,
            )),
            CursorShape::Hidden => {}
        }
    }

    let alpha = opts.bg_alpha;
    for shape in &mut backgrounds {
        if let Shape::Rect(r) = shape {
            r.fill = r.fill.gamma_multiply(alpha.max(0.85));
        }
    }
    painter.extend(backgrounds);
    painter.extend(texts);
    painter.extend(decorations);
}

/// Convert a position inside the terminal rect into a grid point (accounting for
/// scrollback) and which half of the cell was hit.
pub fn point_at(
    pos: Pos2,
    rect: Rect,
    m: CellMetrics,
    terminal: &Terminal,
) -> (Point, alacritty_terminal::index::Side, usize, usize) {
    let size = terminal.size();
    let rel = pos - rect.min;
    let col_f = (rel.x / m.width).max(0.0);
    let col = (col_f as usize).min(size.cols.saturating_sub(1));
    let line = ((rel.y / m.height).max(0.0) as usize).min(size.lines.saturating_sub(1));
    let side = if col_f.fract() < 0.5 {
        alacritty_terminal::index::Side::Left
    } else {
        alacritty_terminal::index::Side::Right
    };
    let offset = terminal.term.lock().grid().display_offset();
    let point = Point::new(Line(line as i32 - offset as i32), Column(col));
    (point, side, col, line)
}
