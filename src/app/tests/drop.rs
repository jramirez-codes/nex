//! Touch drag from the explorer onto a terminal, replayed through egui as egui-winit sends it.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, TouchDeviceId, TouchId, TouchPhase, Vec2};

use std::path::{Path, PathBuf};

use crate::app::App;
use crate::layout::{Axis, PaneId};

struct Sim {
    ctx: egui::Context,
    app: App,
    t: f64,
    /// Where the (hidden) mouse cursor sits: sway moves the pointer back there when a finger
    /// lifts, in the same frame as the release.
    mouse: Option<Pos2>,
}

impl Sim {
    fn frame(&mut self, events: Vec<Event>) {
        self.frame_with(RawInput { events, ..Default::default() });
    }
    fn frame_with(&mut self, mut input: RawInput) {
        self.t += 0.016;
        input.screen_rect = Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0)));
        input.time = Some(self.t);
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| {
            let mut frame = eframe::Frame::_new_kittest();
            eframe::App::update(app, ctx, &mut frame);
        });
    }
    fn touch(&mut self, phase: TouchPhase, pos: Pos2) {
        let mut ev = vec![Event::Touch { device_id: TouchDeviceId(1), id: TouchId(0), phase, pos, force: None }];
        match phase {
            TouchPhase::Start => {
                ev.push(Event::PointerMoved(pos));
                ev.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
            }
            TouchPhase::Move => ev.push(Event::PointerMoved(pos)),
            _ => {
                ev.push(Event::PointerButton { pos, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
                ev.push(Event::PointerGone);
                if let Some(m) = self.mouse {
                    ev.push(Event::PointerMoved(m));
                }
            }
        }
        self.frame(ev);
    }
}

/// A temporary folder with two files, removed when dropped.
struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

impl std::ops::Deref for TempDir {
    type Target = Path;
    fn deref(&self) -> &Path {
        &self.0
    }
}

fn setup() -> (Sim, TempDir) {
    static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
    let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!("nex-drop-{}-{n}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for f in ["alpha.txt", "beta.txt"] {
        std::fs::write(dir.join(f), "x").unwrap();
    }
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let app = App::new(&cc, crate::config::Config::default(), Default::default(), None);
    let mut s = Sim { ctx, app, t: 0.0, mouse: None };
    s.frame(vec![]);
    (s, TempDir(dir))
}

impl Sim {
    fn settle(&mut self) {
        for _ in 0..40 {
            self.frame(vec![]);
        }
    }
    fn card(&self, id: PaneId) -> Rect {
        self.app.card_rects.iter().find(|(p, _)| *p == id).expect("pane not shown").1
    }
    fn terminals(&self) -> Vec<PaneId> {
        self.app.card_rects.iter().map(|(id, _)| *id).filter(|id| self.app.panes.contains_key(id)).collect()
    }
    fn explorer(&self) -> PaneId {
        *self.app.card_rects.iter().map(|(id, _)| id).find(|id| self.app.explorers.contains_key(id)).unwrap()
    }
    /// Long-press a file in explorer `ex`, drag it onto `target` and lift; was it dropped there?
    fn touch_drop(&mut self, ex: PaneId, target: PaneId, dir: &Path) -> bool {
        self.app.explorers.get_mut(&ex).unwrap().navigate(dir.to_path_buf());
        self.settle();
        let r = self.card(ex);
        let mut start = None;
        'find: for x in [60.0, 120.0] {
            for y in (r.top() as i32 + 40..r.bottom() as i32).step_by(10) {
                let p = Pos2::new(r.left() + x, y as f32);
                self.touch(TouchPhase::Start, p);
                self.settle();
                if !self.app.explorers[&ex].marked.is_empty() {
                    start = Some(p);
                    break 'find;
                }
                self.touch(TouchPhase::End, p);
                self.frame(vec![]);
            }
        }
        let start = start.expect("long press never selected an item");
        let end = self.card(target).center();
        for k in 1..=20 {
            self.touch(TouchPhase::Move, start + (end - start) * (k as f32 / 20.0));
            assert!(egui::DragAndDrop::has_any_payload(&self.ctx), "drag ended early at step {k}");
        }
        self.touch(TouchPhase::End, end);
        self.frame(vec![]);
        // A drop on a terminal types the paths there and focuses it.
        self.app.focused_pane() == Some(target)
    }
}

