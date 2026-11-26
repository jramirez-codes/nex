//! Scrolling a list with a finger: the content follows the finger up and down, and a quick
//! swipe flings on after it lifts, slowing down. For panes that draw their own scrolling
//! (Agents, Changes) and so get no touch scrolling from egui. A finger that starts sideways
//! is left alone, since a sideways swipe on a pane switches tabs; a pinch, too.

use egui::{Pos2, Rect, Ui};

use crate::ui::glide;

/// A finger moving further than this (in points) is a swipe, not a tap or a long press.
pub const SLOP: f32 = 10.0;
/// A fling starts above this speed and stops below [`FLING_STOP`] (points a second).
const FLING_START: f32 = 300.0;
const FLING_STOP: f32 = 5.0;
/// How quickly a fling slows down (the same as the terminals' and the title-bar tabs').
const FLING_DECAY: f32 = 4.0;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
enum Phase {
    /// No finger, no fling.
    #[default]
    Idle,
    /// A finger down in the area, not yet moved far.
    Pending,
    /// Following the finger.
    Panning,
    /// The finger went sideways (or a second one landed): not ours.
    Ignored,
    /// Scrolling on after the finger lifted.
    Flinging,
}

/// One scrollable area's finger scrolling. Keep it between frames.
#[derive(Clone, Copy, Debug, Default)]
pub struct TouchPan {
    phase: Phase,
    origin: Pos2,
    last_y: f32,
    /// Points a second, in scroll direction (positive moves the content up).
    velocity: f32,
}

impl TouchPan {
    /// How far to scroll `area` this frame, in points (positive moves the content up, as
    /// a growing scroll offset does). Call once a frame, whether or not anything is touched.
    pub fn delta(&mut self, ui: &Ui, area: Rect) -> f32 {
        let (touching, pressed, down, pos, origin, vy, pinching, dt) = ui.input(|i| {
            (
                i.any_touches(),
                i.pointer.primary_pressed(),
                i.pointer.primary_down(),
                i.pointer.interact_pos(),
                i.pointer.press_origin(),
                i.pointer.velocity().y,
                i.multi_touch().is_some(),
                i.stable_dt.min(0.1),
            )
        });
        if pressed {
            // Any press stops a fling; a finger in the area may start a pan.
            self.phase = Phase::Idle;
            if let Some(o) = origin.filter(|&o| touching && area.contains(o)) {
                *self = TouchPan { phase: Phase::Pending, origin: o, last_y: o.y, velocity: 0.0 };
            }
        }
        if pinching && matches!(self.phase, Phase::Pending | Phase::Panning) {
            self.phase = Phase::Ignored;
        }
        match self.phase {
            Phase::Pending | Phase::Panning if !down => {
                let fling = self.phase == Phase::Panning && glide::animations(ui.ctx()) && self.velocity.abs() > FLING_START;
                self.phase = if fling { Phase::Flinging } else { Phase::Idle };
                if fling {
                    ui.ctx().request_repaint();
                }
                0.0
            }
            Phase::Pending => {
                let Some(p) = pos else { return 0.0 };
                let d = p - self.origin;
                if d.length() <= SLOP {
                    return 0.0;
                }
                // The first clear direction decides, as for the terminals and tab swipes.
                if d.y.abs() >= d.x.abs() {
                    self.phase = Phase::Panning;
                    self.follow(p, vy)
                } else {
                    self.phase = Phase::Ignored;
                    0.0
                }
            }
            Phase::Panning => pos.map_or(0.0, |p| self.follow(p, vy)),
            Phase::Ignored if !down => {
                self.phase = Phase::Idle;
                0.0
            }
            Phase::Flinging => {
                let d = self.velocity * dt;
                self.velocity *= (-dt * FLING_DECAY).exp();
                if self.velocity.abs() < FLING_STOP {
                    self.phase = Phase::Idle;
                } else {
                    ui.ctx().request_repaint();
                }
                d
            }
            Phase::Idle | Phase::Ignored => 0.0,
        }
    }

    /// The finger at `p`, moving at `vy` points a second: scroll along with it.
    fn follow(&mut self, p: Pos2, vy: f32) -> f32 {
        let d = self.last_y - p.y;
        self.last_y = p.y;
        self.velocity = -vy;
        d
    }

    /// Leave the rest of this touch alone (something else took it, as a press and hold does):
    /// it scrolls nothing until the finger lifts.
    pub fn ignore(&mut self) {
        if matches!(self.phase, Phase::Pending | Phase::Panning) {
            self.phase = Phase::Ignored;
        }
    }

    /// Stop a fling (the content reached its end).
    pub fn stop(&mut self) {
        if self.phase == Phase::Flinging {
            self.phase = Phase::Idle;
        }
    }
}

#[cfg(test)]
mod tests {
    use egui::{Event, PointerButton, RawInput, TouchDeviceId, TouchId, TouchPhase, Vec2};

    use super::*;

