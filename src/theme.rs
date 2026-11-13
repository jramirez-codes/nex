//! Window colors derived from the config.

use egui::Color32;

use crate::config::Config;

pub struct Theme {
    pub window: Color32,
    pub card: Color32,
    pub text: Color32,
    pub text_bright: Color32,
    pub text_dim: Color32,
}

pub fn lighten(c: Color32, f: f32) -> Color32 {
    let l = |v: u8| (v as f32 + (255.0 - v as f32) * f) as u8;
    Color32::from_rgba_unmultiplied(l(c.r()), l(c.g()), l(c.b()), c.a())
}

pub fn darken(c: Color32, f: f32) -> Color32 {
    Color32::from_rgba_unmultiplied((c.r() as f32 * f) as u8, (c.g() as f32 * f) as u8, (c.b() as f32 * f) as u8, c.a())
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
