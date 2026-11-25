//! The "Viewing" mark: how a card shows that its agent is on screen (see
//! [`Viewing`](crate::agents::model::Viewing)). A small pill with an eye left of the status
//! pill, and a soft accent ring around the card: full, with a glow, for the agent in the
//! focused pane; fainter for one sharing its tab. A closed team's card is marked when a
//! helper is in view; in an open team, the card is marked only for its lead, and a viewed
//! helper's row gets a soft rounded accent tint, like a selected row in an inset list, and
//! an eye. Each eases in and out ([`SeenFade`](crate::agents::motion::SeenFade)): drawn
//! here `shown` (0 to 1) of the way in, `primary` (0 to 1) of the way from a neighbour's
//! look to the focused pane's.

use egui::epaint::Shadow;
use egui::{Color32, Painter, Pos2, Rect, Vec2};

use crate::agents::group::Group;
use crate::agents::model::{Seen, Viewing};
use crate::ui::glide::Lerp;
use crate::ui::icons;
use crate::ui::paint;
use crate::ui::theme::{ACCENT, white_alpha};

use super::CARD_RADIUS;

/// The pill's height, and the eye's size inside it.
const PILL_H: f32 = 20.0;
const EYE: f32 = 13.0;
/// Narrower than this left for the title, the pill keeps only its eye.
const TITLE_ROOM_MIN: f32 = 120.0;
const WORD: &str = "Viewing";

/// How a team's card is on screen: as its lead is, else (a helper in view) as a neighbour.
/// Once the card is `open`, a viewed helper's own row says so, so only the lead marks the card.
pub(super) fn team_seen(viewing: &Viewing, g: &Group, open: bool) -> Option<Seen> {
    viewing.of(&g.lead).or_else(|| (!open && g.members.iter().any(|m| viewing.of(&m.agent).is_some())).then_some(Seen::Shared))
}

/// The ring a viewed card heads for (see `border_for`).
pub(super) fn ring(seen: Seen) -> (Color32, f32) {
    match seen {
        Seen::Primary => (ACCENT.gamma_multiply(0.7), 1.5),
        Seen::Shared => (ACCENT.gamma_multiply(0.3), 1.0),
    }
}

/// `base` (the border the card has otherwise), `shown` of the way to its viewing ring.
pub(super) fn ring_over(base: (Color32, f32), shown: f32, primary: f32) -> (Color32, f32) {
    let ((shared, shared_w), (main, main_w)) = (ring(Seen::Shared), ring(Seen::Primary));
    let (color, width) = (Color32::lerp(shared, main, primary), f32::lerp(shared_w, main_w, primary));
    (Color32::lerp(base.0, color, shown), f32::lerp(base.1, width, shown))
}

/// The eye is drawn open for the focused pane's look.
fn open(primary: f32) -> bool {
    primary >= 0.5
}

/// The soft accent glow around the primary card, `t` (0 to 1) of the way in.
pub(super) fn glow(p: &Painter, r: Rect, t: f32) {
    if t <= 0.01 {
        return;
    }
    let shadow = Shadow { offset: [0, 0], blur: 18, spread: 1, color: ACCENT.gamma_multiply(0.22 * t) };
    p.add(shadow.as_shape(r, CARD_RADIUS));
}

/// How wide the pill is, with its word or (`compact`) only its eye.
fn pill_width(p: &Painter, compact: bool) -> f32 {
    if compact { PILL_H + 4.0 } else { 9.0 + EYE + 5.0 + p.layout_no_wrap(WORD.into(), paint::bold(11.0), ACCENT).size().x + 10.0 }
}

/// Whether the pill drops its word: when, with it, the title would have less than
/// [`TITLE_ROOM_MIN`] between `title_left` and the pill ending at `right`.
pub(super) fn compact(p: &Painter, title_left: f32, right: f32) -> bool {
    right - pill_width(p, false) - 8.0 - title_left < TITLE_ROOM_MIN
}

/// The pill, ending at `right` and centred on `y`, `t` (0 to 1) of the way in. Returns
/// its left edge (where the title must stop).
pub(super) fn pill(p: &Painter, right: f32, y: f32, primary: f32, t: f32, compact: bool) -> f32 {
    let w = pill_width(p, compact);
    let r = Rect::from_min_size(Pos2::new(right - w, y - PILL_H / 2.0), Vec2::new(w, PILL_H));
    if t <= 0.01 {
        return r.left();
    }
    let ink = Color32::lerp(ACCENT.gamma_multiply(0.65), ACCENT, primary).gamma_multiply(t);
    let fill = Color32::lerp(white_alpha(10), ACCENT.gamma_multiply(0.14), primary).gamma_multiply(t);
    p.rect_filled(r, PILL_H / 2.0, fill);
    let eye_x = if compact { r.center().x } else { r.left() + 9.0 + EYE / 2.0 };
    icons::icon_eye(p, Rect::from_center_size(Pos2::new(eye_x, y), Vec2::splat(EYE)), ink, open(primary));
    if !compact {
        let word = p.layout_no_wrap(WORD.into(), paint::bold(11.0), ink);
        p.galley(Pos2::new(eye_x + EYE / 2.0 + 5.0, y - word.size().y / 2.0), word, ink);
    }
    r.left()
}

