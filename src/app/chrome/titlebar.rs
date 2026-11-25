//! The title bar: window buttons, the workspace (Files) label and the tab strip.

use std::collections::HashMap;
use std::time::Duration;

use egui::emath::TSTransform;
use egui::{Color32, FontId, Id, PointerButton, Pos2, Rect, Sense, Shape, Stroke, StrokeKind, Ui, Vec2, ViewportCommand};

use crate::agents::notify;
use crate::agents::model::{AgentKind, Status, tile_phase};
use crate::config::AgentsTab;
use crate::layout::PaneId;
use crate::platform;
use crate::ui::theme::{HOVER, HOVER_STRONG, Theme, white_alpha};
use crate::ui::widgets::{HoverShape, IconButton, is_fullscreen, is_maximized};
use crate::ui::glide::{Glide, Lerp};
use crate::ui::{dog, glide, icons, paint};

use crate::app::App;
use crate::app::actions::Action;
use crate::app::workspace::PaneRef;
use crate::app::workspace::pane_drag::{DropTarget, SPRING_DELAY};
use crate::app::animation::tab_close::{GLYPH, TabDrawn, TabGhost};

const TAB_HEIGHT: f32 = 30.0;
/// Padding between the tab track's edge and the tabs inside it.
pub(in crate::app) const TRACK_PAD: f32 = 3.0;
const MIN_TAB_WIDTH: f32 = 80.0;
const MAX_TAB_WIDTH: f32 = 232.0;
const PLUS_SIZE: f32 = 28.0;
const LABEL_WIDTH: f32 = 96.0;
/// Tabs across the track when they keep one width and swipe ([`TabScroll`]): the half tab
/// peeking out at the edge shows there are more.
const SCROLL_TABS_ACROSS: f32 = 2.5;
/// Holding a tab this long (seconds) before moving makes the drag reorder it instead of
/// swiping the strip.
const LONG_PRESS: f64 = 0.35;
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
/// right (title bar) or down (sidebar).
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
    /// The track, when the tabs scroll inside it: tabs are painted and hit only within it.
    clip: Option<Rect>,
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

/// The tab strip (title bar or sidebar) between frames: where each tab was drawn, the
/// active-tab pill and closing tabs animating, and the Files tab's dog. Kept by `App` as `strip`.
pub(in crate::app) struct TabStrip {
    /// Animated position of the active-tab highlight, in tab slots.
    pub(in crate::app) pill_glide: Glide,
    /// Where each title-bar tab was drawn last frame, by tab uid (closes start from there).
    pub(in crate::app) tab_rects: HashMap<u64, TabDrawn>,
    /// Closed tabs still folding away.
    pub(in crate::app) tab_ghosts: Vec<TabGhost>,
    /// Where the Files tab (the directory label, or the sidebar tile) was drawn this frame.
    pub(in crate::app) files_rect: Option<Rect>,
    /// Where the Files tab's dog is in its intro, as the time the intro would have started
    /// (set at launch; moved so the blep plays each time Files is shown), and whether Files
    /// was showing last frame.
    pub(in crate::app) dog_wake: Option<f64>,
    pub(in crate::app) dog_was_active: bool,
    /// Where the Files tab's dog was drawn this frame (the connect handoff's dog glides there).
    pub(in crate::app) dog_rect: Option<Rect>,
    /// Where each visible tab was drawn this frame (title bar or sidebar), by tab uid: where
    /// a dragged pane can be dropped into another tab.
    pub(in crate::app) tab_hits: Vec<(u64, Rect)>,
    /// The title bar or sidebar this frame: dropping a pane on its empty space gives the
    /// pane a tab of its own.
    pub(in crate::app) strip_rect: Option<Rect>,
    /// The title-bar tabs' sideways scroll, on phones.
    pub(in crate::app) scroll: TabScroll,
    /// What the Agents tab's badge showed last (it keeps showing it while it fades out).
    pub(in crate::app) badge: Option<notify::Badge>,
}

