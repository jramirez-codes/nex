//! Small interactive pieces built from the painting primitives: icon buttons, plus helpers
//! for questions every part of the UI asks (is the window maximized? was this rect pressed?).

use egui::{Color32, Id, Painter, PointerButton, Rect, Sense, Ui};

use super::theme::{ACCENT, HOVER};

/// Draws an icon into a rect in a color (the shape of every `icons::icon_*` function).
pub type IconFn<'a> = dyn Fn(&Painter, Rect, Color32) + 'a;

/// How a button's hover background is drawn.
#[derive(Clone, Copy)]
pub enum HoverShape {
    /// A rounded rectangle filling the button.
    Rounded(u8),
    /// A circle of this radius at the button's center.
    Circle(f32),
}

/// A clickable icon: a hover background, an icon that brightens on hover, an optional tooltip.
///
/// ```ignore
/// let clicked = IconButton::new(rect, Id::new("close"), theme.text_dim, theme.text_bright)
///     .tooltip("Close")
///     .show(ui, painter, |p, r, c| icons::icon_close(p, r, c));
/// ```
pub struct IconButton<'a> {
    rect: Rect,
    icon_rect: Rect,
    id: Id,
    tip: Option<&'a str>,
    color: Color32,
    hover_color: Color32,
    hover_fill: Color32,
    shape: HoverShape,
    enabled: bool,
    active: bool,
}

impl<'a> IconButton<'a> {
    /// A button at `rect`; the icon is drawn in `color`, or `hover_color` while hovered.
    pub fn new(rect: Rect, id: Id, color: Color32, hover_color: Color32) -> Self {
        Self {
            rect,
            icon_rect: rect,
            id,
            tip: None,
            color,
            hover_color,
            hover_fill: HOVER,
            shape: HoverShape::Rounded(6),
            enabled: true,
            active: false,
        }
    }

    pub fn tooltip(mut self, tip: &'a str) -> Self {
        self.tip = Some(tip);
        self
    }

    /// Where the icon is drawn (defaults to the whole button).
    pub fn icon_rect(mut self, r: Rect) -> Self {
        self.icon_rect = r;
        self
    }

    pub fn hover(mut self, shape: HoverShape, fill: Color32) -> Self {
        self.shape = shape;
        self.hover_fill = fill;
        self
    }

    /// A disabled button is drawn faded and never reports clicks.
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// An active (toggled on, or drop-target) button gets an accent background.
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Draw the button with `draw(painter, icon_rect, color)`; returns whether it was clicked.
    pub fn show(self, ui: &Ui, painter: &Painter, draw: impl FnOnce(&Painter, Rect, Color32)) -> bool {
        let resp = ui.interact(self.rect, self.id, Sense::click());
        let hovered = resp.hovered();
        if self.active {
            self.fill(painter, ACCENT.gamma_multiply(0.25));
        } else if self.enabled && hovered {
            self.fill(painter, self.hover_fill);
        }
        let color = match (self.enabled, hovered || self.active) {
            (false, _) => self.color.gamma_multiply(0.4),
            (true, true) => self.hover_color,
            (true, false) => self.color,
        };
        draw(painter, self.icon_rect, color);
        let resp = match self.tip {
            Some(tip) => resp.on_hover_text(tip),
            None => resp,
        };
        self.enabled && resp.clicked_by(PointerButton::Primary)
    }

    fn fill(&self, painter: &Painter, color: Color32) {
        match self.shape {
            HoverShape::Rounded(radius) => {
                painter.rect_filled(self.rect, radius, color);
            }
            HoverShape::Circle(radius) => {
                painter.circle_filled(self.rect.center(), radius, color);
            }
        }
    }
}

pub fn is_maximized(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().maximized.unwrap_or(false))
}

/// Whether the window is in full screen (on macOS this hides the traffic lights).
pub fn is_fullscreen(ctx: &egui::Context) -> bool {
    ctx.input(|i| i.viewport().fullscreen.unwrap_or(false))
}

/// Whether the primary button was pressed this frame with the pointer inside `rect`.
pub fn pressed_in(ui: &Ui, rect: Rect) -> bool {
    ui.rect_contains_pointer(rect) && ui.input(|i| i.pointer.primary_pressed())
}
