//! The connect screen handing over to the workspace (Android, iOS, `--remote`): once logged
//! in, the connect card's contents fade, the card morphs onto the focused pane's card, the
//! dog in its header glides to the Files tile playing its blep, then a cover of the window's
//! color fades away to show the panes, sidebar and tabs underneath.
//!
//! The workspace draws normally under it; the overlay is painted at the end of the frame, on
//! the foreground layer, once the workspace has said where the focused pane and the Files dog
//! are this frame.

use egui::{Id, LayerId, Order, Rect, Stroke, Vec2};

use crate::app::workspace::draw::{PANE_RADIUS, header_title};
use crate::app::{App, header_height};
use crate::ui::glide::{Lerp, ease_in, ease_in_out, ease_out};
use crate::ui::theme::{HAIRLINE, Theme};
use crate::ui::{dog, paint};

/// The card's title and hairline fading out (ease-in).
const FADE: (f32, f32) = (0.0, 0.25);
/// The card morphing onto the pane, and the dog gliding to the Files tile (ease-in-out).
const MORPH: (f32, f32) = (0.1, 0.75);
/// The cover fading away and the workspace fading in (ease-out). It starts once the card
/// has landed, so the card and the new content never show at once.
const REVEAL: (f32, f32) = (0.75, 1.1);
/// When the overlay is done (the dog's blep may still finish on the tile).
pub(in crate::app) const END: f32 = REVEAL.1;

/// A handoff under way: when it started, and where the connect card and its dog were on the
/// screen's last frame.
pub(in crate::app) struct Handoff {
    pub start: f64,
    pub card: Rect,
    pub dog: Rect,
    /// The card's title then, and whether it had mobile mode's taller header.
    pub title: &'static str,
    pub sidebar: bool,
}

impl Handoff {
    pub(in crate::app) fn elapsed(&self, now: f64) -> f32 {
        (now - self.start).max(0.0) as f32
    }

    /// When the Files dog's intro would have started for it to be where the overlay's dog is:
    /// the tile picks up the blep without a jump, and the launch intro doesn't replay.
    pub(in crate::app) fn dog_wake(&self) -> f64 {
        self.start - dog::BLEP_START as f64
    }
}

/// How far along each phase is, 0 to 1 (linear; each is eased where it's used).
#[derive(Clone, Copy, Debug, PartialEq)]
pub(in crate::app) struct Phases {
    pub fade: f32,
    pub morph: f32,
    pub reveal: f32,
}

/// The timeline: `elapsed` seconds since connecting → each phase's progress.
pub(in crate::app) fn phases(elapsed: f32) -> Phases {
    let at = |(from, to): (f32, f32)| ((elapsed - from) / (to - from)).clamp(0.0, 1.0);
    Phases { fade: at(FADE), morph: at(MORPH), reveal: at(REVEAL) }
}

/// The morphing card's rect, `morph` of the way from the connect card to the pane's.
pub(in crate::app) fn morph_rect(from: Rect, to: Rect, morph: f32) -> Rect {
    Rect::lerp(from, to, ease_in_out(morph))
}

impl App {
    /// Paint the handoff over everything drawn this frame, or end it.
    pub(in crate::app) fn paint_handoff(&mut self, ctx: &egui::Context) {
        let Some(h) = &self.handoff else { return };
        let now = ctx.input(|i| i.time);
        let elapsed = h.elapsed(now);
        if elapsed >= END || !crate::ui::glide::animations(ctx) {
            self.strip.dog_wake = Some(h.dog_wake());
            self.handoff = None;
            ctx.request_repaint();
            return;
        }
        ctx.request_repaint();
        let theme = Theme::from_config(&self.cfg);
        let ph = phases(elapsed);
        let layer = LayerId::new(Order::Foreground, Id::new("connect-handoff"));
        ctx.move_to_top(layer);
        let painter = ctx.layer_painter(layer);
        // Until the workspace has laid out a pane, the card stays where it was.
        let target = self.focused_card.unwrap_or(h.card);
        let shown = 1.0 - ease_out(ph.reveal);

        let mut cover = painter.clone();
        cover.multiply_opacity(shown);
        cover.rect_filled(ctx.viewport_rect(), 0, theme.window);

        let card = morph_rect(h.card, target, ph.morph);
        paint::card(&cover, card, PANE_RADIUS, theme.card, 0.0, None);
        let mut label = cover.with_clip_rect(card.shrink(1.0));
        label.multiply_opacity(1.0 - ease_in(ph.fade));
        let header = Rect::from_min_size(card.min, Vec2::new(card.width(), header_height(h.sidebar)));
        let font = if h.sidebar { 15.0 } else { 13.5 };
        header_title(&label, header, h.title, theme.text_bright, header.right() - 20.0, font);
        label.hline(header.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0_f32, HAIRLINE));

        // The dog stays opaque on the tile once it's there; without a tile it fades with the card.
        let (to, dog_painter) = match self.strip.dog_rect {
            Some(r) => (r, painter.clone()),
            None => (h.dog, cover),
        };
        let at = Rect::lerp(h.dog, to, ease_in_out(ph.morph));
        let dim = (if self.home_active { 0.0 } else { 0.3 }) * ease_in_out(ph.morph);
        dog::paint(&dog_painter, at, dog::BLEP_START + elapsed, (theme.window, dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Pos2;

    #[test]
    fn phases_clamp_to_0_1() {
        for i in -20..=40 {
            let p = phases(i as f32 * 0.1);
            for v in [p.fade, p.morph, p.reveal] {
                assert!((0.0..=1.0).contains(&v), "{p:?} at {i}");
            }
        }
        assert_eq!(phases(-1.0), Phases { fade: 0.0, morph: 0.0, reveal: 0.0 });
        assert_eq!(phases(0.0), Phases { fade: 0.0, morph: 0.0, reveal: 0.0 });
    }

    #[test]
    fn every_phase_is_done_at_the_end() {
        assert_eq!(phases(END), Phases { fade: 1.0, morph: 1.0, reveal: 1.0 });
        assert_eq!(phases(END + 5.0), Phases { fade: 1.0, morph: 1.0, reveal: 1.0 });
    }

    #[test]
    fn the_cover_holds_until_the_card_lands() {
        let p = phases(MORPH.1);
        assert_eq!((p.morph, p.reveal), (1.0, 0.0));
        assert!(phases(MORPH.1 - 0.01).reveal == 0.0);
    }

    #[test]
    fn the_card_lands_exactly_on_the_pane() {
        let from = Rect::from_min_max(Pos2::new(17.3, 88.1), Pos2::new(373.9, 512.7));
        let to = Rect::from_min_max(Pos2::new(6.0, 41.5), Pos2::new(389.25, 801.0));
        assert_eq!(morph_rect(from, to, 1.0), to);
        assert_eq!(morph_rect(from, to, 0.0), from);
        assert_eq!(morph_rect(from, to, phases(END).morph), to);
    }

    #[test]
    fn the_tile_continues_the_overlays_blep() {
        let h = Handoff { start: 100.0, card: Rect::ZERO, dog: Rect::ZERO, title: "", sidebar: true };
        // The tile's `t` is now - dog_wake; the overlay's is BLEP_START + elapsed.
        let now = 100.0 + END as f64;
        assert!(((now - h.dog_wake()) as f32 - (dog::BLEP_START + h.elapsed(now))).abs() < 1e-4);
    }
}
