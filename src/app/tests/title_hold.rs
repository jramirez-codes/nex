//! Holding a pane header to read a title too long for it: the title scrolls sideways while
//! the finger (or, on desktop, the secondary button) is held, snaps back when it lifts, and
//! the hold never drags the pane. Replayed through egui with the touch helpers of
//! `touch_scroll.rs`.

use egui::{Event, PointerButton, Pos2, TouchPhase, Vec2};

use super::touch_scroll::Sim;
use crate::layout::{Axis, PaneId};

const LONG: &str = "cargo nextest run --workspace --profile ci -- a title much too long for half a window to show whole";

/// Two panes side by side (so a header drag would have somewhere to go), the first one's
/// terminal titled `title`. Returns that pane and a point on its header's title.
fn split_with_title(title: &str) -> (Sim, PaneId, Pos2) {
    let mut s = Sim::new(1);
    let (id, _) = s.terminal();
    let c = s.ctx.clone();
    s.app.split(&c, id, Axis::Horizontal);
    s.settle();
    s.app.panes.terminal_mut(id).unwrap().set_title(title);
    s.frame(vec![]);
    let card = s.app.card_rects.iter().find(|(p, _)| *p == id).expect("pane not shown").1;
    let at = Pos2::new(card.left() + 60.0, card.top() + s.app.header_height() / 2.0);
    (s, id, at)
}

/// Hold a finger at `at` for `frames` frames (long enough to be a long press past ~50).
fn hold(s: &mut Sim, at: Pos2, frames: usize) {
    s.touch(0, TouchPhase::Start, at);
    for _ in 0..frames {
        s.frame(vec![]);
    }
}

#[test]
fn holding_a_long_title_scrolls_it_and_letting_go_snaps_it_back() {
    let (mut s, id, at) = split_with_title(LONG);
    hold(&mut s, at, 60);
    let (held, first) = s.app.title_scroll.shown().expect("the hold didn't take");
    assert_eq!(held, id);
    for _ in 0..40 {
        s.frame(vec![]);
    }
    let (_, later) = s.app.title_scroll.shown().unwrap();
    assert!(later > first, "the title didn't scroll: {first} -> {later}");

    s.touch(0, TouchPhase::End, at);
    s.frame(vec![]);
    assert_eq!(s.app.title_scroll.shown(), None, "letting go didn't snap the title back");
    assert!(s.app.input.pane_drag.is_none());
}

#[test]
fn a_title_that_fits_ignores_the_hold() {
    let (mut s, _, at) = split_with_title("zsh");
    hold(&mut s, at, 100);
    assert_eq!(s.app.title_scroll.shown(), None);
    s.touch(0, TouchPhase::End, at);
    s.frame(vec![]);
    assert_eq!(s.app.title_scroll.shown(), None);
}

#[test]
fn the_hold_never_drags_the_pane() {
    let (mut s, id, at) = split_with_title(LONG);
    hold(&mut s, at, 60);
    assert!(s.app.title_scroll.shown().is_some());
    // Sliding the held finger along still doesn't pick the pane up.
    for k in 1..=10 {
        s.touch(0, TouchPhase::Move, at + Vec2::new(30.0 * k as f32, 0.0));
        assert!(s.app.input.pane_drag.is_none(), "the hold started a pane drag");
    }
    s.touch(0, TouchPhase::End, at + Vec2::new(300.0, 0.0));
    s.settle();
    assert!(s.app.input.pane_drag.is_none());
    assert_eq!(s.app.card_rects.first().map(|r| r.0), Some(id), "the pane moved");
}

#[test]
fn a_quick_drag_of_the_header_still_moves_the_pane() {
    let (mut s, _, at) = split_with_title(LONG);
    s.touch(0, TouchPhase::Start, at);
    s.touch(0, TouchPhase::Move, at + Vec2::new(20.0, 0.0));
    s.touch(0, TouchPhase::Move, at + Vec2::new(60.0, 0.0));
    assert!(s.app.input.pane_drag.is_some(), "the header no longer drags");
    assert_eq!(s.app.title_scroll.shown(), None, "a drag scrolled the title");
    s.touch(0, TouchPhase::End, at + Vec2::new(60.0, 0.0));
}

#[test]
fn holding_the_secondary_button_scrolls_it_on_desktop() {
    let (mut s, id, at) = split_with_title(LONG);
    let button = |pressed| Event::PointerButton { pos: at, button: PointerButton::Secondary, pressed, modifiers: Default::default() };
    s.frame(vec![Event::PointerMoved(at), button(true)]);
    for _ in 0..80 {
        s.frame(vec![]);
    }
    let (held, offset) = s.app.title_scroll.shown().expect("the secondary hold didn't take");
    assert_eq!(held, id);
    assert!(offset > 0.0, "the title didn't scroll");
    s.frame(vec![button(false)]);
    assert_eq!(s.app.title_scroll.shown(), None);
    assert!(s.app.input.pane_drag.is_none());
}
