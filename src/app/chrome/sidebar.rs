//! The sidebar (`tab-bar = sidebar`): the title bar's contents stacked down the left edge
//! as large, touch-sized tiles. Window buttons on top, then the Files tile and the tabs (an
//! app icon, or pane deck, over its title), and the new-tab button at the bottom.

use egui::{FontId, Id, PointerButton, Pos2, Rect, Sense, Ui, Vec2, ViewportCommand};

use crate::ui::theme::{HOVER, Theme};
use crate::ui::widgets::{HoverShape, IconButton};
use crate::ui::{icons, paint};

use crate::app::App;
use crate::app::actions::Action;
use crate::app::chrome::titlebar::{TRACK_PAD, paint_lights, sidebar_tile_parts, toggle_maximized};

pub(in crate::app) const SIDEBAR_WIDTH: f32 = 96.0;
/// A tab or Files tile: a 56pt icon with its title underneath.
const TILE: Vec2 = Vec2::new(82.0, 86.0);
/// The new-tab button.
const PLUS: f32 = 60.0;
/// Gap between the sidebar's edge and the first or last item.
const EDGE: f32 = 14.0;
/// The window lights, relative to their title-bar size.
const LIGHT_SCALE: f32 = 1.6;

impl App {
    /// Laid out top to bottom and registered in that order (later ones win hit tests),
    /// starting with the background drag area.
    pub(in crate::app) fn sidebar(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        let ctx = ui.ctx().clone();
        self.strip.strip_rect = Some(rect);
        let drag = ui.interact(rect, Id::new("sidebar"), Sense::click_and_drag());
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            toggle_maximized(&ctx);
        }

        let cx = rect.center().x;
        let mut y = rect.top() + EDGE;
        if self.cfg.window_controls {
            // Touch has no hover, so the ×, − and + glyphs always show.
            // Side by side, like the title bar, while they fit across the sidebar.
            let step = (rect.width() - 2.0 * EDGE) / 3.0;
            let centers = (0..3).map(|k| Pos2::new(rect.left() + EDGE + step * (k as f32 + 0.5), y + 11.0));
            paint_lights(ui, centers, LIGHT_SCALE, step, true);
            y += 22.0 + EDGE;
        }

        let files = Rect::from_min_size(Pos2::new(cx - TILE.x / 2.0, y), TILE);
        let plus = Rect::from_center_size(Pos2::new(cx, rect.bottom() - EDGE - PLUS / 2.0), Vec2::splat(PLUS));

        // The tab track, between the Files tile and the new-tab button.
        let top = files.bottom() + 8.0;
        let avail = (plus.top() - 8.0 - top - 2.0 * TRACK_PAD).max(TILE.y);
        let visible = ((avail / TILE.y).floor() as usize).clamp(1, self.tabs.len().max(1));
        let track = Rect::from_min_size(
            Pos2::new(cx - TILE.x / 2.0 - TRACK_PAD, top),
            Vec2::new(TILE.x + 2.0 * TRACK_PAD, TILE.y * visible as f32 + 2.0 * TRACK_PAD),
        );
        let slots = self.tab_track(ui, track, TILE, visible, true);
        // The pill glides between the Files tile and the tabs, as in the title bar.
        if let Some(pill) = slots.pill_rect(files) {
            paint::glass_pill(ui.painter(), pill, 14);
        }
        self.files_tile(ui, files, theme);
        self.tab_strip(ui, &slots, theme);

        if IconButton::new(plus, Id::new("new-tab"), theme.text, theme.text_bright)
            .icon_rect(plus.shrink(14.0))
            .hover(HoverShape::Rounded(14), HOVER)
            .tooltip("New tab")
            .show(ui, ui.painter(), icons::icon_plus)
        {
            self.pending.push(Action::NewTab);
        }
    }

    /// The Files tab as a tile: tap toggles it, like the title bar's directory label.
    fn files_tile(&mut self, ui: &Ui, r: Rect, theme: &Theme) {
        self.strip.files_rect = Some(r);
        let painter = ui.painter();
        let active = self.home_active;
        let resp = ui.interact(r, Id::new("workspace-label"), Sense::click());
        if !active && resp.hovered() {
            painter.rect_filled(r, 14, HOVER);
        }
        let files = self.home_is_files();
        let project = self.home_label(files);
        let tip = if files {
            format!("Files: {project} on {}", self.hostname)
        } else {
            format!("Agents: {}", self.agent_tracker.counts().summary())
        };
        let resp = resp.on_hover_text(tip);
        if resp.clicked_by(PointerButton::Middle) {
            self.pending.push(Action::CloseExplorer);
        } else if resp.clicked_by(PointerButton::Primary) {
            self.pending.push(if files { Action::ToggleExplorer } else { Action::ToggleAgentsTab });
        }
        let color = if active { theme.text_bright } else { theme.text_dim };
        let (icon, text) = sidebar_tile_parts(r);
        self.files_dog(ui.ctx(), painter, Rect::from_center_size(icon.center(), Vec2::splat(36.0)), theme);
        paint::caption(painter, text, &project, FontId::proportional(12.0), color);
    }
}
