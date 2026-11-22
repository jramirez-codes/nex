//! The title bar: window buttons, the workspace (Files) label and the tab strip.

use std::collections::HashMap;

use egui::emath::TSTransform;
use egui::{Color32, FontId, Id, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, ViewportCommand};

use crate::layout::{PaneId, PaneKind};
use crate::platform;
use crate::ui::theme::{HOVER, HOVER_STRONG, Theme, white_alpha};
use crate::ui::widgets::{HoverShape, IconButton, is_fullscreen, is_maximized};
use crate::ui::{dog, glide, icons, paint};

use crate::app::App;
use crate::app::actions::Action;
use crate::app::workspace::pane_drag::{DropTarget, SPRING_DELAY};
use crate::app::animation::tab_close::{GLYPH, TabDrawn};

const TAB_HEIGHT: f32 = 30.0;
/// Padding between the tab track's edge and the tabs inside it.
pub(in crate::app) const TRACK_PAD: f32 = 3.0;
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

    /// The glyph shown on every light while the group is hovered: ×, − or +. `scale` is the
    /// light's size relative to the title bar's.
    fn paint_glyph(self, painter: &egui::Painter, center: Pos2, scale: f32) {
        let s = Stroke::new(1.2 * scale, Color32::from_black_alpha(150));
        let line = |a: Vec2, b: Vec2| {
            painter.line_segment([center + a * scale, center + b * scale], s);
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

pub(in crate::app) fn toggle_maximized(ctx: &egui::Context) {
    ctx.send_viewport_cmd(ViewportCommand::Maximized(!is_maximized(ctx)));
}

/// Where the tabs sit this frame: `shown` slots of `size`, starting at `origin` and running
/// right (title bar) or down (mobile sidebar).
pub(in crate::app) struct TabSlots {
    origin: Pos2,
    size: Vec2,
    vertical: bool,
    shown: usize,
    /// Position of the active pill, in slots (it glides between them). `-1.0` is the
    /// Files label, so the pill glides between it and the tabs too.
    pill: Option<f32>,
    /// Slots left by closing tabs, still closing up: `(index, fraction of a slot)`. Tabs from
    /// `index` on are pushed along by that much.
    gaps: Vec<(usize, f32)>,
    /// The pill as painted this frame, if it is somewhere other than `pill` (mid close).
    pill_shape: Option<Rect>,
}

impl TabSlots {
    fn rect(&self, slot: f32) -> Rect {
        let step = if self.vertical { Vec2::new(0.0, self.size.y) } else { Vec2::new(self.size.x, 0.0) };
        // Fractional slots (the gliding pill) ease across a gap rather than jumping it.
        let held: f32 = self.gaps.iter().map(|&(i, f)| f * (slot - i as f32 + 1.0).clamp(0.0, 1.0)).sum();
        Rect::from_min_size(self.origin + step * (slot + held), self.size)
    }

    /// The slot under `pos` (along the strip's direction).
    fn slot_at(&self, pos: Pos2) -> usize {
        let (along, end) = if self.vertical { (pos.y, Rect::bottom as fn(&Rect) -> f32) } else { (pos.x, Rect::right as fn(&Rect) -> f32) };
        (0..self.shown).find(|&k| along < end(&self.rect(k as f32))).unwrap_or(self.shown - 1)
    }

    /// Whether the pill covers the separator at `at` (along the strip), just after slot
    /// `boundary - 1`.
    fn pill_covers(&self, boundary: f32, at: f32) -> bool {
        match self.pill_shape {
            Some(p) if self.vertical => at > p.top() - 1.0 && at < p.bottom() + 1.0,
            Some(p) => at > p.left() - 1.0 && at < p.right() + 1.0,
            None => self.pill.is_some_and(|p| boundary > p - 0.05 && boundary < p + 1.05),
        }
    }

    /// Where the pill is this frame. Between the Files label (`-1.0`) and the first tab it
    /// morphs from the label's rect into the slot's.
    pub(in crate::app) fn pill_rect(&self, label: Rect) -> Option<Rect> {
        let p = self.pill?;
        Some(if p >= 0.0 { self.rect(p) } else { label.lerp_towards(&self.rect(0.0), p + 1.0) })
    }
}

impl App {
    /// Widgets are laid out left to right and registered in that order (later ones win hit
    /// tests), starting with the background drag area.
    pub(in crate::app) fn titlebar(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        let ctx = ui.ctx().clone();
        self.strip_rect = Some(rect);
        let drag = ui.interact(rect, Id::new("titlebar"), Sense::click_and_drag());
        if drag.drag_started_by(PointerButton::Primary) {
            ctx.send_viewport_cmd(ViewportCommand::StartDrag);
        }
        if drag.double_clicked() {
            toggle_maximized(&ctx);
        }

        let cy = rect.center().y;
        // Leave room for window buttons the OS draws over the title bar (macOS traffic lights),
        // which it hides in full screen.
        let overlaid = self.cfg.window_decorations && !is_fullscreen(&ctx);
        let inset = if overlaid { platform::WINDOW.titlebar_inset } else { 0.0 };
        let left = rect.left() + 18.0 + inset;
        let x = if self.cfg.window_controls { window_controls(ui, left, cy) } else { left };
        let label_r = Rect::from_min_max(Pos2::new(x - 7.0, cy - 15.0), Pos2::new(x + 30.0 + LABEL_WIDTH, cy + 15.0));
        let now = ctx.input(|i| i.time);
        self.advance_tab_ghosts(&ctx, now);
        let mut slots = self.horizontal_tab_slots(ui, label_r.right() + 6.0, rect.right() - PLUS_SIZE - 22.0, cy, now);
        // The active pill is painted once, beneath the label and the tabs, and glides
        // between them. While the active tab closes it folds into that tab's × and grows
        // back out onto the tab now showing.
        let target = slots.pill_rect(label_r);
        let pill = match self.tab_ghosts.iter().rev().find(|g| g.drawn.active) {
            Some(g) => g.pill(now, target),
            None => target.map(|r| (r, 8.0)),
        };
        slots.pill_shape = pill.map(|(r, _)| r);
        if let Some((r, radius)) = pill {
            let radius = radius.min(r.width().min(r.height()) / 2.0).max(0.0);
            paint::glass_pill(ui.painter(), r, radius.round() as u8);
        }
        self.workspace_label(ui, label_r, theme);
        self.tab_strip(ui, &slots, theme);
        self.paint_tab_ghosts(ui, now, theme);
        self.new_tab_button(ui, rect, cy, theme);
    }

    /// Start closed tabs' animations on their first frame and drop finished ones.
    fn advance_tab_ghosts(&mut self, ctx: &egui::Context, now: f64) {
        if !glide::animations(ctx) {
            self.tab_ghosts.clear();
            return;
        }
        for g in &mut self.tab_ghosts {
            g.start.get_or_insert(now);
        }
        self.tab_ghosts.retain(|g| !g.done(now));
        if !self.tab_ghosts.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Closed tabs going into their ×. A background tab (tint, icon and title) is painted as
    /// it was and then scaled, as one piece, toward the × center; the active tab's title and
    /// icon fade under the folding pill (painted already). The × itself goes last.
    fn paint_tab_ghosts(&self, ui: &Ui, now: f64, theme: &Theme) {
        let painter = ui.painter();
        for g in &self.tab_ghosts {
            let (f, d) = (g.frame(now), &g.drawn);
            if f.scale > 0.0 && (f.content > 0.0 || f.tint > 0.0) {
                let start = painter.add(Shape::Noop);
                if f.tint > 0.0 {
                    let fill = TAB_HOVER.gamma_multiply(f.tint);
                    painter.rect_filled(d.rect.shrink(1.0), 8, fill);
                    painter.rect_stroke(d.rect.shrink(1.0), 8, Stroke::new(1.0_f32, TAB_SEPARATOR.gamma_multiply(f.tint * 0.6)), StrokeKind::Inside);
                }
                if f.content > 0.0 {
                    let mut p = painter.clone();
                    p.multiply_opacity(f.content);
                    icons::app_tile(&p, d.icon, &g.process, g.mark.as_deref());
                    let color = if d.active { theme.text_bright } else { theme.text_dim };
                    let width = d.close.left() - 2.0 - d.text_left;
                    paint::faded_text(&p, Pos2::new(d.text_left, d.rect.center().y), &g.title, FontId::proportional(13.5), color, width, 26.0);
                }
                let end = painter.add(Shape::Noop);
                if f.scale < 1.0 {
                    // Scale about the × center: x' = s·x + (1 − s)·pivot.
                    let shrink = TSTransform::new(g.pivot().to_vec2() * (1.0 - f.scale), f.scale);
                    ui.ctx().graphics_mut(|l| l.entry(painter.layer_id()).transform_range(start, end, shrink));
                }
            }
            if f.glyph > 0.0 {
                let r = Rect::from_center_size(g.pivot(), Vec2::splat(GLYPH * f.glyph_scale));
                icons::icon_close(painter, r, theme.text_bright.gamma_multiply(f.glyph));
            }
        }
    }

    /// The Agents tab's label: "Agents" over a live summary ("2 working · 1 needs you"), or,
    /// while that tab is a file explorer (`--explorer`, a folder argument), the current
    /// directory name and hostname. Clicking it toggles the tab, and the glass pill glides
    /// onto it while the tab is showing.
    fn workspace_label(&mut self, ui: &Ui, label_r: Rect, theme: &Theme) {
        self.files_rect = Some(label_r);
        let painter = ui.painter();
        let (x, cy) = (label_r.left() + 7.0, label_r.center().y);
        let label = ui.interact(label_r, Id::new("workspace-label"), Sense::click());
        let active = self.explorer_active;
        if !active && label.hovered() {
            painter.rect_filled(label_r, 8, HOVER);
        }
        let files = self.home_is_files();
        let tip = match (active, files) {
            (true, _) => "Back to terminal  (middle-click closes)",
            (false, true) => "Files",
            (false, false) => "Agents",
        };
        let label = label.on_hover_text(tip);
        if label.clicked_by(PointerButton::Middle) {
            self.pending.push(Action::CloseExplorer);
        } else if label.clicked_by(PointerButton::Primary) {
            self.pending.push(if files { Action::ToggleExplorer } else { Action::ToggleAgentsTab });
        }

        self.files_dog(ui.ctx(), painter, Rect::from_center_size(Pos2::new(x + 9.0, cy), Vec2::splat(21.0)), theme);
        let x = x + 26.0;
        let title_color = if active { theme.text_bright } else { theme.text };
        let (title, detail, detail_color) = if files {
            let project = self
                .focused_cwd()
                .and_then(|p| p.file_name().map(|n| n.to_string_lossy().to_string()))
                .unwrap_or_else(|| "~".into());
            (project, self.hostname.clone(), theme.text_dim)
        } else {
            let counts = self.agent_tracker.counts();
            let color = if counts.needs_you > 0 { theme.yellow } else { theme.text_dim };
            ("Agents".to_string(), counts.headline(), color)
        };
        paint::faded_text(painter, Pos2::new(x, cy - 7.0), &title, paint::bold(13.0), title_color, LABEL_WIDTH, 16.0);
        paint::faded_text(painter, Pos2::new(x, cy + 8.0), &detail, FontId::proportional(11.0), detail_color, LABEL_WIDTH, 16.0);
    }

    /// The nex dog marking the Files tab (title-bar label or sidebar tile). It wakes up at
    /// launch, sticks its tongue out each time Files is shown, and rests dimmed while Files
    /// is hidden.
    pub(in crate::app) fn files_dog(&mut self, ctx: &egui::Context, painter: &egui::Painter, r: Rect, theme: &Theme) {
        let now = ctx.input(|i| i.time);
        let active = self.explorer_active;
        match self.dog_wake {
            None => self.dog_wake = Some(now),
            // Switching to Files plays just the blep, unless the launch intro hasn't reached
            // it yet (it's on its way).
            Some(start) if active && !self.dog_was_active && now - start >= dog::BLEP_START as f64 => {
                self.dog_wake = Some(now - dog::BLEP_START as f64);
            }
            Some(_) => {}
        }
        self.dog_was_active = active;
        let t = if glide::animations(ctx) { (now - self.dog_wake.unwrap_or(now)) as f32 } else { dog::INTRO };
        if t < dog::INTRO {
            ctx.request_repaint();
        }
        dog::paint(painter, r, t, (theme.window, if active { 0.0 } else { 0.3 }));
    }

    /// Lays out the terminal tabs in a recessed segmented-control style track between `x`
    /// and `right`, paints the track, and advances the pill toward the active tab (or the
    /// Files label).
    /// Slots left by closing tabs count as part-tabs while they close up, so the other tabs
    /// widen and the track narrows smoothly.
    fn horizontal_tab_slots(&mut self, ui: &Ui, x: f32, right: f32, cy: f32, now: f64) -> TabSlots {
        let gaps: Vec<(usize, f32)> =
            self.tab_ghosts.iter().map(|g| (g.index, g.frame(now).gap)).filter(|&(_, f)| f > 0.0).collect();
        let held: f32 = gaps.iter().map(|&(_, f)| f).sum();
        let n = self.tabs.len();
        let avail = (right - x - 2.0 * TRACK_PAD).max(MIN_TAB_WIDTH);
        let width = (avail / (n as f32 + held).max(1.0)).clamp(MIN_TAB_WIDTH, MAX_TAB_WIDTH);
        let visible = ((avail / width - held).floor() as usize).clamp(1, n.max(1));
        let across = (visible.min(n) as f32 + held).max(1.0).min(avail / width);
        let track = Rect::from_min_size(
            Pos2::new(x, cy - TAB_HEIGHT / 2.0 - TRACK_PAD),
            Vec2::new(width * across + 2.0 * TRACK_PAD, TAB_HEIGHT + 2.0 * TRACK_PAD),
        );
        let mut slots = self.tab_track(ui, track, Vec2::new(width, TAB_HEIGHT), visible, false);
        slots.gaps = gaps;
        slots
    }

    /// Paints `track` (holding `visible` slots of `size`) and advances the pill toward the
    /// active tab, or the Files label.
    pub(in crate::app) fn tab_track(&mut self, ui: &Ui, track: Rect, size: Vec2, visible: usize, vertical: bool) -> TabSlots {
        let ctx = ui.ctx().clone();
        paint::tab_track(ui.painter(), track);
        let shown = visible.min(self.tabs.len());
        let target = if self.explorer_active { Some(-1.0) } else { (self.active < shown).then_some(self.active as f32) };
        // While the active tab closes, the pill's path is the close animation's: be at the
        // new tab already when it hands back.
        let folding = self.tab_ghosts.iter().any(|g| g.drawn.active);
        let pill = target.map(|t| {
            if folding {
                self.pill_glide.snap(&ctx, t);
                t
            } else {
                self.pill_glide.value(&ctx, t)
            }
        });
        let origin = track.min + Vec2::splat(TRACK_PAD);
        TabSlots { origin, size, vertical, shown, pill, gaps: Vec::new(), pill_shape: None }
    }

    /// The terminal tabs, drawn into `slots` over the track and pill.
    pub(in crate::app) fn tab_strip(&mut self, ui: &Ui, slots: &TabSlots, theme: &Theme) {
        self.tab_rects.clear();
        self.tab_hits.clear();
        for i in 0..slots.shown {
            if self.tab_item(ui, slots, i, theme) {
                break;
            }
        }
        // A pane dragged onto the strip's empty space: a dashed slot for its new tab, just
        // past the last one (or, with no room, around the whole track).
        if self.pane_drag.as_ref().is_some_and(|d| d.over == Some(DropTarget::NewTab)) && slots.shown > 0 {
            let next = slots.rect(slots.shown as f32).shrink(1.0);
            let r = match self.strip_rect {
                Some(strip) if strip.contains_rect(next) => next,
                _ => slots.rect(0.0).union(slots.rect(slots.shown as f32 - 1.0)),
            };
            dashed_slot(ui.painter(), r, theme);
        }
    }

    /// Marks a tab a pane is dragged over: tinted, with a bar filling up until it opens.
    fn paint_tab_drop_hint(&self, ui: &Ui, uid: u64, r: Rect, theme: &Theme) {
        let Some(d) = self.pane_drag.as_ref().filter(|d| d.over == Some(DropTarget::Tab(uid))) else { return };
        let painter = ui.painter();
        painter.rect_filled(r.shrink(1.0), 8, HOVER_STRONG);
        if let Some((_, since)) = d.hover.filter(|(u, _)| *u == uid) {
            let now = ui.input(|i| i.time);
            let f = (((now - since) / SPRING_DELAY) as f32).clamp(0.0, 1.0);
            let track = Rect::from_min_max(Pos2::new(r.left() + 10.0, r.bottom() - 4.0), Pos2::new(r.right() - 10.0, r.bottom() - 2.0));
            painter.rect_filled(track, 1, TAB_SEPARATOR);
            painter.rect_filled(Rect::from_min_max(track.min, Pos2::new(track.left() + track.width() * f, track.bottom())), 1, theme.text_bright);
        }
    }

    /// One tab: icon (or pane deck), title, hover close button; click to show, drag to reorder.
    /// In the vertical (mobile) strip a tab is just a large icon, its title a tooltip.
    /// Returns `true` if the tabs were reordered (the rest of the strip is drawn next frame).
    fn tab_item(&mut self, ui: &Ui, slots: &TabSlots, i: usize, theme: &Theme) -> bool {
        let ctx = ui.ctx().clone();
        let painter = ui.painter();
        let r = slots.rect(i as f32);
        let uid = self.tabs[i].uid;
        let resp = ui.interact(r, Id::new(("tab", uid)), Sense::click_and_drag());
        if resp.dragged_by(PointerButton::Primary)
            && let Some(pos) = resp.interact_pointer_pos() {
                let target = slots.slot_at(pos);
                if target != i {
                    let tab = self.tabs.remove(i);
                    self.tabs.insert(target, tab);
                    self.show_tab(target);
                    ctx.request_repaint();
                    return true;
                }
            }
        if resp.drag_started_by(PointerButton::Primary) {
            self.show_tab(i);
        }
        self.tab_hits.push((uid, r));
        let active = i == self.active && !self.explorer_active;
        let hovered = resp.hovered();
        if !active && hovered {
            painter.rect_filled(r.shrink(1.0), 8, TAB_HOVER);
        }
        self.paint_tab_drop_hint(ui, uid, r, theme);

        // Hairline separators between neighbours, hidden wherever the pill covers them.
        let boundary = (i + 1) as f32;
        let covered = slots.pill_covers(boundary, if slots.vertical { r.bottom() } else { r.right() });
        if i + 1 < slots.shown && !covered {
            let seg = if slots.vertical {
                let sy = r.bottom();
                [Pos2::new(r.center().x - 10.0, sy), Pos2::new(r.center().x + 10.0, sy)]
            } else {
                let sx = r.right();
                [Pos2::new(sx, r.center().y - 8.0), Pos2::new(sx, r.center().y + 8.0)]
            };
            painter.line_segment(seg, Stroke::new(1.0_f32, TAB_SEPARATOR));
        }

        if slots.vertical {
            // A large icon (or pane deck) with the title underneath.
            let (icon, text) = sidebar_tile_parts(r);
            // A tad smaller than the tile's icon square, leaving room for the deck to fan out.
            let icon = Rect::from_center_size(icon.center(), icon.size() * 0.84);
            self.tab_icon(&ctx, painter, i, icon, icons::SIDEBAR_STACK_STEP, true);
            if self.tab_needs_attention(i) {
                paint_attention(&ctx, painter, icon.left_top() + Vec2::splat(2.0), self.cfg.attention_color(), theme, self.cfg.animations);
            }
            let (title, process) = self.pane_label(self.tabs[i].focused);
            let title = icons::title_mark(&process, &title).1.to_string();
            let color = if active { theme.text_bright } else { theme.text_dim };
            paint::caption(painter, text, &title, FontId::proportional(12.0), color);
            let resp = resp.on_hover_text(title);
            if resp.clicked_by(PointerButton::Middle) {
                self.pending.push(Action::CloseTab(i));
            } else if resp.clicked_by(PointerButton::Primary) {
                self.show_tab(i);
            }
            return false;
        }

        let icon = Rect::from_center_size(Pos2::new(r.left() + 19.0, r.center().y), Vec2::splat(20.0));
        let text_left = self.tab_icon(&ctx, painter, i, icon, icons::STACK_STEP, false);
        let (title, process) = self.pane_label(self.tabs[i].focused);
        let title = icons::title_mark(&process, &title).1;
        let close_r = Rect::from_center_size(Pos2::new(r.right() - 15.0, r.center().y), Vec2::splat(18.0));
        self.tab_rects.insert(uid, TabDrawn { rect: r, close: close_r, icon, text_left, active });
        let text_right = if hovered { close_r.left() - 2.0 } else { r.right() - 10.0 };
        let title_color = if active { theme.text_bright } else { theme.text_dim };
        paint::faded_text(painter, Pos2::new(text_left, r.center().y), title, FontId::proportional(13.5), title_color, text_right - text_left, 26.0);

        if self.tab_needs_attention(i) {
            paint_attention(&ctx, painter, icon.left_top() + Vec2::splat(1.0), self.cfg.attention_color(), theme, self.cfg.animations);
        }

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

    /// The app tile pane `id` shows in its tab's icon deck: its process and title mark.
    fn deck_card(&self, id: PaneId) -> (String, Option<String>) {
        match self.pane_kind(id) {
            Some(PaneKind::Terminal) => {
                let t = &self.panes[&id];
                let title = t.title();
                (t.process.clone(), icons::title_mark(&t.process, &title).0.map(str::to_string))
            }
            Some(PaneKind::Explorer) => (icons::EXPLORER_PROCESS.to_string(), None),
            Some(PaneKind::Diff) => (icons::DIFF_PROCESS.to_string(), None),
            Some(PaneKind::Agents) => (icons::AGENTS_PROCESS.to_string(), None),
            None => Default::default(),
        }
    }

    /// The tab's icon: the focused pane's app tile, or a deck with one card per pane (focused
    /// on top, up to 3 peeking out behind, each `step` further). With `center`, the whole deck
    /// is centered on `icon` rather than fanning out from it. Returns where the title text
    /// starts.
    fn tab_icon(&mut self, ctx: &egui::Context, painter: &egui::Painter, i: usize, icon: Rect, step: Vec2, center: bool) -> f32 {
        let (title, process) = self.pane_label(self.tabs[i].focused);
        let mark = icons::title_mark(&process, &title).0.map(str::to_string);
        let card_of: HashMap<PaneId, (String, Option<String>)> =
            self.tabs[i].root.panes().into_iter().map(|id| (id, self.deck_card(id))).collect();
        let cards = self.tabs[i].deck_cards(ctx, |id| card_of.get(&id).cloned().unwrap_or_default());
        let mut text_left = icon.right() + 9.0;
        if cards.len() > 1 {
            let cards: Vec<(&str, Option<&str>, f32)> = cards.iter().map(|((p, m), d)| (p.as_str(), m.as_deref(), *d)).collect();
            let behind = (cards.len() as f32 - 1.0).min(icons::STACK_DEPTH - 1.0);
            let icon = if center { icon.translate(-step * behind / 2.0) } else { icon };
            icons::icon_stack(painter, icon, &cards, step);
            text_left += step.x * behind;
        } else {
            icons::app_tile(painter, icon, &process, mark.as_deref());
        }
        text_left
    }

    /// Whether a pane in tab `i` rang the bell or sent a notification the user hasn't seen.
    fn tab_needs_attention(&self, i: usize) -> bool {
        // Not the sticky pane, which every tab has.
        self.tabs[i].root.panes().iter().filter(|&&id| !self.is_sticky(id)).any(|id| self.panes.get(id).is_some_and(|t| t.attention))
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

/// One pulse of the attention dot, in seconds.
const ATTENTION_PULSE: f64 = 1.6;

/// The dot on a tab whose program is waiting for the user, on its icon's top-left corner and
/// ringed so it stands out on the icon and the active pill. With `animate`, it gently swells
/// and sends out a fading ring each [`ATTENTION_PULSE`].
fn paint_attention(ctx: &egui::Context, painter: &egui::Painter, center: Pos2, color: Color32, theme: &Theme, animate: bool) {
    let mut radius = 4.0;
    if animate {
        let phase = (ctx.input(|i| i.time) % ATTENTION_PULSE / ATTENTION_PULSE) as f32;
        painter.circle_filled(center, radius + 5.0 * phase, color.gamma_multiply(0.4 * (1.0 - phase)));
        radius += 0.6 * (phase * std::f32::consts::TAU).sin();
        // ~30 fps is plenty for a slow pulse, and only while a dot is showing.
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }
    painter.circle(center, radius, color, Stroke::new(1.5_f32, theme.window));
}

/// Where a pane dropped on the tab strip's empty space would open: a faint, dashed slot.
fn dashed_slot(painter: &egui::Painter, r: Rect, theme: &Theme) {
    // Rounded like the tabs (and their hover tint).
    painter.rect_filled(r, 8, TAB_HOVER);
    let mut outline = Vec::new();
    egui::epaint::tessellator::path::rounded_rectangle(&mut outline, r.shrink(0.5), egui::epaint::CornerRadiusF32::same(8.0));
    if let Some(&first) = outline.first() {
        outline.push(first);
    }
    let stroke = Stroke::new(1.0_f32, theme.text_dim);
    painter.extend(Shape::dashed_line(&outline, stroke, 4.0, 3.0));
}

/// Splits a mobile-sidebar tile into its icon square and the caption line below it.
pub(in crate::app) fn sidebar_tile_parts(r: Rect) -> (Rect, Rect) {
    let side = (r.height() - 26.0).min(r.width() - 16.0).max(8.0);
    let icon = Rect::from_center_size(Pos2::new(r.center().x, r.top() + 5.0 + side / 2.0), Vec2::splat(side));
    let text = Rect::from_min_max(Pos2::new(r.left() + 5.0, icon.bottom() + 2.0), Pos2::new(r.right() - 5.0, r.bottom() - 2.0));
    (icon, text)
}

/// The close / minimize / maximize buttons, starting at `x`. Returns where the next item goes.
fn window_controls(ui: &Ui, x: f32, cy: f32) -> f32 {
    let group = Rect::from_min_size(Pos2::new(x - 8.0, cy - 10.0), Vec2::new(60.0, 20.0));
    let show_glyphs = ui.rect_contains_pointer(group);
    let centers = (0..3).map(|k| Pos2::new(x + 6.0 + 20.0 * k as f32, cy));
    paint_lights(ui, centers, 1.0, 14.0, show_glyphs);
    x + 78.0
}

/// The traffic lights at `centers`, `scale` times their title-bar size, each clickable within
/// a `hit`-sized square.
pub(in crate::app) fn paint_lights(ui: &Ui, centers: impl Iterator<Item = Pos2>, scale: f32, hit: f32, show_glyphs: bool) {
    let painter = ui.painter();
    for ((k, light), center) in Light::ALL.into_iter().enumerate().zip(centers) {
        let resp = ui.interact(Rect::from_center_size(center, Vec2::splat(hit)), Id::new(("light", k)), Sense::click());
        painter.circle_filled(center, 6.0 * scale, light.color());
        if show_glyphs {
            light.paint_glyph(painter, center, scale);
        }
        if resp.clicked_by(PointerButton::Primary) {
            light.press(ui.ctx());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn slots(gaps: Vec<(usize, f32)>) -> TabSlots {
        TabSlots { origin: Pos2::ZERO, size: Vec2::new(100.0, 30.0), vertical: false, shown: 3, pill: None, gaps, pill_shape: None }
    }

    #[test]
    fn closing_gap_pushes_later_tabs_along() {
        let s = slots(vec![(1, 0.5)]);
        assert_eq!(s.rect(0.0).left(), 0.0);
        assert_eq!(s.rect(1.0).left(), 150.0);
        assert_eq!(s.rect(2.0).left(), 250.0);
        // The pill eases across the gap instead of jumping it.
        assert_eq!(s.rect(0.5).left(), 75.0);
    }

    #[test]
    fn slot_at_skips_the_gap() {
        let s = slots(vec![(1, 0.5)]);
        let at = |x| s.slot_at(Pos2::new(x, 10.0));
        assert_eq!((at(-5.0), at(50.0), at(120.0), at(160.0), at(260.0), at(900.0)), (0, 0, 1, 1, 2, 2));
        assert_eq!(slots(Vec::new()).slot_at(Pos2::new(150.0, 0.0)), 1);
    }
}
