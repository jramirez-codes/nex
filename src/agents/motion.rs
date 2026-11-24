//! How the Agents pane's cards and section headers move: each remembers where it was drawn and
//! in what colors, and eases toward where the layout wants it now. A card whose agent changes
//! state glides out of one section into another while the cards around it slide over to
//! make room; cards and headers fade in when they appear and out when they go; borders and
//! status tints cross-fade.
//!
//! Inside a team's card the same goes for its helpers ([`TeamMotion`], by the team's lead):
//! when one changes state and the order changes, its row in the open list and its capsule
//! in the closed card's strip glide to their new places, the others making room. A row's
//! place is its offset from the top of the list and a capsule's from the left of the strip,
//! so the card moving or growing around them doesn't set them off.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use egui::{Color32, Rect};

use crate::layout::PaneId;
use crate::ui::glide::{Lerp, ease_in_out, ease_out};

use super::group::Member;
use super::model::{Agent, Status};

/// A card or header moving to its new place.
pub const MOVE: f64 = 0.45;
/// Appearing or going away.
pub const FADE: f64 = 0.25;
/// A border or tint changing color.
pub const COLOR: f64 = 0.3;

/// A value easing from where it was toward its latest target over `duration` seconds.
pub struct Tween<T: Lerp> {
    from: T,
    to: T,
    start: f64,
    duration: f64,
    ease: fn(f32) -> f32,
}

impl<T: Lerp> Tween<T> {
    pub fn new(v: T, duration: f64, ease: fn(f32) -> f32) -> Self {
        Tween { from: v, to: v, start: f64::NEG_INFINITY, duration, ease }
    }

    fn progress(&self, now: f64) -> f32 {
        ((now - self.start) / self.duration).clamp(0.0, 1.0) as f32
    }

    pub fn get(&self, now: f64) -> T {
        T::lerp(self.from, self.to, (self.ease)(self.progress(now)))
    }

    /// Head for `target` (from wherever the value is now); `animate` false jumps there.
    pub fn set(&mut self, now: f64, target: T, animate: bool) {
        if !animate {
            self.snap(target);
        } else if target != self.to {
            self.from = self.get(now);
            self.to = target;
            self.start = now;
        }
    }

    /// Where it's headed.
    pub fn target(&self) -> T {
        self.to
    }

    pub fn snap(&mut self, v: T) {
        self.from = v;
        self.to = v;
        self.start = f64::NEG_INFINITY;
    }

    /// Still on its way.
    pub fn moving(&self, now: f64) -> bool {
        self.progress(now) < 1.0
    }

    /// How far along the current move is, 0 to 1 (1 when settled).
    pub fn t(&self, now: f64) -> f32 {
        self.progress(now)
    }
}

/// A card's place and colors, plus the agent it showed last (to draw it while it fades out
/// after its pane is gone): the tracker's own, shared, not a copy.
pub struct CardMotion {
    pub agent: Arc<Agent>,
    /// In content coordinates (scrolling doesn't animate).
    pub rect: Tween<Rect>,
    pub border: Tween<Color32>,
    pub border_width: Tween<f32>,
    pub tint: Tween<Color32>,
    pub opacity: Tween<f32>,
    pub leaving: bool,
    /// Just created: its border and tint start out in their colors instead of fading in.
    pub new_colors: bool,
    /// Its current move is into another section (not just making room for one that is):
    /// only such a card lifts off the grid on its way.
    pub section_change: bool,
    /// When it last bubbled for its tab being switched to.
    pub pulse_start: f64,
}

impl CardMotion {
    pub fn new(agent: Arc<Agent>, rect: Rect) -> Self {
        CardMotion {
            agent,
            rect: Tween::new(rect, MOVE, ease_in_out),
            border: Tween::new(Color32::TRANSPARENT, COLOR, ease_out),
            border_width: Tween::new(1.0, COLOR, ease_out),
            tint: Tween::new(Color32::TRANSPARENT, COLOR, ease_out),
            opacity: Tween::new(0.0, FADE, ease_out),
            leaving: false,
            new_colors: true,
            section_change: false,
            pulse_start: f64::NEG_INFINITY,
        }
    }

