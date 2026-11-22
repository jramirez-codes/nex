//! The floating keyboard button (Android): a round glass button in the bottom-right corner,
//! shown while the on-screen keyboard is down, that brings it back up. The keyboard goes away
//! the usual Android ways (the back gesture, the keyboard's own hide key), and nex leaves it
//! down until the button is tapped.

use std::f32::consts::PI;

use egui::epaint::{PathShape, PathStroke, Shadow};
use egui::{Color32, Id, LayerId, Order, Painter, Pos2, Rect, Shape, Stroke, Vec2};

use crate::app::App;
use crate::ui::glide::animations;
use crate::ui::icons::icon_keyboard;
use crate::ui::theme::Theme;

/// Apple's minimum tap target.
pub(in crate::app) const SIZE: f32 = 44.0;
/// From the right and bottom edges of the content area.
const MARGIN: f32 = 12.0;
/// How long the button takes to fade in once the keyboard is down.
const FADE_TIME: f32 = 0.15;

/// Where the button goes in `area` (the content area).
pub(in crate::app) fn button_rect(area: Rect) -> Rect {
    let min = Pos2::new(area.right() - MARGIN - SIZE, area.bottom() - MARGIN - SIZE);
    Rect::from_min_size(min, Vec2::splat(SIZE))
}

/// The keyboard button's state, kept across frames.
#[derive(Default)]
pub(in crate::app) struct KeyboardButton {
    /// The keyboard is down and nex keeps it down until the button is tapped.
    pub hidden: bool,
    /// Where the button was drawn this frame, for taps (`None` while it isn't shown).
    pub rect: Option<Rect>,
    /// A press started on the button and hasn't lifted yet.
    pressed: bool,
    /// What the system last said about the keyboard, to notice it coming up or going away.
    last_visible: Option<bool>,
}

impl App {
    /// Whether a terminal has focus with nothing in the way (a drag, a dialog, a pinch), so
    /// the keyboard is the terminal's.
    fn terminal_wants_keyboard(&self, ctx: &egui::Context) -> bool {
        self.soft_keyboard
            && self.focused_pane().is_some_and(|id| self.panes.contains_key(&id))
            && self.pane_drag.is_none()
            && !self.mouse.pinching()
            && ctx.memory(|m| m.top_modal_layer()).is_none()
    }

    /// Follow the keyboard when the system moves it: put away with the back gesture, nex
    /// keeps it down; brought back by Android (returning to the app), nex keeps it up.
    fn follow_system_keyboard(&mut self, terminal: bool) {
        let kb = &mut self.keyboard_button;
        let visible = crate::platform::soft_keyboard_visible();
        if visible != kb.last_visible {
            kb.last_visible = visible;
            if let (true, Some(visible)) = (terminal, visible) {
                kb.hidden = !visible;
            }
        }
    }

    /// Draw the button while the keyboard is down and bring it up on a tap. Runs before
    /// `handle_mouse`, which leaves presses on the button alone.
    pub(in crate::app) fn keyboard_button(&mut self, ctx: &egui::Context) {
        let terminal = self.terminal_wants_keyboard(ctx);
        self.follow_system_keyboard(terminal);
        let shown = terminal && self.keyboard_button.hidden;
        let fade = if animations(ctx) {
            ctx.animate_bool_with_time(Id::new("keyboard-button"), shown, FADE_TIME)
        } else if shown {
            1.0
        } else {
            0.0
        };
        if !shown {
            self.keyboard_button.rect = None;
            self.keyboard_button.pressed = false;
            return;
        }
        let rect = button_rect(self.content_area);
        self.keyboard_button.rect = Some(rect);

        let events = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } = event {
                if pressed {
                    self.keyboard_button.pressed = rect.contains(pos);
                } else if std::mem::take(&mut self.keyboard_button.pressed) && rect.expand(8.0).contains(pos) {
                    self.keyboard_button.hidden = false;
                    ctx.request_repaint();
                    return;
                }
            }
        }
        if self.keyboard_button.pressed && !ctx.input(|i| i.pointer.any_down()) {
            self.keyboard_button.pressed = false;
        }
        self.paint_keyboard_button(ctx, rect, fade);
    }

    fn paint_keyboard_button(&self, ctx: &egui::Context, rect: Rect, fade: f32) {
        let theme = Theme::from_config(&self.cfg);
        let pressed = self.keyboard_button.pressed;
        let press = if animations(ctx) {
            ctx.animate_bool_with_time(Id::new("keyboard-button-pressed"), pressed, 0.08)
        } else if pressed {
            1.0
        } else {
            0.0
        };
        let mut p = ctx.layer_painter(LayerId::new(Order::Foreground, Id::new("keyboard-button")));
        p.multiply_opacity(fade);
        let r = Rect::from_center_size(rect.center(), rect.size() * (1.0 - 0.08 * press));
        glass_circle(&p, r, press);
        let glyph = Rect::from_center_size(r.center(), Vec2::splat(r.width() * 0.5));
        icon_keyboard(&p, glyph, theme.text_bright.gamma_multiply(0.88));
    }
}

/// A round glass button, like [`glass_pill`](crate::ui::paint::glass_pill) for a circle: soft
/// shadow, translucent fill, hairline rim and a bright arc along the top edge. `press` (0 to 1)
/// lightens the fill.
fn glass_circle(p: &Painter, r: Rect, press: f32) {
    let (c, radius) = (r.center(), r.width() / 2.0);
    let shadow = Shadow { offset: [0, 2], blur: 10, spread: 0, color: Color32::from_black_alpha(120) };
    p.add(shadow.as_shape(r, radius as u8));
    let lift = (22.0 * press) as u8;
    p.circle_filled(c, radius, Color32::from_rgba_unmultiplied(0x5a + lift, 0x55 + lift, 0x5e + lift, 165));
    p.circle_stroke(c, radius - 0.5, Stroke::new(1.0_f32, Color32::from_white_alpha(34)));
    // The top highlight: an arc over the upper third, just inside the rim.
    let arc: Vec<Pos2> = (0..=24)
        .map(|i| {
            let a = PI * (1.25 + 0.5 * i as f32 / 24.0);
            c + Vec2::angled(a) * (radius - 1.5)
        })
        .collect();
    p.add(Shape::Path(PathShape::line(arc, PathStroke::new(1.0, Color32::from_white_alpha(50)))));
}
