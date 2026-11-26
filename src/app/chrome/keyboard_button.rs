//! The floating keyboard button (mobile: Android and iOS): a round glass button in the
//! bottom-right corner of a terminal or the Agents tab, above the on-screen keyboard when it's
//! up.
//!
//! - A tap puts the keyboard away or brings it back (it also goes away the usual system
//!   ways, and nex leaves it down until the button brings it back).
//! - A drag makes it a joystick: the button opens into a track, a knob follows the finger,
//!   and the arrow key it points to repeats, faster the further it is pushed. An arrow waits
//!   until the finger has settled on it (pointing one way, hardly turning, for [`SETTLE`]), so
//!   a quick flick out and back up is a nudge: one arrow, sent as it lifts.
//! - Spinning it round in a full circle (either way) is a silent circle: Esc, once a turn, and
//!   no arrows, since a finger going round never settles. Past [`SPIN_QUIET`] of turning no
//!   arrows go at all until it comes back to the center, and a lift after a spin sends nothing.
//! - A double-tap arms Ctrl for the next key (`^C` is double-tap, then `c`); a second double
//!   tap disarms it.
//! - Holding it still (in a terminal) talks to Claude Code's voice mode, which records while
//!   Space is held: a terminal sees that as fast repeated spaces, and the on-screen keyboard
//!   can't send them (holding its Space moves its cursor). After [`HOLD_TO_TALK`] the button
//!   turns red, the microphone is shared with the pane's host (`app/ssh/mic.rs`), and once the
//!   host can record, a space goes to the terminal every [`SPACE_REPEAT`]. Letting go keeps
//!   them coming for [`TALK_TAIL`], so the last words get through the share's delay. Pressing
//!   again during that tail goes on talking.
//!
//! Telling a tap from a double-tap means a tap acts a moment after it lifts. The timing rules
//! are plain functions ([`lift`], [`hold_turns_to_talk`], [`Talk::tick`]), and so are telling
//! a spin ([`Spin::turn`]), a settled finger ([`Settle`]) and a nudge ([`nudge`]), tested below.

use std::f32::consts::{PI, TAU};

use egui::epaint::{PathShape, PathStroke, Shadow};
use egui::{Color32, Id, Key, LayerId, Modifiers, Order, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::App;
use crate::app::input::press::{DOUBLE_TAP, FIRST_REPEAT, HOLD_TO_TALK};
use crate::layout::{PaneId, PaneKind};
use crate::platform::{self, Haptic};
use crate::ui::glide::{Lerp, animations, ease_out};
use crate::ui::icons::{icon_control, icon_keyboard, icon_keyboard_hide, icon_mic};
use crate::ui::theme::Theme;

/// Apple's minimum tap target.
pub(in crate::app) const SIZE: f32 = 44.0;
/// From the right and bottom edges of the content area.
const MARGIN: f32 = 12.0;
/// How long the button takes to fade in.
const FADE_TIME: f32 = 0.15;
/// How far a finger moves before a press becomes the joystick.
const DEAD_ZONE: f32 = 10.0;
/// Holding the joystick: the first repeat comes after [`FIRST_REPEAT`], then every this
/// often, from just past the dead zone to pushed all the way.
const REPEAT_SLOW: f64 = 0.12;
const REPEAT_FAST: f64 = 0.04;
/// How far from the center the repeats are at their fastest.
const FULL_PUSH: f32 = 60.0;
/// The joystick's track, as wide as the button opens to.
const TRACK: f32 = 76.0;
/// The knob follows the finger this far, then gives way less and less, up to `MAX_TRAVEL`.
const FREE_TRAVEL: f32 = 24.0;
const MAX_TRAVEL: f32 = 34.0;
/// How long the knob takes to spring back once let go.
const SPRING_TIME: f64 = 0.45;
/// While talking, a space goes to the terminal this often: faster than a held key repeats,
/// which is what Claude Code listens for.
const SPACE_REPEAT: f64 = 0.03;
/// Spaces go on this long after letting go: the share sends audio 0.2 s late
/// (`remote::mic`'s `LEAD`), and recording stops when the spaces do.
const TALK_TAIL: f64 = 0.35;
/// Turned this far round in one go, the joystick is being spun (for Esc), not pointed: its
/// arrows stop. More than a half turn, so swinging from one side to the other still moves.
const SPIN_QUIET: f32 = 200.0 * PI / 180.0;
/// A direction's first arrow waits until the finger has pointed that way this long, turning
/// less than [`SETTLE_TURN`]: a finger going round never does, so a spin sends no arrows.
pub(in crate::app) const SETTLE: f64 = 0.12;
const SETTLE_TURN: f32 = 25.0 * PI / 180.0;

/// Where the button goes in `area` (the content area).
pub(in crate::app) fn button_rect(area: Rect) -> Rect {
    let min = Pos2::new(area.right() - MARGIN - SIZE, area.bottom() - MARGIN - SIZE);
    Rect::from_min_size(min, Vec2::splat(SIZE))
}

/// Make the button a widget on top of everything, so egui hands presses on it to the button
/// and not to a widget drawn underneath (an Agents card or row, a pane's header button).
/// The button reads its own presses from the raw events; this only shadows what's below.
/// egui hit-tests against the last frame's widgets, so it holds from the next frame on.
fn claim_presses(ctx: &egui::Context, rect: Rect) {
    egui::Area::new(Id::new("keyboard-button-hit")).order(Order::Foreground).fixed_pos(rect.min).show(ctx, |ui| {
        ui.allocate_rect(rect, egui::Sense::click_and_drag());
    });
}

/// What the button's keys go to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Target {
    Terminal,
    Agents,
}

