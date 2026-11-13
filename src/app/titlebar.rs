//! The title bar: window buttons, the workspace (Files) label and the tab strip.

use egui::{Color32, FontId, Id, PointerButton, Pos2, Rect, Sense, Stroke, Ui, Vec2, ViewportCommand};

use crate::ui::theme::{HOVER, HOVER_STRONG, Theme, white_alpha};
use crate::ui::widgets::{HoverShape, IconButton, is_maximized};
use crate::ui::{icons, paint};

use super::App;
use super::actions::Action;

const TAB_HEIGHT: f32 = 30.0;
/// Padding between the tab track's edge and the tabs inside it.
const TRACK_PAD: f32 = 3.0;
const MIN_TAB_WIDTH: f32 = 80.0;
const MAX_TAB_WIDTH: f32 = 232.0;
const PLUS_SIZE: f32 = 28.0;
const LABEL_WIDTH: f32 = 96.0;
const TAB_HOVER: Color32 = white_alpha(10);
const TAB_SEPARATOR: Color32 = white_alpha(24);

/// The macOS-style "traffic light" window buttons.
#[derive(Clone, Copy)]
enum Light {
    Close,
    Minimize,
    Maximize,
}

impl Light {
    const ALL: [Light; 3] = [Light::Close, Light::Minimize, Light::Maximize];

    fn color(self) -> Color32 {
        match self {
            Light::Close => Color32::from_rgb(0xff, 0x5f, 0x57),
            Light::Minimize => Color32::from_rgb(0xfe, 0xbc, 0x2e),
            Light::Maximize => Color32::from_rgb(0x28, 0xc8, 0x40),
        }
    }

    /// The glyph shown on every light while the group is hovered: ×, − or +.
    fn paint_glyph(self, painter: &egui::Painter, center: Pos2) {
        let s = Stroke::new(1.2, Color32::from_black_alpha(150));
        let line = |a: Vec2, b: Vec2| {
            painter.line_segment([center + a, center + b], s);
        };
        match self {
            Light::Close => {
                line(Vec2::new(-2.5, -2.5), Vec2::new(2.5, 2.5));
                line(Vec2::new(-2.5, 2.5), Vec2::new(2.5, -2.5));
            }
            Light::Minimize => line(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0)),
            Light::Maximize => {
                line(Vec2::new(-3.0, 0.0), Vec2::new(3.0, 0.0));
                line(Vec2::new(0.0, -3.0), Vec2::new(0.0, 3.0));
            }
        }
    }

    fn press(self, ctx: &egui::Context) {
        match self {
            Light::Close => ctx.send_viewport_cmd(ViewportCommand::Close),
            Light::Minimize => ctx.send_viewport_cmd(ViewportCommand::Minimized(true)),
            Light::Maximize => toggle_maximized(ctx),
        }
    }
}

fn toggle_maximized(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized(ctx)));
}

/// Where the tabs sit this frame: `shown` slots of `width`, starting at `x0`.
struct TabSlots {
    x0: f32,
    top: f32,
    width: f32,
    shown: usize,
    /// Position of the active pill, in slots (it glides between them). `-1.0` is the
    /// Files label, so the pill glides between it and the tabs too.
    pill: Option<f32>,
}

impl TabSlots {
    fn rect(&self, slot: f32) -> Rect {
        Rect::from_min_size(Pos2::new(self.x0 + slot * self.width, self.top), Vec2::new(self.width, TAB_HEIGHT))
    }

    /// The slot under horizontal position `x`.
    fn slot_at(&self, x: f32) -> usize {
        (((x - self.x0) / self.width).floor().max(0.0) as usize).min(self.shown - 1)
    }

    /// Where the pill is this frame. Between the Files label (`-1.0`) and the first tab it
    /// morphs from the label's rect into the slot's.
    fn pill_rect(&self, label: Rect) -> Option<Rect> {
        let p = self.pill?;
        Some(if p >= 0.0 { self.rect(p) } else { label.lerp_towards(&self.rect(0.0), p + 1.0) })
    }
}

impl App {
    /// Widgets are laid out left to right and registered in that order (later ones win hit
    /// tests), starting with the background drag area.
    pub(super) fn titlebar(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        let ctx = ui.ctx().clone();
        let drag = ui.interact(rect, Id::new("titlebar"), Sense::click_and_drag());
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            toggle_maximized(&ctx);
        }

