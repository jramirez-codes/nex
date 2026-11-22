//! Mouse input over terminals: selection, mouse reporting, wheel scrolling and links. On
//! touch screens a finger is not a mouse: a swipe up or down scrolls (and flings on after
//! the finger lifts), a swipe sideways is left to `swipe.rs` (it switches tabs), a long press
//! selects a word (and a following drag extends it), and a tap focuses (and clicks in
//! programs that take the mouse).

use std::time::{Duration, Instant};

use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use egui::{CursorIcon, Event, Modifiers, MouseWheelUnit, PointerButton, Pos2, Rect, Vec2};

use crate::terminal::input::{self, MouseAction, MouseButton};
use crate::layout::PaneId;
use crate::render;

use crate::app::App;
use crate::app::actions::Action;
use crate::app::input::swipe::{LONG_PRESS, TOUCH_SLOP};
use crate::ui::glide;

/// Clicks closer together than this (in time and distance) count as a double/triple click.
const MULTI_CLICK_TIME: Duration = Duration::from_millis(400);
const MULTI_CLICK_DISTANCE: f32 = 6.0;

/// A fling starts above this speed and stops below [`FLING_STOP`] (points a second).
const FLING_START: f32 = 300.0;
const FLING_STOP: f32 = 5.0;
/// How quickly a fling slows down (the same as the title-bar tabs').
const FLING_DECAY: f32 = 4.0;

/// A finger down on a terminal.
struct TouchPress {
    id: PaneId,
    rect: Rect,
    origin: Pos2,
    /// Where the finger was at the last move.
    last: Pos2,
    at: f64,
    /// The finger's speed at the last move, for a fling or a flick.
    velocity: Vec2,
    gesture: TouchGesture,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum TouchGesture {
    /// Down and not yet moved: may become a tap, a long press, a scroll or a swipe.
    Pending,
    /// Moving up or down: scrolls.
    Scrolling,
    /// Moving sideways: the terminal leaves it alone (it switches tabs, see `swipe.rs`).
    Swiping,
    /// Held: selecting (the selection is then driven as a mouse drag, through `capture`).
    Selecting,
}

/// Scrolling on after a swipe, slowing down.
struct Fling {
    id: PaneId,
    rect: Rect,
    pos: Pos2,
    /// Points a second; positive shows older lines.
    velocity: f32,
}

#[derive(Default)]
pub(in crate::app) struct MouseState {
    /// Pane receiving drag/release events, with its terminal rect.
    capture: Option<(PaneId, Rect)>,
    /// Button held while reporting to the program, or `None` if we're selecting text.
    reporting: Option<MouseButton>,
    last_cell: (usize, usize),
    moved: bool,
    last_click: Option<(Instant, Pos2, u8)>,
    click_count: u8,
    /// A two-finger pinch is (or was, until every finger lifts) in progress: the finger that
    /// arrived as a pointer press must not select or click.
    pinching: bool,
    /// A finger down on a terminal (touch screens).
    touch: Option<TouchPress>,
    /// Scrolling on after a swipe.
    fling: Option<Fling>,
}

impl MouseState {
    /// A two-finger pinch is in progress.
    pub(in crate::app) fn pinching(&self) -> bool {
        self.pinching
    }
}

impl App {
    pub(in crate::app) fn handle_mouse(&mut self, ctx: &egui::Context) {
        let (events, hover) = ctx.input(|i| (i.events.clone(), i.pointer.hover_pos()));
        if self.track_pinch(ctx) {
            return;
        }
        // A pane is being dragged by its header: the terminals under it don't see the pointer.
        if self.pane_drag.is_some() {
            self.mouse.touch = None;
            self.mouse.fling = None;
            return;
        }
        // A dialog is on top (the delete confirmation): it gets the pointer, not the terminals.
        if ctx.memory(|m| m.top_modal_layer()).is_some() {
            self.cancel_press();
            return;
        }
        self.tick_touch(ctx);
        for event in events {
            match event {
                Event::PointerButton { pos, button, pressed: true, modifiers } => self.on_press(ctx, pos, button, modifiers),
                Event::PointerMoved(pos) => self.on_move(ctx, pos),
                Event::PointerButton { pos, button, pressed: false, modifiers } => self.on_release(ctx, pos, button, modifiers),
                Event::MouseWheel { unit, delta, modifiers } => {
                    if let (false, Some(pos)) = (modifiers.command, hover) {
                        self.on_wheel(pos, unit, delta, modifiers);
                    }
                }
                _ => {}
            }
        }
        self.update_hover_link(ctx, hover);
        self.update_pointer_cursor(ctx, hover);
    }