/// The keyboard button's state, kept across frames.
#[derive(Default)]
pub(in crate::app) struct KeyboardButton {
    /// The keyboard is down over terminals, and nex keeps it down until the button is tapped.
    pub hidden: bool,
    /// The keyboard is up over an Agents pane (it starts down there: the pane is mostly
    /// looked at, and the joystick moves through it).
    pub agents_keyboard: bool,
    /// Where the button was this frame, for taps (`None` while it isn't shown).
    pub rect: Option<Rect>,
    /// Ctrl is armed (double-tap): the next key goes to the program with Ctrl held.
    pub ctrl_armed: bool,
    /// A finger on the button.
    press: Option<Press>,
    /// When the last tap lifted, while it's still waiting to see if it's a double-tap.
    pending_tap: Option<f64>,
    /// What the system last said about the keyboard, to notice it coming up or going away.
    last_visible: Option<bool>,
    /// The pane focused last frame: Ctrl armed for one pane isn't kept for another.
    last_focus: Option<PaneId>,
    /// The knob let go: where it was and when, as it springs back to the center.
    spring: Option<(Vec2, f64)>,
    /// When Ctrl was last armed, for the button's pulse.
    armed_at: f64,
    /// When the joystick last sent an arrow, for its chevron's tick.
    tick_at: f64,
    /// Hold-to-talk is sending spaces (through the tail after letting go, too).
    talk: Option<Talk>,
}

/// A finger on the button.
struct Press {
    origin: Pos2,
    /// From where it went down to where it is.
    offset: Vec2,
    /// It moved past the dead zone: the press is the joystick.
    stick: Option<Stick>,
    /// When it went down.
    down_at: f64,
    /// It was held still long enough to talk (even if the share was refused): letting go
    /// isn't a tap.
    talking: bool,
}

/// Hold-to-talk sending spaces.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Talk {
    /// When the next space is due.
    next: f64,
    /// Let go: spaces stop at this time.
    until: Option<f64>,
}

impl Talk {
    fn new(now: f64) -> Self {
        Self { next: now, until: None }
    }

    /// The finger lifted at `now`: the tail starts.
    fn release(&mut self, now: f64) {
        self.until.get_or_insert(now + TALK_TAIL);
    }

    /// One frame at `now`, with the host able to record (`ready`) or not yet: whether to send
    /// a space (at most one a frame), and when to look again (`None`: talking is over).
    fn tick(&mut self, now: f64, ready: bool) -> (bool, Option<f64>) {
        if self.until.is_some_and(|until| now >= until) {
            return (false, None);
        }
        let send = ready && now >= self.next;
        if send {
            self.next = now + SPACE_REPEAT;
        }
        // Not ready yet: the share's events repaint, but look again soon anyway.
        let wake = if ready { self.next } else { now + SPACE_REPEAT };
        (send, Some(self.until.map_or(wake, |until| wake.min(until))))
    }
}

/// A press that went down at `down_at` and hasn't moved past the dead zone (`still`), and
/// isn't talking yet, starts talking at `now`.
fn hold_turns_to_talk(down_at: f64, still: bool, now: f64) -> bool {
    still && now - down_at >= HOLD_TO_TALK
}

/// What a finger lifting off the button means.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Lift {
    /// The end of a joystick drag.
    Stick,
    /// The end of a hold to talk.
    Talk,
    /// The second tap of a double-tap.
    DoubleTap,
    /// A tap (acted on once no second one follows).
    Tap,
}

/// The press lifting at `now` was a joystick (`stick`) or a hold to talk (`talking`), or a
/// plain press; `pending_tap` is when the tap before it lifted, if it's still waiting.
fn lift(stick: bool, talking: bool, pending_tap: Option<f64>, now: f64) -> Lift {
    if stick {
        Lift::Stick
    } else if talking {
        Lift::Talk
    } else if pending_tap.is_some_and(|at| now - at < DOUBLE_TAP) {
        Lift::DoubleTap
    } else {
        Lift::Tap
    }
}

struct Stick {
    /// The arrow being pointed to, `None` while the finger is back inside the dead zone.
    dir: Option<Key>,
    /// When the arrow repeats next (once it has settled).
    next: f64,
    /// How far the finger has gone round the center.
    spin: Spin,
    /// Whether the finger has settled on `dir` yet (`None` inside the dead zone).
    settle: Option<Settle>,
    /// The drag has sent a key (an arrow or Esc): lifting it isn't a nudge.
    sent: bool,
}

/// Whether the joystick's finger has settled on a direction, for its first arrow.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Settle {
    dir: Key,
    /// The finger's angle when it started settling.
    angle: f32,
    /// When it started settling.
    since: f64,
    /// It settled and the first arrow went: it repeats from here, however the finger turns
    /// while it points the same way.
    held: bool,
}

impl Settle {
    /// The finger is at `offset` (past the dead zone) at `now`, starting to settle.
    fn new(offset: Vec2, now: f64) -> Self {
        Self { dir: direction(offset), angle: offset.y.atan2(offset.x), since: now, held: false }
    }

