//! A macOS-style alert: a narrow centered card over a dimmed window, with an icon tile, a bold
//! centered title, softer body text, optional numbered steps, and equal-width buttons
//! (secondary on the left, primary on the right). Enter picks the primary button; Escape or a
//! tap outside picks the secondary one (or dismisses an alert with a single button).

use egui::epaint::Shadow;
use egui::text::{LayoutJob, TextFormat};
use egui::{Align, Color32, CursorIcon, FontId, Id, Key, Layout, Painter, Pos2, Rect, RichText, Sense, Stroke, StrokeKind, Ui, Vec2};

use super::paint::bold;
use super::theme::{ACCENT, Theme, lighten, white_alpha};

const WIDTH: f32 = 300.0;
const RADIUS: u8 = 16;
const ICON: f32 = 52.0;
const TITLE_SIZE: f32 = 15.0;
const BODY_SIZE: f32 = 12.5;
const BUTTON_RADIUS: u8 = 8;
/// Apple's system blue (dark appearance): deeper than the UI accent, so white text on it reads.
const BLUE: Color32 = Color32::from_rgb(0x0a, 0x84, 0xff);

/// The symbol in the alert's icon tile.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum AlertIcon {
    /// Something asks for a login.
    Lock,
    /// Something is done.
    Done,
    /// Something went wrong.
    Warning,
}

pub struct Alert<'a> {
    pub id: Id,
    pub icon: AlertIcon,
    pub title: &'a str,
    pub body: &'a str,
    /// Numbered steps under the body, if any. Text in backticks is shown as code.
    pub steps: &'a [String],
    pub primary: &'a str,
    pub secondary: Option<&'a str>,
    /// Touch-sized buttons and text (the sidebar tab bar).
    pub touch: bool,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum AlertChoice {
    Primary,
    /// The secondary button, Escape, or a tap outside the alert.
    Secondary,
}

impl Alert<'_> {
    /// Draw the alert for this frame; returns the choice made in it, if any.
    pub fn show(&self, ctx: &egui::Context, theme: &Theme) -> Option<AlertChoice> {
        let scale = if self.touch { 1.15 } else { 1.0 };
        let fill = lighten(theme.card.to_opaque(), 0.05);
        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(RADIUS)
            .stroke(Stroke::new(1.0_f32, white_alpha(26)))
            .shadow(Shadow { offset: [0, 18], blur: 48, spread: 0, color: Color32::from_black_alpha(150) })
            .inner_margin(egui::Margin { left: 20, right: 20, top: 22, bottom: 18 });
        let (enter, escape) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        let mut choice = None;
        let modal = egui::Modal::new(self.id).frame(frame).backdrop_color(Color32::from_black_alpha(120)).show(ctx, |ui| {
            ui.set_width((WIDTH * scale).min(ctx.content_rect().width() - 48.0));
            ui.spacing_mut().item_spacing = Vec2::ZERO;

            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                let (rect, _) = ui.allocate_exact_size(Vec2::splat(ICON * scale), Sense::hover());
                icon_tile(ui.painter(), rect, self.icon);
                ui.add_space(14.0);
                ui.label(RichText::new(self.title).font(bold(TITLE_SIZE * scale)).color(theme.text_bright));
                ui.add_space(6.0);
                ui.label(RichText::new(self.body).size(BODY_SIZE * scale).color(theme.text_bright.gamma_multiply(0.62)));
            });
            if !self.steps.is_empty() {
                ui.add_space(14.0);
                steps(ui, self.steps, scale, theme);
            }
            ui.add_space(20.0);
            choice = buttons(ui, self, scale, theme);
        });
        let dismissed = escape || modal.backdrop_response.clicked();
        choice.or(if enter {
            Some(AlertChoice::Primary)
        } else if dismissed {
            Some(AlertChoice::Secondary)
        } else {
            None
        })
    }
}

/// Numbered steps in a recessed panel.
fn steps(ui: &mut Ui, steps: &[String], scale: f32, theme: &Theme) {
    egui::Frame::new().fill(Color32::from_black_alpha(46)).stroke(Stroke::new(1.0_f32, white_alpha(10))).corner_radius(10).inner_margin(12).show(ui, |ui| {
        ui.set_width(ui.available_width());
        for (n, step) in steps.iter().enumerate() {
            if n > 0 {
                ui.add_space(9.0);
            }
            ui.horizontal_top(|ui| {
                let d = 18.0 * scale;
                let (badge, _) = ui.allocate_exact_size(Vec2::splat(d), Sense::hover());
                ui.painter().circle_filled(badge.center(), d / 2.0, ACCENT.gamma_multiply(0.22));
                ui.painter().text(badge.center(), egui::Align2::CENTER_CENTER, n + 1, bold(10.5 * scale), ACCENT);
                ui.add_space(10.0);
                ui.add(egui::Label::new(step_text(step, scale, theme)).wrap());
            });
        }
    });
}

/// A step's text, with `backticked` parts in a monospace chip.
fn step_text(step: &str, scale: f32, theme: &Theme) -> LayoutJob {
    let size = BODY_SIZE * scale;
    let plain = TextFormat { font_id: FontId::proportional(size), color: theme.text_bright.gamma_multiply(0.86), ..Default::default() };
    let code = TextFormat {
        font_id: FontId::monospace(size * 0.95),
        color: theme.text_bright,
        background: white_alpha(22),
        ..Default::default()
    };
    let mut job = LayoutJob::default();
    for (i, part) in step.split('`').enumerate() {
        if part.is_empty() {
            continue;
        }
        if i % 2 == 1 {
            // Thin spaces pad the chip's background past the glyphs.
            job.append(&format!("\u{2009}{part}\u{2009}"), 0.0, code.clone());
        } else {
            job.append(part, 0.0, plain.clone());
        }
    }
    job
}

