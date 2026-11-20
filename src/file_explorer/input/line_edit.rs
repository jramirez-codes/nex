//! A one-line text field for the location bar and renaming. Like the rest of the explorer it
//! is painted and fed by hand: keys and typed text are routed to it by the app, and a press
//! inside it places the caret. Positions are char indices into `text`.

use egui::text::CCursor;
use egui::{Color32, FontId, Key, Modifiers, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::ui::theme::{ACCENT, Theme};

pub(in crate::file_explorer) struct LineEdit {
    pub(in crate::file_explorer) text: String,
    cursor: usize,
    /// The other end of the selection (equal to `cursor` when nothing is selected).
    anchor: usize,
}

/// What a key press did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::file_explorer) enum EditKey {
    /// Not a key the field uses.
    Ignored,
    /// Edited the text or moved the caret.
    Used,
    Commit,
    Cancel,
    /// Tab (the location bar completes paths with it).
    Complete,
}

impl LineEdit {
    /// A field holding `text`, all of it selected.
    pub(in crate::file_explorer) fn new(text: impl Into<String>) -> Self {
        let text = text.into();
        let n = text.chars().count();
        Self { text, cursor: n, anchor: 0 }
    }

    /// A field holding `text` with chars `start..end` selected (a name without its extension).
    pub(in crate::file_explorer) fn with_selection(text: impl Into<String>, start: usize, end: usize) -> Self {
        let text = text.into();
        let end = end.min(text.chars().count());
        Self { text, cursor: end, anchor: start.min(end) }
    }

    fn len(&self) -> usize {
        self.text.chars().count()
    }

    fn byte(&self, i: usize) -> usize {
        self.text.char_indices().nth(i).map_or(self.text.len(), |(b, _)| b)
    }

    fn selection(&self) -> (usize, usize) {
        (self.cursor.min(self.anchor), self.cursor.max(self.anchor))
    }

    pub(in crate::file_explorer) fn selected_text(&self) -> String {
        let (a, b) = self.selection();
        self.text[self.byte(a)..self.byte(b)].to_string()
    }

    /// Remove the selected text. Returns false when nothing was selected.
    fn delete_selection(&mut self) -> bool {
        let (a, b) = self.selection();
        if a == b {
            return false;
        }
        let (ba, bb) = (self.byte(a), self.byte(b));
        self.text.replace_range(ba..bb, "");
        self.cursor = a;
        self.anchor = a;
        true
    }

    /// Cut the selection out (for Ctrl+X).
    pub(in crate::file_explorer) fn cut(&mut self) -> String {
        let s = self.selected_text();
        self.delete_selection();
        s
    }

    /// Type `s` over the selection. Line breaks and control characters are dropped.
    pub(in crate::file_explorer) fn insert(&mut self, s: &str) {
        let s: String = s.chars().filter(|c| !c.is_control()).collect();
        self.delete_selection();
        let at = self.byte(self.cursor);
        self.text.insert_str(at, &s);
        self.cursor += s.chars().count();
        self.anchor = self.cursor;
    }

    /// Replace the text, with the caret at the end.
    pub(in crate::file_explorer) fn set_text(&mut self, s: String) {
        self.text = s;
        self.cursor = self.len();
        self.anchor = self.cursor;
    }

    /// The start of the word before `i` (or the end of the one after it).
    fn word_edge(&self, i: usize, forward: bool) -> usize {
        let chars: Vec<char> = self.text.chars().collect();
        let word = |c: char| c.is_alphanumeric();
        let mut j = i;
        if forward {
            while j < chars.len() && !word(chars[j]) {
                j += 1;
            }
            while j < chars.len() && word(chars[j]) {
                j += 1;
            }
        } else {
            while j > 0 && !word(chars[j - 1]) {
                j -= 1;
            }
            while j > 0 && word(chars[j - 1]) {
                j -= 1;
            }
        }
        j
    }