    /// The finger moved to `offset` at `now`: pointing somewhere new, or (before it settled)
    /// turning past [`SETTLE_TURN`], starts it over.
    fn moved(&mut self, offset: Vec2, now: f64) {
        let turned = ((offset.y.atan2(offset.x) - self.angle + PI).rem_euclid(TAU) - PI).abs();
        if direction(offset) != self.dir || (!self.held && turned > SETTLE_TURN) {
            *self = Self::new(offset, now);
        }
    }

    /// Whether the first arrow goes at `now` (once: it's held after).
    fn due(&mut self, now: f64) -> bool {
        let due = !self.held && now - self.since >= SETTLE;
        self.held |= due;
        due
    }
}

/// Lifting a joystick drag that `sent` keys or not, having turned at most `turned` round the
/// center, pointing to `dir`: the arrow it sends. A quick flick that never settled and
/// didn't turn is a nudge, one arrow; anything else sent what it meant already.
fn nudge(sent: bool, turned: f32, dir: Option<Key>) -> Option<Key> {
    dir.filter(|_| !sent && turned <= SETTLE_TURN)
}

/// How far a joystick drag has gone round the center, to send Esc for each full circle.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
struct Spin {
    /// The finger's angle last time, `None` inside the dead zone.
    last: Option<f32>,
    /// The turning since the last full circle (or since leaving the dead zone), signed:
    /// turning back undoes it.
    turned: f32,
    /// It turned past [`SPIN_QUIET`] (or a full circle): no arrows until it comes back to
    /// the center.
    spinning: bool,
    /// The most it has turned either way (a full circle counts in full), for telling a nudge.
    most: f32,
}

impl Spin {
    /// The finger moved to `offset` from the center: whether that finished a full circle.
    /// Inside the dead zone, the spin starts over.
    fn turn(&mut self, offset: Vec2) -> bool {
        if offset.length() <= DEAD_ZONE {
            *self = Self::default();
            return false;
        }
        let angle = offset.y.atan2(offset.x);
        if let Some(last) = self.last.replace(angle) {
            // The short way round from last time.
            self.turned += (angle - last + PI).rem_euclid(TAU) - PI;
        }
        self.most = self.most.max(self.turned.abs());
        if self.turned.abs() >= SPIN_QUIET {
            self.spinning = true;
        }
        // A hair under a full turn counts: the steps of an exact one can add up just short.
        if self.turned.abs() >= TAU - 1e-3 {
            self.turned -= TAU.copysign(self.turned);
            return true;
        }
        false
    }
}

/// The arrow `offset` points to, along its longer axis.
fn direction(offset: Vec2) -> Key {
    if offset.x.abs() >= offset.y.abs() {
        if offset.x > 0.0 { Key::ArrowRight } else { Key::ArrowLeft }
    } else if offset.y > 0.0 {
        Key::ArrowDown
    } else {
        Key::ArrowUp
    }
}

fn unit(dir: Key) -> Vec2 {
    match dir {
        Key::ArrowRight => Vec2::X,
        Key::ArrowLeft => -Vec2::X,
        Key::ArrowDown => Vec2::Y,
        _ => -Vec2::Y,
    }
}

/// How often the arrow repeats with the finger `distance` from the center.
fn repeat_interval(distance: f32) -> f64 {
    let t = ((distance - DEAD_ZONE) / (FULL_PUSH - DEAD_ZONE)).clamp(0.0, 1.0) as f64;
    REPEAT_SLOW + (REPEAT_FAST - REPEAT_SLOW) * t
}

/// Where the knob sits for a finger `offset` from where it went down: with it at first, then
/// rubber-banding (as iOS lists do past their ends), never past `MAX_TRAVEL`.
fn knob_travel(offset: Vec2) -> Vec2 {
    let d = offset.length();
    if d <= FREE_TRAVEL {
        return offset;
    }
    let give = MAX_TRAVEL - FREE_TRAVEL;
    let extra = give * (1.0 - (-(d - FREE_TRAVEL) / (give * 2.5)).exp());
    offset * ((FREE_TRAVEL + extra) / d)
}

/// A damped spring from 1 to 0 over `SPRING_TIME`, overshooting a little.
fn spring(t: f64) -> f32 {
    let t = t as f32;
    (-9.0 * t).exp() * (14.0 * t).cos()
}

impl KeyboardButton {
    /// Whether the keyboard is (to be) down for `target`.
    pub(in crate::app) fn down(&self, target: Target) -> bool {
        match target {
            Target::Terminal => self.hidden,
            Target::Agents => !self.agents_keyboard,
        }
    }

    fn set_down(&mut self, target: Target, down: bool) {
        match target {
            Target::Terminal => self.hidden = down,
            Target::Agents => self.agents_keyboard = !down,
        }
    }

    /// Whether the button is being used as a joystick.
    fn stick_active(&self) -> bool {
        self.press.as_ref().is_some_and(|p| p.stick.is_some())
    }

    /// The knob's offset from the center at time `now`.
    fn knob(&self, now: f64) -> Vec2 {
        match (&self.press, self.spring) {
            (Some(p), _) if p.stick.is_some() => knob_travel(p.offset),
            (_, Some((from, at))) if now - at < SPRING_TIME => from * spring(now - at),
            _ => Vec2::ZERO,
        }
    }
}