/// Title-bar tabs that keep one width, [`SCROLL_TABS_ACROSS`] to the track, and swipe
/// sideways like a scroll view with no scroll bar (`platform::WINDOW.scroll_tabs`). A swipe
/// flings on after the finger lifts; a long press then drag reorders instead; and a newly
/// shown tab scrolls into view.
pub(in crate::app) struct TabScroll {
    pub(in crate::app) on: bool,
    /// How far the tabs are scrolled, in points from the first tab.
    pub(in crate::app) offset: f32,
    /// Fling speed after a swipe, in points a second (positive scrolls toward later tabs).
    velocity: f32,
    /// Easing toward this offset to bring the active tab into view.
    target: Option<f32>,
    /// A swipe is underway (the finger is down and moved off a tab).
    swiping: bool,
    /// The drag under way began with a long press: it moves the tab instead.
    reordering: bool,
    /// The tab last brought into view (by uid), and the layout it was brought into view in
    /// (tab count, tab width): a new tab, or the strip laid out again (the window reaching
    /// its size at launch), brings the active tab into view again.
    revealed: Option<(u64, usize, u32)>,
}

impl TabScroll {
    pub(in crate::app) fn new(on: bool) -> Self {
        TabScroll { on, offset: 0.0, velocity: 0.0, target: None, swiping: false, reordering: false, revealed: None }
    }

    /// Advances the scroll one frame, with slots `width` apart, `max` the furthest it goes
    /// and `view` the track's inner width, and returns the offset. `active` is the shown
    /// tab's slot and uid.
    fn advance(&mut self, ctx: &egui::Context, width: f32, view: f32, max: f32, tabs: usize, active: Option<(usize, u64)>) -> f32 {
        let animate = glide::animations(ctx);
        let (down, delta, speed, dt) = ctx.input(|i| (i.pointer.primary_down(), i.pointer.delta().x, i.pointer.velocity().x, i.stable_dt.min(0.1)));
        if let Some((slot, uid)) = active
            && self.revealed != Some((uid, tabs, width.to_bits())) {
                self.revealed = Some((uid, tabs, width.to_bits()));
                let left = slot as f32 * width;
                let to = if left < self.offset {
                    Some(left)
                } else if left + width > self.offset + view {
                    Some(left + width - view)
                } else {
                    None
                };
                if let Some(to) = to {
                    self.velocity = 0.0;
                    if animate { self.target = Some(to) } else { self.offset = to }
                }
            }
        if self.swiping {
            if down {
                self.offset -= delta;
                self.velocity = -speed;
                self.target = None;
            } else {
                self.swiping = false;
                if !animate {
                    self.velocity = 0.0;
                }
            }
        } else if let Some(to) = self.target {
            self.offset += (to - self.offset) * (dt * 14.0).min(1.0);
            if (to - self.offset).abs() < 0.5 {
                self.offset = to;
                self.target = None;
            }
            ctx.request_repaint();
        } else if self.velocity.abs() > 5.0 {
            self.offset += self.velocity * dt;
            self.velocity *= (-dt * 4.0).exp();
            ctx.request_repaint();
        } else {
            self.velocity = 0.0;
        }
        if !down {
            self.reordering = false;
        }
        let clamped = self.offset.clamp(0.0, max);
        if clamped != self.offset {
            self.offset = clamped;
            self.velocity = 0.0;
        }
        self.offset
    }
}

impl App {
    /// Widgets are laid out left to right and registered in that order (later ones win hit
    /// tests), starting with the background drag area.
    pub(in crate::app) fn titlebar(&mut self, ui: &mut Ui, rect: Rect, theme: &Theme) {
        let ctx = ui.ctx().clone();
        self.strip.strip_rect = Some(rect);
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
        let label_right = if self.agents_tab_icon_only() { x + 25.0 } else { x + 30.0 + LABEL_WIDTH };
        let label_r = Rect::from_min_max(Pos2::new(x - 7.0, cy - 15.0), Pos2::new(label_right, cy + 15.0));
        let now = ctx.input(|i| i.time);
        self.advance_tab_ghosts(&ctx, now);
        let mut slots = self.horizontal_tab_slots(ui, label_r.right() + 6.0, rect.right() - PLUS_SIZE - 22.0, cy, now);
        // The active pill is painted once, beneath the label and the tabs, and glides
        // between them. While the active tab closes it folds into that tab's × and grows
        // back out onto the tab now showing.
        let target = slots.pill_rect(label_r);
        let pill = match self.strip.tab_ghosts.iter().rev().find(|g| g.drawn.active) {
            Some(g) => g.pill(now, target),
            None => target.map(|r| (r, 8.0)),
        };
        slots.pill_shape = pill.map(|(r, _)| r);
        // Scrolling tabs show only inside the track; the pill too, unless it is on (or
        // gliding from) the Agents label.
        let clipped = |clip: Rect| {
            let mut child = ui.new_child(egui::UiBuilder::new());
            child.set_clip_rect(clip.intersect(ui.clip_rect()));
            child
        };
        let tabs_ui = slots.clip.map(clipped);
        let tabs_ui = tabs_ui.as_ref().unwrap_or(ui);
        if let Some((r, radius)) = pill {
            let radius = radius.min(r.width().min(r.height()) / 2.0).max(0.0);
            let painter = match slots.clip {
                Some(clip) if slots.pill.is_some_and(|p| p < 0.0) => ui.painter().with_clip_rect(clip.union(label_r)),
                _ => tabs_ui.painter().clone(),
            };
            paint::glass_pill(&painter, r, radius.round() as u8);
        }
        self.workspace_label(ui, label_r, theme);
        self.tab_strip(tabs_ui, &slots, theme);
        self.paint_tab_ghosts(tabs_ui, now, theme);
        self.new_tab_button(ui, rect, cy, theme);
    }

