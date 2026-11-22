//! How the Agents pane's cards and section headers move: each remembers where it was drawn and
//! in what colors, and eases toward where the layout wants it now. A card whose agent changes
//! state glides out of one section into another while the cards around it slide over to
//! make room; cards and headers fade in when they appear and out when they go; borders and
//! status tints cross-fade.

use std::collections::HashMap;

use egui::{Color32, Rect};

use crate::layout::PaneId;
use crate::ui::glide::{ease_in_out, ease_out};

use super::model::{Agent, Status};

/// A card or header moving to its new place.
pub const MOVE: f64 = 0.45;
/// Appearing or going away.
pub const FADE: f64 = 0.25;
/// A border or tint changing color.
pub const COLOR: f64 = 0.3;

/// Something a [`Tween`] can blend.
pub trait Lerp: Copy + PartialEq {
    fn lerp(a: Self, b: Self, t: f32) -> Self;
}

impl Lerp for f32 {
    fn lerp(a: f32, b: f32, t: f32) -> f32 {
        a + (b - a) * t
    }
}

impl Lerp for Rect {
    fn lerp(a: Rect, b: Rect, t: f32) -> Rect {
        Rect::from_min_max(a.min.lerp(b.min, t), a.max.lerp(b.max, t))
    }
}

impl Lerp for Color32 {
    /// Channel by channel on the premultiplied values, which is how colors blend.
    fn lerp(a: Color32, b: Color32, t: f32) -> Color32 {
        let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
        Color32::from_rgba_premultiplied(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()), ch(a.a(), b.a()))
    }
}

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
/// after its pane is gone).
pub struct CardMotion {
    pub agent: Agent,
    /// In content coordinates (scrolling doesn't animate).
    pub rect: Tween<Rect>,
    pub border: Tween<Color32>,
    pub border_width: Tween<f32>,
    pub tint: Tween<Color32>,
    pub opacity: Tween<f32>,
    pub leaving: bool,
    /// Just created: its border and tint start out in their colors instead of fading in.
    pub new_colors: bool,
}

impl CardMotion {
    pub fn new(agent: Agent, rect: Rect) -> Self {
        CardMotion {
            agent,
            rect: Tween::new(rect, MOVE, ease_in_out),
            border: Tween::new(Color32::TRANSPARENT, COLOR, ease_out),
            border_width: Tween::new(1.0, COLOR, ease_out),
            tint: Tween::new(Color32::TRANSPARENT, COLOR, ease_out),
            opacity: Tween::new(0.0, FADE, ease_out),
            leaving: false,
            new_colors: true,
        }
    }

    /// Head for these colors; a new card takes them at once.
    pub fn set_colors(&mut self, now: f64, border: Color32, border_width: f32, tint: Color32, animate: bool) {
        let animate = animate && !std::mem::take(&mut self.new_colors);
        self.border.set(now, border, animate);
        self.border_width.set(now, border_width, animate);
        self.tint.set(now, tint, animate);
    }

    /// Whether anything about it is still changing.
    pub fn busy(&self, now: f64) -> bool {
        self.rect.moving(now) || self.border.moving(now) || self.border_width.moving(now) || self.tint.moving(now) || self.opacity.moving(now)
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

/// Every card and header the pane has drawn, by agent pane and by section.
#[derive(Default)]
pub struct Motion {
    pub cards: HashMap<PaneId, CardMotion>,
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
    pub fn update(&mut self, now: f64, animate: bool, sections: &[(Status, usize, Rect)], cards: &[(&Agent, Rect)], width: f32) {
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
            let c = self.cards.entry(agent.pane).or_insert_with(|| CardMotion::new(agent.clone(), r));
            c.agent = agent.clone();
            c.leaving = false;
            c.rect.set(now, r, slide);
            c.opacity.set(now, 1.0, animate);
        }
        for (pane, c) in &mut self.cards {
            if !cards.iter().any(|(a, _)| a.pane == *pane) {
                c.leaving = true;
                c.opacity.set(now, 0.0, animate);
            }
        }
        self.forget_gone(now);
    }

    /// Drop the cards and headers that have finished fading out.
    pub fn forget_gone(&mut self, now: f64) {
        self.cards.retain(|_, c| !c.leaving || c.opacity.moving(now));
        self.sections.retain(|_, s| !s.leaving || s.opacity.moving(now));
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

    fn agent(pane: PaneId, status: Status) -> Agent {
        use super::super::model::{AgentKind, Source};
        Agent { pane, tab: Some(0), kind: AgentKind::Claude, name: "a".into(), mark: None, cwd: None, status, since: std::time::Instant::now(), source: Source::Local }
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

        // A resize puts cards in place instead of sliding them.
        m.update(3.0, true, &[sec(Status::Idle, 0.0)], &[(&now_idle, at(500.0))], 900.0);
        assert!(!m.cards[&1].rect.moving(3.0));
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