#[test]
fn touch_drop_files_tab_side_by_side() {
    let (mut s, dir) = setup();
    s.app.toggle_explorer();
    s.settle();
    let ex = s.explorer();
    let c = s.ctx.clone();
    s.app.split(&c, ex, Axis::Horizontal);
    s.settle();
    let t = s.terminals()[0];
    assert!(s.touch_drop(ex, t, &dir));
}

#[test]
fn touch_drop_files_tab_stacked() {
    let (mut s, dir) = setup();
    s.app.toggle_explorer();
    s.settle();
    let ex = s.explorer();
    let c = s.ctx.clone();
    s.app.split(&c, ex, Axis::Vertical);
    s.settle();
    let t = s.terminals()[0];
    assert!(s.touch_drop(ex, t, &dir));
}

#[test]
fn touch_drop_explorer_split_in_terminal_tab() {
    let (mut s, dir) = setup();
    s.settle();
    let t1 = s.terminals()[0];
    let c = s.ctx.clone();
    s.app.split(&c, t1, Axis::Vertical);
    s.settle();
    let t2 = *s.terminals().iter().find(|&&t| t != t1).unwrap();
    s.app.split_explorer(&c, t2, Axis::Horizontal);
    s.settle();
    let ex = s.explorer();
    let ok1 = s.touch_drop(ex, t1, &dir);
    let ok2 = s.touch_drop(ex, t2, &dir);
    assert!(ok1 && ok2, "first terminal: {ok1}, second terminal: {ok2}");
}

/// The finger lifts over the terminal, but sway then moves the pointer to the mouse cursor,
/// parked over the explorer: the drop still goes where the finger was.
#[test]
fn touch_drop_ignores_parked_mouse_cursor() {
    let (mut s, dir) = setup();
    s.app.toggle_explorer();
    s.settle();
    let ex = s.explorer();
    let c = s.ctx.clone();
    s.app.split(&c, ex, Axis::Horizontal);
    s.settle();
    let t = s.terminals()[0];
    s.mouse = Some(s.card(ex).center());
    assert!(s.touch_drop(ex, t, &dir));
}

/// A file dropped from another window where there's no pane (here the title bar) is ignored,
/// even though the explorer has focus: it used to be copied into the explorer's folder.
#[test]
fn drop_outside_every_pane_does_nothing() {
    let (mut s, dir) = setup();
    s.app.toggle_explorer();
    s.settle();
    let ex = s.explorer();
    s.app.explorers.get_mut(&ex).unwrap().navigate(dir.to_path_buf());
    s.settle();
    assert_eq!(s.app.focused_pane(), Some(ex));
    let elsewhere = std::env::temp_dir().join(format!("nex-drop-src-{}", std::process::id()));
    std::fs::write(&elsewhere, "x").unwrap();
    s.frame_with(RawInput {
        events: vec![Event::PointerMoved(Pos2::new(600.0, 10.0))],
        dropped_files: vec![egui::DroppedFile { path: Some(elsewhere.clone()), ..Default::default() }],
        ..Default::default()
    });
    std::thread::sleep(std::time::Duration::from_millis(300));
    s.settle();
    let _ = std::fs::remove_file(&elsewhere);
    let names: Vec<_> = std::fs::read_dir(&*dir).unwrap().map(|e| e.unwrap().file_name()).collect();
    assert_eq!(names.len(), 2, "the drop was copied in: {names:?}");
}

/// Two terminals side by side, settled.
fn two_terminals() -> (Sim, PaneId, PaneId) {
    let (mut s, _dir) = setup();
    s.settle();
    let left = s.terminals()[0];
    let c = s.ctx.clone();
    s.app.split(&c, left, Axis::Horizontal);
    s.settle();
    let right = *s.terminals().iter().find(|&&t| t != left).unwrap();
    s.app.cur_tab_mut().unwrap().focused = left;
    s.settle();
    (s, left, right)
}

