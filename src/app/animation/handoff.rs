//! The connect screen handing over to the workspace (Android, iOS, `--remote`): the connect
//! card *becomes* the focused pane, in one motion. Nothing is laid over the pane and then
//! removed; things only fill in around and inside the one card:
//!
//! - **Body out** — the card's fields, button and notes (a copy of their last frame) fade and
//!   drift up a little, inside the card.
//! - **Morph** — the card glides onto the focused pane's card, its border going from the
//!   connect card's focus to the pane's; the header dog glides to the Files tile, playing its
//!   blep.
//! - **Title** — "Connect to Your Server" cross-fades to the pane's own header label (process
//!   glyph and title), drawn as the pane draws it.
//! - **Chrome in** — the window-colored cover is a frame around the card, and fades while the
//!   card settles: the sidebar, tabs and other panes rise in around it.
//! - **Content in** — once landed, the card (now identical to the pane's, edge and header
//!   alike) fades, showing the live terminal and header buttons through it.
//!
//! The workspace draws normally under it; the overlay is painted at the end of the frame, on
//! the foreground layer, once the workspace has said where the focused pane and the Files dog
//! are this frame.

use egui::epaint::{Mesh, Vertex, WHITE_UV};
use egui::{Color32, Id, LayerId, Order, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::connect::DrawnCard;
use crate::app::workspace::draw::{PANE_RADIUS, header_label, header_title};
use crate::app::{App, header_height};
use crate::ui::glide::{Lerp, ease_in, ease_in_out, ease_out};
use crate::ui::theme::{HAIRLINE, Theme};
use crate::ui::{dog, paint};

/// The connect card's body fading out and drifting up (ease-in).
const BODY: (f32, f32) = (0.0, 0.18);
/// The card morphing onto the pane, and the dog gliding to the Files tile (ease-in-out).
const MORPH: (f32, f32) = (0.05, 0.6);
/// The connect title cross-fading to the pane's header label (ease-in-out).
const TITLE: (f32, f32) = (0.15, 0.5);
/// The cover around the card fading, the workspace's chrome rising in (ease-out). It overlaps
/// the morph's tail, so the card is still settling as the rest arrives.
const CHROME: (f32, f32) = (0.35, 0.75);
/// The landed card fading to show the live pane under it (ease-out).
const CONTENT: (f32, f32) = (0.6, 0.9);
/// When the overlay is done (the dog's blep may still finish on the tile).
pub(in crate::app) const END: f32 = CONTENT.1;
/// How far up the body drifts as it fades.
const DRIFT: f32 = 6.0;

/// A handoff under way: when it started, and the connect card on the screen's last frame.
pub(in crate::app) struct Handoff {
    pub start: f64,
    pub drawn: DrawnCard,
    /// Mobile mode's taller header.
    pub sidebar: bool,
    /// How many frames the overlay has painted (the first two set the clock).
    pub painted: u8,
}

/// The focused pane's card as the workspace drew it this frame: where the handoff lands, and
/// what it looks like there.
pub(in crate::app) struct FocusedCard {
    /// The layout's rect (not where the card is mid-animation).
    pub rect: Rect,
    /// How lit its border is (`paint::card`'s `focus`).
    pub ring: f32,
    /// Its header label, when pane headers are shown.
    pub label: Option<HeaderLabel>,
}

/// A pane header's label as `draw_pane` paints it.
pub(in crate::app) struct HeaderLabel {
    pub process: String,
    pub title: String,
    pub color: Color32,
    /// How far left of the header's right edge the title fades out (the header's buttons).
    pub right_inset: f32,
    pub font: f32,
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
    pub body: f32,
    pub morph: f32,
    pub title: f32,
    pub chrome: f32,
    pub content: f32,
}

/// The timeline: `elapsed` seconds since connecting → each phase's progress.
pub(in crate::app) fn phases(elapsed: f32) -> Phases {
    let at = |(from, to): (f32, f32)| ((elapsed - from) / (to - from)).clamp(0.0, 1.0);
    Phases { body: at(BODY), morph: at(MORPH), title: at(TITLE), chrome: at(CHROME), content: at(CONTENT) }
}

/// The morphing card's rect, `morph` of the way from the connect card to the pane's.
pub(in crate::app) fn morph_rect(from: Rect, to: Rect, morph: f32) -> Rect {
    Rect::lerp(from, to, ease_in_out(morph))
}

/// The morphing card's border focus: the connect card's full focus, eased to the pane's ring.
fn morph_focus(ring: f32, morph: f32) -> f32 {
    let t = ease_in_out(morph);
    // Written so the landed value is exactly the ring, as the pane paints it.
    (1.0 - t) + ring * t
}

/// The cover's and the card's fills. On an opaque surface (Android, iOS) they are flattened
/// onto the black it clears to: what the connect screen's translucent fills came to there, and
/// solid, so nothing of the workspace under the overlay shows before its phase. A see-through
/// window keeps the translucent fills, which are what its connect screen showed.
fn fills(theme: &Theme, transparent: bool) -> (Color32, Color32) {
    if transparent {
        return (theme.window, theme.card);
    }
    let window = Color32::BLACK.blend(theme.window);
    (window, window.blend(theme.card))
}

/// The parts of `window` outside `card`'s rounded rect, as rects and corner fans: the cover
/// as a frame around the card, so the card itself is never covered.
fn frame_around(window: Rect, card: Rect, radius: f32, color: Color32) -> Vec<Shape> {
    let mut shapes: Vec<Shape> = [
        Rect::from_min_max(window.min, Pos2::new(window.right(), card.top())),
        Rect::from_min_max(Pos2::new(window.left(), card.bottom()), window.max),
        Rect::from_min_max(Pos2::new(window.left(), card.top()), Pos2::new(card.left(), card.bottom())),
        Rect::from_min_max(Pos2::new(card.right(), card.top()), Pos2::new(window.right(), card.bottom())),
    ]
    .into_iter()
    .filter(|r| r.is_positive())
    .map(|r| Shape::rect_filled(r, 0, color))
    .collect();
    let r = radius.min(card.width() / 2.0).min(card.height() / 2.0);
    if r > 0.0 {
        let mut mesh = Mesh::default();
        // Each corner: the square corner point, the arc's center, and the angle the arc starts at.
        let corners = [
            (card.left_top(), Vec2::new(r, r), 180.0_f32),
            (card.right_top(), Vec2::new(-r, r), 270.0),
            (card.right_bottom(), Vec2::new(-r, -r), 0.0),
            (card.left_bottom(), Vec2::new(r, -r), 90.0),
        ];
        const STEPS: u32 = 16;
        for (corner, to_center, start) in corners {
            let center = corner + to_center;
            let base = mesh.vertices.len() as u32;
            mesh.vertices.push(Vertex { pos: corner, uv: WHITE_UV, color });
            for i in 0..=STEPS {
                let a = (start + 90.0 * i as f32 / STEPS as f32).to_radians();
                mesh.vertices.push(Vertex { pos: center + r * Vec2::new(a.cos(), a.sin()), uv: WHITE_UV, color });
            }
            for i in 0..STEPS {
                mesh.add_triangle(base, base + 1 + i, base + 2 + i);
            }
        }
        shapes.push(Shape::mesh(mesh));
    }
    shapes
}

impl App {
    /// Paint the handoff over everything drawn this frame, or end it.
    pub(in crate::app) fn paint_handoff(&mut self, ctx: &egui::Context) {
        let Some(h) = &mut self.handoff else { return };
        let now = ctx.input(|i| i.time);
        // The clock starts with the overlay's second frame: the first one is drawn along with
        // the workspace's first, which is slow (panes spawn, glyphs are rasterized), and would
        // otherwise eat the start of the motion.
        match h.painted {
            0 => h.start = now,
            1 => h.start = now - ctx.input(|i| i.predicted_dt) as f64,
            _ => {}
        }
        h.painted = h.painted.saturating_add(1);
        let elapsed = h.elapsed(now);
        if elapsed >= END || !crate::ui::glide::animations(ctx) {
            self.chrome.strip.dog_wake = Some(h.dog_wake());
            self.handoff = None;
            ctx.request_repaint();
            return;
        }
        ctx.request_repaint();
        let theme = Theme::from_config(&self.cfg);
        let ph = phases(elapsed);
        let (window_fill, card_fill) = fills(&theme, crate::platform::WINDOW.transparent);
        let layer = LayerId::new(Order::Foreground, Id::new("connect-handoff"));
        ctx.move_to_top(layer);
        let painter = ctx.layer_painter(layer);
        let from = h.drawn.card;
        // Until the workspace has laid out a pane, the card stays where it was.
        let (target, ring, label) = match &self.focused_card {
            Some(f) => (f.rect, f.ring, f.label.as_ref()),
            None => (from, 1.0, None),
        };
        let card = morph_rect(from, target, ph.morph);

        // The cover, around the card only, and the card's shadow with it: the pane's own
        // shadow takes over as the cover goes.
        let mut cover = painter.clone();
        cover.multiply_opacity(1.0 - ease_out(ph.chrome));
        cover.extend(frame_around(ctx.viewport_rect(), card, PANE_RADIUS as f32, window_fill));
        paint::card_shadow(&cover, card, PANE_RADIUS);

        // The card: opaque until it has landed, then fading over the identical pane under it.
        let mut over = painter.clone();
        over.multiply_opacity(1.0 - ease_out(ph.content));
        over.rect_filled(card, PANE_RADIUS, card_fill);
        let inside = over.with_clip_rect(card.shrink(1.0));

        // The connect card's body, riding along with the card's corner as it fades and drifts up.
        if ph.body < 1.0 {
            let mut body = inside.clone();
            body.multiply_opacity(1.0 - ease_in(ph.body));
            let shift = card.min - from.min - Vec2::new(0.0, DRIFT * ease_in(ph.body));
            for s in &h.drawn.body {
                let clip = s.clip_rect.translate(shift).intersect(body.clip_rect());
                let mut shape = s.shape.clone();
                shape.translate(shift);
                body.with_clip_rect(clip).add(shape);
            }
        }

        // The header: the connect title and hairline out, the pane's label in.
        let header = Rect::from_min_size(card.min, Vec2::new(card.width(), header_height(h.sidebar)));
        let title = ease_in_out(ph.title);
        if title < 1.0 {
            let mut old = inside.clone();
            old.multiply_opacity(1.0 - title);
            let font = if h.sidebar { 15.0 } else { 13.5 };
            header_title(&old, header, h.drawn.title, theme.text_bright, header.right() - 20.0, font);
            old.hline(header.x_range().shrink(1.0), header.bottom(), Stroke::new(1.0_f32, HAIRLINE));
        }
        if let Some(l) = label.filter(|_| title > 0.0) {
            let mut new = inside.clone();
            new.multiply_opacity(title);
            header_label(&new, header, &l.process, &l.title, l.color, header.right() - l.right_inset, l.font);
        }
        paint::card_edge(&over, card, PANE_RADIUS, morph_focus(ring, ph.morph), None);

        // The dog stays opaque on the tile once it's there; without a tile it fades with the cover.
        let (to, dog_painter) = match self.chrome.strip.dog_rect {
            Some(r) => (r, painter.clone()),
            None => (h.drawn.dog, cover),
        };
        let glide = ease_in_out(ph.morph);
        let at = Rect::lerp(h.drawn.dog, to, glide);
        let dim = (if self.home_active { 0.0 } else { 0.3 }) * glide;
        // Dimmed toward what's behind it: the card it starts on, the window it lands on.
        let bg = card_fill.lerp_to_gamma(window_fill, glide);
        dog::paint(&dog_painter, at, dog::BLEP_START + elapsed, (bg, dim));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ZERO: Phases = Phases { body: 0.0, morph: 0.0, title: 0.0, chrome: 0.0, content: 0.0 };
    const DONE: Phases = Phases { body: 1.0, morph: 1.0, title: 1.0, chrome: 1.0, content: 1.0 };

    #[test]
    fn phases_clamp_to_0_1() {
        for i in -20..=40 {
            let p = phases(i as f32 * 0.05);
            for v in [p.body, p.morph, p.title, p.chrome, p.content] {
                assert!((0.0..=1.0).contains(&v), "{p:?} at {i}");
            }
        }
        assert_eq!(phases(-1.0), ZERO);
        assert_eq!(phases(0.0), ZERO);
    }

    #[test]
    fn every_phase_is_done_at_the_end() {
        assert_eq!(phases(END), DONE);
        assert_eq!(phases(END + 5.0), DONE);
        // END is the last phase's end: nothing is still moving when the overlay goes.
        for (_, to) in [BODY, MORPH, TITLE, CHROME, CONTENT] {
            assert!(to <= END);
        }
    }

    #[test]
    fn the_body_starts_fading_on_the_first_frame() {
        // No frame-0 pop: the body is fully there at 0 and already on its way out just after.
        assert_eq!(phases(0.0).body, 0.0);
        assert!(phases(0.01).body > 0.0);
        // Gone before the title has finished changing, well before the card lands.
        assert!(BODY.1 < TITLE.1 && BODY.1 < MORPH.1);
    }

    #[test]
    fn the_chrome_rises_while_the_card_is_still_settling() {
        // The cover starts going before the card lands (one motion, not morph → pause → fade)…
        assert!(CHROME.0 < MORPH.1 && MORPH.1 < CHROME.1);
        let p = phases((CHROME.0 + MORPH.1) / 2.0);
        assert!(p.chrome > 0.0 && p.morph < 1.0);
        // …and the card's contents only fade once it has landed and shows the pane's label.
        assert!(CONTENT.0 >= MORPH.1 && CONTENT.0 >= TITLE.1);
        let p = phases(CONTENT.0);
        assert_eq!((p.morph, p.title, p.content), (1.0, 1.0, 0.0));
    }

    #[test]
    fn the_card_lands_exactly_on_the_pane() {
        let from = Rect::from_min_max(Pos2::new(17.3, 88.1), Pos2::new(373.9, 512.7));
        let to = Rect::from_min_max(Pos2::new(6.0, 41.5), Pos2::new(389.25, 801.0));
        assert_eq!(morph_rect(from, to, 1.0), to);
        assert_eq!(morph_rect(from, to, 0.0), from);
        assert_eq!(morph_rect(from, to, phases(CONTENT.0).morph), to);
    }

    #[test]
    fn the_border_goes_from_the_connect_card_to_the_panes_ring() {
        for ring in [0.0, 0.4, 1.0] {
            // The connect card paints focus 1.0; the landed card, the pane's own ring.
            assert_eq!(morph_focus(ring, 0.0), 1.0);
            assert_eq!(morph_focus(ring, 1.0), ring);
            let mid = morph_focus(ring, 0.5);
            assert!((ring.min(1.0)..=1.0).contains(&mid));
        }
    }

    #[test]
    fn the_cover_is_a_frame_around_the_card() {
        let window = Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 800.0));
        let card = Rect::from_min_max(Pos2::new(20.0, 60.0), Pos2::new(380.0, 700.0));
        let shapes = frame_around(window, card, 12.0, Color32::BLACK);
        let mut rects = Vec::new();
        let mut meshes = 0;
        for s in &shapes {
            match s {
                Shape::Rect(r) => rects.push(r.rect),
                Shape::Mesh(m) => {
                    meshes += 1;
                    // The corner fans stay in the card's corner squares, outside its rounded rect.
                    for v in &m.vertices {
                        assert!(card.expand(0.01).contains(v.pos), "{:?}", v.pos);
                        let inner = card.shrink(12.0);
                        assert!(!inner.contains(v.pos) || inner.distance_to_pos(v.pos) == 0.0);
                    }
                }
                other => panic!("unexpected shape {other:?}"),
            }
        }
        assert_eq!(meshes, 1);
        assert_eq!(rects.len(), 4);
        // Nothing of the four rects covers the card; together with it they tile the window.
        for r in &rects {
            assert!(r.intersect(card).area() <= 0.0, "{r:?}");
        }
        let area: f32 = rects.iter().map(|r| r.area()).sum::<f32>() + card.area();
        assert!((area - window.area()).abs() < 0.01);
        // A card past the window's edge leaves no inverted rects.
        let off = Rect::from_min_max(Pos2::new(-10.0, -10.0), Pos2::new(410.0, 810.0));
        assert!(frame_around(window, off, 12.0, Color32::BLACK).iter().all(|s| matches!(s, Shape::Mesh(_))));
    }

    #[test]
    fn an_opaque_surface_gets_solid_fills() {
        let mut theme = Theme::from_config(&crate::config::Config::default());
        theme.window = Color32::from_rgba_unmultiplied(20, 20, 24, 230);
        theme.card = Color32::from_rgba_unmultiplied(30, 30, 36, 230);
        // A see-through window keeps what its connect screen showed.
        assert_eq!(fills(&theme, true), (theme.window, theme.card));
        // An opaque one: the same fills as they came out over black, hiding what's under them.
        let (window, card) = fills(&theme, false);
        assert!(window.is_opaque() && card.is_opaque());
        assert_eq!(window, Color32::BLACK.blend(theme.window));
        assert_eq!(card, Color32::BLACK.blend(theme.window).blend(theme.card));
        // Already solid fills are left as they are.
        theme.window = Color32::from_rgb(20, 20, 24);
        theme.card = Color32::from_rgb(30, 30, 36);
        assert_eq!(fills(&theme, false), (theme.window, theme.card));
    }

    #[test]
    fn the_tile_continues_the_overlays_blep() {
        let drawn = DrawnCard { card: Rect::ZERO, dog: Rect::ZERO, title: "", body: Vec::new() };
        let h = Handoff { start: 100.0, drawn, sidebar: true, painted: 2 };
        // The tile's `t` is now - dog_wake; the overlay's is BLEP_START + elapsed.
        let now = 100.0 + END as f64;
        assert!(((now - h.dog_wake()) as f32 - (dog::BLEP_START + h.elapsed(now))).abs() < 1e-4);
    }
}
