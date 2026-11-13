//! The title bar: window buttons, the workspace (Files) label and the tab strip.


use egui::{
    Color32, FontId, Id, PointerButton, Pos2, Rect, Sense, Stroke, Ui,
    Vec2, ViewportCommand,
};

use crate::chrome::{self, bold};

use crate::theme::Theme;
use super::App;
use super::actions::Action;

impl App {
    pub(super) fn titlebar(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        let ctx = ui.ctx().clone();
        let painter = ui.painter().clone();

        // Background drag area (registered first so tabs & buttons placed later win hit tests).
        let drag = ui.interact(rect, Id::new("titlebar"), Sense::click_and_drag());
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
            ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
        }

        let cy = rect.center().y;
        let mut x = rect.left() + 18.0;

        // Window controls (close / minimize / maximize), macOS style.
        let lights = [
            (Color32::from_rgb(0xff, 0x5f, 0x57), "close"),
            (Color32::from_rgb(0xfe, 0xbc, 0x2e), "min"),
            (Color32::from_rgb(0x28, 0xc8, 0x40), "max"),
        ];
        let group = Rect::from_min_size(Pos2::new(x - 8.0, cy - 10.0), Vec2::new(60.0, 20.0));
        let group_hovered = ui.rect_contains_pointer(group);
        for (color, what) in lights {
            let center = Pos2::new(x + 6.0, cy);
            let r = Rect::from_center_size(center, Vec2::splat(14.0));
            let resp = ui.interact(r, Id::new(("light", what)), Sense::click());
            painter.circle_filled(center, 6.0, color);
            if group_hovered {
                let s = Stroke::new(1.2, Color32::from_black_alpha(150));
                match what {
                    "close" => {
                        painter.line_segment([center + Vec2::new(-2.5, -2.5), center + Vec2::new(2.5, 2.5)], s);
                        painter.line_segment([center + Vec2::new(-2.5, 2.5), center + Vec2::new(2.5, -2.5)], s);
                    }
                    "min" => {
                        painter.line_segment([center + Vec2::new(-3.0, 0.0), center + Vec2::new(3.0, 0.0)], s);
                    }
                    _ => {
                        painter.line_segment([center + Vec2::new(-3.0, 0.0), center + Vec2::new(3.0, 0.0)], s);
                        painter.line_segment([center + Vec2::new(0.0, -3.0), center + Vec2::new(0.0, 3.0)], s);
                    }
                }
            }
            if resp.clicked_by(PointerButton::Primary) {
                match what {
                    "close" => ctx.send_viewport_cmd(ViewportCommand::Close),
                    "min" => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
                    _ => {
                        let maximized = ctx.input(|i| i.viewport().maximized.unwrap_or(false));
                        ctx.send_viewport_cmd(ViewportCommand::Maximized(!maximized));
                    }
                }
            }
            x += 20.0;
        }
        x += 18.0;

        // Workspace label: current directory name + hostname. Clicking it toggles the Files tab.
        let project = self
            .focused_cwd()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".into());
        // The label *is* the Files tab: it gets the glass pill while Files is showing.
        let label_w = 96.0;
        let label_r = Rect::from_min_max(Pos2::new(x - 7.0, cy - 15.0), Pos2::new(x + 26.0 + label_w + 4.0, cy + 15.0));
        let label = ui.interact(label_r, Id::new("workspace-label"), Sense::click());
        let files_active = self.files_active;
        if files_active {
            chrome::glass_pill(&painter, label_r, 8);
        } else if label.hovered() {
            painter.rect_filled(label_r, 8, Color32::from_white_alpha(14));
        }
        let label_tip = if files_active { "Back to terminal  (middle-click closes)" } else { "Files" };
        let label = label.on_hover_text(label_tip);
        if label.clicked_by(PointerButton::Middle) {
            self.pending.push(Action::CloseFiles);
        } else if label.clicked_by(PointerButton::Primary) {
            self.pending.push(Action::ToggleFiles);
        }
        let glyph_color = if files_active { theme.text_bright } else { theme.text_dim };
        chrome::workspace_glyph(&painter, Rect::from_center_size(Pos2::new(x + 8.0, cy), Vec2::new(16.0, 13.0)), glyph_color);
        x += 26.0;
        let project_color = if files_active { theme.text_bright } else { theme.text };
        chrome::faded_text(&painter, Pos2::new(x, cy - 7.0), &project, bold(13.0), project_color, label_w, 16.0);
        chrome::faded_text(&painter, Pos2::new(x, cy + 8.0), &self.hostname, FontId::proportional(11.0), theme.text_dim, label_w, 16.0);
        x += label_w + 10.0;

        // Tabs live in a recessed, segmented-control style track.
        let plus_size = 28.0;
        let tabs_right = rect.right() - plus_size - 22.0;
        let pad = 3.0;
        let tab_h = 30.0;
        let n = self.tabs.len().max(1);
        let avail = (tabs_right - x - 2.0 * pad).max(80.0);
        let tab_w = (avail / n as f32).clamp(80.0, 232.0);
        let visible = ((avail / tab_w).floor() as usize).clamp(1, n);
        let track = Rect::from_min_size(
            Pos2::new(x, cy - tab_h / 2.0 - pad),
            Vec2::new(tab_w * visible as f32 + 2.0 * pad, tab_h + 2.0 * pad),
        );
        chrome::tab_track(&painter, track);
        let x0 = track.left() + pad;
        let shown = visible.min(self.tabs.len());

