//! The floating keyboard button (Android): a tap toggles the keyboard, a drag is a joystick
//! for the arrow keys, a double-tap arms Ctrl for the next key; none of it reaches the
//! terminal underneath.

use egui::{Event, Key, Modifiers, PointerButton, Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::app::chrome::keyboard_button::DOUBLE_TAP;

const STEP: f64 = 0.016;

/// One frame with `events`; whether it asked for the on-screen keyboard (IME output).
fn frame(app: &mut App, ctx: &egui::Context, t: &mut f64, events: Vec<Event>) -> bool {
    *t += STEP;
    let input = RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 800.0))),
        time: Some(*t),
        events,
        ..Default::default()
    };
    let out = ctx.run(input, |ctx| {
        let mut frame = eframe::Frame::_new_kittest();
        eframe::App::update(app, ctx, &mut frame);
    });
    out.platform_output.ime.is_some()
}

/// Frames with nothing happening for `secs`; whether the last asked for the keyboard.
fn wait(app: &mut App, ctx: &egui::Context, t: &mut f64, secs: f64) -> bool {
    let mut ime = false;
    for _ in 0..(secs / STEP).ceil() as usize {
        ime = frame(app, ctx, t, vec![]);
    }
    ime
}

fn press(pos: Pos2, pressed: bool) -> Event {
    Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() }
}

/// A quick tap at `pos` (it lifts in the next frame).
fn tap(app: &mut App, ctx: &egui::Context, t: &mut f64, pos: Pos2) {
    frame(app, ctx, t, vec![Event::PointerMoved(pos), press(pos, true)]);
    frame(app, ctx, t, vec![press(pos, false), Event::PointerGone]);
}

/// A tap, then long enough for it to count as one (not half of a double-tap).
fn tap_and_settle(app: &mut App, ctx: &egui::Context, t: &mut f64, pos: Pos2) -> bool {
    tap(app, ctx, t, pos);
    wait(app, ctx, t, DOUBLE_TAP + 0.05)
}

fn phone(ctx: &egui::Context) -> App {
    let mut app = super::new_app(ctx, crate::config::Config::default());
    app.soft_keyboard = true;
    app
}

/// What the focused terminal has been sent (and forget it).
fn sent(app: &mut App) -> Vec<u8> {
    let id = app.focused_pane().unwrap();
    std::mem::take(&mut *app.panes[&id].written.lock().unwrap())
}

fn count(haystack: &[u8], needle: &[u8]) -> usize {
    haystack.windows(needle.len()).filter(|w| *w == needle).count()
}

#[test]
fn a_tap_toggles_the_keyboard() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    let mut t = 0.0;
    frame(&mut app, &ctx, &mut t, vec![]);
    assert!(frame(&mut app, &ctx, &mut t, vec![]), "a focused terminal asks for the keyboard");
    let rect = app.keyboard_button.rect.expect("the button is there with the keyboard up");
    assert!(app.content_area.contains_rect(rect));

    // A tap waits a moment (it could be the first of a double-tap), then puts it away.
    tap(&mut app, &ctx, &mut t, rect.center());
    assert!(frame(&mut app, &ctx, &mut t, vec![]), "not yet");
    assert!(!wait(&mut app, &ctx, &mut t, DOUBLE_TAP), "put away");
    assert!(app.keyboard_button.hidden);
    assert!(app.keyboard_button.rect.is_some(), "the button stays to bring it back");

    // A tap on the terminal itself doesn't bring it back.
    let term = app.term_rects[0].1;
    assert!(!tap_and_settle(&mut app, &ctx, &mut t, term.center()));

    let rect = app.keyboard_button.rect.unwrap();
    assert!(tap_and_settle(&mut app, &ctx, &mut t, rect.center()), "the button brings it back up");
    assert!(sent(&mut app).is_empty(), "no taps reached the shell");
}

#[test]
fn dragging_the_button_sends_arrows_that_repeat_while_held() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    let mut t = 0.0;
    wait(&mut app, &ctx, &mut t, 0.1);
    sent(&mut app);
    let c = app.keyboard_button.rect.unwrap().center();

    frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(c), press(c, true)]);
    frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(c + Vec2::new(5.0, 0.0))]);
    assert!(sent(&mut app).is_empty(), "inside the dead zone: nothing yet");
    frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(c + Vec2::new(30.0, 4.0))]);
    assert_eq!(sent(&mut app), b"\x1b[C", "right, at once");

    // Held there: it repeats.
    wait(&mut app, &ctx, &mut t, 1.0);
    let held = sent(&mut app);
    assert!(count(&held, b"\x1b[C") >= 4, "repeats while held: {held:?}");

    // Swung round to up: up at once.
    frame(&mut app, &ctx, &mut t, vec![Event::PointerMoved(c + Vec2::new(0.0, -40.0))]);
    assert_eq!(sent(&mut app), b"\x1b[A");

    // Let go: no more arrows, and it wasn't a tap (the keyboard stays up).
    let up = c + Vec2::new(0.0, -40.0);
    frame(&mut app, &ctx, &mut t, vec![press(up, false), Event::PointerGone]);
    assert!(wait(&mut app, &ctx, &mut t, 0.5), "still up");
    assert!(sent(&mut app).is_empty());
    assert!(!app.keyboard_button.hidden);
}

