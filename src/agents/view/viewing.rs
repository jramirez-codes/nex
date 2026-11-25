//! The "Viewing" mark: how a card shows that its agent is on screen (see
//! [`Viewing`](crate::agents::model::Viewing)). A small pill with an eye left of the status
//! pill, and a soft accent ring around the card: full, with a glow, for the agent in the
//! focused pane; fainter for one sharing its tab. A team's card is marked only for its lead
//! (or pipeline); a viewed helper or stage gets the same ring and glow on its own row,
//! rounded like the row, and an eye, never both at once. A closed team opens when one of
//! its helpers comes into view, so the row is there to mark. Each eases in and out ([`SeenFade`](crate::agents::motion::SeenFade)): drawn
//! here `shown` (0 to 1) of the way in, `primary` (0 to 1) of the way from a neighbour's
//! look to the focused pane's.

use egui::epaint::Shadow;
use egui::{Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::agents::group::{Group, Member};
use crate::agents::model::{Seen, Viewing};
use crate::layout::PaneId;
use crate::ui::glide::Lerp;
use crate::ui::icons;
use crate::ui::paint;
use crate::ui::theme::{ACCENT, white_alpha};

/// The pill's height, and the eye's size inside it.
const PILL_H: f32 = 20.0;
const EYE: f32 = 13.0;
/// Narrower than this left for the title, the pill keeps only its eye.
const TITLE_ROOM_MIN: f32 = 120.0;
const WORD: &str = "Viewing";

/// How a team's card is on screen: as its lead is. A helper in view marks its own row
/// instead ([`member_seen`]), never the card.
pub(super) fn team_seen(viewing: &Viewing, g: &Group) -> Option<Seen> {
    viewing.of(&g.lead)
}

/// How a helper's row is on screen: as the helper is (a sub-agent by its own tab, as a
/// stage), unless the pane showing it is the one showing the agent that started it, which is
/// marked instead, so one look never marks two items.
pub(super) fn member_seen(viewing: &Viewing, g: &Group, m: &Member) -> Option<Seen> {
    let parent = m.agent.team.parent.and_then(|p| if p == g.lead.pane { Some(&*g.lead) } else { g.members.iter().find(|o| o.agent.pane == p).map(|o| &*o.agent) });
    let shown = m.agent.shown_pane();
    if parent.is_some_and(|p| p.shown_pane() == shown) {
        return None;
    }
    viewing.of(&m.agent)
}

/// The teams to open now that `viewing` is on screen (it was `was`): each closed one with a
/// helper that has just come into view, and whether it must show all its rows to list it
/// (it's past the first `rows_shown`). Only a change opens a team, so one you close stays
/// closed while you keep looking.
pub(super) fn teams_to_open(viewing: &Viewing, was: &Viewing, teams: &[std::sync::Arc<Group>], rows_shown: usize) -> Vec<(PaneId, bool)> {
    if viewing == was {
        return Vec::new();
    }
    teams
        .iter()
        .filter(|g| g.is_team())
        .filter_map(|g| {
            let new = |m: &Member| member_seen(viewing, g, m).is_some() && member_seen(was, g, m).is_none();
            let i = g.members.iter().position(new)?;
            Some((g.key(), i >= rows_shown))
        })
        .collect()
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

/// How strong the glow is, `shown` of the way in and `primary` of the way to the focused
/// pane's look: only the focused pane's agent glows.
pub(super) fn glow_strength(shown: f32, primary: f32) -> f32 {
    shown * primary
}

/// The soft accent glow around the primary item (`r`, rounded `radius`), `t` (0 to 1) of
/// the way in: a card's ([`CARD_RADIUS`](super::CARD_RADIUS)) or a helper row's.
pub(super) fn glow(p: &Painter, r: Rect, radius: u8, t: f32) {
    if t <= 0.01 {
        return;
    }
    let shadow = Shadow { offset: [0, 0], blur: 18, spread: 1, color: ACCENT.gamma_multiply(0.22 * t) };
    p.add(shadow.as_shape(r, radius));
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

/// A helper's row's ink, `shown` of the way in.
fn row_ink(dim: f32, shown: f32, primary: f32) -> Color32 {
    Color32::lerp(ACCENT.gamma_multiply(dim), ACCENT, primary).gamma_multiply(shown)
}

/// The ring a viewed helper's row has, `shown` of the way in: a card's ring
/// ([`ring_over`]) faded in from nothing, as the row has no border of its own.
pub(super) fn row_ring(shown: f32, primary: f32) -> (Color32, f32) {
    ring_over((Color32::TRANSPARENT, 0.0), shown, primary)
}

/// A viewed helper's row in an open team (`inner`, rounded `radius`): a viewed card's glow
/// and ring, `shown` of the way in. `ring` false leaves the ring to the keyboard's (as a
/// card does).
pub(super) fn row_mark(p: &Painter, inner: Rect, radius: u8, shown: f32, primary: f32, ring: bool) {
    if shown <= 0.01 {
        return;
    }
    glow(p, inner, radius, glow_strength(shown, primary));
    if ring {
        let (color, width) = row_ring(shown, primary);
        p.rect_stroke(inner, radius, Stroke::new(width, color), StrokeKind::Inside);
    }
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
            kind: AgentKind::CLAUDE,
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
    fn a_teams_card_is_seen_only_as_its_lead_and_a_helper_only_on_its_own_row() {
        let teams = groups(&Agents::new(vec![agent(1, None), agent(2, Some(1)), agent(3, Some(1)), agent(9, None)]));
        let team = teams.iter().find(|g| g.key() == 1).unwrap();
        let on = |panes: &[PaneId], primary| Viewing { panes: panes.to_vec(), primary };
        let row = |v: &Viewing, pane| member_seen(v, team, team.members.iter().find(|m| m.agent.pane == pane).unwrap());
        assert_eq!(team_seen(&on(&[1, 9], Some(1)), team), Some(Seen::Primary));
        assert_eq!(team_seen(&on(&[1, 9], Some(9)), team), Some(Seen::Shared));
        assert_eq!(team_seen(&on(&[9], Some(9)), team), None);
        // A helper in view marks its row, at the strength its pane gives, never the card.
        let child = on(&[3, 9], Some(3));
        assert_eq!((team_seen(&child, team), row(&child, 3), row(&child, 2)), (None, Some(Seen::Primary), None));
        assert_eq!(row(&on(&[3, 9], Some(9)), 3), Some(Seen::Shared));
        // The lead in view marks the card and no row.
        let lead = on(&[1], Some(1));
        assert_eq!((team_seen(&lead, team), row(&lead, 2), row(&lead, 3)), (Some(Seen::Primary), None, None));
    }

    #[test]
    fn a_sub_agents_tab_marks_its_row_as_a_stages_does_and_its_agents_pane_does_not() {
        let with_tab = |pane| {
            let mut sub = agent(5, Some(1));
            sub.source = Source::Sub { lead: 1, id: "x".into(), pane };
            sub.team.sub = true;
            groups(&Agents::new(vec![agent(1, None), sub]))
        };
        // No tab on it: its agent's pane in view marks the card, not the row.
        let teams = with_tab(None);
        let team = &teams[0];
        let lead = Viewing { panes: vec![1], primary: Some(1) };
        assert_eq!((team_seen(&lead, team), member_seen(&lead, team, &team.members[0])), (Some(Seen::Primary), None));
        // Its tab (pane 9) in view: the row, as a stage's, and a closed card opens for it.
        let teams = with_tab(Some(9));
        let team = &teams[0];
        let tab = Viewing { panes: vec![9], primary: Some(9) };
        assert_eq!((team_seen(&tab, team), member_seen(&tab, team, &team.members[0])), (None, Some(Seen::Primary)));
        assert_eq!(member_seen(&lead, team, &team.members[0]), None);
        assert_eq!(teams_to_open(&tab, &Viewing::default(), &teams, 6), vec![(1, false)]);
    }

    #[test]
    fn a_helper_coming_into_view_opens_its_team_once() {
        let mut list = vec![agent(1, None)];
        list.extend((2..=9).map(|p| agent(p, Some(1))));
        let teams = groups(&Agents::new(list));
        let none = Viewing::default();
        let first = teams[0].members[0].agent.pane;
        let last = teams[0].members.last().unwrap().agent.pane;
        let on = |pane| Viewing { panes: vec![pane], primary: Some(pane) };
        assert_eq!(teams_to_open(&on(first), &none, &teams, 6), vec![(1, false)]);
        assert_eq!(teams_to_open(&on(last), &none, &teams, 6), vec![(1, true)], "past the first rows, it shows them all");
        assert_eq!(teams_to_open(&on(first), &on(first), &teams, 6), vec![], "unchanged, a team you closed stays closed");
        assert_eq!(teams_to_open(&on(1), &none, &teams, 6), vec![], "the lead in view marks the card, no need to open");
    }

    #[test]
    fn a_viewed_row_has_a_cards_ring_and_glow() {
        // All the way in, the row's ring is the card's, exactly, for either look.
        assert_eq!(row_ring(1.0, 1.0), ring(Seen::Primary));
        assert_eq!(row_ring(1.0, 0.0), ring(Seen::Shared));
        assert_eq!(row_ring(0.0, 1.0).0.a(), 0, "out of view, no ring");
        // Card and row glow the same: fully for the focused pane's agent, not for a neighbour.
        assert_eq!((glow_strength(1.0, 1.0), glow_strength(1.0, 0.0)), (1.0, 0.0));
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
