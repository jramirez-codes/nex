//! The floating keyboard button (Android): shown while the keyboard is down, a tap brings it
//! back up, and the tap doesn't reach the terminal underneath.

use egui::{Event, PointerButton, Pos2, RawInput, Rect, Vec2};

use crate::app::App;

/// One frame with `events`; whether it asked for the on-screen keyboard (IME output).
fn frame(app: &mut App, ctx: &egui::Context, t: &mut f64, events: Vec<Event>) -> bool {
    *t += 0.016;
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

fn tap(app: &mut App, ctx: &egui::Context, t: &mut f64, pos: Pos2) -> bool {
    let press = |pressed| Event::PointerButton { pos, button: PointerButton::Primary, pressed, modifiers: Default::default() };
    frame(app, ctx, t, vec![Event::PointerMoved(pos), press(true)]);
    frame(app, ctx, t, vec![press(false), Event::PointerGone]);
    frame(app, ctx, t, vec![])
}

#[test]
fn button_shows_only_while_the_keyboard_is_down() {
    let ctx = egui::Context::default();
    let mut app = super::new_app(&ctx, crate::config::Config::default());
    app.soft_keyboard = true;
    let mut t = 0.0;
    frame(&mut app, &ctx, &mut t, vec![]);
    assert!(frame(&mut app, &ctx, &mut t, vec![]), "a focused terminal asks for the keyboard");
    assert!(app.keyboard_button.rect.is_none(), "no button while the keyboard is up");

    // Put away (on a phone: the back gesture, which `follow_system_keyboard` turns into this).
    app.keyboard_button.hidden = true;
    assert!(!frame(&mut app, &ctx, &mut t, vec![]), "nex leaves it down");
    let rect = app.keyboard_button.rect.expect("button shown while the keyboard is down");
    assert!(app.content_area.contains_rect(rect));

    // A tap on the terminal itself doesn't bring it back.
    let term = app.term_rects[0].1;
    assert!(!tap(&mut app, &ctx, &mut t, term.center()));
    assert!(app.keyboard_button.rect.is_some());

    assert!(tap(&mut app, &ctx, &mut t, rect.center()), "tapping the button brings the keyboard up");
    assert!(app.keyboard_button.rect.is_none(), "and the button goes away");
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
