//! A short-lived message pill ("Copied 42 characters") that slides in at the bottom of an
//! area, stays for a moment, then fades out. Drawn on a foreground layer above everything.
//!
//! A message wider than its area (a phone held upright) wraps onto more lines rather than
//! running off the sides, so the whole toast stays on screen.

use std::sync::Arc;

use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Galley, Id, LayerId, Order, Pos2, Rect, Vec2};

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
/// Least gap kept between the toast and the sides (and top) of its area.
const SIDE_MARGIN: f32 = 10.0;
/// Most lines a toast wraps onto; anything past them ends in "…".
const MAX_LINES: usize = 6;

/// How long a toast worth reading (an error) is on screen (seconds).
const LONG_TOAST_TIME: f64 = 5.0;

pub struct Toast {
    text: String,
    shown_at: f64,
    /// Seconds on screen, including the fades.
    time: f64,
}

impl Toast {
    pub fn new(ctx: &egui::Context, text: String) -> Self {
        Self { text, shown_at: ctx.input(|i| i.time), time: TOAST_TIME }
    }

    /// A toast that stays long enough to read a sentence (an error).
    pub fn long(ctx: &egui::Context, text: String) -> Self {
        Self { time: LONG_TOAST_TIME, ..Self::new(ctx, text) }
    }

    /// Paint the toast centered near the bottom of `area`. Returns `false` once it has expired.
    pub fn paint(&self, ctx: &egui::Context, area: Rect, theme: &Theme) -> bool {
        let age = ctx.input(|i| i.time) - self.shown_at;
        if age >= self.time {
            return false;
        }
        // 0 → 1 while fading in, 1 while shown, 1 → 0 while fading out.
        let (alpha, rise) = if animations(ctx) {
            let fade_in = ease_out((age / FADE_TIME) as f32);
            let fade_out = ease_out(((self.time - age) / FADE_TIME) as f32);
            (fade_in.min(fade_out), fade_in)
        } else {
            (1.0, 1.0)
        };

        let mut painter = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("nex-toast"))).with_clip_rect(area);
        painter.multiply_opacity(alpha);
        let (galley, rest) = layout(ctx, &self.text, area, theme.text_bright);
        let rect = rest.translate(Vec2::new(0.0, SLIDE * (1.0 - rise)));
        glass_pill(&painter, rect, radius(&galley, rect));
        painter.galley(rect.min + PAD, galley, Color32::PLACEHOLDER);

        ctx.request_repaint();
        true
    }
}

/// Lay out `text` for a toast in `area`, and where the toast sits once it has slid in:
/// centered, `MARGIN` above the bottom. Text wraps to fit between the side margins and stops
/// at as many lines as fit the area (at most `MAX_LINES`), so the toast never leaves it.
fn layout(ctx: &egui::Context, text: &str, area: Rect, color: Color32) -> (Arc<Galley>, Rect) {
    let font = bold(FONT_SIZE);
    let row = ctx.fonts_mut(|f| f.row_height(&font));
    let max_width = (area.width() - 2.0 * (SIDE_MARGIN + PAD.x)).max(row);
    let room = area.height() - MARGIN - SIDE_MARGIN - 2.0 * PAD.y;
    let max_rows = ((room / row).floor() as usize).clamp(1, MAX_LINES);
    let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
    job.wrap = TextWrapping { max_width, max_rows, break_anywhere: false, overflow_character: Some('…') };
    let galley = ctx.fonts_mut(|f| f.layout_job(job));
    let size = galley.size() + PAD * 2.0;
    let bottom = area.bottom() - MARGIN;
    let rect = Rect::from_min_size(Pos2::new(area.center().x - size.x / 2.0, bottom - size.y), size);
    (galley, rect)
}

/// A one-line toast is a full pill; one of several lines keeps the same corners.
fn radius(galley: &Galley, rect: Rect) -> u8 {
    let one_line = galley.rows.first().map_or(rect.height(), |r| r.height()) + PAD.y * 2.0;
    (rect.height().min(one_line) / 2.0) as u8
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
    use egui::{Color32, Pos2, Rect, Vec2};

    use super::{MARGIN, SIDE_MARGIN, describe, layout};

    /// A context with nex's fonts, after one pass so they are loaded.
    fn ctx() -> egui::Context {
        let ctx = egui::Context::default();
        crate::render::fonts::install(&ctx, &crate::config::Config::default());
        let _ = ctx.run(Default::default(), |_| {});
        ctx
    }

    /// A terminal `cols` cells wide at a phone's cell width (about 8 points).
    fn phone(cols: f32) -> Rect {
        Rect::from_min_size(Pos2::ZERO, Vec2::new(cols * 8.0, 700.0))
    }

    const LONG: &str = "Couldn't save the image: No space left on device (os error 28) while writing /data/user/0/nex/cache/upload-3.png";

    fn assert_inside(toast: Rect, area: Rect) {
        assert!(area.shrink(SIDE_MARGIN - 0.01).contains_rect(toast), "toast {toast:?} leaves {area:?}");
    }

    #[test]
    fn short_toast_is_one_line_centered_above_the_bottom() {
        let ctx = ctx();
        let area = Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 800.0));
        let (galley, rect) = layout(&ctx, "Copied 42 characters", area, Color32::WHITE);
        let (unwrapped, _) = layout(&ctx, "Copied 42 characters", Rect::from_min_size(Pos2::ZERO, Vec2::splat(1e5)), Color32::WHITE);
        assert_eq!(galley.rows.len(), 1);
        assert_eq!(galley.size(), unwrapped.size());
        assert!((rect.center().x - area.center().x).abs() < 0.01);
        assert!((rect.bottom() - (area.bottom() - MARGIN)).abs() < 0.01);
    }

    #[test]
    fn long_toast_wraps_to_fit_narrow_screens() {
        let ctx = ctx();
        for cols in [30.0, 35.0, 40.0, 45.0] {
            let area = phone(cols);
            let (galley, rect) = layout(&ctx, LONG, area, Color32::WHITE);
            assert!(galley.rows.len() > 1, "{cols} columns: not wrapped");
            assert!(!galley.elided, "{cols} columns: text cut off");
            assert_inside(rect, area);
        }
    }

    #[test]
    fn unbroken_text_wraps_too() {
        let ctx = ctx();
        let area = phone(30.0);
        let (galley, rect) = layout(&ctx, &"x".repeat(120), area, Color32::WHITE);
        assert!(galley.rows.len() > 1);
        assert_inside(rect, area);
    }

    #[test]
    fn height_is_capped_and_overflow_ends_in_an_ellipsis() {
        let ctx = ctx();
        let area = phone(30.0);
        let (galley, rect) = layout(&ctx, &LONG.repeat(10), area, Color32::WHITE);
        assert_eq!(galley.rows.len(), super::MAX_LINES);
        assert!(galley.elided);
        assert_inside(rect, area);

        // A short area (a phone on its side with the keyboard up) takes fewer lines.
        let short = Rect::from_min_size(Pos2::ZERO, Vec2::new(240.0, 80.0));
        let (galley, rect) = layout(&ctx, LONG, short, Color32::WHITE);
        assert!(galley.rows.len() < super::MAX_LINES);
        assert!(short.contains_rect(rect), "toast {rect:?} leaves {short:?}");
    }

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