        let cy = rect.center().y;
        let x = window_controls(ui, rect.left() + 18.0, cy);
        let label_r = Rect::from_min_max(Pos2::new(x - 7.0, cy - 15.0), Pos2::new(x + 30.0 + LABEL_WIDTH, cy + 15.0));
        let slots = self.tab_slots(ui, label_r.right() + 6.0, rect.right() - PLUS_SIZE - 22.0, cy);
        // The active pill is painted once, beneath the label and the tabs, and glides
        // between them.
        if let Some(pill) = slots.pill_rect(label_r) {
            paint::glass_pill(ui.painter(), pill, 8);
        }
        self.workspace_label(ui, label_r, theme);
        self.tab_strip(ui, &slots, theme);
        self.new_tab_button(ui, rect, cy, theme);
    }

    /// The current directory name and hostname. This label *is* the Files tab: clicking it
    /// toggles the tab, and the glass pill glides onto it while Files is showing.
    fn workspace_label(&mut self, ui: &Ui, label_r: Rect, theme: &Theme) {
        let painter = ui.painter();
        let project = self
            .focused_cwd()
            .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
            .unwrap_or_else(|| "~".into());
        let (x, cy) = (label_r.left() + 7.0, label_r.center().y);
        let label = ui.interact(label_r, Id::new("workspace-label"), Sense::click());
        let active = self.explorer_active;
        if !active && label.hovered() {
            painter.rect_filled(label_r, 8, HOVER);
        }
        let tip = if active { "Back to terminal  (middle-click closes)" } else { "Files" };
        let label = label.on_hover_text(tip);
        if label.clicked_by(PointerButton::Middle) {
            self.pending.push(Action::CloseExplorer);
        } else if label.clicked_by(PointerButton::Primary) {
            self.pending.push(Action::ToggleExplorer);
        }

        let glyph_color = if active { theme.text_bright } else { theme.text_dim };
        icons::workspace_glyph(painter, Rect::from_center_size(Pos2::new(x + 8.0, cy), Vec2::new(16.0, 13.0)), glyph_color);
        let x = x + 26.0;
        let project_color = if active { theme.text_bright } else { theme.text };
        paint::faded_text(painter, Pos2::new(x, cy - 7.0), &project, paint::bold(13.0), project_color, LABEL_WIDTH, 16.0);
        paint::faded_text(painter, Pos2::new(x, cy + 8.0), &self.hostname, FontId::proportional(11.0), theme.text_dim, LABEL_WIDTH, 16.0);
    }

    /// Lays out the terminal tabs in a recessed segmented-control style track between `x`
    /// and `right`, paints the track, and advances the pill toward the active tab (or the
    /// Files label).
    fn tab_slots(&mut self, ui: &Ui, x: f32, right: f32, cy: f32) -> TabSlots {
        let ctx = ui.ctx().clone();
        let painter = ui.painter();
        let n = self.tabs.len().max(1);
        let avail = (right - x - 2.0 * TRACK_PAD).max(MIN_TAB_WIDTH);
        let width = (avail / n as f32).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH);
        let visible = ((avail / width).floor() as usize).clamp(1, n);
        let track = Rect::from_min_size(
            Pos2::new(x, cy - TAB_HEIGHT / 2.0 - TRACK_PAD),
            Vec2::new(width * visible as f32 + 2.0 * TRACK_PAD, TAB_HEIGHT + 2.0 * TRACK_PAD),
        );
        paint::tab_track(painter, track);

        let shown = visible.min(self.tabs.len());
        let target = if self.explorer_active { Some(-1.0) } else { (self.active < shown).then_some(self.active as f32) };
        let pill = target.map(|t| self.pill_glide.value(&ctx, t));
        TabSlots { x0: track.left() + TRACK_PAD, top: track.top() + TRACK_PAD, width, shown, pill }
    }

    /// The terminal tabs, drawn into `slots` over the track and pill.
    fn tab_strip(&mut self, ui: &Ui, slots: &TabSlots, theme: &Theme) {
        for i in 0..slots.shown {
            if self.tab_item(ui, slots, i, theme) {
                break;
            }
        }
    }

    /// One tab: icon (or pane deck), title, hover close button; click to show, drag to reorder.
    /// Returns `true` if the tabs were reordered (the rest of the strip is drawn next frame).
    fn tab_item(&mut self, ui: &Ui, slots: &TabSlots, i: usize, theme: &Theme) -> bool {
        let ctx = ui.ctx().clone();
        let painter = ui.painter();
        let r = slots.rect(i as f32);
        let uid = self.tabs[i].uid;
        let resp = ui.interact(r, Id::new(("tab", uid)), Sense::click_and_drag());
        if resp.dragged_by(PointerButton::Primary) {
            if let Some(pos) = resp.interact_pointer_pos() {
                let target = slots.slot_at(pos.x);
                if target != i {
                    let tab = self.tabs.remove(i);
                    self.tabs.insert(target, tab);
                    self.show_tab(target);
                    ctx.request_repaint();
                    return true;
                }
            }
        }
        if resp.drag_started_by(PointerButton::Primary) {
            self.show_tab(i);
        }
        let active = i == self.active && !self.explorer_active;
        let hovered = resp.hovered();
        if !active && hovered {
            painter.rect_filled(r.shrink(1.0), 8, TAB_HOVER);
        }

        // Hairline separators between neighbours, hidden wherever the pill covers them.
        let boundary = (i + 1) as f32;
        let covered = slots.pill.is_some_and(|p| boundary > p - 0.05 && boundary < p + 1.05);
        if i + 1 < slots.shown && !covered {
            let sx = r.right();
            painter.line_segment(
                [Pos2::new(sx, r.center().y - 8.0), Pos2::new(sx, r.center().y + 8.0)],
                Stroke::new(1.0, TAB_SEPARATOR),
            );
        }

        let icon = Rect::from_center_size(Pos2::new(r.left() + 19.0, r.center().y), Vec2::splat(20.0));
        let text_left = self.tab_icon(&ctx, painter, i, icon);
        let (title, _) = self.pane_label(self.tabs[i].focused);
        let close_r = Rect::from_center_size(Pos2::new(r.right() - 15.0, r.center().y), Vec2::splat(18.0));
        let text_right = if hovered { close_r.left() - 2.0 } else { r.right() - 10.0 };
        let title_color = if active { theme.text_bright } else { theme.text_dim };
        paint::faded_text(painter, Pos2::new(text_left, r.center().y), &title, FontId::proportional(13.5), title_color, text_right - text_left, 26.0);

        // Close button only on hover, like Safari/Finder tabs.
        let close_clicked = (hovered || ui.rect_contains_pointer(close_r))
            && IconButton::new(close_r, Id::new(("tab-close", uid)), theme.text_dim, theme.text_bright)
                .icon_rect(close_r.shrink(3.0))
                .hover(HoverShape::Circle(9.0), HOVER_STRONG)
                .show(ui, painter, icons::icon_close);
        if close_clicked || resp.clicked_by(PointerButton::Middle) {
            self.pending.push(Action::CloseTab(i));
        } else if resp.clicked_by(PointerButton::Primary) {
            self.show_tab(i);
        }
        false
    }

    /// The tab's icon: the focused pane's app tile, or a deck with one card per pane (focused
    /// on top, up to 3 peeking out behind). Returns where the title text starts.
    fn tab_icon(&mut self, ctx: &egui::Context, painter: &egui::Painter, i: usize, icon: Rect) -> f32 {
        let (_, process) = self.pane_label(self.tabs[i].focused);
        let (panes, explorers) = (&self.panes, &self.explorers);
        let cards = self.tabs[i].deck_cards(ctx, |id| {
            if explorers.contains_key(&id) {
                icons::EXPLORER_PROCESS.into()
            } else {
                panes.get(&id).map(|t| t.process.clone()).unwrap_or_default()
            }
        });
        let mut text_left = icon.right() + 9.0;
        if cards.len() > 1 {
            let cards: Vec<(&str, f32)> = cards.iter().map(|(p, d)| (p.as_str(), *d)).collect();
            icons::icon_stack(painter, icon, &cards);
            let behind = (cards.len() as f32 - 1.0).min(icons::STACK_DEPTH - 1.0);
            text_left += icons::STACK_STEP.x * behind;
        } else {
            icons::app_tile(painter, icon, &process);
        }
        text_left
    }

    fn new_tab_button(&mut self, ui: &Ui, rect: Rect, cy: f32, theme: &Theme) {
        let plus = Rect::from_center_size(Pos2::new(rect.right() - 14.0 - PLUS_SIZE / 2.0, cy), Vec2::splat(PLUS_SIZE));
        if IconButton::new(plus, Id::new("new-tab"), theme.text, theme.text_bright)
            .hover(HoverShape::Rounded(8), HOVER)
            .show(ui, ui.painter(), icons::icon_plus)
        {
            self.pending.push(Action::NewTab);
        }
    }
}

/// The close / minimize / maximize buttons, starting at `x`. Returns where the next item goes.
fn window_controls(ui: &Ui, x: f32, cy: f32) -> f32 {
    let painter = ui.painter();
    let group = Rect::from_min_size(Pos2::new(x - 8.0, cy - 10.0), Vec2::new(60.0, 20.0));
    let group_hovered = ui.rect_contains_pointer(group);
    let mut x = x;
    for (k, light) in Light::ALL.into_iter().enumerate() {
        let center = Pos2::new(x + 6.0, cy);
        let resp = ui.interact(Rect::from_center_size(center, Vec2::splat(14.0)), Id::new(("light", k)), Sense::click());
        painter.circle_filled(center, 6.0, light.color());
        if group_hovered {
            light.paint_glyph(painter, center);
        }
        if resp.clicked_by(PointerButton::Primary) {
            light.press(ui.ctx());
        }
        x += 20.0;
    }
    x + 18.0
}