    /// Start or end ignoring the pointer for a pinch. Returns true while it is ignored.
    fn track_pinch(&mut self, ctx: &egui::Context) -> bool {
        let (multi, touching) = ctx.input(|i| (i.multi_touch().is_some(), i.any_touches()));
        if multi && !self.mouse.pinching {
            self.mouse.pinching = true;
            self.cancel_press();
            self.pending.push(Action::CancelPaneDrag);
        } else if !touching {
            self.mouse.pinching = false;
        }
        self.mouse.pinching
    }

    /// Forget the current press: clear the selection it started, or, if it was reported to
    /// the program, report its release now (the real one will be ignored).
    fn cancel_press(&mut self) {
        self.mouse.touch = None;
        self.mouse.fling = None;
        let Some((id, _)) = self.mouse.capture.take() else { return };
        let Some(t) = self.panes.get_mut(&id) else { return };
        if let Some(btn) = self.mouse.reporting.take() {
            let mode = *t.term.lock().mode();
            let (col, line) = self.mouse.last_cell;
            if let Some(b) = input::encode_mouse(MouseAction::Release, btn, col, line, Modifiers::NONE, mode) {
                t.write(b);
            }
        } else {
            t.term.lock().selection = None;
        }
    }

    /// The terminal under `pos`, with its content rect.
    pub(in crate::app) fn terminal_at(&self, pos: Pos2) -> Option<(PaneId, Rect)> {
        self.term_rects.iter().find(|(_, r)| r.contains(pos)).copied()
    }

    /// A button press: open a Ctrl+clicked (Cmd+clicked on macOS) link, report it to the program, or start a selection.
    fn on_press(&mut self, ctx: &egui::Context, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
        // Any press stops a fling, as a finger on a scrolling list does.
        self.mouse.fling = None;
        // The keyboard button floats over the terminal: its taps are its own.
        if self.keyboard_button.rect.is_some_and(|r| r.contains(pos)) {
            return;
        }
        let Some((id, rect)) = self.terminal_at(pos) else { return };
        // A finger decides what it is doing once it moves, is held, or lifts.
        if button == PointerButton::Primary && ctx.input(|i| i.any_touches()) {
            self.pending.push(Action::FocusPane(id));
            let at = ctx.input(|i| i.time);
            self.mouse.touch = Some(TouchPress { id, rect, origin: pos, last: pos, at, velocity: Vec2::ZERO, gesture: TouchGesture::Pending });
            return;
        }
        if modifiers.command && button == PointerButton::Primary
            && let Some((link_pane, link)) = &self.hover_link
                && *link_pane == id {
                    crate::platform::open_uri(link.uri.as_ref());
                    return;
                }
        self.pending.push(Action::FocusPane(id));
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get_mut(&id) else { return };
        let mode = *t.term.lock().mode();
        let (point, side, col, line) = render::point_at(pos, rect, m, t);
        self.mouse.capture = Some((id, rect));
        self.mouse.last_cell = (col, line);
        self.mouse.moved = false;

        let btn = match button {
            PointerButton::Primary => MouseButton::Left,
            PointerButton::Middle => MouseButton::Middle,
            PointerButton::Secondary => MouseButton::Right,
            _ => return,
        };
        if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
            self.mouse.reporting = Some(btn);
            if let Some(b) = input::encode_mouse(MouseAction::Press, btn, col, line, modifiers, mode) {
                t.write(b);
            }
            return;
        }
        self.mouse.reporting = None;
        if btn != MouseButton::Left {
            self.mouse.capture = None;
            return;
        }

