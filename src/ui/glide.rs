//! The animation basics every animation shares: easing curves, blending ([`Lerp`]), a value
//! that animates toward its target with an ease-out curve ([`Glide`]), and the global switch
//! (`animations` in the config) that turns every animation off.

use egui::{Color32, Id, Rect};

/// Ease-out cubic: fast start, gentle landing. Shared by every animation so they feel alike.
pub fn ease_out(t: f32) -> f32 {
    1.0 - (1.0 - t.clamp(0.0, 1.0)).powi(3)
}

/// Ease-in-out cubic: gathers, moves, settles. For things leaving the screen.
pub fn ease_in_out(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    if t < 0.5 { 4.0 * t * t * t } else { 1.0 - (-2.0 * t + 2.0).powi(3) / 2.0 }
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
