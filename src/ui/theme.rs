//! Colors: the window theme derived from the config, plus the fixed UI accents shared by
//! every part of the chrome.

use egui::Color32;

use crate::config::Config;

/// `Color32::from_white_alpha`, usable in constants.
pub const fn white_alpha(a: u8) -> Color32 {
    Color32::from_rgba_premultiplied(a, a, a, a)
}

/// Highlights, selections and primary buttons.
pub const ACCENT: Color32 = Color32::from_rgb(0x6c, 0xa8, 0xff);
/// Error text (failed operations, searches with no match).
pub const ERROR: Color32 = Color32::from_rgb(0xf2, 0x77, 0x7a);
/// Background of a hovered button or label.
pub const HOVER: Color32 = white_alpha(16);
/// Background of a hovered round close button (stronger, it's a smaller target).
pub const HOVER_STRONG: Color32 = white_alpha(28);
/// Thin separator lines.
pub const HAIRLINE: Color32 = white_alpha(14);

/// Colors derived from the user's config.
pub struct Theme {
    pub window: Color32,
    pub card: Color32,
    pub text: Color32,
    pub text_bright: Color32,
    pub text_dim: Color32,
}

impl Theme {
    pub fn from_config(cfg: &Config) -> Self {
        let bg = cfg.background;
        let alpha = cfg.background_opacity;
        Theme {
            window: darken(bg, 0.78).gamma_multiply(alpha.max(0.6)),
            card: lighten(bg, 0.03).gamma_multiply(alpha),
            text: cfg.foreground,
            text_bright: Color32::from_rgb(0xf5, 0xf2, 0xf6),
            text_dim: cfg.foreground.gamma_multiply(0.6),
        }
    }
}

pub fn lighten(c: Color32, f: f32) -> Color32 {
    let l = |v: u8| (v as f32 + (255.0 - v as f32) * f) as u8;
    Color32::from_rgba_unmultiplied(l(c.r()), l(c.g()), l(c.b()), c.a())
}

pub fn darken(c: Color32, f: f32) -> Color32 {
    Color32::from_rgba_unmultiplied((c.r() as f32 * f) as u8, (c.g() as f32 * f) as u8, (c.b() as f32 * f) as u8, c.a())
}
