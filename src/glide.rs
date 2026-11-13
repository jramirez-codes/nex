//! A value that animates toward its target with an ease-out curve.


/// A value that glides toward its target with an ease-out curve (the active-tab pill, the
/// pane-icon decks).
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
    pub fn value(&mut self, ctx: &egui::Context, target: f32) -> f32 {
        let now = ctx.input(|i| i.time);
        if !self.init {
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
        let t = ((now - self.start) / self.duration).clamp(0.0, 1.0) as f32;
        if t >= 1.0 {
            return self.to;
        }
        let eased = 1.0 - (1.0 - t).powi(3);
        self.from + (self.to - self.from) * eased
    }
}