impl App {
    /// What the keyboard and the button are for this frame: a focused terminal or Agents
    /// pane with nothing in the way (a drag, a dialog, a pinch).
    pub(in crate::app) fn keyboard_target(&self, ctx: &egui::Context) -> Option<Target> {
        if !self.chrome.soft_keyboard
            || self.input.pane_drag.is_some()
            || self.input.mouse.pinching()
            || ctx.memory(|m| m.top_modal_layer()).is_some()
        {
            return None;
        }
        match self.pane_kind(self.focused_pane()?)? {
            PaneKind::Terminal => Some(Target::Terminal),
            PaneKind::Agents => Some(Target::Agents),
            _ => None,
        }
    }

    /// Follow the keyboard when the system moves it: put away with the back gesture, nex
    /// keeps it down; brought back by Android (returning to the app), nex keeps it up.
    fn follow_system_keyboard(&mut self, target: Option<Target>) {
        let kb = &mut self.chrome.keyboard_button;
        let visible = platform::soft_keyboard_visible();
        if visible != kb.last_visible {
            kb.last_visible = visible;
            if let (Some(target), Some(visible)) = (target, visible) {
                kb.set_down(target, !visible);
            }
        }
    }

    /// Draw the button and read its taps, double-taps and drags. Runs before `handle_mouse`,
    /// which leaves presses on the button alone.
    pub(in crate::app) fn keyboard_button(&mut self, ctx: &egui::Context) {
        let target = self.keyboard_target(ctx);
        self.follow_system_keyboard(target);
        let focus = self.focused_pane();
        if focus != self.chrome.keyboard_button.last_focus {
            self.chrome.keyboard_button.last_focus = focus;
            self.chrome.keyboard_button.ctrl_armed = false;
            self.chrome.keyboard_button.talk = None;
        }
        // It waits for the connect handoff to finish, then fades in as usual.
        let shown = target.is_some() && self.handoff.is_none();
        let fade = if animations(ctx) {
            ctx.animate_bool_with_time(Id::new("keyboard-button"), shown, FADE_TIME)
        } else if shown {
            1.0
        } else {
            0.0
        };
        let Some(target) = target else {
            let kb = &mut self.chrome.keyboard_button;
            kb.rect = None;
            kb.press = None;
            kb.pending_tap = None;
            kb.talk = None;
            return;
        };
        let rect = button_rect(self.content_area);
        self.chrome.keyboard_button.rect = Some(rect);
        claim_presses(ctx, rect);
        let now = ctx.input(|i| i.time);

        let events = ctx.input(|i| i.events.clone());
        for event in events {
            match event {
                egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => {
                    if rect.contains(pos) {
                        let kb = &mut self.chrome.keyboard_button;
                        // Pressed again while the spaces trail off: go on talking.
                        let talking = match kb.talk.as_mut() {
                            Some(talk) => {
                                talk.until = None;
                                true
                            }
                            None => false,
                        };
                        kb.press = Some(Press { origin: pos, offset: Vec2::ZERO, stick: None, down_at: now, talking });
                        kb.spring = None;
                    }
                }
                egui::Event::PointerMoved(pos) => self.joystick_move(pos, now),
                egui::Event::PointerButton { button: egui::PointerButton::Primary, pressed: false, .. } => self.release(now, target),
                // A touch that ends without a release (the system took it): no tap.
                egui::Event::PointerGone => self.cancel_button_press(now),
                _ => {}
            }
        }
        if self.chrome.keyboard_button.press.is_some() && !ctx.input(|i| i.pointer.any_down()) {
            self.cancel_button_press(now);
        }
        if target == Target::Terminal {
            self.hold_to_talk(ctx, now);
        }
        self.repeat_arrow(ctx, now, target);
        // A tap with no second one after it: it was a tap.
        if let Some(at) = self.chrome.keyboard_button.pending_tap {
            if now - at >= DOUBLE_TAP {
                self.chrome.keyboard_button.pending_tap = None;
                let down = self.chrome.keyboard_button.down(target);
                self.chrome.keyboard_button.set_down(target, !down);
            } else {
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(DOUBLE_TAP - (now - at)));
            }
        }
        self.paint_keyboard_button(ctx, rect, fade, target, now);
    }

    /// A press held still turns to talking; while talking, spaces go to the terminal.
    fn hold_to_talk(&mut self, ctx: &egui::Context, now: f64) {
        if let Some(press) = self.chrome.keyboard_button.press.as_mut()
            && !press.talking
        {
            if hold_turns_to_talk(press.down_at, press.stick.is_none(), now) {
                press.talking = true;
                self.chrome.keyboard_button.pending_tap = None;
                if self.share_mic_for_talk(ctx) {
                    self.chrome.keyboard_button.talk = Some(Talk::new(now));
                    platform::haptic(Haptic::Confirm);
                }
            } else if press.stick.is_none() {
                // Wake when it would turn.
                ctx.request_repaint_after(std::time::Duration::from_secs_f64(HOLD_TO_TALK - (now - press.down_at)));
            }
        }
        let Some(mut talk) = self.chrome.keyboard_button.talk else { return };
        // No share any more (it failed, or was stopped): nothing to talk through.
        let Some(ready) = self.mic_ready_for_talk() else {
            self.chrome.keyboard_button.talk = None;
            return;
        };
        let (send, wake) = talk.tick(now, ready);
        if send && let Some(t) = self.focused_terminal() {
            t.input(b" ".to_vec());
        }
        self.chrome.keyboard_button.talk = wake.map(|_| talk);
        if let Some(at) = wake {
            ctx.request_repaint_after(std::time::Duration::from_secs_f64((at - now).max(0.0)));
        }
    }

    /// The finger moved: past the dead zone the press is the joystick, pointing to an arrow
    /// (sent once it settles, in `repeat_arrow`) and sending Esc for each full circle. Not
    /// while talking.
    fn joystick_move(&mut self, pos: Pos2, now: f64) {
        let Some(press) = self.chrome.keyboard_button.press.as_mut() else { return };
        if press.talking {
            return;
        }
        press.offset = pos - press.origin;
        let distance = press.offset.length();
        let out = distance > DEAD_ZONE;
        let dir = out.then(|| direction(press.offset));
        let circle = match &mut press.stick {
            None if out => {
                let mut spin = Spin::default();
                spin.turn(press.offset);
                let settle = Some(Settle::new(press.offset, now));
                press.stick = Some(Stick { dir, next: now, spin, settle, sent: false });
                false
            }
            Some(stick) => {
                stick.dir = dir;
                match (&mut stick.settle, out) {
                    (Some(settle), true) => settle.moved(press.offset, now),
                    (settle, true) => *settle = Some(Settle::new(press.offset, now)),
                    (settle, false) => *settle = None,
                }
                let circle = stick.spin.turn(press.offset);
                stick.sent |= circle;
                circle
            }
            None => false,
        };
        if circle {
            self.key_to_pane(Key::Escape, Modifiers::NONE);
            self.chrome.keyboard_button.tick_at = now;
            platform::haptic(Haptic::Confirm);
        }
    }

    /// Send the joystick's arrow once the finger has settled on it, then repeat it while it's
    /// held there. Never while spinning.
    fn repeat_arrow(&mut self, ctx: &egui::Context, now: f64, target: Target) {
        let Some(press) = self.chrome.keyboard_button.press.as_mut() else { return };
        let distance = press.offset.length();
        let Some(stick) = press.stick.as_mut() else { return };
        if stick.spin.spinning {
            return;
        }
        let Some(settle) = stick.settle.as_mut() else { return };
        let dir = settle.dir;
        let send = if settle.due(now) {
            stick.next = now + FIRST_REPEAT;
            true
        } else if settle.held && now >= stick.next {
            stick.next = now + repeat_interval(distance);
            true
        } else {
            false
        };
        let wake = if settle.held { stick.next } else { settle.since + SETTLE };
        if send {
            stick.sent = true;
            self.send_arrow(dir, now, target);
        }
        ctx.request_repaint_after(std::time::Duration::from_secs_f64((wake - now).max(0.0)));
    }

    /// One arrow from the joystick, with Ctrl if it's armed (in a terminal).
    fn send_arrow(&mut self, dir: Key, now: f64, target: Target) {
        let mut mods = Modifiers::NONE;
        if target == Target::Terminal && std::mem::take(&mut self.chrome.keyboard_button.ctrl_armed) {
            mods = Modifiers::CTRL;
        }
        self.key_to_pane(dir, mods);
        self.chrome.keyboard_button.tick_at = now;
        platform::haptic(Haptic::Tick);
    }

    /// The finger lifted: a tap (or the second of a double-tap), the end of a drag, or of a
    /// hold to talk.
    fn release(&mut self, now: f64, target: Target) {
        let kb = &mut self.chrome.keyboard_button;
        let Some(press) = kb.press.take() else { return };
        let mut arrow = None;
        match lift(press.stick.is_some(), press.talking, kb.pending_tap, now) {
            Lift::Stick => {
                kb.spring = Some((knob_travel(press.offset), now));
                arrow = press.stick.as_ref().and_then(|s| nudge(s.sent, s.spin.most, s.dir));
            }
            Lift::Talk => {
                if let Some(talk) = kb.talk.as_mut() {
                    talk.release(now);
                }
            }
            Lift::DoubleTap => {
                kb.pending_tap = None;
                kb.ctrl_armed = !kb.ctrl_armed;
                if kb.ctrl_armed {
                    kb.armed_at = now;
                    // Ctrl needs a key to go with it.
                    kb.set_down(target, false);
                    platform::haptic(Haptic::Confirm);
                } else {
                    platform::haptic(Haptic::Release);
                }
            }
            Lift::Tap => {
                kb.pending_tap = Some(now);
                platform::haptic(Haptic::Tap);
            }
        }
        // A quick flick that never settled: one arrow, as it lifts.
        if let Some(dir) = arrow {
            self.send_arrow(dir, now, target);
        }
    }

    /// The touch ended without a release (the system took it): no tap, and talking stops at
    /// once.
    fn cancel_button_press(&mut self, now: f64) {
        let kb = &mut self.chrome.keyboard_button;
        kb.talk = None;
        if let Some(press) = kb.press.take()
            && press.stick.is_some()
        {
            kb.spring = Some((knob_travel(press.offset), now));
        }
    }

    fn paint_keyboard_button(&self, ctx: &egui::Context, rect: Rect, fade: f32, target: Target, now: f64) {
        let theme = Theme::from_config(&self.cfg);
        let kb = &self.chrome.keyboard_button;
        let anim = animations(ctx);
        let ease = |id: &str, on: bool, time: f32| {
            if anim {
                ctx.animate_bool_with_time(Id::new(("keyboard-button", id)), on, time)
            } else if on {
                1.0
            } else {
                0.0
            }
        };
        let pressing = kb.press.as_ref().is_some_and(|p| p.stick.is_none());
        let press = ease("pressed", pressing, 0.08);
        let talk = ease("talk", kb.talk.is_some(), 0.15);
        let track = ease_out(ease("track", kb.stick_active(), 0.18));
        let armed = ease("armed", kb.ctrl_armed, 0.2);
        let up = ease("up", !kb.down(target), 0.15);
        // The button glides up and down with the keyboard rather than jumping.
        let cy = if anim { ctx.animate_value_with_time(Id::new(("keyboard-button", "y")), rect.center().y, 0.25) } else { rect.center().y };
        let center = Pos2::new(rect.center().x, cy);
        // Without animations the knob doesn't spring back: it is centered once let go.
        let knob = if anim || kb.stick_active() { kb.knob(now) } else { Vec2::ZERO };
        if anim && (kb.spring.is_some_and(|(_, at)| now - at < SPRING_TIME) || kb.ctrl_armed || kb.talk.is_some()) {
            ctx.request_repaint();
        }

        let mut p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("keyboard-button")));
        p.multiply_opacity(fade);

        // The joystick's track, opening out from under the button.
        if track > 0.0 {
            let radius = SIZE / 2.0 + (TRACK - SIZE) / 2.0 * track;
            glass(&p, center, radius, Glass { fill: fill_color(0.0).gamma_multiply(0.55), rim: Color32::from_white_alpha(26), shadow: 0.5 }, track);
            let active = kb.press.as_ref().and_then(|p| p.stick.as_ref()).and_then(|s| s.dir);
            let tick = if anim { (1.0 - (now - kb.tick_at) as f32 / 0.12).clamp(0.0, 1.0) } else { 0.0 };
            for dir in [Key::ArrowUp, Key::ArrowRight, Key::ArrowDown, Key::ArrowLeft] {
                let on = active == Some(dir);
                let nudge = if on { 2.0 * tick } else { 0.0 };
                let at = center + unit(dir) * (radius - 9.0 + nudge);
                let color = theme.text_bright.gamma_multiply(if on { 0.95 } else { 0.32 } * track);
                chevron(&p, at, unit(dir), 4.0, color);
            }
            if tick > 0.0 {
                ctx.request_repaint();
            }
        }

        // The button itself (the knob, while it's a joystick).
        let pulse = if kb.ctrl_armed && anim && now - kb.armed_at < 0.3 { 1.0 + 0.08 * (PI * ((now - kb.armed_at) / 0.3) as f32).sin() } else { 1.0 };
        // Talking: a slow, gentle swell, as if breathing with the voice.
        let swell = if anim { 1.0 + 0.05 * talk * (0.5 + 0.5 * (TAU * (now / 1.1) as f32).sin()) } else { 1.0 };
        let radius = SIZE / 2.0 * (1.0 - 0.08 * press) * pulse * swell;
        let c = center + knob;
        let tint = theme.blue.gamma_multiply(0.5);
        let fill = Color32::lerp(Color32::lerp(fill_color(press), tint, armed), theme.red.gamma_multiply(0.8), talk);
        let rim = Color32::lerp(Color32::lerp(Color32::from_white_alpha(34), theme.blue.gamma_multiply(0.85), armed), theme.red, talk);
        glass(&p, c, radius, Glass { fill, rim, shadow: 1.0 }, 1.0);

        // Armed: a hairline ring that breathes while it waits for the key.
        if armed > 0.0 {
            let breath = if anim { 0.4 + 0.4 * (0.5 + 0.5 * (TAU * (now / 1.6) as f32).sin()) } else { 0.8 };
            p.circle_stroke(c, radius + 3.5, Stroke::new(1.0_f32, theme.blue.gamma_multiply(breath * armed)));
        }

        // The glyph: the keyboard (with a down chevron while it's up, to put it away), turning
        // into ⌃ while Ctrl is armed; gone while the button is a joystick.
        let glyph_alpha = 1.0 - track;
        if glyph_alpha > 0.0 {
            let color = theme.text_bright.gamma_multiply(0.88 * glyph_alpha);
            let side = radius * 2.0 * 0.5;
            let kb_scale = 1.0 - 0.4 * armed;
            let kb_rect = Rect::from_center_size(c, Vec2::splat(side * kb_scale));
            // The mic takes the place of the other glyphs while talking.
            let rest = 1.0 - talk;
            if up < 1.0 {
                icon_keyboard(&p, kb_rect, color.gamma_multiply((1.0 - up) * (1.0 - armed) * rest));
            }
            if up > 0.0 {
                icon_keyboard_hide(&p, kb_rect, color.gamma_multiply(up * (1.0 - armed) * rest));
            }
            if armed > 0.0 {
                let ctrl = Rect::from_center_size(c, Vec2::splat(side * (0.6 + 0.4 * armed)));
                icon_control(&p, ctrl, theme.text_bright.gamma_multiply(armed * glyph_alpha * rest));
            }
            if talk > 0.0 {
                let mic = Rect::from_center_size(c, Vec2::splat(side * (0.7 + 0.3 * talk)));
                icon_mic(&p, mic, theme.text_bright.gamma_multiply(talk * glyph_alpha), true);
            }
        }
    }
}

