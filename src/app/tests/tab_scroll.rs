//! Phone title-bar tabs (`platform::WINDOW.scroll_tabs`): one width, two and a half to the track, and
//! a strip that swipes sideways.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};

use crate::app::App;

struct Sim {
    ctx: egui::Context,
    app: App,
    t: f64,
    width: f32,
}

impl Sim {
    fn new(tabs: usize) -> Self {
        let ctx = egui::Context::default();
        let mut app = super::new_app(&ctx, crate::config::Config::default());
        app.strip.scroll = crate::app::chrome::titlebar::TabScroll::new(true);
        let mut s = Sim { ctx, app, t: 0.0, width: 420.0 };
        s.settle();
        let c = s.ctx.clone();
        for _ in 1..tabs {
            s.app.new_tab(&c, None, None);
        }
        s.settle();
        s
    }
    fn frame(&mut self, events: Vec<Event>) {
        self.t += 0.016;
        let input = RawInput {
            events,
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(self.width, 800.0))),
            time: Some(self.t),
            ..Default::default()
        };
        let app = &mut self.app;
        let _ = self.ctx.run(input, |ctx| {
            let mut frame = eframe::Frame::_new_kittest();
            eframe::App::update(app, ctx, &mut frame);
        });
    }
    fn settle(&mut self) {
        for _ in 0..90 {
            self.frame(vec![]);
        }
    }
    /// Where each tab was drawn, in tab order.
    fn rects(&self) -> Vec<Rect> {
        self.app.tabs.iter().map(|t| self.app.strip.tab_rects[&t.uid].rect).collect()
    }
    fn track(&self) -> Rect {
        let r = self.rects();
        // The first tab sits flush with the track's left edge when unscrolled.
        Rect::from_min_max(Pos2::new(r[0].left() + self.app.strip.scroll.offset, r[0].top()), r[0].max)
    }
    /// Press at `from`, move to `to` over `steps` frames and lift.
    fn drag(&mut self, from: Pos2, to: Pos2, steps: usize, hold: usize) {
        self.frame(vec![Event::PointerMoved(from), btn(from, true)]);
        for _ in 0..hold {
            self.frame(vec![]);
        }
        for k in 1..=steps {
            self.frame(vec![Event::PointerMoved(from + (to - from) * (k as f32 / steps as f32))]);
        }
        self.frame(vec![btn(to, false)]);
    }
}

fn btn(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

#[test]
fn tabs_keep_one_width_two_and_a_half_to_the_track() {
    let mut s = Sim::new(2);
    let two = s.rects()[0].width();
    let c = s.ctx.clone();
    for _ in 0..4 {
        s.app.new_tab(&c, None, None);
    }
    s.settle();
    let rects = s.rects();
    assert_eq!(rects.len(), 6, "every tab is laid out, scrolled or not");
    for r in &rects {
        assert!((r.width() - two).abs() < 0.01, "tabs don't shrink as more open: {} vs {two}", r.width());
    }
    // The newest tab (shown) was scrolled into view at the end of the track.
    let offset = s.app.strip.scroll.offset;
    assert!((offset - two * 3.5).abs() < 0.5, "scrolled so the last tab ends the track: {offset}");
    let track = s.track();
    let right = track.left() + 2.5 * two;
    assert!((rects[5].right() - right).abs() < 0.5);
}

#[test]
fn swipe_scrolls_and_tap_still_switches() {
    let mut s = Sim::new(6);
    let before = s.app.strip.scroll.offset;
    assert!(before > 0.0);
    let shown = s.app.active;
    let at = s.rects()[shown].center();
    // Swipe right: back toward the first tabs, without switching or reordering.
    let order: Vec<u64> = s.app.tabs.iter().map(|t| t.uid).collect();
    s.drag(at, at + Vec2::new(120.0, 0.0), 10, 0);
    let after = s.app.strip.scroll.offset;
    assert!(after < before - 100.0, "swipe moved the strip: {before} -> {after}");
    assert_eq!(s.app.active, shown);
    assert_eq!(order, s.app.tabs.iter().map(|t| t.uid).collect::<Vec<_>>());
    // It stops at the first tab.
    s.drag(at, at + Vec2::new(400.0, 0.0), 10, 0);
    s.settle();
    assert_eq!(s.app.strip.scroll.offset, 0.0);
    // A tap on the (now visible) first tab shows it.
    let first = s.rects()[0].center();
    s.frame(vec![Event::PointerMoved(first), btn(first, true)]);
    s.frame(vec![btn(first, false)]);
    s.settle();
    assert_eq!(s.app.active, 0);
}

#[test]
fn active_tab_comes_into_view_once_the_window_has_its_size() {
    let mut s = Sim::new(4);
    // As at launch: the active tab was revealed while the window was still tiny, then the
    // window grew.
    s.app.strip.scroll.offset = 0.0;
    s.width = 200.0;
    s.settle();
    s.width = 420.0;
    s.settle();
    let rects = s.rects();
    let track = s.track();
    let right = track.left() + 2.5 * rects[0].width();
    assert!(rects[s.app.active].right() <= right + 0.5, "active tab cut off: {:?} past {right}", rects[s.app.active]);
}

#[test]
fn long_press_then_drag_reorders() {
    let mut s = Sim::new(3);
    let rects = s.rects();
    let first = s.app.tabs[0].uid;
    // Held ~0.5 s before moving: the tab is picked up rather than the strip swiped.
    s.drag(rects[0].center(), rects[2].center(), 10, 30);
    assert_eq!(s.app.tabs[2].uid, first);
}