#[test]
fn a_double_tap_arms_ctrl_for_the_next_key() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    let mut t = 0.0;
    wait(&mut app, &ctx, &mut t, 0.1);
    let c = app.keyboard_button.rect.unwrap().center();

    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    assert!(app.keyboard_button.ctrl_armed);
    assert!(wait(&mut app, &ctx, &mut t, DOUBLE_TAP + 0.05), "the keyboard stays up");
    sent(&mut app);

    // A keyboard that sends key events: the key and its text make one ^C.
    let key = Event::Key { key: Key::C, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, &mut t, vec![key, Event::Text("c".into())]);
    assert_eq!(sent(&mut app), [0x03]);
    assert!(!app.keyboard_button.ctrl_armed, "used up");

    // A keyboard that only sends text: the first character takes Ctrl, the rest are typed.
    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    frame(&mut app, &ctx, &mut t, vec![Event::Text("ax".into())]);
    assert_eq!(sent(&mut app), b"\x01x");

    // A second double-tap disarms it.
    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    assert!(app.keyboard_button.ctrl_armed);
    wait(&mut app, &ctx, &mut t, DOUBLE_TAP + 0.05);
    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    assert!(!app.keyboard_button.ctrl_armed);
}

#[test]
fn armed_ctrl_runs_ctrl_shortcuts_like_a_held_ctrl() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    let mut t = 0.0;
    wait(&mut app, &ctx, &mut t, 0.1);
    let c = app.keyboard_button.rect.unwrap().center();
    let shell = app.focused_pane().unwrap();
    sent(&mut app);

    // A keyboard that sends key events: armed Ctrl, then E, is Ctrl+E: the file explorer.
    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    let key = Event::Key { key: Key::E, physical_key: None, pressed: true, repeat: false, modifiers: Modifiers::NONE };
    frame(&mut app, &ctx, &mut t, vec![key, Event::Text("e".into())]);
    wait(&mut app, &ctx, &mut t, 0.1);
    assert!(!app.keyboard_button.ctrl_armed, "used up");
    assert_eq!(app.explorers.len(), 1, "Ctrl+E opened the file explorer");
    assert!(app.panes[&shell].written.lock().unwrap().is_empty(), "no ^E or e reached the shell");

    // A keyboard that only sends text: armed Ctrl, then "g", is Ctrl+G: the Changes pane.
    let mut app = phone(&ctx);
    wait(&mut app, &ctx, &mut t, 0.1);
    let shell = app.focused_pane().unwrap();
    sent(&mut app);
    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    frame(&mut app, &ctx, &mut t, vec![Event::Text("g".into())]);
    wait(&mut app, &ctx, &mut t, 0.1);
    assert!(!app.keyboard_button.ctrl_armed, "used up");
    assert_eq!(app.diffs.len(), 1, "Ctrl+G opened the Changes pane");
    assert!(app.panes[&shell].written.lock().unwrap().is_empty(), "no ^G or g reached the shell");
}

#[test]
fn a_double_tap_with_the_keyboard_down_brings_it_up() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    app.keyboard_button.hidden = true;
    let mut t = 0.0;
    assert!(!wait(&mut app, &ctx, &mut t, 0.1));
    let c = app.keyboard_button.rect.unwrap().center();
    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    assert!(app.keyboard_button.ctrl_armed);
    assert!(wait(&mut app, &ctx, &mut t, DOUBLE_TAP + 0.05), "Ctrl needs a key to go with it");
}

#[test]
fn the_agents_tab_has_the_button_too() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    let mut t = 0.0;
    wait(&mut app, &ctx, &mut t, 0.1);
    app.toggle_agents_tab();
    assert!(!wait(&mut app, &ctx, &mut t, 0.1), "the keyboard starts down on the Agents tab");
    let rect = app.keyboard_button.rect.expect("the button is there");
    assert!(tap_and_settle(&mut app, &ctx, &mut t, rect.center()), "a tap brings it up");

    // Back on the terminal, the keyboard is as it was there.
    app.toggle_agents_tab();
    assert!(wait(&mut app, &ctx, &mut t, 0.1));
}

#[test]
fn no_button_without_an_on_screen_keyboard() {
    let ctx = egui::Context::default();
    let mut app = super::new_app(&ctx, crate::config::Config::default());
    app.keyboard_button.hidden = true;
    let mut t = 0.0;
    frame(&mut app, &ctx, &mut t, vec![]);
    frame(&mut app, &ctx, &mut t, vec![]);
    assert!(app.keyboard_button.rect.is_none());
}

#[test]
fn armed_ctrl_turns_the_close_button_into_a_pin() {
    let ctx = egui::Context::default();
    let mut app = phone(&ctx);
    let mut t = 0.0;
    wait(&mut app, &ctx, &mut t, 0.1);
    let c = app.keyboard_button.rect.unwrap().center();
    let id = app.focused_pane().unwrap();
    let card = app.card_rects.iter().find(|(p, _)| *p == id).unwrap().1;
    // The header's first button (the ×), right to left.
    let close = Pos2::new(card.right() - 22.0, card.top() + app.header_height() / 2.0 + 1.0);

    tap(&mut app, &ctx, &mut t, c);
    tap(&mut app, &ctx, &mut t, c);
    assert!(app.keyboard_button.ctrl_armed);
    wait(&mut app, &ctx, &mut t, DOUBLE_TAP + 0.05);

    tap(&mut app, &ctx, &mut t, close);
    wait(&mut app, &ctx, &mut t, 0.1);
    assert!(app.is_sticky(id), "pinned, not closed");
    assert!(app.panes.contains_key(&id));
    assert!(!app.keyboard_button.ctrl_armed, "used up");

    // Without Ctrl, the pinned pane's button is the pin: a tap unpins it.
    tap(&mut app, &ctx, &mut t, close);
    wait(&mut app, &ctx, &mut t, 0.1);
    assert!(!app.is_sticky(id));
    assert!(app.panes.contains_key(&id));
}