    pub(in crate::file_explorer) fn key(&mut self, key: Key, mods: Modifiers) -> EditKey {
        // Ctrl jumps by word on Linux; Option does on macOS.
        let by_word = mods.ctrl || mods.alt;
        let len = self.len();
        let (a, b) = self.selection();
        let move_to = |s: &mut Self, to: usize| {
            s.cursor = to;
            if !mods.shift {
                s.anchor = to;
            }
        };
        match key {
            Key::Enter => return EditKey::Commit,
            Key::Escape => return EditKey::Cancel,
            Key::Tab => return EditKey::Complete,
            Key::ArrowLeft if a != b && !mods.shift => move_to(self, a),
            Key::ArrowRight if a != b && !mods.shift => move_to(self, b),
            Key::ArrowLeft => {
                let to = if by_word { self.word_edge(self.cursor, false) } else { self.cursor.saturating_sub(1) };
                move_to(self, to);
            }
            Key::ArrowRight => {
                let to = if by_word { self.word_edge(self.cursor, true) } else { (self.cursor + 1).min(len) };
                move_to(self, to);
            }
            Key::Home | Key::ArrowUp => move_to(self, 0),
            Key::End | Key::ArrowDown => move_to(self, len),
            Key::A if mods.command => {
                self.anchor = 0;
                self.cursor = len;
            }
            Key::Backspace => {
                if !self.delete_selection() && self.cursor > 0 {
                    self.anchor = if by_word { self.word_edge(self.cursor, false) } else { self.cursor - 1 };
                    self.delete_selection();
                }
            }
            Key::Delete => {
                if !self.delete_selection() && self.cursor < len {
                    self.anchor = if by_word { self.word_edge(self.cursor, true) } else { self.cursor + 1 };
                    self.delete_selection();
                }
            }
            _ => return EditKey::Ignored,
        }
        EditKey::Used
    }

    /// Draw the field in `r`. A press inside it this frame moves the caret there (Shift
    /// extends the selection); a double-click selects everything.
    pub(in crate::file_explorer) fn paint(&mut self, p: &Painter, r: Rect, font: FontId, c: &Theme) {
        p.rect_filled(r, 6, Color32::from_black_alpha(90));
        p.rect_stroke(r, 6, Stroke::new(1.0_f32, ACCENT.gamma_multiply(0.85)), StrokeKind::Inside);
        let inner = r.shrink2(Vec2::new(6.0, 0.0));
        let galley = p.layout_no_wrap(self.text.clone(), font, c.text_bright);
        let x_of = |i: usize| galley.pos_from_cursor(CCursor::new(i)).min.x;

        // Keep the caret in view: the text slides left once it reaches the right edge.
        let scroll = (x_of(self.cursor) - inner.width() + 2.0).max(0.0);
        let origin = Pos2::new(inner.left() - scroll, r.center().y - galley.size().y / 2.0);

        let (press, double, shift) = p.ctx().input(|i| {
            let at = i.pointer.press_origin().filter(|_| i.pointer.primary_pressed());
            (at, i.pointer.button_double_clicked(egui::PointerButton::Primary), i.modifiers.shift)
        });
        let on_field = |at: &Pos2| r.contains(*at) && p.ctx().layer_id_at(*at) == Some(p.layer_id());
        if let Some(at) = press.filter(on_field) {
            self.cursor = galley.cursor_from_pos(at - origin).index;
            if !shift {
                self.anchor = self.cursor;
            }
        }
        if double && p.ctx().input(|i| i.pointer.interact_pos()).is_some_and(|at| on_field(&at)) {
            self.anchor = 0;
            self.cursor = self.len();
        }

        let clip = p.with_clip_rect(inner.expand2(Vec2::new(2.0, 0.0)).intersect(p.clip_rect()));
        let h = galley.size().y;
        let (a, b) = self.selection();
        if a != b {
            let sel = Rect::from_min_max(Pos2::new(origin.x + x_of(a), origin.y), Pos2::new(origin.x + x_of(b), origin.y + h));
            clip.rect_filled(sel, 2, ACCENT.gamma_multiply(0.4));
        }
        let x = origin.x + x_of(self.cursor);
        clip.galley(origin, galley, c.text_bright);
        clip.line_segment([Pos2::new(x, origin.y + 1.0), Pos2::new(x, origin.y + h - 1.0)], Stroke::new(1.5_f32, ACCENT));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(f: &mut LineEdit, key: Key) -> EditKey {
        f.key(key, Modifiers::NONE)
    }

    #[test]
    fn typing_replaces_the_selection() {
        let mut f = LineEdit::with_selection("report.pdf", 0, 6);
        f.insert("notes");
        assert_eq!(f.text, "notes.pdf");
        key(&mut f, Key::End);
        key(&mut f, Key::Backspace);
        assert_eq!(f.text, "notes.pd");
        assert_eq!(key(&mut f, Key::Enter), EditKey::Commit);
        assert_eq!(key(&mut f, Key::F5), EditKey::Ignored);
    }

    #[test]
    fn unicode_and_words() {
        let mut f = LineEdit::new("héllo wörld");
        key(&mut f, Key::End);
        f.key(Key::Backspace, Modifiers::CTRL);
        assert_eq!(f.text, "héllo ");
        key(&mut f, Key::Home);
        key(&mut f, Key::Delete);
        assert_eq!(f.text, "éllo ");
        f.key(Key::ArrowRight, Modifiers::SHIFT);
        assert_eq!(f.selected_text(), "é");
        assert_eq!(f.cut(), "é");
        f.insert("a\nb");
        assert_eq!(f.text, "abllo ");
    }
}