    /// Head for these colors; a new card takes them at once.
    pub fn set_colors(&mut self, now: f64, border: Color32, border_width: f32, tint: Color32, animate: bool) {
        let animate = animate && !std::mem::take(&mut self.new_colors);
        self.border.set(now, border, animate);
        self.border_width.set(now, border_width, animate);
        self.tint.set(now, tint, animate);
    }

    /// Bubble once: its tab was just switched to.
    pub fn pulse(&mut self, now: f64) {
        self.pulse_start = now;
    }

    /// How far into its bubble it is, 0 to 1, while one plays.
    fn pulse_t(&self, now: f64) -> Option<f32> {
        let p = (now - self.pulse_start) / MOVE;
        (0.0..1.0).contains(&p).then_some(p as f32)
    }

    /// How far it is raised off the grid, 0 to 1. Only an agent on the shown tab ever lifts:
    /// up and back down when that tab is switched to, or over its own move into another
    /// section. The rest stay flat.
    pub fn lift(&self, now: f64, shown_tab: Option<usize>) -> f32 {
        if self.agent.tab.is_none() || self.agent.tab != shown_tab {
            return 0.0;
        }
        let arc = |t: f32| (t * std::f32::consts::PI).sin();
        let moving = if self.section_change && self.rect.moving(now) { arc(self.rect.t(now)) } else { 0.0 };
        moving.max(self.pulse_t(now).map_or(0.0, arc))
    }

    /// Whether anything about it is still changing.
    pub fn busy(&self, now: f64) -> bool {
        self.pulse_t(now).is_some() || self.rect.moving(now) || self.border.moving(now) || self.border_width.moving(now) || self.tint.moving(now) || self.opacity.moving(now)
    }
}

/// A section header's place and how visible it is.
pub struct SectionMotion {
    pub count: usize,
    pub rect: Tween<Rect>,
    pub opacity: Tween<f32>,
    pub leaving: bool,
}

impl SectionMotion {
    pub fn new(count: usize, rect: Rect) -> Self {
        SectionMotion { count, rect: Tween::new(rect, MOVE, ease_in_out), opacity: Tween::new(0.0, FADE, ease_out), leaving: false }
    }

    pub fn busy(&self, now: f64) -> bool {
        self.rect.moving(now) || self.opacity.moving(now)
    }
}

/// A helper's row in an open team's list, plus the helper it showed last (to draw it while
/// it fades out after its helper is gone, or folded away by "Show Fewer").
pub struct RowMotion {
    pub member: Member,
    pub tag: String,
    /// Its top, from the top of the list.
    pub y: Tween<f32>,
    pub opacity: Tween<f32>,
    pub leaving: bool,
    /// Its current move is its own helper changing state (not just making room for one
    /// that did): only such a row is drawn over the others on its way.
    pub status_change: bool,
    /// When its helper last changed state: the latest one is drawn on top.
    pub changed: f64,
}

impl RowMotion {
    fn new(member: Member, tag: &str, y: f32) -> Self {
        RowMotion {
            member,
            tag: tag.to_string(),
            y: Tween::new(y, MOVE, ease_in_out),
            opacity: Tween::new(0.0, FADE, ease_out),
            leaving: false,
            status_change: false,
            changed: f64::NEG_INFINITY,
        }
    }

    /// On its way to another slot.
    pub fn sliding(&self, now: f64) -> bool {
        self.y.moving(now)
    }

    pub fn busy(&self, now: f64) -> bool {
        self.y.moving(now) || self.opacity.moving(now)
    }
}

/// A helper's capsule in a closed team's strip.
pub struct CapsuleMotion {
    /// Its left edge, from the left of the strip.
    pub x: Tween<f32>,
    pub opacity: Tween<f32>,
    pub leaving: bool,
}

impl CapsuleMotion {
    pub fn busy(&self, now: f64) -> bool {
        self.x.moving(now) || self.opacity.moving(now)
    }
}