        // Multi-click detection: double = word, triple = line.
        let now = Instant::now();
        let count = match self.mouse.last_click {
            Some((at, p, n)) if now - at < MULTI_CLICK_TIME && p.distance(pos) < MULTI_CLICK_DISTANCE => n % 3 + 1,
            _ => 1,
        };
        self.mouse.last_click = Some((now, pos, count));
        self.mouse.click_count = count;
        let ty = match count {
            1 if modifiers.alt => SelectionType::Block,
            1 => SelectionType::Simple,
            2 => SelectionType::Semantic,
            _ => SelectionType::Lines,
        };
        t.term.lock().selection = Some(Selection::new(ty, point, side));
    }

    /// Pointer motion: extend the selection or report a drag while a button is held, or report
    /// plain motion to programs that asked for it.
    fn on_move(&mut self, ctx: &egui::Context, pos: Pos2) {
        if self.mouse.touch.as_ref().is_some_and(|p| p.gesture != TouchGesture::Selecting) {
            self.on_touch_move(ctx, pos);
            return;
        }
        if let Some((id, rect)) = self.mouse.capture {
            self.on_drag(ctx, id, rect, pos);
            return;
        }
        // Plain motion reporting (no button held), e.g. for hover effects in TUIs.
        let Some((id, rect)) = self.terminal_at(pos) else { return };
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get_mut(&id) else { return };
        let mode = *t.term.lock().mode();
        if !mode.contains(TermMode::MOUSE_MOTION) {
            return;
        }
        let (_, _, col, line) = render::point_at(pos, rect, m, t);
        if (col, line) != self.mouse.last_cell {
            self.mouse.last_cell = (col, line);
            let mods = ctx.input(|i| i.modifiers);
            if let Some(b) = input::encode_mouse(MouseAction::Motion(None), MouseButton::Left, col, line, mods, mode) {
                t.write(b);
            }
        }
    }

    /// Motion with a button held in pane `id`.
    fn on_drag(&mut self, ctx: &egui::Context, id: PaneId, rect: Rect, pos: Pos2) {
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get_mut(&id) else { return };
        let (point, side, col, line) = render::point_at(pos, rect, m, t);
        let cell_changed = (col, line) != self.mouse.last_cell;
        self.mouse.last_cell = (col, line);

        if let Some(btn) = self.mouse.reporting {
            let mode = *t.term.lock().mode();
            if cell_changed && mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION) {
                let mods = ctx.input(|i| i.modifiers);
                if let Some(b) = input::encode_mouse(MouseAction::Motion(Some(btn)), btn, col, line, mods, mode) {
                    t.write(b);
                }
            }
            return;
        }

