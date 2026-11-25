//! How the Agents pane's cards and section headers move: each remembers where it was drawn and
//! in what colors, and eases toward where the layout wants it now. A card whose agent changes
//! state glides out of one section into another while the cards around it slide over to
//! make room; cards and headers fade in when they appear and out when they go; borders and
//! status tints cross-fade, and the "Viewing" highlight eases in and out ([`SeenFade`]).
//! A pipeline's tile has a progress ring that eases round as its stages finish and a corner
//! badge that pops in when it's done or breathes while it needs you ([`TileMotion`]); a
//! helper's tick draws itself in when it finishes ([`TeamMotion::tick`]).
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
use super::model::{Agent, Seen, Status};

/// A card or header moving to its new place.
pub const MOVE: f64 = 0.45;
/// Appearing or going away.
pub const FADE: f64 = 0.25;
/// A border or tint changing color.
pub const COLOR: f64 = 0.3;
/// The "Viewing" highlight coming on or going off.
pub const VIEW: f64 = 0.2;

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

/// How far a card's (or a helper row's) "Viewing" highlight is in: `shown`, 0 (not on
/// screen) to 1, and `primary`, 0 (a neighbour's look) to 1 (the focused pane's). Each eases
/// over [`VIEW`] and lands exactly on 0 or 1. Going off, it keeps the look it had, so it
/// fades out as it was; coming on from nothing, it takes its look at once and only fades in.
pub struct SeenFade {
    shown: Tween<f32>,
    primary: Tween<f32>,
}

impl Default for SeenFade {
    fn default() -> Self {
        SeenFade { shown: Tween::new(0.0, VIEW, ease_in_out), primary: Tween::new(0.0, VIEW, ease_in_out) }
    }
}

impl SeenFade {
    /// Head for how it's on screen now; `animate` false jumps there.
    pub fn set(&mut self, now: f64, seen: Option<Seen>, animate: bool) {
        if let Some(seen) = seen {
            let primary = f32::from(u8::from(seen == Seen::Primary));
            let off = self.shown.get(now) == 0.0;
            self.primary.set(now, primary, animate && !off);
        }
        self.shown.set(now, f32::from(u8::from(seen.is_some())), animate);
    }

    /// How far in it is, 0 to 1.
    pub fn shown(&self, now: f64) -> f32 {
        self.shown.get(now)
    }

    /// How far from a neighbour's look to the focused pane's it is, 0 to 1.
    pub fn primary(&self, now: f64) -> f32 {
        self.primary.get(now)
    }

    /// Still fading.
    pub fn busy(&self, now: f64) -> bool {
        self.shown.moving(now) || self.primary.moving(now)
    }
}

/// A stage's tick drawing itself in, once it's done.
pub const TICK: f64 = 0.3;
/// The Done badge popping in.
pub const POP: f64 = 0.35;
/// One slow breath of a Needs-you badge or capsule tint, in seconds.
pub const BREATH: f64 = 2.4;

/// Scale of a badge `t` (0 to 1) into its pop: from nothing, a little past full size
/// (about 1.1), and settling back to 1. An ease-out-back, the overshoot a small spring gives.
pub fn spring_pop(t: f32) -> f32 {
    const C1: f32 = 1.701_58;
    const C3: f32 = C1 + 1.0;
    let u = t.clamp(0.0, 1.0) - 1.0;
    1.0 + C3 * u * u * u + C1 * u * u
}

/// How far into a breath it is at `time` seconds, 0 (out) to 1 (in): a slow, even sine,
/// flat at both ends so it never snaps.
pub fn breath(time: f64) -> f32 {
    (0.5 - 0.5 * (time / BREATH * std::f64::consts::TAU).cos()) as f32
}

/// How far a pipeline's stages are through, 0 to 1, given each stage's status: the ones
/// done (or done and since looked at: idle) over all of them. `None` without stages.
pub fn stage_progress(stages: impl IntoIterator<Item = Status>) -> Option<f32> {
    let (mut done, mut all) = (0, 0);
    for s in stages {
        all += 1;
        done += usize::from(matches!(s, Status::Done | Status::Idle));
    }
    (all > 0).then(|| done as f32 / all as f32)
}

/// The badge in a pipeline tile's corner.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    /// An amber `!`: a stage needs you.
    NeedsYou,
    /// A green tick: it finished, and you haven't looked yet.
    Done,
}