/// Where the helpers inside one team's card are on their way to: the rows of its list
/// while it's open, the capsules of its strip while it's closed.
#[derive(Default)]
pub struct TeamMotion {
    pub rows: HashMap<PaneId, RowMotion>,
    pub capsules: HashMap<PaneId, CapsuleMotion>,
    /// How wide the card was last laid out: when that changes, everything inside is put in
    /// place instead of sliding.
    pub width: f32,
}

impl TeamMotion {
    /// Note the width the card is laid out at. Returns whether it changed (a resize, or the
    /// card opening across the row): what's inside then jumps into place.
    pub fn resized(&mut self, width: f32) -> bool {
        let changed = (self.width - width).abs() >= 0.5;
        self.width = width;
        changed
    }

    /// Head each of the list's rows (`rows`: the helper, its tag, and its slot's top from
    /// the top of the list) for its slot. New ones fade in where they belong, ones no
    /// longer listed fade out where they were. `animate` false, or a list that has just
    /// opened, puts them in place.
    pub fn update_rows(&mut self, now: f64, animate: bool, rows: &[(&Member, &str, f32)]) {
        let animate = animate && !self.rows.is_empty();
        for &(member, tag, y) in rows {
            let r = self.rows.entry(member.agent.pane).or_insert_with(|| RowMotion::new(member.clone(), tag, y));
            // A new status is what moves a row; a settled one only pushed along by the
            // others slides under it.
            if !animate {
                r.status_change = false;
            } else if r.member.agent.status != member.agent.status {
                r.status_change = true;
                r.changed = now;
            } else if !r.y.moving(now) {
                r.status_change = false;
            }
            r.member = member.clone();
            r.tag = tag.to_string();
            r.leaving = false;
            r.y.set(now, y, animate);
            r.opacity.set(now, 1.0, animate);
        }
        for (pane, r) in &mut self.rows {
            if !rows.iter().any(|(m, ..)| m.agent.pane == *pane) {
                r.leaving = true;
                r.opacity.set(now, 0.0, animate);
            }
        }
        self.forget_gone(now);
    }

    /// Head each of the strip's capsules (`capsules`: the helper's pane and its left edge
    /// from the left of the strip) for its place, by the same rules as the rows: one that
    /// is new, or back from the `+N`, fades in, one no longer shown fades out.
    pub fn update_capsules(&mut self, now: f64, animate: bool, capsules: &[(PaneId, f32)]) {
        let animate = animate && !self.capsules.is_empty();
        for &(pane, x) in capsules {
            let c = self.capsules.entry(pane).or_insert_with(|| CapsuleMotion {
                x: Tween::new(x, MOVE, ease_in_out),
                opacity: Tween::new(0.0, FADE, ease_out),
                leaving: false,
            });
            // One fading back in comes back where it belongs now.
            let slide = animate && !std::mem::take(&mut c.leaving);
            c.x.set(now, x, slide);
            c.opacity.set(now, 1.0, animate);
        }
        for (pane, c) in &mut self.capsules {
            if !capsules.iter().any(|(p, _)| p == pane) {
                c.leaving = true;
                c.opacity.set(now, 0.0, animate);
            }
        }
        self.forget_gone(now);
    }

    /// The list is closed: its rows appear in place the next time it opens.
    pub fn close_rows(&mut self) {
        self.rows.clear();
    }

    /// The strip is covered by the open list: likewise.
    pub fn close_capsules(&mut self) {
        self.capsules.clear();
    }

    /// Whether a row or capsule is still moving or fading.
    pub fn busy(&self, now: f64) -> bool {
        self.rows.values().any(|r| r.busy(now)) || self.capsules.values().any(|c| c.busy(now))
    }

    /// Drop the rows and capsules that have finished fading out.
    pub fn forget_gone(&mut self, now: f64) {
        self.rows.retain(|_, r| !r.leaving || r.opacity.moving(now));
        self.capsules.retain(|_, c| !c.leaving || c.opacity.moving(now));
    }
}