    /// Start closed tabs' animations on their first frame and drop finished ones.
    fn advance_tab_ghosts(&mut self, ctx: &egui::Context, now: f64) {
        if !glide::animations(ctx) {
            self.strip.tab_ghosts.clear();
            return;
        }
        for g in &mut self.strip.tab_ghosts {
            g.start.get_or_insert(now);
        }
        self.strip.tab_ghosts.retain(|g| !g.done(now));
        if !self.strip.tab_ghosts.is_empty() {
            ctx.request_repaint();
        }
    }

    /// Closed tabs going into their ×. A background tab (tint, icon and title) is painted as
    /// it was and then scaled, as one piece, toward the × center; the active tab's title and
    /// icon fade under the folding pill (painted already). The × itself goes last.
    fn paint_tab_ghosts(&self, ui: &Ui, now: f64, theme: &Theme) {
        let painter = ui.painter();
        for g in &self.strip.tab_ghosts {
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
                    icons::app_tile(&p, d.icon, &g.process, g.mark.as_deref(), None);
                    let color = if d.active { theme.text_bright } else { theme.text_dim };
                    let right = if self.strip.scroll.on { d.rect.right() - 10.0 } else { d.close.left() - 2.0 };
                    let width = right - d.text_left;
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
        self.strip.files_rect = Some(label_r);
        let painter = ui.painter();
        let (x, cy) = (label_r.left() + 7.0, label_r.center().y);
        let label = ui.interact(label_r, Id::new("workspace-label"), Sense::click());
        let active = self.home_active;
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

        let dog = Rect::from_center_size(Pos2::new(x + 9.0, cy), Vec2::splat(21.0));
        self.files_dog(ui.ctx(), painter, dog, theme);
        let icon_only = self.agents_tab_icon_only();
        if !files {
            self.agents_badge(ui.ctx(), painter, dog.left_top() + Vec2::new(1.0, 1.0), icon_only, theme);
        }
        if icon_only {
            return;
        }
        let x = x + 26.0;
        let title_color = if active { theme.text_bright } else { theme.text };
        let title = self.home_label(files);
        let (detail, detail_color) = if files {
            (self.hostname.clone(), theme.text_dim)
        } else {
            let counts = self.agent_tracker.counts();
            let color = if counts.needs_you > 0 {
                theme.yellow
            } else if counts.done > 0 {
                theme.blue
            } else {
                theme.text_dim
            };
            (self.agent_tracker.headline().to_string(), color)
        };
        paint::faded_text(painter, Pos2::new(x, cy - 7.0), &title, paint::bold(13.0), title_color, LABEL_WIDTH, 16.0);
        paint::faded_text(painter, Pos2::new(x, cy + 8.0), &detail, FontId::proportional(11.0), detail_color, LABEL_WIDTH, 16.0);
    }

    /// Whether the Agents tab is the dog alone (`agents-tab = icon`). A Files tab keeps its
    /// folder and host name.
    fn agents_tab_icon_only(&self) -> bool {
        self.cfg.agents_tab == AgentsTab::Icon && !self.home_is_files()
    }

    /// The badge on the Agents tab's dog while you're elsewhere ([`notify::badge`]): an amber
    /// dot while an agent needs you (a bubble counting them on the icon-only tab, pulsing),
    /// a smaller, quieter blue one while any is done and not yet looked at. It scales and fades
    /// in and out, and keeps showing what it last showed while it goes.
    pub(in crate::app) fn agents_badge(&mut self, ctx: &egui::Context, painter: &egui::Painter, center: Pos2, count: bool, theme: &Theme) {
        let shown = notify::badge(self.agent_tracker.counts(), self.home_active);
        if shown.is_some() {
            self.strip.badge = shown;
        }
        let animate = glide::animations(ctx);
        let id = Id::new("agents-tab-badge");
        let t = if animate { ctx.animate_bool_with_time_and_easing(id, shown.is_some(), BADGE_FADE, glide::ease_out) } else { f32::from(u8::from(shown.is_some())) };
        let Some(badge) = self.strip.badge.filter(|_| t > 0.0) else { return };
        // Needs you ↔ done cross-fades too: color and size.
        let urgent = matches!(badge, notify::Badge::NeedsYou(_));
        let u = if animate { ctx.animate_bool_with_time(id.with("urgent"), urgent, BADGE_FADE) } else { f32::from(u8::from(urgent)) };
        let color = Color32::lerp(theme.blue.gamma_multiply(0.85), theme.yellow, u).gamma_multiply(t);
        let ring = Stroke::new(1.5_f32, theme.window.gamma_multiply(t));
        let scale = 0.4 + 0.6 * t;
        if let (true, notify::Badge::NeedsYou(n)) = (count, badge) {
            let font = paint::bold(9.5 * scale);
            let galley = painter.layout_no_wrap(notify::badge_count(n), font, theme.window.gamma_multiply(t));
            let height = 14.0 * scale;
            let r = Rect::from_center_size(center, Vec2::new((galley.size().x + 7.0 * scale).max(height), height));
            if self.cfg.animations && t >= 1.0 {
                let phase = (ctx.input(|i| i.time) % ATTENTION_PULSE / ATTENTION_PULSE) as f32;
                painter.rect_filled(r.expand(5.0 * phase), 12, color.gamma_multiply(0.4 * (1.0 - phase)));
                ctx.request_repaint_after(std::time::Duration::from_millis(33));
            }
            painter.rect(r, 7.0 * scale, color, ring, StrokeKind::Outside);
            painter.galley(r.center() - galley.size() / 2.0, galley, theme.window.gamma_multiply(t));
        } else {
            let radius = (3.0 + 1.0 * u) * scale;
            painter.circle(center, radius, color, ring);
        }
    }

    /// The nex dog marking the Files tab (title-bar label or sidebar tile). It wakes up at
    /// launch, sticks its tongue out each time Files is shown, and rests dimmed while Files
    /// is hidden.
    pub(in crate::app) fn files_dog(&mut self, ctx: &egui::Context, painter: &egui::Painter, r: Rect, theme: &Theme) {
        let now = ctx.input(|i| i.time);
        let active = self.home_active;
        self.strip.dog_rect = Some(r);
        // The connect handoff's dog is on its way here: it paints the dog until it ends, and
        // the tile carries on from the same point of the blep.
        if let Some(h) = self.handoff.as_ref().filter(|h| h.elapsed(now) < crate::app::animation::handoff::END) {
            self.strip.dog_wake = Some(h.dog_wake());
            self.strip.dog_was_active = active;
            ctx.request_repaint();
            return;
        }
        match self.strip.dog_wake {
            None => self.strip.dog_wake = Some(now),
            // Switching to Files plays just the blep, unless the launch intro hasn't reached
            // it yet (it's on its way).
            Some(start) if active && !self.strip.dog_was_active && now - start >= dog::BLEP_START as f64 => {
                self.strip.dog_wake = Some(now - dog::BLEP_START as f64);
            }
            Some(_) => {}
        }
        self.strip.dog_was_active = active;
        let t = if glide::animations(ctx) { (now - self.strip.dog_wake.unwrap_or(now)) as f32 } else { dog::INTRO };
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
            self.strip.tab_ghosts.iter().map(|g| (g.index, g.frame(now).gap)).filter(|&(_, f)| f > 0.0).collect();
        let held: f32 = gaps.iter().map(|&(_, f)| f).sum();
        let n = self.tabs.len();
        let avail = (right - x - 2.0 * TRACK_PAD).max(MIN_TAB_WIDTH);
        if self.strip.scroll.on {
            return self.scrolling_tab_slots(ui, x, cy, avail, held, gaps);
        }
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

    /// [`horizontal_tab_slots`](Self::horizontal_tab_slots) with tabs of one width, two and a
    /// half to the `avail`-wide track, all of them laid out and scrolled by [`TabScroll`].
    fn scrolling_tab_slots(&mut self, ui: &Ui, x: f32, cy: f32, avail: f32, held: f32, gaps: Vec<(usize, f32)>) -> TabSlots {
        let n = self.tabs.len();
        let width = avail / SCROLL_TABS_ACROSS;
        let max = (width * (n as f32 + held) - avail).max(0.0);
        let active = (!self.home_active && self.active < n).then(|| (self.active, self.tabs[self.active].uid));
        let offset = self.strip.scroll.advance(ui.ctx(), width, avail, max, n, active);
        let across = (n as f32 + held).clamp(1.0, SCROLL_TABS_ACROSS);
        let track = Rect::from_min_size(
            Pos2::new(x, cy - TAB_HEIGHT / 2.0 - TRACK_PAD),
            Vec2::new(width * across + 2.0 * TRACK_PAD, TAB_HEIGHT + 2.0 * TRACK_PAD),
        );
        let mut slots = self.tab_track(ui, track, Vec2::new(width, TAB_HEIGHT), n, false);
        slots.origin.x -= offset;
        slots.gaps = gaps;
        slots.clip = Some(track.shrink(1.0));
        slots
    }

    /// Paints `track` (holding `visible` slots of `size`) and advances the pill toward the
    /// active tab, or the Files label.
    pub(in crate::app) fn tab_track(&mut self, ui: &Ui, track: Rect, size: Vec2, visible: usize, vertical: bool) -> TabSlots {
        let ctx = ui.ctx().clone();
        paint::tab_track(ui.painter(), track);
        let shown = visible.min(self.tabs.len());
        let target = if self.home_active { Some(-1.0) } else { (self.active < shown).then_some(self.active as f32) };
        // While the active tab closes, the pill's path is the close animation's: be at the
        // new tab already when it hands back.
        let folding = self.strip.tab_ghosts.iter().any(|g| g.drawn.active);
        let pill = target.map(|t| {
            if folding {
                self.strip.pill_glide.snap(&ctx, t);
                t
            } else {
                self.strip.pill_glide.value(&ctx, t)
            }
        });
        let origin = track.min + Vec2::splat(TRACK_PAD);
        TabSlots { origin, size, vertical, shown, pill, gaps: Vec::new(), pill_shape: None, clip: None }
    }

    /// The terminal tabs, drawn into `slots` over the track and pill.
    pub(in crate::app) fn tab_strip(&mut self, ui: &Ui, slots: &TabSlots, theme: &Theme) {
        self.strip.tab_rects.clear();
        self.strip.tab_hits.clear();
        for i in 0..slots.shown {
            if self.tab_item(ui, slots, i, theme) {
                break;
            }
        }
        // A pane dragged onto the strip's empty space: a dashed slot for its new tab, just
        // past the last one (or, with no room, around the whole track).
        if self.pane_drag.as_ref().is_some_and(|d| d.over == Some(DropTarget::NewTab)) && slots.shown > 0 {
            let next = slots.rect(slots.shown as f32).shrink(1.0);
            let r = match self.strip.strip_rect {
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

    /// One tab: icon (or pane deck), title, hover close button (not on phones); click to show,
    /// drag to reorder.
    /// In the vertical (sidebar) strip a tab is just a large icon, its title a tooltip.
    /// Returns `true` if the tabs were reordered (the rest of the strip is drawn next frame).
    fn tab_item(&mut self, ui: &Ui, slots: &TabSlots, i: usize, theme: &Theme) -> bool {
        let ctx = ui.ctx().clone();
        let painter = ui.painter();
        let r = slots.rect(i as f32);
        let uid = self.tabs[i].uid;
        let resp = ui.interact(r, Id::new(("tab", uid)), Sense::click_and_drag());
        // Scrolling tabs swipe the strip when dragged, unless held a moment first.
        let scroll = slots.clip.is_some();
        if scroll && resp.drag_started_by(PointerButton::Primary) {
            let now = ctx.input(|i| i.time);
            let held = ctx.input(|i| i.pointer.press_start_time()).is_some_and(|t| now - t >= LONG_PRESS);
            self.strip.scroll.reordering = held;
            self.strip.scroll.swiping = !held;
        }
        let reorders = !scroll || self.strip.scroll.reordering;
        if reorders
            && resp.dragged_by(PointerButton::Primary)
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
        if reorders && resp.drag_started_by(PointerButton::Primary) {
            self.show_tab(i);
        }
        self.strip.tab_hits.push((uid, slots.clip.map_or(r, |c| r.intersect(c))));
        let active = i == self.active && !self.home_active;
        let hovered = resp.hovered();
        if !active && hovered {
            painter.rect_filled(r.shrink(1.0), 8, TAB_HOVER);
        }
        self.paint_tab_drop_hint(ui, uid, r, theme);
        // The icon deck turns toward the focused pane; the tab's label is then read once, and
        // borrowed, for its icon and its title.
        let deck = self.tabs[i].deck_cards(&ctx, |id| id);
        let header = self.tab_header_pane(&self.tabs[i]);
        let (title, process) = self.pane_label(header);
        let (mark, title) = icons::title_mark(process, &title);
        let tile = self.pane_tile(&ctx, header, (process, mark));

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
            self.tab_icon(painter, &deck, tile, icon, icons::SIDEBAR_STACK_STEP, true);
            if self.tab_needs_attention(i) {
                paint_attention(&ctx, painter, icon.left_top() + Vec2::splat(2.0), self.cfg.attention_color(), theme, self.cfg.animations);
            }
            let color = if active { theme.text_bright } else { theme.text_dim };
            paint::caption(painter, text, title, FontId::proportional(12.0), color);
            let resp = resp.on_hover_text(title);
            if resp.clicked_by(PointerButton::Middle) {
                self.pending.push(Action::CloseTab(i));
            } else if resp.clicked_by(PointerButton::Primary) {
                self.show_tab(i);
            }
            return false;
        }

        let icon = Rect::from_center_size(Pos2::new(r.left() + 19.0, r.center().y), Vec2::splat(20.0));
        let text_left = self.tab_icon(painter, &deck, tile, icon, icons::STACK_STEP, false);
        let close_r = Rect::from_center_size(Pos2::new(r.right() - 15.0, r.center().y), Vec2::splat(18.0));
        // Phones have no hover (a tap counts as one), so no ×: tabs close from the pane
        // header's × instead, and the title gets the whole tab.
        let closable = !self.strip.scroll.on;
        let text_right = if closable && hovered { close_r.left() - 2.0 } else { r.right() - 10.0 };
        let title_color = if active { theme.text_bright } else { theme.text_dim };
        paint::faded_text(painter, Pos2::new(text_left, r.center().y), title, FontId::proportional(13.5), title_color, text_right - text_left, 26.0);
        self.strip.tab_rects.insert(uid, TabDrawn { rect: r, close: close_r, icon, text_left, active });

        if self.tab_needs_attention(i) {
            paint_attention(&ctx, painter, icon.left_top() + Vec2::splat(1.0), self.cfg.attention_color(), theme, self.cfg.animations);
        }

        // Close button only on hover, like Safari/Finder tabs.
        let close_clicked = closable
            && (hovered || ui.rect_contains_pointer(close_r))
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

    /// The app tile pane `id` shows in its tab's icon deck: its process, title mark and
    /// animation phase (see [`Self::pane_tile`]).
    fn deck_card(&self, ctx: &egui::Context, id: PaneId) -> Tile<'_> {
        let label = match self.pane(id) {
            Some(PaneRef::Terminal(t)) => (&*t.process, icons::title_mark(&t.process, t.title()).0),
            Some(PaneRef::Explorer(_)) => (icons::EXPLORER_PROCESS, None),
            Some(PaneRef::Diff(_)) => (icons::DIFF_PROCESS, None),
            Some(PaneRef::Agents(_)) => (icons::AGENTS_PROCESS, None),
            None => Default::default(),
        };
        self.pane_tile(ctx, id, label)
    }

    /// The tile for pane `id`, whose own label is `label` (process and title mark): a pane
    /// showing a pipeline gets the pipeline's tile, moving while it runs (see [`tab_tile`]).
    fn pane_tile<'a>(&'a self, ctx: &egui::Context, id: PaneId, label: (&'a str, Option<&'a str>)) -> Tile<'a> {
        tab_tile(label, self.pane_pipeline(id), glide::animations(ctx), ctx.input(|i| i.time))
    }

    /// The tab's icon: the focused pane's app tile, or a deck with one card per pane (focused
    /// on top, up to 3 peeking out behind, each `step` further). With `center`, the whole deck
    /// is centered on `icon` rather than fanning out from it. `deck` is each pane and its
    /// card's depth (see [`Tab::deck_cards`]), `tile` the tab's own (see [`Self::pane_tile`]).
    /// A running pipeline's tile keeps the strip repainting while it moves.
    /// Returns where the title text starts.
    ///
    /// [`Tab::deck_cards`]: crate::app::workspace::tab::Tab::deck_cards
    fn tab_icon(&self, painter: &egui::Painter, deck: &[(PaneId, f32)], tile: Tile<'_>, icon: Rect, step: Vec2, center: bool) -> f32 {
        let ctx = painter.ctx();
        let mut text_left = icon.right() + 9.0;
        let moving = if deck.len() > 1 {
            let cards: Vec<(&str, Option<&str>, Option<f32>, f32)> = deck
                .iter()
                .map(|&(id, depth)| {
                    let (process, mark, phase) = self.deck_card(ctx, id);
                    (process, mark, phase, depth)
                })
                .collect();
            let behind = (cards.len() as f32 - 1.0).min(icons::STACK_DEPTH - 1.0);
            let icon = if center { icon.translate(-step * behind / 2.0) } else { icon };
            icons::icon_stack(painter, icon, &cards, step);
            text_left += step.x * behind;
            cards.iter().any(|c| c.2.is_some())
        } else {
            let (process, mark, phase) = tile;
            icons::app_tile(painter, icon, process, mark, phase);
            phase.is_some()
        };
        if moving {
            ctx.request_repaint_after(TILE_FRAME);
        }
        text_left
    }

    /// Whether a pane in tab `i` rang the bell or sent a notification the user hasn't seen,
    /// or shows an agent that finished or started needing you since it was last looked at
    /// (agents over ssh rarely ring the local bell).
    fn tab_needs_attention(&self, i: usize) -> bool {
        // Not the sticky pane, which every tab has.
        self.tabs[i]
            .root
            .iter()
            .filter(|&id| !self.is_sticky(id))
            .any(|id| self.panes.get(&id).is_some_and(|t| t.attention) || self.agent_tracker.wants_you(id))
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

/// How long the Agents tab's badge takes to scale and fade in or out, in seconds.
const BADGE_FADE: f32 = 0.22;

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

/// Splits a sidebar tile into its icon square and the caption line below it.
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

/// An app tile to draw: process, title mark, and animation phase (see [`icons::app_tile`]).
type Tile<'a> = (&'a str, Option<&'a str>, Option<f32>);

/// How often a moving tab tile is redrawn.
const TILE_FRAME: Duration = Duration::from_millis(50);

/// The tile for a pane labelled `label` (its process and title mark) that shows a pipeline in
/// state `pipeline`, if it shows one: the pipeline's tile then (a pane attached to one runs
/// `ssh`), moving while the pipeline runs and `animate` is on, at `time` seconds. Otherwise
/// its own, still.
fn tab_tile<'a>(label: (&'a str, Option<&'a str>), pipeline: Option<Status>, animate: bool, time: f64) -> Tile<'a> {
    match pipeline {
        Some(status) => (icons::PIPELINE_PROCESS, None, tile_phase(AgentKind::Pipeline, status, animate, time)),
        None => (label.0, label.1, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_tab_showing_a_pipeline_gets_its_tile_moving_while_it_runs() {
        // The pane runs ssh (attached to the pipeline's session); its tab shows the pipeline.
        let label = ("ssh", Some("✳"));
        let running = Status::Working;
        let (process, mark, phase) = tab_tile(label, Some(running), true, 0.4);
        assert_eq!((process, mark), (icons::PIPELINE_PROCESS, None));
        assert_eq!(phase, Some(icons::pipeline_phase(0.4)));
        // Still when animations are off, or once it stops running.
        assert_eq!(tab_tile(label, Some(running), false, 0.4), (icons::PIPELINE_PROCESS, None, None));
        assert_eq!(tab_tile(label, Some(Status::Idle), true, 0.4), (icons::PIPELINE_PROCESS, None, None));
        // Any other pane keeps its own tile, still.
        assert_eq!(tab_tile(label, None, true, 0.4), ("ssh", Some("✳"), None));
    }

    fn slots(gaps: Vec<(usize, f32)>) -> TabSlots {
        TabSlots { origin: Pos2::ZERO, size: Vec2::new(100.0, 30.0), vertical: false, shown: 3, pill: None, gaps, pill_shape: None, clip: None }
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