impl Badge {
    pub fn of(status: Status) -> Option<Badge> {
        match status {
            Status::NeedsYou => Some(Badge::NeedsYou),
            Status::Done => Some(Badge::Done),
            _ => None,
        }
    }
}

/// A pipeline tile's progress ring and corner badge. The ring eases to each new fraction of
/// stages done; a Done badge pops in on a small spring, a Needs-you one fades in and breathes
/// until its card is looked at; either fades out (as it was) when it goes.
pub struct TileMotion {
    ring: Tween<f32>,
    /// The badge shown, or the last one while it fades out.
    badge: Option<Badge>,
    opacity: Tween<f32>,
    /// When the Done badge started popping in.
    popped: f64,
    /// Looked at since it started needing you: the badge holds still.
    looked: bool,
    animate: bool,
    fresh: bool,
}

impl Default for TileMotion {
    fn default() -> Self {
        TileMotion { ring: Tween::new(0.0, MOVE, ease_in_out), badge: None, opacity: Tween::new(0.0, VIEW, ease_in_out), popped: f64::NEG_INFINITY, looked: false, animate: true, fresh: true }
    }
}

impl TileMotion {
    /// Head for `progress` (see [`stage_progress`]) and the badge `status` calls for (Done
    /// only once every stage has finished); `seen`:
    /// its card is on screen. `animate` false (or the first update) puts it all in place.
    pub fn update(&mut self, now: f64, progress: f32, status: Status, seen: bool, animate: bool) {
        let animate = animate && !std::mem::take(&mut self.fresh);
        self.animate = animate;
        self.ring.set(now, progress, animate);
        // A finished stage makes the team Done while the others still work: the tick waits
        // for the last one.
        let badge = Badge::of(status).filter(|b| *b != Badge::Done || progress >= 1.0);
        if badge != self.badge && badge.is_some() {
            if badge == Some(Badge::Done) && animate {
                // It pops in from nothing: the scale is its entrance.
                self.popped = now;
                self.opacity.snap(1.0);
            } else {
                self.popped = f64::NEG_INFINITY;
                self.opacity.set(now, if self.badge.is_some() { 1.0 } else { 0.0 }, false);
            }
            self.badge = badge;
        }
        self.opacity.set(now, f32::from(u8::from(badge.is_some())), animate);
        if !self.opacity.moving(now) && badge.is_none() {
            self.badge = None;
        }
        self.looked = status == Status::NeedsYou && (self.looked || seen);
    }

    /// The ring's fraction, 0 to 1.
    pub fn ring(&self, now: f64) -> f32 {
        self.ring.get(now)
    }

    /// The badge to draw: which, its scale, and its opacity.
    pub fn badge(&self, now: f64) -> Option<(Badge, f32, f32)> {
        let badge = self.badge?;
        let t = ((now - self.popped) / POP) as f32;
        Some((badge, spring_pop(t), self.opacity.get(now)))
    }

    /// A Needs-you badge nobody has looked at yet: it breathes.
    pub fn breathing(&self) -> bool {
        self.badge == Some(Badge::NeedsYou) && !self.looked && self.animate
    }

    /// The ring easing, or the badge popping or fading.
    pub fn busy(&self, now: f64) -> bool {
        self.ring.moving(now) || self.opacity.moving(now) || (now - self.popped) < POP
    }
}

