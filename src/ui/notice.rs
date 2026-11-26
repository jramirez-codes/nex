//! A quiet status card, iOS-style: a light translucent rounded card with a soft tinted dot,
//! a short bold title and one or two lines of softer detail. Used for what a pane wants you
//! to know but needn't act on now ("Not connected", "Password needed"), drawn the same on
//! desktop and on a phone (the detail wraps rather than running off a narrow pane).
//!
//! [`Notices`] keeps a pane's notices between frames, so each fades and eases in when it
//! appears and out when it goes, and the content below glides instead of jumping.

use std::sync::Arc;

use egui::epaint::Shadow;
use egui::text::{LayoutJob, TextFormat, TextWrapping};
use egui::{Color32, Galley, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use super::glide::ease_out;
use super::paint::bold;
use super::theme::{Theme, white_alpha};

/// How long a notice takes to fade in, or out (seconds).
pub const FADE: f64 = 0.28;
const RADIUS: u8 = 12;
const PAD: Vec2 = Vec2::new(12.0, 10.0);
/// The tinted circle at the left, and the gap after it.
const BADGE: f32 = 22.0;
const BADGE_GAP: f32 = 10.0;
const TITLE_SIZE: f32 = 12.5;
const DETAIL_SIZE: f32 = 11.5;
const TITLE_GAP: f32 = 2.0;
/// Most lines of detail; the rest ends in "…".
const DETAIL_LINES: usize = 2;
/// How far a notice drifts down while fading in (points).
const DRIFT: f32 = 4.0;

/// What a notice is about, which tints its dot.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Tone {
    /// Something isn't working (amber).
    Warning,
    /// Worth knowing, nothing wrong (blue).
    Info,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Notice {
    /// What the notice is about (a host), so a notice whose wording changes stays one card.
    pub key: String,
    pub title: String,
    pub detail: String,
    pub tone: Tone,
}

/// A notice laid out for a width.
pub struct Laid {
    title: Arc<Galley>,
    detail: Option<Arc<Galley>>,
    /// The card's full height.
    pub height: f32,
}

/// Lay `n` out for a card `width` wide.
pub fn layout(p: &Painter, n: &Notice, width: f32, theme: &Theme) -> Laid {
    let text_w = (width - 2.0 * PAD.x - BADGE - BADGE_GAP).max(10.0);
    let job = |text: &str, font, color, rows| {
        let mut job = LayoutJob::single_section(text.to_owned(), TextFormat::simple(font, color));
        job.wrap = TextWrapping { max_width: text_w, max_rows: rows, break_anywhere: false, overflow_character: Some('…') };
        job
    };
    let title = p.layout_job(job(&n.title, bold(TITLE_SIZE), theme.text_bright, 1));
    let detail = (!n.detail.is_empty()).then(|| p.layout_job(job(&n.detail, egui::FontId::proportional(DETAIL_SIZE), theme.text_dim, DETAIL_LINES)));
    let text_h = title.size().y + detail.as_ref().map_or(0.0, |d| TITLE_GAP + d.size().y);
    Laid { title, detail, height: 2.0 * PAD.y + text_h.max(BADGE) }
}

/// Paint `n` (laid out as `laid`) in `r`, at `presence` (0 → 1 as it fades in).
pub fn paint(p: &Painter, r: Rect, n: &Notice, laid: &Laid, theme: &Theme, presence: f32) {
    let mut p = p.clone();
    p.multiply_opacity(presence.clamp(0.0, 1.0));
    let r = r.translate(Vec2::new(0.0, -DRIFT * (1.0 - presence)));

    p.add(Shadow { offset: [0, 2], blur: 12, spread: 0, color: Color32::from_black_alpha(36) }.as_shape(r, RADIUS));
    p.rect_filled(r, RADIUS, white_alpha(13));
    p.rect_stroke(r, RADIUS, Stroke::new(1.0_f32, white_alpha(18)), StrokeKind::Inside);

    let tint = tone_color(n.tone, theme);
    let text_h = laid.title.size().y + laid.detail.as_ref().map_or(0.0, |d| TITLE_GAP + d.size().y);
    let badge = Pos2::new(r.left() + PAD.x + BADGE / 2.0, r.top() + PAD.y + BADGE / 2.0);
    p.circle_filled(badge, BADGE / 2.0, tint.gamma_multiply(0.16));
    p.circle_filled(badge, 3.5, tint);

    let x = r.left() + PAD.x + BADGE + BADGE_GAP;
    // Text centered on the badge when it's one line, from the top when it's taller.
    let top = r.top() + PAD.y + ((BADGE - text_h) / 2.0).max(0.0);
    p.galley(Pos2::new(x, top), laid.title.clone(), theme.text_bright);
    if let Some(d) = &laid.detail {
        p.galley(Pos2::new(x, top + laid.title.size().y + TITLE_GAP), d.clone(), theme.text_dim);
    }
}

fn tone_color(t: Tone, theme: &Theme) -> Color32 {
    match t {
        Tone::Warning => theme.yellow,
        Tone::Info => theme.blue,
    }
}

struct Shown {
    notice: Notice,
    /// When it started fading in.
    born: f64,
    /// When it started fading out, once it's gone from the list.
    gone: Option<f64>,
}

/// A pane's notices as they fade in and out.
#[derive(Default)]
pub struct Notices {
    shown: Vec<Shown>,
}

impl Notices {
    /// Catch up with `current` at `now`: new notices start fading in (at the end), ones no
    /// longer there start fading out, and a notice whose wording changed keeps its card.
    /// Returns each notice on screen with its presence (0 to 1). With `animate` off, notices
    /// come and go at once.
    pub fn update(&mut self, now: f64, current: &[Notice], animate: bool) -> Vec<(&Notice, f32)> {
        let at = |s: &Shown| presence(s, now, animate);
        for s in &mut self.shown {
            match current.iter().find(|n| n.key == s.notice.key) {
                Some(n) => {
                    if s.gone.is_some() {
                        // Back before it finished leaving: fade in from where it is.
                        s.born = now - FADE * f64::from(level(s, now));
                        s.gone = None;
                    }
                    s.notice = n.clone();
                }
                None if s.gone.is_none() => s.gone = Some(now),
                None => {}
            }
        }
        for n in current {
            if !self.shown.iter().any(|s| s.notice.key == n.key) {
                self.shown.push(Shown { notice: n.clone(), born: now, gone: None });
            }
        }
        self.shown.retain(|s| s.gone.is_none() || at(s) > 0.0);
        self.shown.iter().map(|s| (&s.notice, at(s))).collect()
    }
}

fn presence(s: &Shown, now: f64, animate: bool) -> f32 {
    if !animate {
        return if s.gone.is_some() { 0.0 } else { 1.0 };
    }
    ease_out(level(s, now))
}

/// How far in a notice is (0 to 1), before easing.
fn level(s: &Shown, now: f64) -> f32 {
    let fade_in = ((now - s.born) / FADE).clamp(0.0, 1.0) as f32;
    match s.gone {
        Some(gone) => fade_in.min((1.0 - (now - gone) / FADE).clamp(0.0, 1.0) as f32),
        None => fade_in,
    }
}

#[cfg(test)]
mod tests {
    use super::{FADE, Notice, Notices, Tone};

    fn note(key: &str, title: &str) -> Notice {
        Notice { key: key.into(), title: title.into(), detail: String::new(), tone: Tone::Warning }
    }

    fn presences(n: &mut Notices, now: f64, current: &[Notice]) -> Vec<(String, f32)> {
        n.update(now, current, true).into_iter().map(|(n, p)| (n.title.clone(), p)).collect()
    }

    #[test]
    fn a_notice_fades_in_then_out() {
        let mut n = Notices::default();
        let a = [note("a", "Not connected")];
        assert_eq!(presences(&mut n, 0.0, &a), [("Not connected".to_string(), 0.0)]);
        let half = presences(&mut n, FADE / 2.0, &a)[0].1;
        assert!(half > 0.0 && half < 1.0);
        assert_eq!(presences(&mut n, FADE, &a)[0].1, 1.0);

        // Gone from the list: still drawn while it fades, then dropped.
        let leaving = presences(&mut n, 10.0 + FADE / 2.0, &[]);
        assert_eq!(leaving.len(), 1);
        assert_eq!(leaving[0].1, 1.0, "it starts fading on the frame it went");
        let leaving = presences(&mut n, 10.0 + FADE, &[]);
        assert!(leaving[0].1 < 1.0 && leaving[0].1 > 0.0);
        assert!(presences(&mut n, 11.0, &[]).is_empty());
    }

    #[test]
    fn new_wording_keeps_the_card() {
        let mut n = Notices::default();
        presences(&mut n, 0.0, &[note("a", "Not connected")]);
        let now = presences(&mut n, 5.0, &[note("a", "Password needed")]);
        assert_eq!(now, [("Password needed".to_string(), 1.0)]);
    }

    #[test]
    fn a_notice_back_while_leaving_fades_in_from_where_it_was() {
        let mut n = Notices::default();
        let a = [note("a", "Not connected")];
        presences(&mut n, 0.0, &a);
        presences(&mut n, 5.0, &a);
        presences(&mut n, 6.0, &[]);
        let mid = presences(&mut n, 6.0 + FADE * 0.6, &[])[0].1;
        let back = presences(&mut n, 6.0 + FADE * 0.6, &a)[0].1;
        assert!((mid - back).abs() < 0.05, "{mid} then {back}");
        assert_eq!(presences(&mut n, 8.0, &a)[0].1, 1.0);
    }

    #[test]
    fn new_notices_go_last_and_without_animations_come_and_go_at_once() {
        let mut n = Notices::default();
        let both = [note("b", "B"), note("a", "A")];
        n.update(0.0, &both[..1], false);
        let order: Vec<_> = n.update(0.0, &both, false).into_iter().map(|(n, p)| (n.key.clone(), p)).collect();
        assert_eq!(order, [("b".to_string(), 1.0), ("a".to_string(), 1.0)]);
        assert_eq!(n.update(0.0, &both[1..], false).len(), 1);
    }
}
