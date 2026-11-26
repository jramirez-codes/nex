//! The animation basics every animation shares: easing curves, blending ([`Lerp`]), a value
//! that animates toward its target with an ease-out curve ([`Glide`]), a value of any `Lerp`
//! type easing toward its latest target along a chosen curve on a clock you pass in
//! ([`Tween`]: the Agents pane's cards, rows and capsules), the timing of a held line
//! scrolling to show its end ([`marquee`]), and the global switch
//! (`animations` in the config) that turns every animation off.

use egui::{Color32, Id, Rect};

/// Ease-out cubic: fast start, gentle landing. Shared by every animation so they feel alike.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Ease-in cubic: slow start, quick finish. For things fading away.
pub fn ease_in(t: f32) -> f32 {
    t.clamp(0.0, 1.0).powi(3)
}

/// Ease-in-out cubic: gathers, moves, settles. For things leaving the screen.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
}

/// How long a [`marquee`] waits before it first moves, in seconds: just long enough to see
/// the line's start held, since the hold itself already took a moment to be recognized.
const MARQUEE_LEAD: f64 = 0.05;
/// The pause at each end of a [`marquee`] after that, in seconds: long enough to read the
/// line's end (and, round again, its start).
const MARQUEE_PAUSE: f64 = 0.6;
/// How fast a [`marquee`] scrolls, in points a second.
const MARQUEE_SPEED: f32 = 60.0;

/// How far along to draw a line `overflow` points too long for its room, `held` seconds into
/// a press and hold that reveals it: still for a blink, along to its end, still there for a
/// moment, back, still again, and round again. The way out starts at full speed and slows
/// into its end (a slow start was hard to see begin); the way back eases in and out. A line
/// that fits (`overflow` 0 or less) stays put; with animations off (`animate`) the end
/// shows. Shared by the Agents pane's held cards and held pane headers.
pub fn marquee(held: f64, overflow: f32, animate: bool) -> f32 {
    if overflow <= 0.0 {
        return 0.0;
    }
    if !animate {
        return overflow;
    }
    let travel = f64::from(overflow / MARQUEE_SPEED);
    let t = held - MARQUEE_LEAD;
    if t <= 0.0 {
        return 0.0;
    }
    // Each round: out, a pause at the end, back, a pause at the start.
    let t = t % (2.0 * (travel + MARQUEE_PAUSE));
    if t < travel {
        let along = (t / travel) as f32;
        overflow * (1.0 - (1.0 - along).powi(2))
    } else if t < travel + MARQUEE_PAUSE {
        overflow
    } else if t < 2.0 * travel + MARQUEE_PAUSE {
        overflow * (1.0 - ease_in_out(((t - travel - MARQUEE_PAUSE) / travel) as f32))
    } else {
        0.0
    }
}

/// Smoothstep: eases in and out, flat at both ends. For cross-fades.
pub fn smoothstep(x: f32) -> f32 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// A value an animation can blend between two ends (`t` from 0 to 1).
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

fn switch_id() -> Id {
    Id::new("nex-animations")
}

/// Turn all UI animations on or off (the `animations` config key).
pub fn set_animations(ctx: &egui::Context, on: bool) {
    ctx.data_mut(|d| d.insert_temp(switch_id(), on));
}

/// Whether UI animations are enabled.
pub fn animations(ctx: &egui::Context) -> bool {
    ctx.data(|d| d.get_temp(switch_id())).unwrap_or(true)
}

/// A value that glides toward its target with an ease-out curve (the active-tab pill, the
/// pane-icon decks, pane focus).
pub struct Glide {
    from: f32,
    pub to: f32,
    start: f64,
    init: bool,
    duration: f64,
}

impl Glide {
    pub fn new(duration: f64) -> Self {
        Self { from: 0.0, to: 0.0, start: 0.0, init: false, duration }
    }

    /// Jump straight to `v` without animating.
    pub fn snap(&mut self, ctx: &egui::Context, v: f32) {
        let now = ctx.input(|i| i.time);
        *self = Self { from: v, to: v, start: now, init: true, duration: self.duration };
    }

    /// Current position; retargets (from wherever the value is now) when `target` changes.
    /// With animations off it is always the target.
    pub fn value(&mut self, ctx: &egui::Context, target: f32) -> f32 {
        let now = ctx.input(|i| i.time);
        if !self.init || !animations(ctx) {
            self.snap(ctx, target);
        } else if self.to != target {
            self.from = self.current(now);
            self.to = target;
            self.start = now;
        }
        let v = self.current(now);
        if v != self.to {
            ctx.request_repaint();
        }
        v
    }

    fn current(&self, now: f64) -> f32 {
        let t = ((now - self.start) / self.duration) as f32;
        if t >= 1.0 {
            return self.to;
        }
        self.from + (self.to - self.from) * ease_out(t)
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
