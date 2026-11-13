//! A copy of what a terminal shows right now, taken under the terminal lock.
//!
//! Painting (color resolution, text layout, shape building) is far slower than copying the
//! visible cells, and the PTY thread can't parse output while the UI holds the lock. So the
//! UI copies the screen into a [`Snapshot`], lets go of the lock, and paints from the copy.

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::Point;
use alacritty_terminal::selection::SelectionRange;
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::color::Colors;
use alacritty_terminal::term::{RenderableCursor, Term, TermMode};

use crate::terminal::EventProxy;

/// One visible cell and where it sits in the grid.
pub struct SnapCell {
    pub point: Point,
    pub cell: Cell,
}

pub struct Snapshot {
    /// Visible cells in display order (top-left to bottom-right), spacer cells left out.
    pub cells: Vec<SnapCell>,
    pub colors: Colors,
    pub cursor: RenderableCursor,
    pub cursor_blinking: bool,
    pub selection: Option<SelectionRange>,
    pub mode: TermMode,
    pub display_offset: usize,
    pub screen_lines: usize,
}

impl Snapshot {
    /// Copy the visible part of `term`. Reuses the cell buffer of a previous snapshot.
    pub fn capture(term: &Term<EventProxy>, previous: Option<Snapshot>) -> Self {
        let mut cells = previous.map(|s| s.cells).unwrap_or_default();
        cells.clear();
        let content = term.renderable_content();
        let screen_lines = term.screen_lines();
        let offset = content.display_offset;
        cells.extend(
            content
                .display_iter
                .filter(|c| {
                    let line = c.point.line.0 + offset as i32;
                    line >= 0
                        && (line as usize) < screen_lines
                        && !c.cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                })
                .map(|c| SnapCell { point: c.point, cell: c.cell.clone() }),
        );
        Self {
            cells,
            colors: *content.colors,
            cursor: content.cursor,
            cursor_blinking: term.cursor_style().blinking,
            selection: content.selection,
            mode: content.mode,
            display_offset: offset,
            screen_lines,
        }
    }
}
