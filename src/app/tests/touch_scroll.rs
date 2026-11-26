//! A finger on a terminal: swipes scroll (and fling on), sideways swipes switch tabs, a long
//! press selects, a tap focuses, and with Ctrl
//! armed on the keyboard button a tap on a link opens it. Replayed through egui as egui-winit sends touches.

use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};
use egui::{Event, PointerButton, Pos2, RawInput, Rect, TouchDeviceId, TouchId, TouchPhase, Vec2};

use crate::app::App;
use crate::layout::{PaneId, PaneKind};

pub(super) struct Sim {
    pub(super) ctx: egui::Context,
    pub(super) app: App,
    pub(super) t: f64,
}

impl Sim {
    pub(super) fn new(tabs: usize) -> Self {
        let ctx = egui::Context::default();
        let app = super::new_app(&ctx, crate::config::Config::default());
        let mut s = Sim { ctx, app, t: 0.0 };
        s.settle();
        let c = s.ctx.clone();
        for _ in 1..tabs {
            s.app.new_tab(&c, None, None);
        }
        s.settle();
        s
    }
    pub(super) fn frame(&mut self, events: Vec<Event>) {
        self.t += 0.016;
        let input = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(self.t),
            ..Default::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| {
            let mut frame = eframe::Frame::_new_kittest();
            eframe::App::update(app, ctx, &mut frame);
        });
    }
    pub(super) fn settle(&mut self) {
        for _ in 0..40 {
            self.frame(vec![]);
        }
    }
    pub(super) fn touch(&mut self, id: u64, phase: TouchPhase, pos: Pos2) {
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
    /// Put a finger down at `from`, move it to `to` over `steps` frames and lift it.
    fn swipe(&mut self, from: Pos2, to: Pos2, steps: usize) {
        self.touch(0, TouchPhase::Start, from);
        for k in 1..=steps {
            self.touch(0, TouchPhase::Move, from + (to - from) * (k as f32 / steps as f32));
        }
        self.touch(0, TouchPhase::End, to);
    }
    /// A finger down and straight up at `p`.
    fn tap(&mut self, p: Pos2) {
        self.touch(0, TouchPhase::Start, p);
        self.touch(0, TouchPhase::End, p);
        self.frame(vec![]);
    }
    /// The shown terminal and its content rect.
    pub(super) fn terminal(&self) -> (PaneId, Rect) {
        *self.app.term_rects.first().expect("no terminal shown")
    }
    /// Fill terminal `id` with numbered lines, leaving plenty of scrollback.
    fn fill(&mut self, id: PaneId) {
        let text: String = (0..400).map(|n| format!("line {n}\r\n")).collect();
        let t = &self.app.panes.terminal(id).unwrap();
        let mut term = t.term.lock();
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut *term, text.as_bytes());
    }
    fn offset(&self, id: PaneId) -> usize {
        self.app.panes.terminal(id).unwrap().term.lock().grid().display_offset()
    }
    fn selected(&self, id: PaneId) -> bool {
        self.app.panes.terminal(id).unwrap().term.lock().selection.is_some()
    }
    /// Clear terminal `id` and write `text` from its top left.
    fn show(&mut self, id: PaneId, text: &str) {
        let t = &self.app.panes.terminal(id).unwrap();
        let mut term = t.term.lock();
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut *term, format!("\x1b[2J\x1b[H{text}").as_bytes());
    }
    /// Record the links nex opens instead of starting the browser.
    fn record_links(&mut self) {
        OPENED.with(|o| o.borrow_mut().clear());
        self.app.input.open_link = |uri| OPENED.with(|o| o.borrow_mut().push(uri.to_string_lossy().into_owned()));
    }
    fn opened(&self) -> Vec<String> {
        OPENED.with(|o| o.borrow().clone())
    }
}

thread_local! {
    static OPENED: std::cell::RefCell<Vec<String>> = const { std::cell::RefCell::new(Vec::new()) };
}