/// The button's glass fill, lightened by `press` (0 to 1).
fn fill_color(press: f32) -> Color32 {
    let lift = (22.0 * press) as u8;
    Color32::from_rgba_unmultiplied(0x5a + lift, 0x55 + lift, 0x5e + lift, 165)
}

struct Glass {
    fill: Color32,
    rim: Color32,
    /// How much of the drop shadow to draw (0 to 1).
    shadow: f32,
}

/// A round glass shape, like [`glass_pill`](crate::ui::paint::glass_pill) for a circle: soft
/// shadow, translucent fill, hairline rim and a bright arc along the top edge, all faded by
/// `alpha`.
fn glass(p: &Painter, c: Pos2, radius: f32, g: Glass, alpha: f32) {
    let r = Rect::from_center_size(c, Vec2::splat(radius * 2.0));
    let shadow = Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha((120.0 * g.shadow * alpha) as u8) };
    p.add(shadow.as_shape(r, radius.min(255.0) as u8));
    p.circle_filled(c, radius, g.fill.gamma_multiply(alpha));
    p.circle_stroke(c, radius - 0.5, Stroke::new(1.0_f32, g.rim.gamma_multiply(alpha)));
    // The top highlight: an arc over the upper third, just inside the rim.
    let arc: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = PI * (1.25 + 0.5 * i as f32 / 24.0);
            c + Vec2::angled(a) * (radius - 1.5)
        })
        .collect();
    p.add(Shape::Path(PathShape::line(arc, PathStroke::new(1.0, Color32::from_white_alpha((50.0 * alpha) as u8)))));
}