/// Every card and header the pane has drawn, by agent pane and by section.
#[derive(Default)]
pub struct Motion {
    pub cards: HashMap<PaneId, CardMotion>,
    /// What's inside each team's card, by its lead: kept as long as the card is.
    pub teams: HashMap<PaneId, TeamMotion>,
    pub sections: HashMap<Status, SectionMotion>,
    /// The content width the layout was last made for: a resize snaps everything into place
    /// instead of sliding it.
    pub width: f32,
    /// Nothing was drawn yet: the first layout appears as it is, without fading in.
    pub fresh: bool,
}

impl Motion {
    pub fn new() -> Self {
        Motion { fresh: true, ..Default::default() }
    }

    /// Head every card and header for its place in the latest layout (content coordinates
    /// at `width`). New ones fade in where they belong, ones no longer there fade out where
    /// they were. `animate` false (or a resize, or the first layout) puts everything in place.
    pub fn update(&mut self, now: f64, animate: bool, sections: &[(Status, usize, Rect)], cards: &[(&Arc<Agent>, Rect)], width: f32) {
        let animate = animate && !std::mem::take(&mut self.fresh);
        let slide = animate && (self.width - width).abs() < 0.5;
        self.width = width;

        for &(status, count, r) in sections {
            let s = self.sections.entry(status).or_insert_with(|| SectionMotion::new(count, r));
            s.count = count;
            s.leaving = false;
            s.rect.set(now, r, slide);
            s.opacity.set(now, 1.0, animate);
        }
        for (status, s) in &mut self.sections {
            if !sections.iter().any(|(st, ..)| st == status) {
                s.leaving = true;
                s.opacity.set(now, 0.0, animate);
            }
        }

        for &(agent, r) in cards {
            let c = self.cards.entry(agent.pane).or_insert_with(|| CardMotion::new(Arc::clone(agent), r));
            // A new status means a new section; a settled card that is only pushed along
            // by the others moves flat.
            if !slide {
                c.section_change = false;
            } else if c.agent.status != agent.status {
                c.section_change = true;
            } else if !c.rect.moving(now) {
                c.section_change = false;
            }
            if !Arc::ptr_eq(&c.agent, agent) {
                c.agent = Arc::clone(agent);
            }
            c.leaving = false;
            c.rect.set(now, r, slide);
            c.opacity.set(now, 1.0, animate);
        }
        let present: HashSet<PaneId> = cards.iter().map(|(a, _)| a.pane).collect();
        for (pane, c) in &mut self.cards {
            if !present.contains(pane) {
                c.leaving = true;
                c.opacity.set(now, 0.0, animate);
            }
        }
        self.forget_gone(now);
    }

    /// Bubble the cards of the agents on tab `tab` (it was just switched to).
    pub fn pulse_tab(&mut self, now: f64, tab: usize) {
        for c in self.cards.values_mut().filter(|c| !c.leaving && c.agent.tab == Some(tab)) {
            c.pulse(now);
        }
    }