        // The active pill is painted once, beneath the tabs, and glides between slots.
        let pill = (!self.files_active && self.active < shown).then(|| self.pill_glide.value(&ctx, self.active as f32));
        if let Some(slot) = pill {
            let r = Rect::from_min_size(Pos2::new(x0 + slot * tab_w, track.top() + pad), Vec2::new(tab_w, tab_h));
            chrome::glass_pill(&painter, r, 8);
        }

        for i in 0..shown {
            let r = Rect::from_min_size(Pos2::new(x0 + i as f32 * tab_w, track.top() + pad), Vec2::new(tab_w, tab_h));
            let uid = self.tabs[i].uid;
            let resp = ui.interact(r, Id::new(("tab", uid)), Sense::click_and_drag());
            if resp.dragged_by(PointerButton::Primary) {
                // Drag to reorder: move to the slot under the pointer.
                if let Some(pos) = resp.interact_pointer_pos() {
                    let target = (((pos.x - x0) / tab_w).floor().max(0.0) as usize).min(shown - 1);
                    if target != i {
                        let tab = self.tabs.remove(i);
                        self.tabs.insert(target, tab);
                        self.show_tab(target);
                        ctx.request_repaint();
                        break;
                    }
                }
            }
            if resp.drag_started_by(PointerButton::Primary) {
                self.show_tab(i);
            }
            let active = i == self.active && !self.files_active;
            let hovered = resp.hovered();
            if !active && hovered {
                painter.rect_filled(r.shrink(1.0), 8, Color32::from_white_alpha(10));
            }

            // Hairline separators between neighbours, hidden wherever the pill covers them.
            let boundary = (i + 1) as f32;
            let covered = pill.is_some_and(|p| boundary > p - 0.05 && boundary < p + 1.05);
            if i + 1 < shown && !covered {
                let sx = r.right();
                painter.line_segment(
                    [Pos2::new(sx, r.center().y - 8.0), Pos2::new(sx, r.center().y + 8.0)],
                    Stroke::new(1.0, Color32::from_white_alpha(24)),
                );
            }

            let (title, process) = self.pane_label(self.tabs[i].focused);
            let (panes, browsers) = (&self.panes, &self.browsers);
            let tab = &mut self.tabs[i];
            let icon = Rect::from_center_size(Pos2::new(r.left() + 19.0, r.center().y), Vec2::splat(20.0));
            let cards = tab.deck_cards(&ctx, |id| {
                if browsers.contains_key(&id) {
                    chrome::FILES_PROCESS.into()
                } else {
                    panes.get(&id).map(|t| t.process.clone()).unwrap_or_default()
                }
            });
            let mut text_left = icon.right() + 9.0;
            if cards.len() > 1 {
                // One card per pane, focused on top; up to 3 peek out behind it.
                let cards: Vec<(&str, f32)> = cards.iter().map(|(p, d)| (p.as_str(), *d)).collect();
                chrome::icon_stack(&painter, icon, &cards);
                let behind = (cards.len() as f32 - 1.0).min(chrome::STACK_DEPTH - 1.0);
                text_left += chrome::STACK_STEP.x * behind;
            } else {
                chrome::app_tile(&painter, icon, &process);
            }

            let close_r = Rect::from_center_size(Pos2::new(r.right() - 15.0, r.center().y), Vec2::splat(18.0));
            let text_right = if hovered { close_r.left() - 2.0 } else { r.right() - 10.0 };
            chrome::faded_text(
                &painter,
                Pos2::new(text_left, r.center().y),
                &title,
                FontId::proportional(13.5),
                if active { theme.text_bright } else { theme.text_dim },
                text_right - text_left,
                26.0,
            );

            // Close button only on hover, like Safari/Finder tabs.
            let mut close_clicked = false;
            if hovered || ui.rect_contains_pointer(close_r) {
                let cresp = ui.interact(close_r, Id::new(("tab-close", uid)), Sense::click());
                if cresp.hovered() {
                    painter.circle_filled(close_r.center(), 9.0, Color32::from_white_alpha(28));
                }
                chrome::icon_close(&painter, close_r.shrink(3.0), if cresp.hovered() { theme.text_bright } else { theme.text_dim });
                close_clicked = cresp.clicked_by(PointerButton::Primary);
            }
            if close_clicked || resp.clicked_by(PointerButton::Middle) {
                self.pending.push(Action::CloseTab(i));
            } else if resp.clicked_by(PointerButton::Primary) {
                self.show_tab(i);
            }
        }

        // New tab button.
        let plus = Rect::from_center_size(Pos2::new(rect.right() - 14.0 - plus_size / 2.0, cy), Vec2::splat(plus_size));
        let presp = ui.interact(plus, Id::new("new-tab"), Sense::click());
        if presp.hovered() {
            painter.rect_filled(plus, 8, Color32::from_white_alpha(14));
        }
        chrome::icon_plus(&painter, plus, if presp.hovered() { theme.text_bright } else { theme.text });
        if presp.clicked_by(PointerButton::Primary) {
            self.pending.push(Action::NewTab);
        }
    }
}