#[test]
fn swipe_down_scrolls_back_without_selecting() {
    let mut s = Sim::new(1);
    let (id, r) = s.terminal();
    s.fill(id);
    let from = Pos2::new(r.center().x, r.top() + 40.0);
    s.swipe(from, from + Vec2::new(4.0, 300.0), 20);
    assert!(s.offset(id) > 0, "the scrollback didn't move");
    assert!(!s.selected(id), "a swipe must not select");

    // And swiping back up returns toward the bottom.
    let up = s.offset(id);
    let from = Pos2::new(r.center().x, r.bottom() - 40.0);
    s.swipe(from, from - Vec2::new(0.0, 200.0), 20);
    assert!(s.offset(id) < up);
}

#[test]
fn quick_swipe_flings_on_after_lifting() {
    let mut s = Sim::new(1);
    let (id, r) = s.terminal();
    s.fill(id);
    let from = Pos2::new(r.center().x, r.top() + 40.0);
    s.swipe(from, from + Vec2::new(0.0, 200.0), 4);
    let lifted = s.offset(id);
    s.settle();
    assert!(s.offset(id) > lifted, "no fling: {} -> {}", lifted, s.offset(id));
}

#[test]
fn long_press_then_drag_selects() {
    let mut s = Sim::new(1);
    let (id, r) = s.terminal();
    s.fill(id);
    let p = Pos2::new(r.left() + 20.0, r.top() + 40.0);
    s.touch(0, TouchPhase::Start, p);
    for _ in 0..35 {
        s.frame(vec![]);
    }
    assert!(s.selected(id), "holding didn't start a selection");
    s.touch(0, TouchPhase::Move, p + Vec2::new(0.0, 60.0));
    s.touch(0, TouchPhase::End, p + Vec2::new(0.0, 60.0));
    s.frame(vec![]);
    assert!(s.selected(id), "the selection didn't survive the lift");
    assert_eq!(s.offset(id), 0, "dragging a selection must not scroll");
}

#[test]
fn tap_focuses_and_selects_nothing() {
    let mut s = Sim::new(1);
    let (id, r) = s.terminal();
    s.fill(id);
    s.touch(0, TouchPhase::Start, r.center());
    s.touch(0, TouchPhase::End, r.center());
    s.frame(vec![]);
    assert!(!s.selected(id));
    assert_eq!(s.offset(id), 0);
    assert_eq!(s.app.focused_pane(), Some(id));
}

#[test]
fn tap_on_a_link_with_ctrl_armed_opens_it() {
    let mut s = Sim::new(1);
    s.record_links();
    let (id, r) = s.terminal();
    // The same link on every line, so a shell prompt landing on one doesn't matter.
    let lines: String = (0..12).map(|_| "https://example.com/some/long/path/to/open\r\n").collect();
    s.show(id, &lines);
    let p = Pos2::new(r.left() + 40.0, r.top() + r.height() * 0.2);

    // Without Ctrl a tap only focuses, as a click does on desktop.
    s.tap(p);
    assert!(s.opened().is_empty(), "opened {:?} without Ctrl", s.opened());

    // Ctrl armed (the keyboard button's double-tap): the tap opens the link and uses it up.
    s.app.chrome.keyboard_button.ctrl_armed = true;
    s.tap(p);
    assert_eq!(s.opened(), ["https://example.com/some/long/path/to/open"]);
    assert!(!s.app.chrome.keyboard_button.ctrl_armed, "opening a link should use up Ctrl");
    assert_eq!(s.app.focused_pane(), Some(id));
}

