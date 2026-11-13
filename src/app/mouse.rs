//! Mouse input over terminals: selection, mouse reporting, wheel scrolling and links.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::TermMode;
use egui::{
    CursorIcon, Event, PointerButton, Pos2, Rect,
};

use crate::input::{self, MouseAction, MouseButton};
use crate::layout::PaneId;
use crate::render::{self, CellMetrics};
use crate::search::{self};

use super::App;
use super::actions::Action;

#[derive(Default)]
pub(super) struct MouseState {
    /// Pane receiving drag/release events, with its terminal rect.
    capture: Option<(PaneId, Rect)>,
    /// Button held while reporting to the program, or `None` if we're selecting text.
    reporting: Option<MouseButton>,
    last_cell: (usize, usize),
    moved: bool,
    last_click: Option<(Instant, Pos2, u8)>,
    click_count: u8,
}

impl App {
    pub(super) fn handle_mouse(&mut self, ctx: &egui::Context) {
        let (events, hover) = ctx.input(|i| (i.events.clone(), i.pointer.hover_pos()));
        // Each pane has its own zoom level, so cell geometry is looked up per pane.
        let pane_m: HashMap<PaneId, CellMetrics> = self.term_rects.iter().map(|&(id, _)| (id, self.pane_metrics(id))).collect();
        let fallback = self.pane_metrics(0);
        let m = |id: PaneId| pane_m.get(&id).copied().unwrap_or(fallback);

        for event in events {
            match event {
                Event::PointerButton { pos, button, pressed: true, modifiers } => {
                    let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) else {
                        continue;
                    };
                    if modifiers.ctrl && button == PointerButton::Primary {
                        if let Some((link_pane, link)) = &self.hover_link {
                            if *link_pane == id {
                                search::open(&link.uri);
                                continue;
                            }
                        }
                    }
                    self.pending.push(Action::FocusPane(id));
                    let Some(t) = self.panes.get_mut(&id) else { continue };
                    let mode = *t.term.lock().mode();
                    let (point, side, col, line) = render::point_at(pos, rect, m(id), t);
                    self.mouse.capture = Some((id, rect));
                    self.mouse.last_cell = (col, line);
                    self.mouse.moved = false;

                    let btn = match button {
                        PointerButton::Primary => MouseButton::Left,
                        PointerButton::Middle => MouseButton::Middle,
                        PointerButton::Secondary => MouseButton::Right,
                        _ => continue,
                    };
                    if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
                        self.mouse.reporting = Some(btn);
                        if let Some(b) = input::encode_mouse(MouseAction::Press, btn, col, line, modifiers, mode) {
                            t.write(b);
                        }
                        continue;
                    }
                    self.mouse.reporting = None;
                    if btn != MouseButton::Left {
                        self.mouse.capture = None;
                        continue;
                    }
                    // Multi-click detection: double = word, triple = line.
                    let now = Instant::now();
                    let count = match self.mouse.last_click {
                        Some((at, p, n)) if now - at < Duration::from_millis(400) && p.distance(pos) < 6.0 => n % 3 + 1,
                        _ => 1,
                    };
                    self.mouse.last_click = Some((now, pos, count));
                    self.mouse.click_count = count;
                    let ty = match count {
                        1 => SelectionType::Simple,
                        2 => SelectionType::Semantic,
                        _ => SelectionType::Lines,
                    };
                    let mut term = t.term.lock();
                    term.selection = Some(Selection::new(ty, point, side));
                    if modifiers.alt && count == 1 {
                        term.selection = Some(Selection::new(SelectionType::Block, point, side));
                    }
                }
                Event::PointerMoved(pos) => {
                    if let Some((id, rect)) = self.mouse.capture {
                        let Some(t) = self.panes.get_mut(&id) else { continue };
                        let (point, side, col, line) = render::point_at(pos, rect, m(id), t);
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
                        } else {
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
                    } else if let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) {
                        // Plain motion reporting (no button held), e.g. for hover effects in TUIs.
                        let Some(t) = self.panes.get_mut(&id) else { continue };
                        let mode = *t.term.lock().mode();
                        if mode.contains(TermMode::MOUSE_MOTION) {
                            let (_, _, col, line) = render::point_at(pos, rect, m(id), t);
                            if (col, line) != self.mouse.last_cell {
                                self.mouse.last_cell = (col, line);
                                let mods = ctx.input(|i| i.modifiers);
                                if let Some(b) = input::encode_mouse(MouseAction::Motion(None), MouseButton::Left, col, line, mods, mode) {
                                    t.write(b);
                                }
                            }
                        }
                    }
                }
                Event::PointerButton { pos, button, pressed: false, modifiers } => {
                    let Some((id, rect)) = self.mouse.capture.take() else { continue };
                    let Some(t) = self.panes.get_mut(&id) else { continue };
                    if let Some(btn) = self.mouse.reporting.take() {
                        let mode = *t.term.lock().mode();
                        let (_, _, col, line) = render::point_at(pos, rect, m(id), t);
                        if let Some(b) = input::encode_mouse(MouseAction::Release, btn, col, line, modifiers, mode) {
                            t.write(b);
                        }
                    } else if button == PointerButton::Primary {
                        if self.mouse.click_count == 1 && !self.mouse.moved {
                            t.term.lock().selection = None;
                        } else if self.cfg.copy_on_select {
                            if let Some(text) = t.selection_text() {
                                ctx.copy_text(text);
                            }
                        }
                    }
                }
                Event::MouseWheel { unit, delta, modifiers } => {
                    if modifiers.ctrl {
                        continue;
                    }
                    let Some(pos) = hover else { continue };
                    let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) else {
                        continue;
                    };
                    let Some(t) = self.panes.get_mut(&id) else { continue };
                    let lines_f = match unit {
                        egui::MouseWheelUnit::Point => delta.y / m(id).height,
                        egui::MouseWheelUnit::Line => delta.y * 3.0,
                        egui::MouseWheelUnit::Page => delta.y * t.size().lines as f32,
                    };
                    t.scroll_accum += lines_f;
                    let lines = t.scroll_accum.trunc() as i32;
                    t.scroll_accum -= lines as f32;
                    if lines == 0 {
                        continue;
                    }
                    let mode = *t.term.lock().mode();
                    if mode.intersects(TermMode::MOUSE_MODE) && !modifiers.shift {
                        let (_, _, col, line) = render::point_at(pos, rect, m(id), t);
                        let btn = if lines > 0 { MouseButton::WheelUp } else { MouseButton::WheelDown };
                        for _ in 0..lines.unsigned_abs() {
                            if let Some(b) = input::encode_mouse(MouseAction::Press, btn, col, line, modifiers, mode) {
                                t.write(b);
                            }
                        }
                    } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) && !modifiers.shift {
                        // Programs on the alternate screen (less, man) get arrow keys instead.
                        let seq: &[u8] = match (lines > 0, mode.contains(TermMode::APP_CURSOR)) {
                            (true, true) => b"\x1bOA",
                            (true, false) => b"\x1b[A",
                            (false, true) => b"\x1bOB",
                            (false, false) => b"\x1b[B",
                        };
                        t.write(seq.repeat(lines.unsigned_abs() as usize));
                    } else {
                        t.scroll(lines);
                    }
                }
                _ => {}
            }
        }

        // Ctrl+hover: find a link under the pointer.
        let ctrl = ctx.input(|i| i.modifiers.ctrl);
        let previous = self.hover_link.as_ref().map(|(id, l)| (*id, l.range.clone()));
        self.hover_link = None;
        if let (true, Some(pos), None) = (ctrl, hover, self.mouse.capture) {
            if let Some(&(id, rect)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) {
                if let Some(t) = self.panes.get(&id) {
                    let (point, ..) = render::point_at(pos, rect, m(id), t);
                    let term = t.term.lock();
                    self.hover_link = self.links.link_at(&term, point).map(|l| (id, l));
                }
            }
        }
        if self.hover_link.is_some() {
            ctx.set_cursor_icon(CursorIcon::PointingHand);
        }
        if previous != self.hover_link.as_ref().map(|(id, l)| (*id, l.range.clone())) {
            ctx.request_repaint();
        }

        // Pointer shape over terminals.
        if let (Some(pos), None) = (hover, &self.hover_link) {
            if let Some((id, _)) = self.term_rects.iter().find(|(_, r)| r.contains(pos)) {
                let mouse_mode = self
                    .panes
                    .get(id)
                    .is_some_and(|t| t.term.lock().mode().intersects(TermMode::MOUSE_MODE));
                if !mouse_mode {
                    ctx.set_cursor_icon(CursorIcon::Text);
                }
            }
        }
    }
}
