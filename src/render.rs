//! Paint a terminal grid with egui's painter.
//!
//! Backgrounds and glyphs become textured quads in one mesh per pane (see [`crate::glyphs`]),
//! so no text is laid out per frame. Box-drawing characters are drawn as geometry, and
//! characters with combining marks are laid out by egui.

use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::search::Match;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, CursorShape, NamedColor};
use egui::{Color32, FontFamily, FontId, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::config::Config;
use crate::glyphs::{GlyphCache, GridMesh};
use crate::snapshot::{SnapCell, Snapshot};
use crate::terminal::Terminal;

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct CellMetrics {
    /// Cell width, snapped to whole physical pixels so every column starts on the same
    /// sub-pixel phase (fractional widths smear glyph stems across two pixels).
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

pub fn font_for(flags: Flags, size: f32) -> FontId {
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

/// Everything a pane's terminal shapes depend on, apart from the cells, which only change
/// along with `generation` (or the view: scrolling and selection). When nothing here has
/// changed since the last frame, the pane's shapes are reused instead of rebuilt.
#[derive(PartialEq)]
pub struct FrameKey {
    /// Bumped by PTY output.
    pub generation: u64,
    pub display_offset: usize,
    pub selection: Option<SelectionRange>,
    pub rect: Rect,
    pub metrics: CellMetrics,
    pub focused: bool,
    pub cursor_visible: bool,
    pub bg_alpha: f32,
    pub matches: Vec<Match>,
    pub current_match: Option<Match>,
    pub link: Option<Match>,
    /// Changes when the glyph cache (and so every texture coordinate) is invalidated.
    pub glyph_epoch: (u64, [usize; 2]),
}

/// Build the shapes for a terminal snapshot drawn into `rect`, in paint order. Works from
/// the copy so the terminal stays unlocked.
pub fn terminal_shapes(
    painter: &Painter,
    rect: Rect,
    snap: &Snapshot,
    glyphs: &mut GlyphCache,
    cfg: &Config,
    m: CellMetrics,
    opts: PaintOptions<'_>,
) -> Vec<Shape> {
    let offset = snap.display_offset;
    let cursor = snap.cursor;

    let mut styler = CellStyler::new(&snap.colors, cfg, &opts, snap.selection, offset);
    let cursor_line = cursor.point.line.0 + offset as i32;
    let cursor_shape = if opts.cursor_visible && offset == 0 && snap.mode.contains(TermMode::SHOW_CURSOR) {
        cursor.shape
    } else {
        CursorShape::Hidden
    };
    styler.block_cursor = opts.focused && cursor_shape == CursorShape::Block;
    let mut layers = Layers::new(painter, glyphs, rect, m, snap.cells.len());
    let mut cursor_cell_width = 1;

    for SnapCell { cell, point } in &snap.cells {
        let point = *point;
        // Snapshot cells are all on screen (see `Snapshot::capture`).
        let line = (point.line.0 + offset as i32) as usize;
        let col = point.column.0;
        let width = if cell.flags.contains(Flags::WIDE_CHAR) { 2 } else { 1 };
        let is_cursor = line as i32 == cursor_line && col == cursor.point.column.0;
        if is_cursor {
            cursor_cell_width = width;
        }
        let (fg, bg) = styler.colors(cell, point, is_cursor);

        let at = CellPos { line, col, width };
        layers.background(at, bg);
        let r = layers.cell_rect(line, col, width);
        let in_link = opts.link.is_some_and(|l| l.contains(&point));
        layers.decorations(r, cell.flags, in_link, fg);
        let c = if cell.flags.contains(Flags::HIDDEN) { ' ' } else { cell.c };
        layers.text(at, c, cell.zerowidth(), fg, cell.flags & (Flags::BOLD | Flags::ITALIC));
    }
    layers.flush_bg();

    if cursor_shape != CursorShape::Hidden && cursor_line >= 0 && (cursor_line as usize) < snap.screen_lines {
        let r = layers.cell_rect(cursor_line as usize, cursor.point.column.0, cursor_cell_width);
        layers.cursor(r, cursor_shape, styler.cursor_color, opts.focused);
    }
    layers.finish(opts.bg_alpha)
}

/// Works out each cell's colors: its own, then dim / inverse, selection, search matches and
/// the block cursor on top.
struct CellStyler<'a> {
    colors: &'a Colors,
    cfg: &'a Config,
    opts: &'a PaintOptions<'a>,
    selection: Option<SelectionRange>,
    default_bg: Color32,
    cursor_color: Color32,
    /// The focused block cursor is drawn as inverted colors (other shapes are outlines).
    block_cursor: bool,
    /// Matches are sorted and cells arrive in order, so the two are walked together.
    match_idx: usize,
}

impl<'a> CellStyler<'a> {
    fn new(colors: &'a Colors, cfg: &'a Config, opts: &'a PaintOptions<'a>, selection: Option<SelectionRange>, offset: usize) -> Self {
        let viewport_top = Point::new(Line(-(offset as i32)), Column(0));
        Self {
            colors,
            cfg,
            opts,
            selection,
            default_bg: resolve(Color::Named(NamedColor::Background), colors, cfg),
            cursor_color: colors[NamedColor::Cursor as usize].map(|c| Color32::from_rgb(c.r, c.g, c.b)).unwrap_or(cfg.cursor_color),
            block_cursor: false,
            match_idx: opts.matches.partition_point(|m| *m.end() < viewport_top),
        }
    }

    /// `(foreground, background)`; `None` background means the default (not painted).
    fn colors(&mut self, cell: &Cell, point: Point, is_cursor: bool) -> (Color32, Option<Color32>) {
        let mut fg = resolve(cell.fg, self.colors, self.cfg);
        let mut bg = match cell.bg {
            Color::Named(NamedColor::Background) => None,
            c => Some(resolve(c, self.colors, self.cfg)),
        };
        if cell.flags.contains(Flags::DIM) {
            fg = dim(fg);
        }
        if cell.flags.contains(Flags::INVERSE) {
            let old_fg = fg;
            fg = bg.unwrap_or(self.default_bg);
            bg = Some(old_fg);
        }
        if self.selection.is_some_and(|s| s.contains(point)) {
            bg = Some(self.cfg.selection_background);
            if let Some(sfg) = self.cfg.selection_foreground {
                fg = sfg;
            }
        }
        let matches = self.opts.matches;
        while self.match_idx < matches.len() && *matches[self.match_idx].end() < point {
            self.match_idx += 1;
        }
        if let Some(m) = matches.get(self.match_idx).filter(|m| *m.start() <= point) {
            let current = self.opts.current_match.is_some_and(|c| c == m);
            bg = Some(if current { CURRENT_MATCH_BG } else { MATCH_BG });
            if current {
                fg = self.default_bg;
            }
        }
        if is_cursor && self.block_cursor {
            bg = Some(self.cursor_color);
            fg = self.default_bg;
        }
        (fg, bg)
    }
}

/// Where a cell sits on screen: its line, column and width in columns (2 for wide chars).
#[derive(Clone, Copy)]
struct CellPos {
    line: usize,
    col: usize,
    width: usize,
}

/// A background run being built on one line: columns `start..end` in `color`.
struct BgRun {
    line: usize,
    start: usize,
    end: usize,
    color: Color32,
}

/// The frame's shapes, built cell by cell in layers: the mesh (backgrounds, then glyphs),
/// other text (box drawing, combining marks), then decorations. Adjacent backgrounds of one
/// color are merged into one quad as they arrive.
struct Layers<'a> {
    painter: &'a Painter,
    rect: Rect,
    m: CellMetrics,
    mesh: GridMesh<'a>,
    texts: Vec<Shape>,
    decorations: Vec<Shape>,
    bg_run: Option<BgRun>,
}

impl<'a> Layers<'a> {
    fn new(painter: &'a Painter, glyphs: &'a mut GlyphCache, rect: Rect, m: CellMetrics, cells: usize) -> Self {
        Self { painter, rect, m, mesh: GridMesh::new(glyphs, painter, m.font_size, cells), texts: Vec::new(), decorations: Vec::new(), bg_run: None }
    }

    fn cell_rect(&self, line: usize, col: usize, width: usize) -> Rect {
        Rect::from_min_size(
            Pos2::new(self.rect.left() + col as f32 * self.m.width, self.rect.top() + line as f32 * self.m.height),
            Vec2::new(self.m.width * width as f32, self.m.height),
        )
    }

    /// Extend the open background run, or start a new one.
    fn background(&mut self, at: CellPos, bg: Option<Color32>) {
        let CellPos { line, col, width } = at;
        match (&mut self.bg_run, bg) {
            (Some(run), Some(b)) if run.line == line && run.end == col && run.color == b => run.end = col + width,
            (_, Some(color)) => {
                self.flush_bg();
                self.bg_run = Some(BgRun { line, start: col, end: col + width, color });
            }
            (_, None) => self.flush_bg(),
        }
    }

    fn flush_bg(&mut self) {
        if let Some(run) = self.bg_run.take() {
            let r = self.cell_rect(run.line, run.start, run.end - run.start);
            self.mesh.background(r, run.color);
        }
    }

    /// Underlines (also for the hovered link) and strikethrough.
    fn decorations(&mut self, r: Rect, flags: Flags, in_link: bool, fg: Color32) {
        if flags.intersects(Flags::ALL_UNDERLINES) || in_link {
            let y = r.bottom() - (self.m.height * 0.12).max(1.5);
            self.decorations.push(Shape::hline(r.x_range(), y, Stroke::new(1.0, fg)));
            if flags.contains(Flags::DOUBLE_UNDERLINE) {
                self.decorations.push(Shape::hline(r.x_range(), y - 2.0, Stroke::new(1.0, fg)));
            }
        }
        if flags.contains(Flags::STRIKEOUT) {
            self.decorations.push(Shape::hline(r.x_range(), r.center().y, Stroke::new(1.0, fg)));
        }
    }

    /// One cell's character: a quad from the glyph cache, box-drawing geometry, or (with
    /// combining marks) laid out by egui.
    fn text(&mut self, at: CellPos, c: char, zerowidth: Option<&[char]>, fg: Color32, style: Flags) {
        let CellPos { line, col, width } = at;
        if c == ' ' && zerowidth.is_none() {
            return;
        }
        let r = self.cell_rect(line, col, width);
        if width == 1 && !c.is_ascii() && crate::boxdraw::draw(c, r, fg, self.m.ppp, &mut self.texts) {
            return;
        }
        let Some(extra) = zerowidth else {
            self.mesh.glyph(r, self.m, c, style, fg);
            return;
        };
        let s: String = std::iter::once(c).chain(extra.iter().copied()).collect();
        let galley = self.painter.layout_no_wrap(s, font_for(style, self.m.font_size), fg);
        let x = r.left() + ((r.width() - galley.size().x) / 2.0).max(0.0).min(self.m.width * 0.5);
        self.texts.push(Shape::galley(Pos2::new(x, r.top() + self.m.baseline_offset), galley, fg));
    }

    /// Cursor shapes other than the focused block (already painted as inverted colors).
    fn cursor(&mut self, r: Rect, shape: CursorShape, color: Color32, focused: bool) {
        let thickness = (self.m.width * 0.12).clamp(1.5, 3.0);
        let shape = match shape {
            CursorShape::Block if focused => return,
            CursorShape::Hidden => return,
            CursorShape::Block | CursorShape::HollowBlock => {
                Shape::rect_stroke(r.shrink(0.5), 1.0, Stroke::new(1.0, color), egui::StrokeKind::Inside)
            }
            CursorShape::Beam => Shape::rect_filled(Rect::from_min_size(r.min, Vec2::new(thickness, r.height())), 0.0, color),
            CursorShape::Underline => {
                Shape::rect_filled(Rect::from_min_max(Pos2::new(r.left(), r.bottom() - thickness), r.max), 0.0, color)
            }
        };
        self.decorations.push(shape);
    }

    /// Hand the layers to the painter, backgrounds faded by the window opacity.
    fn finish(self, bg_alpha: f32) -> Vec<Shape> {
        let mut shapes = Vec::from(self.mesh.into_shapes(bg_alpha));
        shapes.extend(self.texts);
        shapes.extend(self.decorations);
        shapes
    }
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