/// The tint behind a viewed helper's row, `shown` of the way in: the pill's fill for the
/// agent in the focused pane (`primary` 1), half as strong for one sharing its tab.
fn row_tint(shown: f32, primary: f32) -> Color32 {
    Color32::lerp(ACCENT.gamma_multiply(0.07), ACCENT.gamma_multiply(0.14), primary).gamma_multiply(shown)
}

/// A helper's row's ink, `shown` of the way in.
fn row_ink(dim: f32, shown: f32, primary: f32) -> Color32 {
    Color32::lerp(ACCENT.gamma_multiply(dim), ACCENT, primary).gamma_multiply(shown)
}

/// A viewed helper's row in an open team: its inner rect filled with a soft accent tint,
/// rounded like the hover fill (which is drawn over it), `shown` of the way in.
pub(super) fn row_fill(p: &Painter, inner: Rect, shown: f32, primary: f32) {
    if shown <= 0.01 {
        return;
    }
    p.rect_filled(inner, 8, row_tint(shown, primary));
}

/// The eye on a viewed helper's row, centred on `at`, `shown` of the way in.
pub(super) fn row_eye(p: &Painter, at: Pos2, shown: f32, primary: f32) {
    if shown <= 0.01 {
        return;
    }
    icons::icon_eye(p, Rect::from_center_size(at, Vec2::splat(12.0)), row_ink(0.65, shown, primary), open(primary));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::group::groups;
    use crate::agents::model::{Agent, AgentKind, Agents, Source, Status, Team};
    use crate::layout::PaneId;
    use std::time::Instant;

    fn agent(pane: PaneId, parent: Option<PaneId>) -> Agent {
        Agent {
            pane,
            tab: Some(0),
            kind: AgentKind::Claude,
            name: format!("a{pane}"),
            task: None,
            session: None,
            mark: None,
            cwd: None,
            status: Status::Working,
            since: Instant::now(),
            source: Source::Local,
            team: Team { parent, role: None, started: pane, sub: false },
        }
    }

    #[test]
    fn a_team_is_seen_as_its_lead_is_or_as_a_neighbour_through_a_helper() {
        let teams = groups(&Agents::new(vec![agent(1, None), agent(2, Some(1)), agent(3, Some(1)), agent(9, None)]));
        let team = teams.iter().find(|g| g.key() == 1).unwrap();
        let on = |panes: &[PaneId], primary| Viewing { panes: panes.to_vec(), primary };
        for open in [false, true] {
            assert_eq!(team_seen(&on(&[1, 9], Some(1)), team, open), Some(Seen::Primary));
            assert_eq!(team_seen(&on(&[1, 9], Some(9)), team, open), Some(Seen::Shared));
            assert_eq!(team_seen(&on(&[1, 3], Some(3)), team, open), Some(Seen::Shared), "the lead in view still marks the card");
            assert_eq!(team_seen(&on(&[9], Some(9)), team, open), None);
        }
        assert_eq!(team_seen(&on(&[3, 9], Some(3)), team, false), Some(Seen::Shared), "a helper in view marks a closed team as a neighbour");
        assert_eq!(team_seen(&on(&[3, 9], Some(3)), team, true), None, "open, the helper's row shows it, not the card");
    }

    #[test]
    fn a_viewed_row_is_tinted_fuller_for_the_focused_pane_than_for_a_neighbour() {
        let (primary, shared) = (row_tint(1.0, 1.0), row_tint(1.0, 0.0));
        assert_eq!(primary, ACCENT.gamma_multiply(0.14), "the focused agent's row matches the pill's fill");
        assert!(shared.a() < primary.a(), "a neighbour's row is fainter");
        assert!(shared.a() > 0);
    }

    #[test]
    fn the_pill_keeps_only_its_eye_when_the_title_would_be_squeezed() {
        let ctx = egui::Context::default();
        crate::render::fonts::install(&ctx, &crate::config::Config::default());
        let _ = ctx.run(egui::RawInput::default(), |_| {});
        let p = Painter::new(ctx, egui::LayerId::background(), Rect::EVERYTHING);
        assert!(!compact(&p, 0.0, 400.0), "a wide card has room for the word");
        assert!(compact(&p, 0.0, 180.0), "a narrow one shows the eye alone");
        assert!(pill_width(&p, true) < pill_width(&p, false));
    }
}