/// A small chevron at `at` pointing along `dir`, `k` long on each arm.
fn chevron(p: &Painter, at: Pos2, dir: Vec2, k: f32, c: Color32) {
    let side = dir.rot90();
    let tip = at + dir * (k * 0.5);
    let back = tip - dir * k;
    p.add(Shape::line(vec![back + side * k, tip, back - side * k], Stroke::new(1.5_f32, c)));
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Spin `turns` times round the center (negative: the other way) in `steps` moves a
    /// turn, from pointing right; how many full circles it counted.
    fn spin(s: &mut Spin, turns: f32, steps: usize) -> usize {
        let n = (turns.abs() * steps as f32).round() as usize;
        (0..=n).filter(|&i| s.turn(Vec2::angled(TAU * turns.signum() * i as f32 / steps as f32) * 30.0)).count()
    }

    #[test]
    fn a_full_circle_either_way_is_one_esc() {
        for way in [1.0, -1.0] {
            let mut s = Spin::default();
            assert_eq!(spin(&mut s, 0.95 * way, 36), 0, "not quite round");
            assert!(s.spinning, "most of the way round: the arrows stop");
            let mut s = Spin::default();
            assert_eq!(spin(&mut s, way, 36), 1);
            let mut s = Spin::default();
            assert_eq!(spin(&mut s, 2.5 * way, 36), 2, "once a turn");
            // Coarse moves (a slow frame) still count, the short way round each time.
            let mut s = Spin::default();
            assert_eq!(spin(&mut s, way, 5), 1);
        }
    }

    #[test]
    fn pointing_and_swinging_isnt_spinning() {
        let mut s = Spin::default();
        // Right, then swung round to the left and back: half a turn each way.
        assert_eq!(spin(&mut s, 0.5, 36), 0);
        assert!(!s.spinning, "a half-turn swing still sends its arrows");
        for i in (0..=18).rev() {
            assert!(!s.turn(Vec2::angled(PI * i as f32 / 18.0) * 30.0));
        }
        assert!(s.turned.abs() < 1e-3, "turning back undoes it: {}", s.turned);
        // Wobbling back and forth never adds up to a circle.
        for i in 0..200 {
            let a = if i % 2 == 0 { 0.6 } else { -0.6 };
            assert!(!s.turn(Vec2::angled(a) * 30.0));
        }
        assert!(!s.spinning);
    }

    #[test]
    fn back_to_the_center_starts_the_spin_over() {
        let mut s = Spin::default();
        assert_eq!(spin(&mut s, 0.75, 36), 0);
        assert!(s.spinning);
        assert!(!s.turn(Vec2::new(3.0, 2.0)), "in the dead zone");
        assert_eq!(s, Spin::default(), "and the arrows come back");
        assert_eq!(spin(&mut s, 0.75, 36), 0, "three quarters again isn't a circle");
    }

    #[test]
    fn the_first_arrow_waits_for_the_finger_to_settle() {
        let right = Vec2::new(30.0, 0.0);
        let mut s = Settle::new(right, 1.0);
        assert!(!s.due(1.0 + SETTLE / 2.0), "not yet");
        // Wavering a little, still pointing right: it goes on settling.
        s.moved(Vec2::angled(SETTLE_TURN * 0.8) * 30.0, 1.0 + SETTLE / 2.0);
        assert!(s.due(1.0 + SETTLE), "settled: the first arrow");
        assert!(!s.due(1.0 + SETTLE * 3.0), "once: repeats take over");
        // Held, turning within right doesn't stop it.
        s.moved(Vec2::angled(-0.6) * 30.0, 2.0);
        assert!(s.held);

        // Turning past the threshold starts it over, even still pointing right.
        let mut s = Settle::new(right, 0.0);
        s.moved(Vec2::angled(SETTLE_TURN * 1.2) * 30.0, 0.1);
        assert!(!s.due(SETTLE), "started over at 0.1");
        assert!(s.due(0.1 + SETTLE));
        // Pointing somewhere new starts it over, and that arrow waits too.
        s.moved(Vec2::new(0.0, 30.0), 0.5);
        assert_eq!(s.dir, Key::ArrowDown);
        assert!(!s.held && !s.due(0.5 + SETTLE / 2.0));
        assert!(s.due(0.5 + SETTLE));

        // Going round, the finger never settles: 10° a frame.
        let mut s = Settle::new(right, 0.0);
        for i in 1..=72 {
            let now = i as f64 * 0.016;
            s.moved(Vec2::angled(TAU * i as f32 / 36.0) * 30.0, now);
            assert!(!s.due(now), "frame {i}");
        }
    }

    #[test]
    fn a_flick_that_never_settled_is_a_nudge() {
        assert_eq!(nudge(false, 0.1, Some(Key::ArrowLeft)), Some(Key::ArrowLeft));
        assert_eq!(nudge(true, 0.1, Some(Key::ArrowLeft)), None, "it sent its arrows already");
        assert_eq!(nudge(false, 0.1, None), None, "back in the center");
        assert_eq!(nudge(false, PI, Some(Key::ArrowLeft)), None, "it turned: a spin, not a flick");
        // A spin that ended in the center and went out again still turned.
        let mut s = Spin::default();
        spin(&mut s, 1.0, 36);
        assert!(s.most >= TAU - 1e-3);
    }

    #[test]
    fn a_still_press_talks_only_after_the_hold() {
        assert!(!hold_turns_to_talk(0.0, true, DOUBLE_TAP), "a tap lifts before a hold talks");
        assert!(!hold_turns_to_talk(2.0, true, 2.0 + HOLD_TO_TALK - 0.01));
        assert!(hold_turns_to_talk(0.0, true, HOLD_TO_TALK));
        assert!(hold_turns_to_talk(2.0, true, 2.0 + HOLD_TO_TALK + 0.01));
        // A drag (past the dead zone) never talks, however long it's held.
        assert!(!hold_turns_to_talk(1.0, false, 5.0));
    }

    #[test]
    fn lifting_tells_taps_double_taps_drags_and_talk_apart() {
        assert_eq!(lift(false, false, None, 1.0), Lift::Tap);
        assert_eq!(lift(false, false, Some(1.0), 1.0 + DOUBLE_TAP / 2.0), Lift::DoubleTap);
        assert_eq!(lift(false, false, Some(1.0), 1.0 + DOUBLE_TAP), Lift::Tap);
        assert_eq!(lift(true, false, Some(1.0), 1.1), Lift::Stick);
        // A release while talking is never a tap, even right after one.
        assert_eq!(lift(false, true, Some(1.0), 1.1), Lift::Talk);
        assert_eq!(lift(false, true, None, 9.0), Lift::Talk);
    }

    /// Run `talk` frame by frame (`frame` apart) from `from` to `to`; the times spaces went.
    fn run(talk: &mut Option<Talk>, from: f64, to: f64, frame: f64, ready: bool) -> Vec<f64> {
        let mut sent = Vec::new();
        let mut now = from;
        while now < to {
            let Some(t) = talk.as_mut() else { break };
            let (send, wake) = t.tick(now, ready);
            if send {
                sent.push(now);
            }
            if wake.is_none() {
                *talk = None;
            }
            now += frame;
        }
        sent
    }

    #[test]
    fn spaces_wait_for_the_share_then_repeat() {
        let mut talk = Some(Talk::new(0.0));
        assert!(run(&mut talk, 0.0, 1.0, 0.016, false).is_empty());
        let sent = run(&mut talk, 1.0, 2.0, 0.016, true);
        assert_eq!(sent[0], 1.0, "the first space goes as soon as it's ready");
        for w in sent.windows(2) {
            assert!(w[1] - w[0] >= SPACE_REPEAT - 1e-9, "{w:?}");
            assert!(w[1] - w[0] < SPACE_REPEAT + 0.017, "{w:?}");
        }
    }

    #[test]
    fn slow_frames_send_one_space_each_not_a_burst() {
        let mut talk = Some(Talk::new(0.0));
        let sent = run(&mut talk, 0.0, 1.0, 0.25, true);
        assert_eq!(sent, [0.0, 0.25, 0.5, 0.75]);
    }

    #[test]
    fn spaces_trail_on_after_letting_go_then_stop() {
        let mut t = Talk::new(0.0);
        t.release(1.0);
        // Letting go twice doesn't stretch the tail.
        t.release(1.2);
        let mut talk = Some(t);
        let sent = run(&mut talk, 1.0, 3.0, 0.01, true);
        assert!(sent.last().is_some_and(|&at| at < 1.0 + TALK_TAIL && at > 1.0 + TALK_TAIL - 2.0 * SPACE_REPEAT), "{sent:?}");
        assert!(talk.is_none(), "talking ends with the tail");
        // The last wake is the tail's end, not past it.
        let mut t = Talk::new(0.0);
        t.release(0.0);
        assert_eq!(t.tick(TALK_TAIL - 0.01, false).1, Some(TALK_TAIL - 0.01 + SPACE_REPEAT).map(|w: f64| w.min(TALK_TAIL)));
    }

    #[test]
    fn pressing_again_in_the_tail_goes_on_talking() {
        let mut t = Talk::new(0.0);
        t.release(1.0);
        // What a new press does.
        t.until = None;
        let mut talk = Some(t);
        assert!(!run(&mut talk, 1.5, 2.0, 0.016, true).is_empty());
        assert!(talk.is_some());
    }
}