    const AREA: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(400.0, 600.0) };

    struct Sim {
        ctx: egui::Context,
        pan: TouchPan,
        t: f64,
        /// Total scrolled.
        scrolled: f32,
    }

    impl Sim {
        fn new() -> Self {
            Sim { ctx: egui::Context::default(), pan: TouchPan::default(), t: 0.0, scrolled: 0.0 }
        }
        fn frame(&mut self, events: Vec<Event>) {
            self.t += 0.016;
            let input = RawInput { events, time: Some(self.t), ..Default::default() };
            let (pan, scrolled) = (&mut self.pan, &mut self.scrolled);
            let _ = self.ctx.run(input, |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| *scrolled += pan.delta(ui, AREA));
            });
        }
        fn touch(&mut self, id: u64, phase: TouchPhase, pos: Pos2) {
            let mut ev = vec![Event::Touch { device_id: TouchDeviceId(1), id: TouchId(id), phase, pos, force: None }];
            if id == 0 {
                match phase {
                    TouchPhase::Start => {
                        ev.push(Event::PointerMoved(pos));
                        ev.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
                    }
                    TouchPhase::Move => ev.push(Event::PointerMoved(pos)),
                    _ => {
                        ev.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
                        ev.push(Event::PointerGone);
                    }
                }
            }
            self.frame(ev);
        }
        fn swipe(&mut self, from: Pos2, to: Pos2, steps: usize) {
            self.touch(0, TouchPhase::Start, from);
            for k in 1..=steps {
                self.touch(0, TouchPhase::Move, from + (to - from) * (k as f32 / steps as f32));
            }
            self.touch(0, TouchPhase::End, to);
        }
        fn settle(&mut self) {
            for _ in 0..120 {
                self.frame(vec![]);
            }
        }
    }

    #[test]
    fn follows_the_finger_up_and_down() {
        let mut s = Sim::new();
        let p = Pos2::new(200.0, 400.0);
        s.swipe(p, p - Vec2::new(0.0, 200.0), 100);
        s.settle();
        assert!((s.scrolled - 200.0).abs() < 1.0, "a slow 200pt swipe up scrolled {}", s.scrolled);
        s.scrolled = 0.0;
        s.swipe(p, p + Vec2::new(0.0, 100.0), 100);
        s.settle();
        assert!((s.scrolled + 100.0).abs() < 1.0, "a slow 100pt swipe down scrolled {}", s.scrolled);
    }

    #[test]
    fn a_quick_swipe_flings_on() {
        let mut s = Sim::new();
        let p = Pos2::new(200.0, 400.0);
        s.swipe(p, p - Vec2::new(0.0, 200.0), 4);
        let lifted = s.scrolled;
        s.settle();
        assert!(s.scrolled > lifted + 20.0, "no fling: {lifted} -> {}", s.scrolled);
    }

    #[test]
    fn sideways_swipes_and_taps_dont_scroll() {
        let mut s = Sim::new();
        let p = Pos2::new(200.0, 300.0);
        s.swipe(p, p + Vec2::new(150.0, 20.0), 10);
        s.touch(0, TouchPhase::Start, p);
        s.touch(0, TouchPhase::End, p);
        s.settle();
        assert_eq!(s.scrolled, 0.0);
    }

    #[test]
    fn a_finger_outside_the_area_or_a_mouse_drag_doesnt_scroll() {
        let mut s = Sim::new();
        let p = Pos2::new(500.0, 300.0);
        s.swipe(p, p - Vec2::new(0.0, 200.0), 10);
        // A mouse drag (no touches) is the pane's own business (text selection, nothing).
        let q = Pos2::new(200.0, 300.0);
        s.frame(vec![
            Event::PointerMoved(q),
            Event::PointerButton { pos: q, button: PointerButton::Primary, pressed: true, modifiers: Default::default() },
        ]);
        s.frame(vec![Event::PointerMoved(q - Vec2::new(0.0, 200.0))]);
        s.frame(vec![Event::PointerButton {
            pos: q - Vec2::new(0.0, 200.0),
            button: PointerButton::Primary,
            pressed: false,
            modifiers: Default::default(),
        }]);
        s.settle();
        assert_eq!(s.scrolled, 0.0);
    }

    #[test]
    fn a_pinch_stops_the_pan() {
        let mut s = Sim::new();
        let p = Pos2::new(200.0, 400.0);
        s.touch(0, TouchPhase::Start, p);
        s.touch(0, TouchPhase::Move, p - Vec2::new(0.0, 20.0));
        let before = s.scrolled;
        s.touch(1, TouchPhase::Start, p + Vec2::new(100.0, 0.0));
        for k in 1..=10 {
            s.touch(0, TouchPhase::Move, p - Vec2::new(0.0, 20.0 + 10.0 * k as f32));
        }
        s.touch(1, TouchPhase::End, p + Vec2::new(100.0, 0.0));
        s.touch(0, TouchPhase::End, p - Vec2::new(0.0, 120.0));
        s.settle();
        assert_eq!(s.scrolled, before, "a pinch kept scrolling");
    }
}
