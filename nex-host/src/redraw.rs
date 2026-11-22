//! An agent's screen as escape sequences, for a pane attaching to it: its scrollback and
//! screen with their colors, the cursor, the title, and the terminal modes the program
//! turned on (bracketed paste, mouse reporting, the kitty keyboard, …). The pane has just
//! been resized to the screen's size, so line for line it ends up showing the same thing.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line};
use alacritty_terminal::term::cell::{Cell, Flags};
use alacritty_terminal::term::{Term, TermMode};
use alacritty_terminal::vte::ansi::{Color, NamedColor};

/// Turn off everything a program may have turned on, for a pane the agent is leaving.
pub const RESET_MODES: &str = "\x1b[0m\x1b[?1049l\x1b[?25h\x1b[?1l\x1b>\x1b[?2004l\x1b[?1000l\x1b[?1002l\x1b[?1003l\x1b[?1005l\x1b[?1006l\x1b[?1004l\x1b[<u";

/// Colors and attributes of a run of cells.
#[derive(Clone, Copy, PartialEq)]
struct Style {
    fg: Color,
    bg: Color,
    flags: Flags,
}

const SHOWN: Flags = Flags::BOLD
    .union(Flags::DIM)
    .union(Flags::ITALIC)
    .union(Flags::ALL_UNDERLINES)
    .union(Flags::INVERSE)
    .union(Flags::HIDDEN)
    .union(Flags::STRIKEOUT);

impl Style {
    const DEFAULT: Style = Style { fg: Color::Named(NamedColor::Foreground), bg: Color::Named(NamedColor::Background), flags: Flags::empty() };

    fn of(cell: &Cell) -> Style {
        Style { fg: cell.fg, bg: cell.bg, flags: cell.flags & SHOWN }
    }

    /// The SGR sequence that sets exactly this style.
    fn sgr(&self, out: &mut String) {
        out.push_str("\x1b[0");
        let f = self.flags;
        for (flag, code) in [
            (Flags::BOLD, "1"),
            (Flags::DIM, "2"),
            (Flags::ITALIC, "3"),
            (Flags::INVERSE, "7"),
            (Flags::HIDDEN, "8"),
            (Flags::STRIKEOUT, "9"),
        ] {
            if f.contains(flag) {
                out.push(';');
                out.push_str(code);
            }
        }
        if f.contains(Flags::UNDERCURL) {
            out.push_str(";4:3");
        } else if f.contains(Flags::DOUBLE_UNDERLINE) {
            out.push_str(";4:2");
        } else if f.contains(Flags::DOTTED_UNDERLINE) {
            out.push_str(";4:4");
        } else if f.contains(Flags::DASHED_UNDERLINE) {
            out.push_str(";4:5");
        } else if f.intersects(Flags::ALL_UNDERLINES) {
            out.push_str(";4");
        }
        color(out, self.fg, false);
        color(out, self.bg, true);
        out.push('m');
    }
}

fn color(out: &mut String, c: Color, bg: bool) {
    let (base, bright, ext, default) = if bg { (40, 100, 48, 49) } else { (30, 90, 38, 39) };
    let code = match c {
        Color::Spec(rgb) => format!("{ext};2;{};{};{}", rgb.r, rgb.g, rgb.b),
        Color::Indexed(i) => format!("{ext};5;{i}"),
        Color::Named(n) => {
            let i = n as usize;
            match n {
                _ if i < 8 => format!("{}", base + i),
                _ if i < 16 => format!("{}", bright + i - 8),
                NamedColor::DimBlack
                | NamedColor::DimRed
                | NamedColor::DimGreen
                | NamedColor::DimYellow
                | NamedColor::DimBlue
                | NamedColor::DimMagenta
                | NamedColor::DimCyan
                | NamedColor::DimWhite => format!("{}", base + (i - NamedColor::DimBlack as usize)),
                _ => return push_default(out, default),
            }
        }
    };
    out.push(';');
    out.push_str(&code);
}

fn push_default(out: &mut String, default: u32) {
    out.push(';');
    out.push_str(&default.to_string());
}