/// How far a done stage's tick has drawn itself, 0 to 1, `since` seconds after it finished:
/// eased like the rest of the pane, over [`TICK`].
pub fn tick_drawn(since: f64) -> f32 {
    ease_in_out((since / TICK) as f32)
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
    /// Its "Viewing" pill and glow.
    pub seen: SeenFade,
    /// Its "Viewing" ring: as `seen`, but off while the card has the keyboard's ring.
    pub ring: SeenFade,
    /// A pipeline's progress ring and badge on its tile.
    pub tile: TileMotion,
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
            seen: SeenFade::default(),
            ring: SeenFade::default(),
            tile: TileMotion::default(),
            leaving: false,
            new_colors: true,
            section_change: false,
            pulse_start: f64::NEG_INFINITY,
        }
    }

    /// Head for these colors and this "Viewing" highlight (`seen` for its pill and glow,
    /// `ring` for its ring); a new card takes them at once.
    #[allow(clippy::too_many_arguments)]
    pub fn set_colors(&mut self, now: f64, border: Color32, border_width: f32, tint: Color32, seen: Option<Seen>, ring: Option<Seen>, animate: bool) {
        let animate = animate && !std::mem::take(&mut self.new_colors);
        self.seen.set(now, seen, animate);
        self.ring.set(now, ring, animate);
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
        self.pulse_t(now).is_some() || self.rect.moving(now) || self.border.moving(now) || self.border_width.moving(now) || self.tint.moving(now) || self.opacity.moving(now) || self.seen.busy(now) || self.ring.busy(now) || self.tile.busy(now)
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
    /// Its "Viewing" rule and eye.
    pub seen: SeenFade,
    /// A nested pipeline's progress ring and badge on its tile.
    pub tile: TileMotion,
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
            seen: SeenFade::default(),
            tile: TileMotion::default(),
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
        self.y.moving(now) || self.opacity.moving(now) || self.seen.busy(now) || self.tile.busy(now)
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
    /// Each helper's status as last seen, and when it last became done (its tick draws
    /// itself in from then).
    ticks: HashMap<PaneId, (Status, f64)>,
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

    /// Note the helpers' statuses: one that has just become done draws its tick in. One seen
    /// for the first time (or `animate` false) shows it whole.
    pub fn note_statuses<'a>(&mut self, now: f64, animate: bool, helpers: impl IntoIterator<Item = &'a Agent>) {
        let mut present = HashSet::new();
        for a in helpers {
            present.insert(a.pane);
            let t = self.ticks.entry(a.pane).or_insert((a.status, f64::NEG_INFINITY));
            if a.status != t.0 {
                if a.status == Status::Done && animate {
                    t.1 = now;
                } else if !animate {
                    t.1 = f64::NEG_INFINITY;
                }
                t.0 = a.status;
            }
        }
        self.ticks.retain(|p, _| present.contains(p));
    }

    /// How far helper `pane`'s tick is drawn, 0 to 1 (1 for one that isn't drawing).
    pub fn tick(&self, now: f64, pane: PaneId) -> f32 {
        self.ticks.get(&pane).map_or(1.0, |&(_, at)| tick_drawn(now - at))
    }

    /// Whether a row or capsule is still moving or fading, or a tick drawing.
    pub fn busy(&self, now: f64) -> bool {
        self.rows.values().any(|r| r.busy(now)) || self.capsules.values().any(|c| c.busy(now)) || self.ticks.values().any(|&(_, at)| now - at < TICK)
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

    #[test]
    fn the_viewing_highlight_eases_in_and_out_and_lands_exactly() {
        let mut v = SeenFade::default();
        assert_eq!((v.shown(0.0), v.busy(0.0)), (0.0, false), "off, and nothing to redraw");
        // Viewed in the focused pane: it eases in over VIEW, its look taken at once.
        v.set(1.0, Some(Seen::Primary), true);
        assert_eq!((v.shown(1.0), v.primary(1.0)), (0.0, 1.0));
        let quarter = v.shown(1.0 + VIEW / 4.0);
        let half = v.shown(1.0 + VIEW / 2.0);
        assert!(0.0 < quarter && quarter < 0.25, "slow out of the start: {quarter}");
        assert!((half - 0.5).abs() < 1e-6 && v.busy(1.0 + VIEW / 2.0));
        assert_eq!(v.shown(1.0 + VIEW), 1.0, "all the way in, exactly");
        assert!(!v.busy(1.0 + VIEW), "settled: no more redraws");

        // Out of view: it fades out as it was (still the focused pane's look) to exactly none.
        let t = 2.0;
        v.set(t, None, true);
        assert!(v.busy(t + VIEW / 2.0) && (v.shown(t + VIEW / 2.0) - 0.5).abs() < 1e-6);
        assert_eq!(v.primary(t + VIEW / 2.0), 1.0);
        assert_eq!((v.shown(t + VIEW), v.busy(t + VIEW)), (0.0, false));

        // Interrupted halfway in, it heads back from where it is, not from either end.
        let t = 3.0;
        v.set(t, Some(Seen::Shared), true);
        assert_eq!(v.primary(t), 0.0, "coming on from nothing, it takes its look at once");
        v.set(t + VIEW / 2.0, None, true);
        assert!((v.shown(t + VIEW / 2.0) - 0.5).abs() < 1e-6);
        assert!(v.shown(t + VIEW * 0.75) < 0.5);
        assert_eq!(v.shown(t + VIEW * 1.5), 0.0);

        // From a neighbour to the focused pane while shown: the look cross-fades.
        let t = 4.0;
        v.set(t, Some(Seen::Shared), true);
        v.set(t + VIEW, Some(Seen::Primary), true);
        assert!((v.primary(t + VIEW * 1.5) - 0.5).abs() < 1e-6 && v.shown(t + VIEW * 1.5) == 1.0);
        assert_eq!(v.primary(t + VIEW * 2.0), 1.0);

        // With animations off it jumps.
        v.set(9.0, None, false);
        assert_eq!((v.shown(9.0), v.busy(9.0)), (0.0, false));
    }

    #[test]
    fn a_card_is_busy_while_its_viewing_highlight_fades_and_a_new_one_starts_lit() {
        let mut card = CardMotion::new(agent(1, Status::Working), Rect::from_min_size(Pos2::ZERO, Vec2::splat(10.0)));
        card.set_colors(0.0, Color32::WHITE, 1.0, Color32::WHITE, Some(Seen::Primary), Some(Seen::Primary), true);
        assert_eq!((card.seen.shown(0.0), card.ring.shown(0.0)), (1.0, 1.0), "a new card appears as it is");
        assert!(!card.seen.busy(0.0));
        card.set_colors(1.0, Color32::WHITE, 1.0, Color32::WHITE, None, None, true);
        assert!(card.busy(1.0 + VIEW / 2.0));
        assert!(!card.busy(1.0 + VIEW.max(COLOR)) && card.seen.shown(1.0 + VIEW) == 0.0);
    }

    #[test]
    fn the_ring_counts_finished_stages_and_eases_to_each_new_value() {
        assert_eq!(stage_progress([]), None);
        assert_eq!(stage_progress([Status::Done, Status::Idle, Status::Working]), Some(2.0 / 3.0));
        assert_eq!(stage_progress([Status::NeedsYou, Status::Working]), Some(0.0));

        let mut t = TileMotion::default();
        t.update(0.0, 1.0 / 3.0, Status::Working, false, true);
        assert_eq!((t.ring(0.0), t.busy(0.0)), (1.0 / 3.0, false), "the first frame is as it is");
        t.update(1.0, 2.0 / 3.0, Status::Working, false, true);
        assert_eq!(t.ring(1.0), 1.0 / 3.0);
        let mid = t.ring(1.0 + MOVE / 2.0);
        assert!((mid - 0.5).abs() < 1e-5 && t.busy(1.0 + MOVE / 2.0), "halfway: {mid}");
        assert!(t.ring(1.0 + MOVE / 8.0) - 1.0 / 3.0 < 0.01, "eases out of the start");
        assert_eq!((t.ring(1.0 + MOVE), t.busy(1.0 + MOVE)), (2.0 / 3.0, false));
        // With animations off it jumps.
        t.update(2.0, 1.0, Status::Working, false, false);
        assert_eq!((t.ring(2.0), t.busy(2.0)), (1.0, false));
    }

    #[test]
    fn the_done_badge_pops_in_on_a_small_spring_and_stays_until_looked_at() {
        assert_eq!(spring_pop(0.0), 0.0);
        assert!((spring_pop(1.0) - 1.0).abs() < 1e-6);
        let peak = (0..=100).map(|i| spring_pop(i as f32 / 100.0)).fold(0.0, f32::max);
        assert!((1.08..1.12).contains(&peak), "overshoots to about 1.1: {peak}");

        let mut t = TileMotion::default();
        t.update(0.0, 0.5, Status::Working, false, true);
        assert!(t.badge(0.0).is_none());
        // A stage done while others still work makes the team Done, but the run isn't.
        t.update(0.5, 2.0 / 3.0, Status::Done, false, true);
        assert!(t.badge(0.5).is_none() && !t.busy(0.5 + MOVE));
        t.update(1.0, 1.0, Status::Done, false, true);
        let (badge, scale, opacity) = t.badge(1.0).unwrap();
        assert_eq!((badge, scale, opacity), (Badge::Done, 0.0, 1.0), "from nothing");
        assert!(t.badge(1.0 + POP * 0.7).unwrap().1 > 1.05, "past full size on the way");
        assert!(t.busy(1.0 + POP * 0.7));
        assert_eq!(t.badge(1.0 + POP).unwrap().1, 1.0);
        // It stays while Done, still, asking for no redraws.
        t.update(5.0, 1.0, Status::Done, true, true);
        assert!(!t.busy(5.0) && t.badge(5.0).unwrap().1 == 1.0 && !t.breathing());
        // Looked at, the agent turns idle: the badge fades out, then is gone.
        t.update(6.0, 1.0, Status::Idle, true, true);
        assert_eq!(t.badge(6.0 + VIEW / 2.0).unwrap().0, Badge::Done);
        assert!(t.badge(6.0 + VIEW / 2.0).unwrap().2 < 1.0 && t.busy(6.0 + VIEW / 2.0));
        t.update(6.0 + VIEW, 1.0, Status::Idle, true, true);
        assert!(t.badge(6.0 + VIEW).is_none() && !t.busy(6.0 + VIEW));

        // A card that's already done when first drawn, or with animations off, doesn't pop.
        let mut t = TileMotion::default();
        t.update(0.0, 1.0, Status::Done, false, true);
        assert_eq!(t.badge(0.0).map(|b| b.1), Some(1.0));
        t.update(1.0, 1.0, Status::Idle, false, false);
        t.update(2.0, 1.0, Status::Done, false, false);
        assert_eq!((t.badge(2.0).map(|b| b.1), t.busy(2.0)), (Some(1.0), false));
    }

    #[test]
    fn the_needs_you_badge_fades_in_and_breathes_until_looked_at() {
        assert_eq!((breath(0.0), breath(BREATH)), (0.0, 0.0));
        assert!((breath(BREATH / 2.0) - 1.0).abs() < 1e-6);
        assert!((0..100).all(|i| (0.0..=1.0).contains(&breath(i as f64 * 0.137))));

        let mut t = TileMotion::default();
        t.update(0.0, 0.0, Status::Working, false, true);
        t.update(1.0, 0.0, Status::NeedsYou, false, true);
        let (badge, scale, opacity) = t.badge(1.0).unwrap();
        assert_eq!((badge, scale, opacity), (Badge::NeedsYou, 1.0, 0.0), "fades in, no pop");
        assert_eq!(t.badge(1.0 + VIEW).unwrap().2, 1.0);
        assert!(t.breathing());
        // On screen: it holds still from then on, even once you look away again.
        t.update(2.0, 0.0, Status::NeedsYou, true, true);
        assert!(!t.breathing());
        t.update(3.0, 0.0, Status::NeedsYou, false, true);
        assert!(!t.breathing());
        // A new need breathes again.
        t.update(4.0, 0.0, Status::Working, false, true);
        t.update(5.0, 0.0, Status::NeedsYou, false, true);
        assert!(t.breathing());
        // With animations off, nothing breathes.
        t.update(6.0, 0.0, Status::NeedsYou, false, false);
        assert!(!t.breathing());
    }

    #[test]
    fn a_tick_draws_itself_in_when_its_helper_finishes() {
        assert_eq!((tick_drawn(0.0), tick_drawn(TICK)), (0.0, 1.0));
        assert!((tick_drawn(TICK / 2.0) - 0.5).abs() < 1e-6);
        assert!(tick_drawn(TICK / 4.0) < 0.25, "eases in like the rest of the pane");

        let mut t = TeamMotion::default();
        let (a, b) = (agent(1, Status::Working), agent(2, Status::Done));
        t.note_statuses(0.0, true, [&*a, &*b]);
        assert_eq!((t.tick(0.0, 2), t.busy(0.0)), (1.0, false), "one already done is drawn whole");
        let a = agent(1, Status::Done);
        t.note_statuses(1.0, true, [&*a, &*b]);
        assert_eq!(t.tick(1.0, 1), 0.0);
        assert!(t.busy(1.0 + TICK / 2.0) && t.tick(1.0 + TICK / 2.0, 1) > 0.0);
        assert_eq!((t.tick(1.0 + TICK, 1), t.busy(1.0 + TICK)), (1.0, false));
        // With animations off it's simply there.
        let c = agent(3, Status::Working);
        t.note_statuses(2.0, false, [&*a, &*b, &*c]);
        let c = agent(3, Status::Done);
        t.note_statuses(3.0, false, [&*a, &*b, &*c]);
        assert_eq!(t.tick(3.0, 3), 1.0);
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