        if cell_changed {
            self.mouse.moved = true;
        }
        // Auto-scroll when dragging past the top/bottom edge.
        if pos.y < rect.top() {
            t.scroll(1);
        } else if pos.y > rect.bottom() {
            t.scroll(-1);
        }
        if let Some(sel) = t.term.lock().selection.as_mut() {
            sel.update(point, side);
        }
    }

    /// A button release: finish reporting, or finish (and maybe copy) the selection.
    fn on_release(&mut self, ctx: &egui::Context, pos: Pos2, button: PointerButton, modifiers: Modifiers) {
        if let Some(press) = self.mouse.touch.take()
            && press.gesture != TouchGesture::Selecting {
                self.on_touch_release(ctx, press, pos);
                return;
            }
        let Some((id, rect)) = self.mouse.capture.take() else { return };
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get_mut(&id) else { return };
        if let Some(btn) = self.mouse.reporting.take() {
            let mode = *t.term.lock().mode();
            let (_, _, col, line) = render::point_at(pos, rect, m, t);
            if let Some(b) = input::encode_mouse(MouseAction::Release, btn, col, line, modifiers, mode) {
                t.write(b);
            }
        } else if button == PointerButton::Primary {
            if self.mouse.click_count == 1 && !self.mouse.moved {
                // A plain click (no drag) clears the selection.
                t.term.lock().selection = None;
            } else if self.cfg.copy_on_select
                && let Some(text) = t.selection_text() {
                    self.clipboard_toast(ctx, "Copied", &text);
                    ctx.copy_text(text);
                }
        }
    }

    /// Wheel scrolling: reported to mouse-aware programs, turned into arrow keys on the
    /// alternate screen (less, man), otherwise scrolls the scrollback.
    fn on_wheel(&mut self, pos: Pos2, unit: MouseWheelUnit, delta: Vec2, modifiers: Modifiers) {
        let Some((id, rect)) = self.terminal_at(pos) else { return };
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get(&id) else { return };
        let lines_f = match unit {
            MouseWheelUnit::Point => delta.y / m.height,
            MouseWheelUnit::Line => delta.y * 3.0,
            MouseWheelUnit::Page => delta.y * t.size().lines as f32,
        };
        self.scroll_pane(id, rect, pos, lines_f, modifiers);
    }

    /// Scroll pane `id` by `lines_f` lines (positive shows older lines), as the wheel at
    /// `pos` would: reported to mouse-aware programs, turned into arrow keys on the
    /// alternate screen, otherwise through the scrollback. Returns false if the scrollback
    /// was already at its end (or the pane is gone).
    fn scroll_pane(&mut self, id: PaneId, rect: Rect, pos: Pos2, lines_f: f32, modifiers: Modifiers) -> bool {
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get_mut(&id) else { return false };
        // Keep the fractional part so slow trackpad scrolls still add up to lines.
        t.scroll_accum += lines_f;
        let lines = t.scroll_accum.trunc() as i32;
        t.scroll_accum -= lines as f32;
        if lines == 0 {
            return true;
        }

        let mode = *t.term.lock().mode();
        if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
            let (_, _, col, line) = render::point_at(pos, rect, m, t);
            let btn = if lines > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
            for _ in 0..lines.unsigned_abs() {
                if let Some(b) = input::encode_mouse(MouseAction::Press, btn, col, line, modifiers, mode) {
                    t.write(b);
                }
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !modifiers.shift {
            let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                (true, true) => b"\x1bOA",
                (true, false) => b"\x1b[A",
                (false, true) => b"\x1bOB",
                (false, false) => b"\x1b[B",
            };
            t.write(seq.repeat(lines.unsigned_abs() as usize));
        } else {
            let before = t.term.lock().grid().display_offset();
            t.scroll(lines);
            return t.term.lock().grid().display_offset() != before;
        }
        true
    }

    /// Each frame: turn a held finger into a selection, and move a fling on.
    fn tick_touch(&mut self, ctx: &egui::Context) {
        let (now, dt) = ctx.input(|i| (i.time, i.stable_dt.min(0.1)));
        if let Some(press) = self.mouse.touch.as_mut()
            && press.gesture == TouchGesture::Pending {
                let left = LONG_PRESS - (now - press.at);
                if left > 0.0 {
                    ctx.request_repaint_after(Duration::from_secs_f64(left));
                } else {
                    press.gesture = TouchGesture::Selecting;
                    let (id, rect, origin) = (press.id, press.rect, press.origin);
                    self.start_touch_selection(id, rect, origin);
                }
            }
        if let Some(f) = self.mouse.fling.as_mut() {
            let (id, rect, pos, dy) = (f.id, f.rect, f.pos, f.velocity * dt);
            f.velocity *= (-dt * FLING_DECAY).exp();
            let going = f.velocity.abs() > FLING_STOP;
            let lines_f = dy / self.pane_metrics(id).height;
            if going && self.scroll_pane(id, rect, pos, lines_f, Modifiers::NONE) {
                ctx.request_repaint();
            } else {
                self.mouse.fling = None;
            }
        }
    }

    /// A long press: select the word under the finger; moving on extends the selection, as
    /// a mouse drag does.
    fn start_touch_selection(&mut self, id: PaneId, rect: Rect, pos: Pos2) {
        let m = self.pane_metrics(id);
        let Some(t) = self.panes.get_mut(&id) else { return };
        let (point, side, col, line) = render::point_at(pos, rect, m, t);
        t.term.lock().selection = Some(Selection::new(SelectionType::Semantic, point, side));
        self.mouse.capture = Some((id, rect));
        self.mouse.reporting = None;
        self.mouse.last_cell = (col, line);
        self.mouse.moved = false;
        // Not a plain click: the release keeps (and maybe copies) the word.
        self.mouse.click_count = 2;
    }

    /// A finger moving: decide whether it scrolls or swipes, then keep doing that.
    fn on_touch_move(&mut self, ctx: &egui::Context, pos: Pos2) {
        let velocity = ctx.input(|i| i.pointer.velocity());
        let Some(press) = self.mouse.touch.as_mut() else { return };
        if press.gesture == TouchGesture::Pending {
            let d = pos - press.origin;
            if d.length() <= TOUCH_SLOP {
                return;
            }
            // The first clear direction decides, so a scroll that drifts sideways stays a scroll.
            press.gesture = if d.y.abs() >= d.x.abs() { TouchGesture::Scrolling } else { TouchGesture::Swiping };
        }
        press.velocity = velocity;
        let dy = pos.y - press.last.y;
        press.last = pos;
        if press.gesture == TouchGesture::Scrolling {
            let (id, rect) = (press.id, press.rect);
            let lines_f = dy / self.pane_metrics(id).height;
            self.scroll_pane(id, rect, pos, lines_f, Modifiers::NONE);
        }
    }

    /// A finger lifting: a tap, or the end of a scroll (which may fling on).
    fn on_touch_release(&mut self, ctx: &egui::Context, press: TouchPress, pos: Pos2) {
        match press.gesture {
            TouchGesture::Pending => {
                let m = self.pane_metrics(press.id);
                let Some(t) = self.panes.get_mut(&press.id) else { return };
                let mode = *t.term.lock().mode();
                if mode.intersects(TermMode::MOUSE_MODE) {
                    let (_, _, col, line) = render::point_at(press.origin, press.rect, m, t);
                    for action in [MouseAction::Press, MouseAction::Release] {
                        if let Some(b) = input::encode_mouse(action, MouseButton::Left, col, line, Modifiers::NONE, mode) {
                            t.write(b);
                        }
                    }
                } else {
                    t.term.lock().selection = None;
                }
            }
            TouchGesture::Scrolling => {
                let v = press.velocity.y;
                if glide::animations(ctx) && v.abs() > FLING_START {
                    self.mouse.fling = Some(Fling { id: press.id, rect: press.rect, pos, velocity: v });
                    ctx.request_repaint();
                }
            }
            TouchGesture::Swiping | TouchGesture::Selecting => {}
        }
    }

    /// Ctrl+hover (Cmd on macOS): find the link under the pointer (drawn underlined, opened
    /// by Ctrl/Cmd+click).
    fn update_hover_link(&mut self, ctx: &egui::Context, hover: Option<Pos2>) {
        let ctrl = ctx.input(|i| i.modifiers.command);
        let previous = self.hover_link.as_ref().map(|(id, l)| (*id, l.range.clone()));
        self.hover_link = None;
        if let (true, Some(pos), None) = (ctrl, hover, self.mouse.capture)
            && let Some((id, rect)) = self.terminal_at(pos) {
                let m = self.pane_metrics(id);
                if let Some(t) = self.panes.get(&id) {
                    let (point, ..) = render::point_at(pos, rect, m, t);
                    let term = t.term.lock();
                    self.hover_link = self.links.link_at(&term, point).map(|l| (id, l));
                }
            }
        if self.hover_link.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if previous != self.hover_link.as_ref().map(|(id, l)| (*id, l.range.clone())) {
            ctx.request_repaint();
        }
    }

    /// A text cursor over terminals, unless the program handles the mouse itself.
    fn update_pointer_cursor(&self, ctx: &egui::Context, hover: Option<Pos2>) {
        if self.hover_link.is_some() {
            return;
        }
        let Some((id, _)) = hover.and_then(|pos| self.terminal_at(pos)) else { return };
        let mouse_mode = self.panes.get(&id).is_some_and(|t| t.term.lock().mode().intersects(TermMode::MOUSE_MODE));
        if !mouse_mode {
            ctx.set_cursor_icon(CursorIcon::Text);
        }
    }
}