impl Sim {
    /// The middle of pane `id`'s header, away from its buttons.
    fn header(&self, id: PaneId) -> Pos2 {
        let r = self.card(id);
        Pos2::new(r.left() + 60.0, r.top() + 12.0)
    }
    fn order(&self) -> Vec<PaneId> {
        self.app.cur_tab().unwrap().root.panes()
    }
    /// Press the mouse at `from` (a header) and drag to `to`, still holding the button.
    fn grab(&mut self, from: Pos2, to: Pos2) {
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![btn(from, true)]);
        self.move_to(from, to);
        assert!(self.app.pane_drag.is_some(), "header drag never started");
    }
    /// Move the held mouse from `from` to `to`.
    fn move_to(&mut self, from: Pos2, to: Pos2) {
        for k in 1..=20 {
            self.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 20.0))]);
        }
    }
    /// Let go of the mouse at `at`, and let everything settle.
    fn release(&mut self, at: Pos2) {
        self.frame(vec![btn(at, false)]);
        self.settle();
    }
    /// Drag from `from` to `to` with the mouse, then let go.
    fn mouse_drag(&mut self, from: Pos2, to: Pos2) {
        self.grab(from, to);
        self.release(to);
    }
    /// Keep still for `secs`.
    fn hold(&mut self, secs: f64) {
        for _ in 0..(secs / 0.016).ceil() as usize {
            self.frame(vec![]);
        }
    }
    fn escape(&mut self) {
        let key = Event::Key { key: egui::Key::Escape, physical_key: None, pressed: true, repeat: false, modifiers: Default::default() };
        self.frame(vec![key]);
        self.settle();
    }
    /// Where tab `uid` is drawn in the title bar.
    fn tab_at(&self, uid: u64) -> Pos2 {
        self.app.tab_hits.iter().find(|(u, _)| *u == uid).expect("tab not drawn").1.center()
    }
    fn shown_uid(&self) -> u64 {
        self.app.cur_tab().unwrap().uid
    }
    /// The panes of the tab with `uid`.
    fn panes_of(&self, uid: u64) -> Vec<PaneId> {
        self.app.tabs.iter().find(|t| t.uid == uid).expect("no such tab").root.panes()
    }
}