/// The button row: secondary then primary, splitting the width evenly; a lone primary button
/// takes the whole width.
fn buttons(ui: &mut Ui, alert: &Alert, scale: f32, theme: &Theme) -> Option<AlertChoice> {
    let h = if alert.touch { 42.0 } else { 32.0 };
    let gap = 8.0;
    let full = ui.available_width();
    let (row, _) = ui.allocate_exact_size(Vec2::new(full, h), Sense::hover());
    let mut choice = None;
    match alert.secondary {
        Some(label) => {
            let w = (full - gap) / 2.0;
            let left = Rect::from_min_size(row.min, Vec2::new(w, h));
            let right = Rect::from_min_size(Pos2::new(row.right() - w, row.top()), Vec2::new(w, h));
            if button(ui, left, label, false, scale, theme) {
                choice = Some(AlertChoice::Secondary);
            }
            if button(ui, right, alert.primary, true, scale, theme) {
                choice = Some(AlertChoice::Primary);
            }
        }
        None => {
            if button(ui, row, alert.primary, true, scale, theme) {
                choice = Some(AlertChoice::Primary);
            }
        }
    }
    choice
}

/// One alert button, with hover and press states. Returns whether it was clicked.
fn button(ui: &mut Ui, rect: Rect, label: &str, primary: bool, scale: f32, theme: &Theme) -> bool {
    let resp = ui.interact(rect, ui.id().with(label), Sense::click()).on_hover_cursor(CursorIcon::PointingHand);
    let (fill, text) = if primary { (BLUE, Color32::WHITE) } else { (white_alpha(20), theme.text_bright) };
    let fill = if resp.is_pointer_button_down_on() {
        fill.gamma_multiply(0.8)
    } else if resp.hovered() {
        lighten(fill, 0.1)
    } else {
        fill
    };
    let p = ui.painter();
    p.rect_filled(rect, BUTTON_RADIUS, fill);
    if primary {
        // A soft top sheen, so the primary button reads as raised.
        p.rect_stroke(rect, BUTTON_RADIUS, Stroke::new(1.0_f32, white_alpha(30)), StrokeKind::Inside);
    }
    p.text(rect.center(), egui::Align2::CENTER_CENTER, label, bold(13.0 * scale), text);
    resp.clicked()
}

/// A rounded tile in the icon's color with a white symbol, like a macOS alert's app badge.
fn icon_tile(p: &Painter, r: Rect, icon: AlertIcon) {
    let color = match icon {
        AlertIcon::Lock => BLUE,
        AlertIcon::Done => Color32::from_rgb(0x34, 0xc7, 0x59),
        AlertIcon::Warning => Color32::from_rgb(0xff, 0x9f, 0x0a),
    };
    let radius = (r.width() * 0.26) as u8;
    p.add(Shadow { offset: [0, 4], blur: 14, spread: 0, color: color.gamma_multiply(0.35) }.as_shape(r, radius));
    p.rect_filled(r, radius, color);
    // Lighter upper half for a little depth.
    let upper = Rect::from_min_max(r.min, Pos2::new(r.right(), r.center().y));
    p.rect_filled(upper, super::paint::top_radius(radius), white_alpha(22));
    p.rect_stroke(r, radius, Stroke::new(1.0_f32, white_alpha(40)), StrokeKind::Inside);

    let s = r.width() / 52.0;
    let c = r.center();
    let white = Color32::WHITE;
    let line = Stroke::new(3.2 * s, white);
    match icon {
        AlertIcon::Lock => {
            let body = Rect::from_center_size(c + Vec2::new(0.0, 5.0 * s), Vec2::new(22.0 * s, 16.0 * s));
            // The shackle: a half circle on two legs.
            let (rad, top) = (6.5 * s, body.top() - 6.0 * s);
            let mut pts = vec![Pos2::new(c.x - rad, body.top() + 1.0)];
            for i in 0..=16 {
                let a = std::f32::consts::PI * (1.0 + i as f32 / 16.0);
                pts.push(Pos2::new(c.x + rad * a.cos(), top + rad * a.sin()));
            }
            pts.push(Pos2::new(c.x + rad, body.top() + 1.0));
            p.add(egui::Shape::line(pts, line));
            p.rect_filled(body, (4.0 * s) as u8, white);
            p.circle_filled(body.center() - Vec2::new(0.0, 1.0 * s), 2.2 * s, Color32::from_black_alpha(90));
        }
        AlertIcon::Done => {
            let pts = vec![c + Vec2::new(-9.0, 0.5) * s, c + Vec2::new(-2.5, 7.0) * s, c + Vec2::new(10.0, -7.0) * s];
            p.add(egui::Shape::line(pts, Stroke::new(4.0 * s, white)));
        }
        AlertIcon::Warning => {
            p.line_segment([c + Vec2::new(0.0, -10.0) * s, c + Vec2::new(0.0, 3.0) * s], Stroke::new(4.0 * s, white));
            p.circle_filled(c + Vec2::new(0.0, 10.0) * s, 2.4 * s, white);
        }
    }
}