    /// Drop the cards and headers that have finished fading out.
    pub fn forget_gone(&mut self, now: f64) {
        self.cards.retain(|_, c| !c.leaving || c.opacity.moving(now));
        self.sections.retain(|_, s| !s.leaving || s.opacity.moving(now));
        self.teams.retain(|lead, _| self.cards.contains_key(lead));
        for team in self.teams.values_mut() {
            team.forget_gone(now);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::{Pos2, Vec2};

    #[test]
    fn tweens_ease_toward_a_new_target_from_where_they_are() {
        let mut t = Tween::new(0.0_f32, 1.0, |x| x);
        t.set(10.0, 100.0, true);
        assert_eq!(t.get(10.0), 0.0);
        assert_eq!(t.get(10.5), 50.0);
        assert!(t.moving(10.5));
        // Retargeted halfway: it starts from 50, not from 0 or 100.
        t.set(10.5, 0.0, true);
        assert_eq!(t.get(10.5), 50.0);
        assert_eq!(t.get(11.5), 0.0);
        assert!(!t.moving(11.5));
        // With animations off it jumps.
        t.set(12.0, 7.0, false);
        assert_eq!(t.get(12.0), 7.0);
        assert!(!t.moving(12.0));
    }

    fn agent(pane: PaneId, status: Status) -> Arc<Agent> {
        use crate::agents::model::{AgentKind, Source};
        Arc::new(Agent {
            pane,
            tab: Some(0),
            kind: AgentKind::Claude,
            name: "a".into(),
            task: None,
            session: None,
            mark: None,
            cwd: None,
            status,
            since: std::time::Instant::now(),
            source: Source::Local,
            team: Default::default(),
        })
    }

    fn at(y: f32) -> Rect {
        Rect::from_min_size(Pos2::new(0.0, y), Vec2::new(300.0, 74.0))
    }

    #[test]
    fn cards_glide_between_sections_and_fade_out_when_gone() {
        let mut m = Motion::new();
        let (working, idle) = (agent(1, Status::Working), agent(2, Status::Idle));
        let sec = |s, y| (s, 1, at(y));
        // The first layout appears as it is.
        m.update(0.0, true, &[sec(Status::Working, 0.0), sec(Status::Idle, 100.0)], &[(&working, at(30.0)), (&idle, at(130.0))], 300.0);
        assert!(!m.cards[&1].busy(0.0) && m.cards[&1].opacity.get(0.0) == 1.0);

        // Agent 1 goes idle: its card glides down from where it was, the Working header fades.
        let now_idle = agent(1, Status::Idle);
        m.update(1.0, true, &[sec(Status::Idle, 0.0)], &[(&idle, at(30.0)), (&now_idle, at(110.0))], 300.0);
        assert_eq!(m.cards[&1].rect.get(1.0), at(30.0));
        assert!(m.cards[&1].rect.moving(1.2));
        assert_eq!(m.cards[&1].rect.get(1.0 + MOVE), at(110.0));
        assert!(m.sections[&Status::Working].leaving);
        m.forget_gone(1.0 + FADE);
        assert!(!m.sections.contains_key(&Status::Working));

        // Agent 2's pane closes: its card fades out in place, then is forgotten.
        m.update(2.0, true, &[sec(Status::Idle, 0.0)], &[(&now_idle, at(30.0))], 300.0);
        assert!(m.cards[&2].leaving && m.cards[&2].rect.get(2.1) == at(30.0));
        m.forget_gone(2.0 + FADE);
        assert!(!m.cards.contains_key(&2));

        // Only the card that changed sections lifts; the ones making room slide flat.
        let mut m = Motion::new();
        let pushed = agent(2, Status::Idle);
        m.update(0.0, true, &[sec(Status::Working, 0.0), sec(Status::Idle, 100.0)], &[(&working, at(30.0)), (&pushed, at(130.0))], 300.0);
        m.update(1.0, true, &[sec(Status::Idle, 0.0)], &[(&pushed, at(30.0)), (&now_idle, at(110.0))], 300.0);
        let mid = 1.0 + MOVE / 2.0;
        assert!(m.cards[&1].rect.moving(mid) && m.cards[&2].rect.moving(mid));
        assert!(m.cards[&1].lift(mid, Some(0)) > 0.9);
        assert_eq!(m.cards[&2].lift(mid, Some(0)), 0.0);
        assert_eq!(m.cards[&1].lift(1.0 + MOVE, Some(0)), 0.0);
        // ...and only while its tab is the one shown.
        assert_eq!(m.cards[&1].lift(mid, Some(1)), 0.0);
        assert_eq!(m.cards[&1].lift(mid, None), 0.0);

        // A resize puts cards in place instead of sliding them.
        m.update(3.0, true, &[sec(Status::Idle, 0.0)], &[(&now_idle, at(500.0))], 900.0);
        assert!(!m.cards[&1].rect.moving(3.0));
    }

    fn member(pane: PaneId, status: Status) -> Member {
        Member { agent: agent(pane, status), depth: 1 }
    }

    const ROW: f32 = 46.0;

    /// `members` as an open list's rows, one slot each from the top.
    fn slots(members: &[Member]) -> Vec<(&Member, &str, f32)> {
        members.iter().enumerate().map(|(i, m)| (m, "Build-01", i as f32 * ROW)).collect()
    }

    #[test]
    fn rows_glide_to_their_new_slots_when_a_helper_changes_state() {
        let mut t = TeamMotion::default();
        let was = [member(1, Status::Working), member(2, Status::Working), member(3, Status::Idle)];
        // A list that has just opened shows its rows in place.
        t.update_rows(0.0, true, &slots(&was));
        assert!(!t.busy(0.0) && t.rows[&2].y.get(0.0) == ROW && t.rows[&2].opacity.get(0.0) == 1.0);

        // Helper 2 finishes: its row eases up to the top over MOVE, helper 1's makes room.
        let now = [member(2, Status::Done), member(1, Status::Working), member(3, Status::Idle)];
        t.update_rows(1.0, true, &slots(&now));
        assert_eq!((t.rows[&2].y.get(1.0), t.rows[&1].y.get(1.0)), (ROW, 0.0), "from where they were");
        let mid = 1.0 + MOVE / 2.0;
        assert!(t.busy(mid) && (t.rows[&2].y.get(mid) - ROW / 2.0).abs() < 0.01, "halfway there");
        assert_eq!((t.rows[&2].y.get(1.0 + MOVE), t.rows[&1].y.get(1.0 + MOVE)), (0.0, ROW));
        assert!(!t.busy(1.0 + MOVE));
        // Only the row whose own helper changed is flagged, while it's on its way.
        assert!(t.rows[&2].status_change && !t.rows[&1].status_change && !t.rows[&3].status_change);
        assert!(t.rows[&2].sliding(mid) && t.rows[&1].sliding(mid) && !t.rows[&3].sliding(mid));
        t.update_rows(1.0 + MOVE, true, &slots(&now));
        assert!(!t.rows[&2].status_change, "settled");

        // A helper that's gone fades out where it was; a new one fades in where it belongs.
        let later = [member(2, Status::Done), member(4, Status::Working), member(3, Status::Idle)];
        t.update_rows(2.0, true, &slots(&later));
        assert!(t.rows[&1].leaving && t.rows[&1].y.get(2.1) == ROW && t.rows[&1].opacity.get(2.1) < 1.0);
        assert!(t.rows[&4].y.get(2.0) == ROW && !t.rows[&4].sliding(2.0) && t.rows[&4].opacity.get(2.0) == 0.0);
        t.forget_gone(2.0 + FADE);
        assert!(!t.rows.contains_key(&1) && t.rows[&4].opacity.get(2.0 + FADE) == 1.0);
    }

    #[test]
    fn rows_jump_with_animations_off_on_a_resize_and_when_the_list_opens() {
        let was = [member(1, Status::Working), member(2, Status::Working)];
        let now = [member(2, Status::Done), member(1, Status::Working)];
        let settled = |t: &TeamMotion, at: f64| !t.busy(at) && t.rows[&2].y.get(at) == 0.0 && t.rows[&1].y.get(at) == ROW && !t.rows[&2].status_change;

        let mut t = TeamMotion::default();
        t.update_rows(0.0, false, &slots(&was));
        t.update_rows(1.0, false, &slots(&now));
        assert!(settled(&t, 1.0), "animations off");

        // The card is laid out at another width (the view passes that on as no animation).
        let mut t = TeamMotion::default();
        assert!(t.resized(600.0), "the first layout counts");
        t.update_rows(0.0, true, &slots(&was));
        assert!(!t.resized(600.2) && t.resized(900.0));
        t.update_rows(1.0, false, &slots(&now));
        assert!(settled(&t, 1.0), "a resize");

        // Closed and opened again: the new order is simply there.
        let mut t = TeamMotion::default();
        t.update_rows(0.0, true, &slots(&was));
        t.close_rows();
        t.update_rows(1.0, true, &slots(&now));
        assert!(settled(&t, 1.0) && t.rows[&2].opacity.get(1.0) == 1.0, "just opened");
    }

    #[test]
    fn capsules_slide_along_the_strip_and_fade_into_the_overflow() {
        let mut t = TeamMotion::default();
        t.update_capsules(0.0, true, &[(1, 0.0), (2, 70.0), (3, 140.0)]);
        assert!(!t.busy(0.0) && t.capsules[&3].opacity.get(0.0) == 1.0, "the first strip is as it is");

        // 2 and 1 change places; 3 no longer fits (it's in the +N now).
        t.update_capsules(1.0, true, &[(2, 0.0), (1, 70.0)]);
        let mid = 1.0 + MOVE / 2.0;
        assert!((t.capsules[&2].x.get(mid) - 35.0).abs() < 0.01 && (t.capsules[&1].x.get(mid) - 35.0).abs() < 0.01);
        assert_eq!((t.capsules[&2].x.get(1.0 + MOVE), t.capsules[&1].x.get(1.0 + MOVE)), (0.0, 70.0));
        assert!(t.capsules[&3].leaving && t.capsules[&3].x.get(mid) == 140.0 && t.capsules[&3].opacity.get(1.1) < 1.0);

        // Back out of the +N before it has faded: where it belongs now, fading in.
        t.update_capsules(1.1, true, &[(2, 0.0), (1, 70.0), (3, 150.0)]);
        assert!(!t.capsules[&3].leaving && t.capsules[&3].x.get(1.1) == 150.0);
        t.update_capsules(2.0, true, &[(2, 0.0), (1, 70.0)]);
        t.forget_gone(2.0 + FADE);
        assert!(!t.capsules.contains_key(&3) && !t.busy(2.0 + FADE));

        // With animations off they jump.
        t.update_capsules(3.0, false, &[(1, 0.0), (2, 70.0)]);
        assert!(!t.busy(3.0) && t.capsules[&1].x.get(3.0) == 0.0);
    }

    #[test]
    fn a_teams_motion_goes_with_its_card() {
        let mut m = Motion::new();
        let lead = agent(1, Status::Idle);
        m.update(0.0, true, &[(Status::Idle, 1, at(0.0))], &[(&lead, at(30.0))], 300.0);
        m.teams.entry(1).or_default().update_rows(0.0, true, &slots(&[member(2, Status::Idle)]));
        m.forget_gone(0.5);
        assert!(m.teams.contains_key(&1));
        m.update(1.0, true, &[], &[], 300.0);
        m.forget_gone(1.0 + FADE);
        assert!(m.teams.is_empty() && m.cards.is_empty());
    }

    #[test]
    fn switching_to_a_tab_bubbles_only_its_agents() {
        let mut m = Motion::new();
        let here = agent(1, Status::Idle);
        let elsewhere = Arc::new(Agent { tab: Some(1), ..Agent::clone(&agent(2, Status::Idle)) });
        m.update(0.0, true, &[(Status::Idle, 2, at(0.0))], &[(&here, at(30.0)), (&elsewhere, at(110.0))], 300.0);
        m.pulse_tab(1.0, 0);
        let mid = 1.0 + MOVE / 2.0;
        // Nothing moves, but tab 0's card bubbles up and back down.
        assert!(!m.cards[&1].rect.moving(mid));
        assert!(m.cards[&1].lift(mid, Some(0)) > 0.9 && m.cards[&1].busy(mid));
        let after = 1.0 + MOVE + 0.01;
        assert_eq!(m.cards[&1].lift(after, Some(0)), 0.0);
        assert!(!m.cards[&1].busy(after));
        // The agent on another tab stays flat.
        assert_eq!(m.cards[&2].lift(mid, Some(0)), 0.0);
        assert_eq!(m.cards[&2].lift(mid, Some(1)), 0.0);
    }

    #[test]
    fn rects_and_colors_blend() {
        let a = Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0));
        let b = Rect::from_min_size(Pos2::new(20.0, 40.0), Vec2::splat(10.0));
        assert_eq!(Rect::lerp(a, b, 0.5), Rect::from_min_size(Pos2::new(10.0, 20.0), Vec2::splat(10.0)));
        let mid = Color32::lerp(Color32::from_rgb(0, 0, 0), Color32::from_rgb(200, 100, 50), 0.5);
        assert_eq!(mid, Color32::from_rgb(100, 50, 25));
    }
}