fn btn(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// Two terminals side by side in the first tab (shown), and a second tab with one terminal.
/// Returns `(sim, left, right, first tab, second tab, its pane)`.
fn two_tabs() -> (Sim, PaneId, PaneId, u64, u64, PaneId) {
    let (mut s, left, right) = two_terminals();
    let first = s.shown_uid();
    let c = s.ctx.clone();
    s.app.new_tab(&c, None, None);
    s.settle();
    let (second, lone) = (s.shown_uid(), s.order()[0]);
    s.app.show_tab(0);
    s.settle();
    (s, left, right, first, second, lone)
}

#[test]
fn drag_header_onto_pane_swaps_them() {
    let (mut s, left, right) = two_terminals();
    let (from, to) = (s.header(left), s.card(right).center());
    s.mouse_drag(from, to);
    assert_eq!(s.order(), vec![right, left]);
    assert_eq!(s.app.focused_pane(), Some(left));
    assert!(s.card(left).left() > s.card(right).left());
}

#[test]
fn drag_header_onto_bottom_edge_stacks_them() {
    let (mut s, left, right) = two_terminals();
    let r = s.card(right);
    let (from, to) = (s.header(left), Pos2::new(r.center().x, r.bottom() - 15.0));
    s.mouse_drag(from, to);
    assert_eq!(s.order(), vec![right, left]);
    let Some(crate::layout::Node::Split { axis, .. }) = s.app.cur_tab().map(|t| &t.root) else { panic!("not split") };
    assert_eq!(*axis, Axis::Vertical);
    assert!(s.card(left).top() > s.card(right).bottom());
    assert_eq!(s.app.focused_pane(), Some(left));
}

#[test]
fn drag_header_onto_own_slot_or_escape_cancels() {
    let (mut s, left, right) = two_terminals();
    let from = s.header(left);
    let home = s.card(left).center();
    s.mouse_drag(from, home);
    assert_eq!(s.order(), vec![left, right]);
    assert!(s.app.pane_drag.is_none());

    let to = s.card(right).center();
    s.grab(from, to);
    s.escape();
    assert!(s.app.pane_drag.is_none());
    s.release(to);
    assert_eq!(s.order(), vec![left, right]);
}

#[test]
fn drag_header_onto_title_bar_opens_a_new_tab() {
    let (mut s, left, right) = two_terminals();
    let first = s.shown_uid();
    let from = s.header(left);
    s.mouse_drag(from, Pos2::new(600.0, 10.0));
    assert_eq!(s.app.tabs.len(), 2);
    assert_ne!(s.shown_uid(), first);
    assert_eq!(s.order(), vec![left]);
    assert_eq!(s.panes_of(first), vec![right]);
}

#[test]
fn a_lone_pane_has_nowhere_new_to_go() {
    let (mut s, _dir) = setup();
    s.settle();
    let only = s.terminals()[0];
    let from = s.header(only);
    s.frame(vec![Event::PointerMoved(from)]);
    s.frame(vec![btn(from, true)]);
    s.move_to(from, Pos2::new(600.0, 10.0));
    assert!(s.app.pane_drag.is_none(), "a tab's only pane, with no other tab, can't be dragged");
    s.release(Pos2::new(600.0, 10.0));
    assert_eq!(s.app.tabs.len(), 1);
}

#[test]
fn drop_straight_onto_a_tab_moves_the_pane_there() {
    let (mut s, left, right, first, second, lone) = two_tabs();
    let (from, to) = (s.header(left), s.tab_at(second));
    s.mouse_drag(from, to);
    assert_eq!(s.shown_uid(), second);
    assert_eq!(s.panes_of(second), vec![lone, left]);
    assert_eq!(s.panes_of(first), vec![right]);
    assert_eq!(s.app.focused_pane(), Some(left));
}

#[test]
fn holding_over_a_tab_opens_it_to_place_the_pane() {
    let (mut s, left, right, first, second, lone) = two_tabs();
    let (from, tab) = (s.header(left), s.tab_at(second));
    s.grab(from, tab);
    assert_eq!(s.shown_uid(), first, "opened before the delay");
    s.hold(crate::app::pane_drag::SPRING_DELAY + 0.1);
    assert_eq!(s.shown_uid(), second);
    s.settle();
    // Onto the left edge of the lone pane there.
    let r = s.card(lone);
    let to = Pos2::new(r.left() + 20.0, r.center().y);
    s.move_to(tab, to);
    s.release(to);
    assert_eq!(s.shown_uid(), second);
    assert_eq!(s.panes_of(second), vec![left, lone]);
    assert_eq!(s.panes_of(first), vec![right]);
    assert!(s.card(left).right() < s.card(lone).left());
}

#[test]
fn escape_after_opening_another_tab_goes_back() {
    let (mut s, left, right, first, second, lone) = two_tabs();
    let (from, tab) = (s.header(left), s.tab_at(second));
    s.grab(from, tab);
    s.hold(crate::app::pane_drag::SPRING_DELAY + 0.1);
    assert_eq!(s.shown_uid(), second);
    s.escape();
    s.release(tab);
    assert_eq!(s.shown_uid(), first);
    assert_eq!(s.panes_of(first), vec![left, right]);
    assert_eq!(s.panes_of(second), vec![lone]);
}

#[test]
fn moving_a_tabs_last_pane_away_closes_the_tab_but_keeps_the_pane() {
    let (mut s, left, right, first, second, lone) = two_tabs();
    s.app.show_tab(1);
    s.settle();
    let (from, tab) = (s.header(lone), s.tab_at(first));
    s.grab(from, tab);
    s.hold(crate::app::pane_drag::SPRING_DELAY + 0.1);
    assert_eq!(s.shown_uid(), first);
    s.settle();
    let r = s.card(right);
    let to = Pos2::new(r.center().x, r.bottom() - 15.0);
    s.move_to(tab, to);
    s.release(to);
    assert_eq!(s.app.tabs.len(), 1);
    assert!(s.app.tabs.iter().all(|t| t.uid != second));
    assert_eq!(s.panes_of(first), vec![left, right, lone]);
    assert!(s.app.panes.contains_key(&lone), "the terminal was closed");
}

#[test]
fn touch_drag_header_swaps() {
    let (mut s, left, right) = two_terminals();
    let (from, to) = (s.header(left), s.card(right).center());
    s.touch(TouchPhase::Start, from);
    for k in 1..=20 {
        s.touch(TouchPhase::Move, from + (to - from) * (k as f32 / 20.0));
    }
    s.touch(TouchPhase::End, to);
    s.settle();
    assert_eq!(s.order(), vec![right, left]);
}

#[test]
fn keyboard_swap_with_neighbor() {
    let (mut s, left, right) = two_terminals();
    let c = s.ctx.clone();
    s.app.apply(&c, crate::app::actions::Action::Swap(crate::layout::Direction::Right));
    s.settle();
    assert_eq!(s.order(), vec![right, left]);
    assert_eq!(s.app.focused_pane(), Some(left));
}

/// Two tabs of one terminal each (the first shown): `(sim, first, its pane, second, its pane)`.
fn two_lone_tabs() -> (Sim, u64, PaneId, u64, PaneId) {
    let (mut s, _dir) = setup();
    s.settle();
    let (first, a) = (s.shown_uid(), s.order()[0]);
    let c = s.ctx.clone();
    s.app.new_tab(&c, None, None);
    s.settle();
    let (second, b) = (s.shown_uid(), s.order()[0]);
    s.app.show_tab(0);
    s.settle();
    (s, first, a, second, b)
}

#[test]
fn lone_pane_dropped_on_a_lone_tab_merges_them() {
    let (mut s, first, a, second, b) = two_lone_tabs();
    let (from, to) = (s.header(a), s.tab_at(second));
    s.mouse_drag(from, to);
    assert_eq!(s.app.tabs.len(), 1, "the emptied tab stayed");
    assert!(s.app.tabs.iter().all(|t| t.uid != first));
    assert_eq!(s.panes_of(second), vec![b, a]);
}

#[test]
fn lone_pane_placed_in_a_lone_tab_merges_them() {
    let (mut s, first, a, second, b) = two_lone_tabs();
    let (from, tab) = (s.header(a), s.tab_at(second));
    s.grab(from, tab);
    s.hold(crate::app::pane_drag::SPRING_DELAY + 0.1);
    assert_eq!(s.shown_uid(), second);
    s.settle();
    let r = s.card(b);
    let to = Pos2::new(r.center().x, r.bottom() - 15.0);
    s.move_to(tab, to);
    s.release(to);
    assert_eq!(s.app.tabs.len(), 1, "the emptied tab stayed");
    assert!(s.app.tabs.iter().all(|t| t.uid != first));
    assert_eq!(s.panes_of(second), vec![b, a]);
}

#[test]
fn lone_pane_dropped_mid_pane_in_a_lone_tab_merges_them() {
    let (mut s, first, a, second, b) = two_lone_tabs();
    let (from, tab) = (s.header(a), s.tab_at(second));
    s.grab(from, tab);
    s.hold(crate::app::pane_drag::SPRING_DELAY + 0.1);
    s.settle();
    let to = s.card(b).center();
    s.move_to(tab, to);
    s.release(to);
    assert_eq!(s.app.tabs.len(), 1, "the emptied tab stayed");
    assert!(s.app.tabs.iter().all(|t| t.uid != first));
    assert_eq!(s.panes_of(second), vec![b, a]);
}
