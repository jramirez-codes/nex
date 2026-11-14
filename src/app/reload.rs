//! Live config reload: on Ctrl+Shift+, or when the config file changes on disk.

use egui::ViewportCommand;

use crate::config::{self, Config};
use crate::fonts;
use crate::ui::glide;

use super::App;

impl App {
    /// Re-read the config and apply it to the running window and terminals. Most settings are
    /// read every frame and need nothing here; `command` only affects shells started later.
    pub(super) fn reload_config(&mut self, ctx: &egui::Context) {
        let old = std::mem::replace(&mut self.cfg, Config::load());
        self.config_mtime = config::modified();
        let cfg = &self.cfg;

        if cfg.font_family != old.font_family {
            fonts::install(ctx, cfg);
        }
        // Per-pane zoom is kept unless the default size itself changed.
        if cfg.font_size != old.font_size {
            self.pane_fonts.clear();
        }
        if cfg.window_decorations != old.window_decorations {
            ctx.send_viewport_cmd(ViewportCommand::Decorations(cfg.window_decorations));
        }
        glide::set_animations(ctx, cfg.animations);
        for t in self.panes.values_mut() {
            t.apply_config(cfg);
        }
        self.update_metrics(ctx);
        ctx.request_repaint();
    }
}
