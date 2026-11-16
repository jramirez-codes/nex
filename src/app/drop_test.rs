//! Touch drag from the explorer onto a terminal, replayed through egui as egui-winit sends it.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, TouchDeviceId, TouchId, TouchPhase, Vec2};

use std::path::{Path, PathBuf};

use super::App;
use crate::app::layout::{Axis, PaneId};

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
    let app = App::new(&cc, crate::config::Config::default(), Default::default());
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
    /// Drag from `from` to `to` with the mouse, then let go.
    fn mouse_drag(&mut self, from: Pos2, to: Pos2) {
        let btn = |pos, pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
        self.frame(vec![Event::PointerMoved(from)]);
        self.frame(vec![btn(from, true)]);
        for k in 1..=20 {
            self.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / 20.0))]);
        }
        assert!(self.app.cur_tab().unwrap().drag.is_some(), "header drag never started");
        self.frame(vec![btn(to, false)]);
        self.settle();
    }
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
fn drag_header_dropped_on_nothing_cancels() {
    let (mut s, left, right) = two_terminals();
    let from = s.header(left);
    s.mouse_drag(from, Pos2::new(600.0, 10.0));
    assert_eq!(s.order(), vec![left, right]);
    assert!(s.app.cur_tab().unwrap().drag.is_none());
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
    s.app.apply(&c, super::actions::Action::Swap(crate::app::layout::Direction::Right));
    s.settle();
    assert_eq!(s.order(), vec![right, left]);
    assert_eq!(s.app.focused_pane(), Some(left));
}