#[test]
fn ctrl_tap_beside_a_link_or_swipe_over_one_opens_nothing() {
    let mut s = Sim::new(1);
    s.record_links();
    let (id, r) = s.terminal();
    let lines: String = (0..12).map(|_| "https://example.com/x\r\n").collect();
    s.show(id, &lines);
    s.app.chrome.keyboard_button.ctrl_armed = true;
    // Far right of the line, past the link's end: Ctrl stays armed for a key.
    s.tap(Pos2::new(r.right() - 20.0, r.top() + r.height() * 0.2));
    assert!(s.app.chrome.keyboard_button.ctrl_armed);
    // A scroll that starts on the link.
    let on = Pos2::new(r.left() + 30.0, r.top() + r.height() * 0.2);
    s.swipe(on, on + Vec2::new(0.0, 200.0), 10);
    s.frame(vec![]);
    assert!(s.opened().is_empty(), "opened {:?}", s.opened());
}

#[test]
fn sideways_swipes_switch_tabs_without_wrapping() {
    let mut s = Sim::new(3);
    s.app.show_tab(0);
    s.settle();
    let r = s.terminal().1;
    let c = r.center();
    s.swipe(c, c - Vec2::new(200.0, 10.0), 10);
    s.settle();
    assert_eq!(s.app.active, 1, "a left swipe goes to the next tab");
    s.swipe(c, c + Vec2::new(200.0, 10.0), 10);
    s.settle();
    assert_eq!(s.app.active, 0, "a right swipe goes to the previous tab");
    s.swipe(c, c + Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(s.app.home_active, "swiping right on the first tab didn't reach the Agents tab");
    let c = s.other_pane_body().center();
    s.swipe(c, c + Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(s.app.home_active, "swiping right on the Agents tab wrapped around");
    assert_eq!(s.app.active, 0, "swiping right on the Agents tab wrapped to the last");
}

#[test]
fn swiping_right_on_the_first_tab_opens_agents_before_it_was_ever_opened() {
    let mut s = Sim::new(2);
    s.app.show_tab(0);
    s.settle();
    assert!(s.app.home_tab.is_none() && !s.app.panes.has(PaneKind::Agents));
    let c = s.terminal().1.center();
    s.swipe(c, c + Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(s.app.home_active, "a right swipe on the first tab didn't open the Agents tab");
    assert!(s.app.shows_agents_label());
    assert_eq!(s.app.panes.ids(PaneKind::Agents).count(), 1, "no Agents pane");
    let a = s.other_pane_body().center();
    s.swipe(a, a - Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(!s.app.home_active, "a left swipe on Agents didn't leave it");
    assert_eq!(s.app.active, 0);
    // Again: the same Agents tab, not a second one.
    s.swipe(c, c + Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(s.app.home_active);
    assert_eq!(s.app.panes.ids(PaneKind::Agents).count(), 1, "a second swipe opened another Agents pane");
}

#[test]
fn short_or_vertical_drags_keep_the_tab() {
    let mut s = Sim::new(3);
    s.app.show_tab(1);
    s.settle();
    let c = s.terminal().1.center();
    // Too short (and slow) to be a swipe.
    s.swipe(c, c - Vec2::new(25.0, 0.0), 20);
    s.settle();
    assert_eq!(s.app.active, 1);
    // Starts mostly vertical: a scroll, however far it drifts sideways after.
    s.touch(0, TouchPhase::Start, c);
    s.touch(0, TouchPhase::Move, c + Vec2::new(2.0, 30.0));
    for k in 1..=10 {
        s.touch(0, TouchPhase::Move, c + Vec2::new(-30.0 * k as f32, 30.0));
    }
    s.touch(0, TouchPhase::End, c + Vec2::new(-300.0, 30.0));
    s.settle();
    assert_eq!(s.app.active, 1, "a scroll that drifted sideways switched tabs");
}

#[test]
fn second_finger_turns_a_swipe_into_a_pinch() {
    let mut s = Sim::new(1);
    let (id, r) = s.terminal();
    s.fill(id);
    let p = Pos2::new(r.center().x, r.top() + 40.0);
    s.touch(0, TouchPhase::Start, p);
    s.touch(1, TouchPhase::Start, p + Vec2::new(100.0, 100.0));
    for k in 1..=10 {
        s.touch(0, TouchPhase::Move, p + Vec2::new(0.0, 20.0 * k as f32));
    }
    s.touch(1, TouchPhase::End, p + Vec2::new(100.0, 100.0));
    s.touch(0, TouchPhase::End, p + Vec2::new(0.0, 200.0));
    s.settle();
    assert_eq!(s.offset(id), 0, "a pinch scrolled the terminal");
    assert!(!s.selected(id));
}

/// A throwaway folder for the Files pane to show, removed when dropped. Files must never
/// show the working directory in these tests: a long press and drag there moves real files
/// (it once moved the repository's `build.rs` into `site/`).
struct TempDir(std::path::PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl Sim {
    /// Open the Files tab on a throwaway folder of a few files and a subfolder.
    fn open_files(&mut self) -> TempDir {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("nex-touch-{}-{n}", std::process::id()));
        std::fs::create_dir_all(dir.join("inner")).unwrap();
        for f in ["alpha.txt", "beta.txt", "gamma.txt"] {
            std::fs::write(dir.join(f), "x").unwrap();
        }
        self.app.toggle_explorer(&self.ctx);
        for id in self.app.panes.ids(PaneKind::Explorer).collect::<Vec<_>>() {
            self.app.panes.explorer_mut(id).unwrap().navigate(dir.clone());
        }
        self.settle();
        assert!(self.app.home_active, "the Files tab didn't open");
        TempDir(dir)
    }

    /// The body of the first pane shown that isn't a terminal.
    fn other_pane_body(&self) -> Rect {
        let r = self.app.card_rects.iter().find(|(id, _)| !self.app.panes.is_terminal(*id)).expect("no other pane shown").1;
        Rect::from_min_max(Pos2::new(r.left(), r.top() + 60.0), r.max)
    }
}

#[test]
fn sideways_swipes_on_files_reach_the_home_tab_and_back() {
    let mut s = Sim::new(2);
    let _dir = s.open_files();
    let c = s.other_pane_body().center();
    // Left from the home tab: the first terminal tab.
    s.swipe(c, c - Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(!s.app.home_active, "a left swipe on Files didn't leave it");
    assert_eq!(s.app.active, 0);
    // Right from the first tab: back to Files, which is the first of all (no wrapping past it).
    let c = s.terminal().1.center();
    s.swipe(c, c + Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(s.app.home_active, "a right swipe on the first tab didn't go to Files");
    assert!(s.app.home_is_files() && !s.app.panes.has(PaneKind::Agents), "a right swipe opened Agents over Files");
    let c = s.other_pane_body().center();
    s.swipe(c, c + Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert!(s.app.home_active, "swiping right on the home tab wrapped around");
}

#[test]
fn long_press_on_files_then_sideways_drag_keeps_the_tab() {
    let mut s = Sim::new(2);
    let _dir = s.open_files();
    let c = s.other_pane_body().center();
    s.touch(0, TouchPhase::Start, c);
    for _ in 0..35 {
        s.frame(vec![]);
    }
    for k in 1..=10 {
        s.touch(0, TouchPhase::Move, c - Vec2::new(20.0 * k as f32, 0.0));
    }
    s.touch(0, TouchPhase::End, c - Vec2::new(200.0, 0.0));
    s.settle();
    assert!(s.app.home_active, "a long press then drag switched tabs");
}

#[test]
fn sideways_drag_on_a_pane_header_doesnt_switch_tabs() {
    let mut s = Sim::new(3);
    s.app.show_tab(1);
    s.settle();
    let card = s.app.card_rects[0].1;
    let p = Pos2::new(card.center().x, card.top() + 10.0);
    s.swipe(p, p - Vec2::new(200.0, 0.0), 10);
    s.settle();
    assert_eq!(s.app.active, 1);
}
