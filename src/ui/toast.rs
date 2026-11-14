//! A short-lived message pill ("Copied 42 characters") that slides in at the bottom of an
//! area, stays for a moment, then fades out. Drawn on a foreground layer above everything.

use egui::{Color32, Id, LayerId, Order, Pos2, Rect, Vec2};

use super::glide::{animations, ease_out};
use super::paint::{bold, glass_pill};
use super::theme::Theme;

/// How long a toast is on screen, including its fades (seconds).
const TOAST_TIME: f64 = 1.5;
/// Length of the fade in and the fade out (seconds).
const FADE_TIME: f64 = 0.18;
/// How far the toast rises while fading in (points).
const SLIDE: f32 = 8.0;
const FONT_SIZE: f32 = 13.0;
const PAD: Vec2 = Vec2::new(14.0, 7.0);
/// Gap between the toast and the bottom of its area.
const MARGIN: f32 = 18.0;

pub struct Toast {
    text: String,
    shown_at: f64,
}

impl Toast {
    pub fn new(ctx: &egui::Context, text: String) -> Self {
        Self { text, shown_at: ctx.input(|i| i.time) }
    }

    /// Paint the toast centered near the bottom of `area`. Returns `false` once it has expired.
    pub fn paint(&self, ctx: &egui::Context, area: Rect, theme: &Theme) -> bool {
        let age = ctx.input(|i| i.time) - self.shown_at;
        if age >= TOAST_TIME {
            return false;
        }
        // 0 → 1 while fading in, 1 while shown, 1 → 0 while fading out.
        let (alpha, rise) = if animations(ctx) {
            let fade_in = ease_out((age / FADE_TIME) as f32);
            let fade_out = ease_out(((TOAST_TIME - age) / FADE_TIME) as f32);
            (fade_in.min(fade_out), fade_in)
        } else {
            (1.0, 1.0)
        };

        let mut painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("nex-toast"))).with_clip_rect(area);
        painter.multiply_opacity(alpha);
        let galley = painter.layout_no_wrap(self.text.clone(), bold(FONT_SIZE), theme.text_bright);
        let size = galley.size() + PAD * 2.0;
        let bottom = area.bottom() - MARGIN + SLIDE * (1.0 - rise);
        let rect = Rect::from_min_size(Pos2::new(area.center().x - size.x / 2.0, bottom - size.y), size);
        glass_pill(&painter, rect, (size.y / 2.0) as u8);
        painter.galley(rect.min + PAD, galley, Color32::PLACEHOLDER);

        ctx.request_repaint();
        true
    }
}

/// The message for a copy or paste: "Copied 42 characters" for one line, "Pasted 3 lines"
/// for several.
pub fn describe(verb: &str, text: &str) -> String {
    let lines = text.lines().count();
    let (n, unit) = if lines > 1 { (lines, "line") } else { (text.chars().count(), "character") };
    let s = if n == 1 { "" } else { "s" };
    format!("{verb} {n} {unit}{s}")
}

#[cfg(test)]
mod tests {
    use super::describe;

    #[test]
    fn single_line_counts_characters() {
        assert_eq!(describe("Copied", "hello"), "Copied 5 characters");
        assert_eq!(describe("Copied", "x"), "Copied 1 character");
        assert_eq!(describe("Copied", "héllo"), "Copied 5 characters");
    }

    #[test]
    fn multi_line_counts_lines() {
        assert_eq!(describe("Pasted", "a\nb\nc"), "Pasted 3 lines");
        assert_eq!(describe("Pasted", "a\nb\n"), "Pasted 2 lines");
    }

    #[test]
    fn trailing_newline_is_still_one_line() {
        assert_eq!(describe("Copied", "ab\n"), "Copied 3 characters");
    }
}