/// The whole screen, from a cleared terminal of the same size.
pub fn screen<T: EventListener>(term: &Term<T>, title: Option<&str>) -> Vec<u8> {
    let grid = term.grid();
    let mode = *term.mode();
    let mut out = String::with_capacity(16 * 1024);
    // Clear the screen and its scrollback, home the cursor.
    out.push_str("\x1b[0m\x1b[H\x1b[2J\x1b[3J");
    if mode.contains(TermMode::ALT_SCREEN) {
        out.push_str("\x1b[?1049h\x1b[H\x1b[2J");
    }

    let history = grid.history_size() as i32;
    let screen = grid.screen_lines() as i32;
    let cols = grid.columns();
    let mut style = Style::DEFAULT;
    for line in -history..screen {
        if line > -history {
            out.push_str("\r\n");
        }
        let row = &grid[Line(line)];
        // Leave off the blank end of the line.
        let end = (0..cols).rev().find(|&c| !blank(&row[Column(c)])).map_or(0, |c| c + 1);
        for c in 0..end {
            let cell = &row[Column(c)];
            if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                continue;
            }
            let s = Style::of(cell);
            if s != style {
                s.sgr(&mut out);
                style = s;
            }
            out.push(cell.c);
            if let Some(extra) = cell.zerowidth() {
                out.extend(extra.iter());
            }
        }
        if style != Style::DEFAULT {
            out.push_str("\x1b[0m");
            style = Style::DEFAULT;
        }
    }

    // The cursor, then the modes, then the title.
    let cursor = grid.cursor.point;
    out.push_str(&format!("\x1b[{};{}H", cursor.line.0 + 1, cursor.column.0 + 1));
    Style::of(&grid.cursor.template).sgr(&mut out);
    let toggles = [
        (TermMode::APP_CURSOR, "\x1b[?1h"),
        (TermMode::BRACKETED_PASTE, "\x1b[?2004h"),
        (TermMode::FOCUS_IN_OUT, "\x1b[?1004h"),
        (TermMode::MOUSE_REPORT_CLICK, "\x1b[?1000h"),
        (TermMode::MOUSE_DRAG, "\x1b[?1002h"),
        (TermMode::MOUSE_MOTION, "\x1b[?1003h"),
        (TermMode::UTF8_MOUSE, "\x1b[?1005h"),
        (TermMode::SGR_MOUSE, "\x1b[?1006h"),
        (TermMode::INSERT, "\x1b[4h"),
    ];
    for (flag, seq) in toggles {
        if mode.contains(flag) {
            out.push_str(seq);
        }
    }
    if mode.contains(TermMode::APP_KEYPAD) {
        out.push_str("\x1b=");
    }
    if !mode.contains(TermMode::LINE_WRAP) {
        out.push_str("\x1b[?7l");
    }
    let kitty = (mode & TermMode::KITTY_KEYBOARD_PROTOCOL).bits() >> TermMode::DISAMBIGUATE_ESC_CODES.bits().trailing_zeros();
    if kitty != 0 {
        out.push_str(&format!("\x1b[>{kitty}u"));
    }
    out.push_str(if mode.contains(TermMode::SHOW_CURSOR) { "\x1b[?25h" } else { "\x1b[?25l" });
    if let Some(t) = title {
        let t: String = t.chars().filter(|c| !c.is_control()).collect();
        out.push_str(&format!("\x1b]2;{t}\x07"));
    }
    out.into_bytes()
}

/// A cell that looks like nothing: a space on the default background.
fn blank(cell: &Cell) -> bool {
    cell.c == ' '
        && cell.bg == Color::Named(NamedColor::Background)
        && !cell.flags.intersects(Flags::INVERSE | Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
        && cell.zerowidth().is_none()
}

#[cfg(test)]
mod tests {
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::term::{self, Term};
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    use super::*;

    fn term(cols: usize, rows: usize, input: &str) -> Term<VoidListener> {
        let mut t = Term::new(term::Config::default(), &TermSize::new(cols, rows), VoidListener);
        let mut p: Processor<StdSyncHandler> = Processor::new();
        p.advance(&mut t, input.as_bytes());
        t
    }

    /// Feed a redraw to a fresh terminal of the same size: it should look the same.
    fn replay(cols: usize, rows: usize, input: &str) -> (Term<VoidListener>, Term<VoidListener>) {
        let a = term(cols, rows, input);
        let bytes = screen(&a, None);
        let b = term(cols, rows, std::str::from_utf8(&bytes).unwrap());
        (a, b)
    }

    fn text(t: &Term<VoidListener>) -> Vec<String> {
        let g = t.grid();
        (-(g.history_size() as i32)..g.screen_lines() as i32)
            .map(|l| (0..g.columns()).map(|c| g[Line(l)][Column(c)].c).collect::<String>().trim_end().to_string())
            .collect()
    }

    #[test]
    fn redraw_matches_the_screen() {
        let (a, b) = replay(20, 4, "hello\r\n\x1b[1;31mred\x1b[0m and \x1b[38;2;1;2;3mrgb\x1b[0m\r\nwide: 日本\r\n> ");
        assert_eq!(text(&a), text(&b));
        assert_eq!(b.grid().cursor.point, a.grid().cursor.point);
        let red = &b.grid()[Line(1)][Column(0)];
        assert!(red.flags.contains(Flags::BOLD));
        assert_eq!(red.fg, Color::Named(NamedColor::Red));
        assert_eq!(b.grid()[Line(1)][Column(8)].fg, Color::Spec(alacritty_terminal::vte::ansi::Rgb { r: 1, g: 2, b: 3 }));
    }

    #[test]
    fn redraw_brings_scrollback_and_modes() {
        let mut input: String = (0..10).map(|i| format!("line {i}\r\n")).collect();
        input.push_str("\x1b[?2004h\x1b[?1h\x1b[?25l$ ");
        let (a, b) = replay(20, 4, &input);
        assert_eq!(text(&a), text(&b));
        assert!(text(&b).contains(&"line 0".to_string()));
        assert!(b.mode().contains(TermMode::BRACKETED_PASTE | TermMode::APP_CURSOR));
        assert!(!b.mode().contains(TermMode::SHOW_CURSOR));
    }

    #[test]
    fn redraw_of_a_full_screen_program_uses_the_alternate_screen() {
        let (_, b) = replay(20, 4, "shell\r\n\x1b[?1049h\x1b[2;3Hmenu");
        assert!(b.mode().contains(TermMode::ALT_SCREEN));
        assert_eq!(b.grid()[Line(1)][Column(2)].c, 'm');
    }
}
