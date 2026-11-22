//! Swiping sideways on any pane (a terminal, Files, Changes, Agents) to go to the next or
//! previous tab, on touch screens. It watches every finger that lands on a pane's body and
//! steps aside whenever the pane is doing something with it instead: a press held still (a
//! long press selects, in terminals and Files), files dragged out of Files, a pane dragged
//! by its header, a pinch, or a dialog or menu on top. A swipe that starts up or down is the
//! pane's (it scrolls), however far it drifts sideways after.

use egui::{Event, PointerButton, Pos2, Vec2};

use crate::app::App;
use crate::app::actions::Action;

/// A finger moving further than this (in points) is a swipe, not a tap or a long press.
pub(in crate::app) const TOUCH_SLOP: f32 = 10.0;
/// Holding a finger this long (in seconds, without moving) is a long press.
pub(in crate::app) const LONG_PRESS: f64 = 0.45;
/// A sideways swipe this long switches tabs; a quick flick needs only [`FLICK_DISTANCE`].
const SWIPE_DISTANCE: f32 = 80.0;
const FLICK_DISTANCE: f32 = 30.0;
/// Sideways speed (points a second) that makes a short swipe a flick.
const FLICK_SPEED: f32 = 400.0;

#[derive(Clone, Copy, PartialEq, Debug)]
enum Phase {
    /// Down and not yet moved far: may still become a sideways swipe.
    Pending,
    /// Moving sideways: switches tabs when the finger lifts, if it went far enough.
    Sideways,
    /// Something else (a scroll, a long press, a drag, a pinch): nothing more to do here.
    Dead,
}

/// A finger that landed on a pane's body.
pub(in crate::app) struct TabSwipe {
    origin: Pos2,
    at: f64,
    /// The finger's speed at its last move.
    velocity: Vec2,
    phase: Phase,
}

impl App {
    /// Follow a finger on the panes and switch tabs when it swipes sideways. Runs every
    /// frame, alongside whatever the pane under the finger does with it.
    pub(in crate::app) fn handle_tab_swipe(&mut self, ctx: &egui::Context) {
        let (events, touching, pinching, now, velocity) =
            ctx.input(|i| (i.events.clone(), i.any_touches(), i.multi_touch().is_some(), i.time, i.pointer.velocity()));
        let busy = pinching
            || self.pane_drag.is_some()
            || egui::DragAndDrop::has_any_payload(ctx)
            || ctx.memory(|m| m.top_modal_layer()).is_some();
        if let Some(s) = self.tab_swipe.as_mut() {
            if busy || (s.phase == Phase::Pending && now - s.at >= LONG_PRESS) {
                s.phase = Phase::Dead;
            }
        }
        for event in events {
            match event {
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, .. } => {
                    self.tab_swipe = (touching && !busy && self.swipe_starts_at(ctx, pos))
                        .then_some(TabSwipe { origin: pos, at: now, velocity: Vec2::ZERO, phase: Phase::Pending });
                }
                Event::PointerMoved(pos) => {
                    let Some(s) = self.tab_swipe.as_mut() else { continue };
                    s.velocity = velocity;
                    let d = pos - s.origin;
                    if s.phase == Phase::Pending && d.length() > TOUCH_SLOP {
                        // The first clear direction decides.
                        s.phase = if d.x.abs() > d.y.abs() { Phase::Sideways } else { Phase::Dead };
                    }
                }
                Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, .. } => {
                    let Some(s) = self.tab_swipe.take() else { continue };
                    if s.phase != Phase::Sideways {
                        continue;
                    }
                    let dx = pos.x - s.origin.x;
                    let vx = s.velocity.x;
                    let flick = dx.abs() >= FLICK_DISTANCE && vx.abs() >= FLICK_SPEED && vx.signum() == dx.signum();
                    if dx.abs() >= SWIPE_DISTANCE || flick {
                        // Swiping left brings in the tab on the right, as pages do.
                        self.pending.push(Action::SwipeTab(dx < 0.0));
                    }
                }
                _ => {}
            }
        }
    }

    /// A swipe may start at `pos`: on the body of a pane (not its header, which drags the
    /// pane), with nothing drawn over it.
    fn swipe_starts_at(&self, ctx: &egui::Context, pos: Pos2) -> bool {
        if self.keyboard_button.rect.is_some_and(|r| r.contains(pos)) || egui::Popup::is_any_open(ctx) {
            return false;
        }
        let header = if self.cfg.pane_headers { self.header_height() } else { 0.0 };
        self.card_rects.iter().any(|(_, r)| r.contains(pos) && pos.y >= r.top() + header)
    }

    /// The tab beside the shown one: the next (`forward`) or the previous. The home tab (Files
    /// or Agents), when open, comes before the first; the ends don't wrap around.
    pub(in crate::app) fn swipe_tab(&mut self, forward: bool) {
        let n = self.tabs.len();
        match (self.home_active, forward) {
            (true, true) if n > 0 => self.show_tab(0),
            (true, _) => {}
            (false, true) if self.active + 1 < n => self.show_tab(self.active + 1),
            (false, false) if self.active > 0 => self.show_tab(self.active - 1),
            (false, false) if self.home_tab.is_some() => self.home_active = true,
            (false, _) => {}
        }
    }
}
