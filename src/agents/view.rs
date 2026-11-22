//! Drawing the Agents pane, in the style of a macOS / iOS dashboard: grouped sections of
//! rounded agent cards (two or three
//! to a row when there's room). Each card has the agent's app tile, its task and folder, and
//! a status pill whose dot breathes while the agent works. Mouse handling lives here too.

use std::time::{Duration, Instant};

use egui::epaint::Shadow;
use egui::{Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::ui::glide::animations;
use crate::layout::PaneId;
use crate::ui::theme::{ACCENT, Theme, white_alpha};
use crate::ui::widgets::{ScrollSpan, overlay_scrollbar};
use crate::ui::{icons, paint};

use super::AgentsPane;
use super::model::{Agent, Source, Status, elapsed};
use super::motion::Motion;

const SIDE: f32 = 18.0;
const CARD_H: f32 = 74.0;
const CARD_RADIUS: u8 = 14;
const GAP: f32 = 10.0;
const SECTION_H: f32 = 34.0;
const MIN_CARD_W: f32 = 300.0;
const MAX_COLUMNS: usize = 3;
const TILE: f32 = 40.0;

/// What the view keeps between frames.
pub(super) struct ViewState {
    /// Scroll so the highlighted card is in view on the next frame (after a key moved it).
    pub reveal_selection: bool,
    scroll_active: f64,
    /// Where each card and section header is on its way to.
    motion: Motion,
}

impl ViewState {
    pub(super) fn new() -> ViewState {
        ViewState { reveal_selection: false, scroll_active: -10.0, motion: Motion::new() }
    }
}

/// Everything a frame's drawing shares.
struct Frame<'a> {
    ui: &'a Ui,
    c: &'a Theme,
    focused: bool,
    pane: u64,
    /// Where interaction is allowed (the pane's visible content).
    clip: Rect,
    now: Instant,
    /// Seconds, for the breathing dots.
    time: f64,
    animate: bool,
}

impl Frame<'_> {
    fn id(&self, what: impl std::hash::Hash) -> Id {
        Id::new(("agents", self.pane, what))
    }

    /// A click target, cut to the visible area so off-screen cards never take clicks.
    fn interact(&self, r: Rect, what: impl std::hash::Hash) -> Option<egui::Response> {
        let r = r.intersect(self.clip);
        r.is_positive().then(|| self.ui.interact(r, self.id(what), Sense::click()))
    }
}

fn ui_font(size: f32) -> FontId {
    FontId::proportional(size)
}

/// The tint of a status: green while working, amber when it needs you, quiet grey at rest.
fn status_color(s: Status, c: &Theme) -> Color32 {
    match s {
        Status::Working => c.green,
        Status::NeedsYou => c.yellow,
        Status::Idle => c.text_dim,
    }
}

impl AgentsPane {
    /// Draw the pane into `rect` (below the pane header, edge to edge of the card), showing
    /// `agents` (all of them, sorted), under `notes` (one line per ssh host that can't be
    /// watched).
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: u64, focused: bool, c: &Theme, agents: &[Agent], notes: &[String]) {
        let ctx = ui.ctx().clone();
        let f = Frame {
            ui,
            c,
            focused,
            pane,
            clip: rect.intersect(painter.clip_rect()),
            now: Instant::now(),
            time: ctx.input(|i| i.time),
            animate: animations(&ctx),
        };
        let painter = &painter.with_clip_rect(f.clip);

        let mut body = Rect::from_min_max(rect.min + Vec2::new(0.0, 6.0), rect.max);
        if !notes.is_empty() {
            let strip = host_notes(&f, painter, body, notes);
            body.min.y += strip;
        }
        if !body.is_positive() {
            return;
        }

        // Lay the sections and cards out in content coordinates.
        let width = body.width() - 2.0 * SIDE;
        let columns = (((width + GAP) / (MIN_CARD_W + GAP)).floor() as usize).clamp(1, MAX_COLUMNS);
        let card_w = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
        let mut sections: Vec<(Status, usize, Rect)> = Vec::new();
        let mut cards: Vec<(&Agent, Rect)> = Vec::new();
        let mut y = 4.0;
        for status in Status::ALL {
            let group: Vec<&Agent> = agents.iter().filter(|a| a.status == status).collect();
            if group.is_empty() {
                continue;
            }
            sections.push((status, group.len(), Rect::from_min_size(Pos2::new(SIDE, y), Vec2::new(width, SECTION_H))));
            y += SECTION_H;
            for (i, agent) in group.iter().enumerate() {
                let (row, col) = (i / columns, i % columns);
                let at = Pos2::new(SIDE + col as f32 * (card_w + GAP), y + row as f32 * (CARD_H + GAP));
                cards.push((agent, Rect::from_min_size(at, Vec2::new(card_w, CARD_H))));
            }
            y += group.len().div_ceil(columns) as f32 * (CARD_H + GAP) + 6.0;
        }
        let content_h = y + 12.0;
        self.columns = columns;
        self.order = cards.iter().map(|(a, _)| a.pane).collect();
        if self.selected.is_none_or(|p| !self.order.contains(&p)) {
            self.selected = self.order.first().copied();
        }
        let mut motion = std::mem::take(&mut self.view.motion);
        motion.update(f.time, f.animate, &sections, &cards, width);
        // Empty once the last card has faded away.
        if motion.cards.is_empty() {
            self.view.motion = motion;
            empty_state(&f, painter, body);
            return;
        }

        // Scroll: the wheel, then keep a card moved to with the keyboard in view.
        let view_h = body.height();
        if ui.rect_contains_pointer(body) && !ui.input(|i| i.modifiers.command) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if d != 0.0 {
                self.scroll -= d;
                self.view.scroll_active = f.time;
            }
        }
        if std::mem::take(&mut self.view.reveal_selection)
            && let Some(&(_, r)) = cards.iter().find(|(a, _)| Some(a.pane) == self.selected)
        {
            // Its section header comes along when it's the first card.
            let top = r.top() - SECTION_H;
            if top < self.scroll {
                self.scroll = top.max(0.0);
            } else if r.bottom() + GAP > self.scroll + view_h {
                self.scroll = r.bottom() + GAP - view_h;
            }
        }
        self.scroll = self.scroll.clamp(0.0, (content_h - view_h).max(0.0));

        let now = f.time;
        let origin = body.min.to_vec2() - Vec2::new(0.0, self.scroll);
        let bp = painter.with_clip_rect(body.intersect(f.clip));
        let mut busy = false;
        for (status, sec) in &motion.sections {
            busy |= sec.busy(now);
            let mut p = bp.clone();
            p.multiply_opacity(sec.opacity.get(now));
            section_header(&p, sec.rect.get(now).translate(origin), *status, sec.count, f.c);
        }
        // Cards changing sections are drawn last, over the ones they pass.
        let mut ids: Vec<PaneId> = motion.cards.keys().copied().collect();
        ids.sort_by_key(|id| (motion.cards[id].lift(now) > 0.0, *id));
        let mut breathing = false;
        for id in ids {
            let Some(card) = motion.cards.get_mut(&id) else { continue };
            busy |= card.busy(now);
            let opacity = card.opacity.get(now);
            let r = card.rect.get(now).translate(origin);
            // Appearing and leaving cards grow from / shrink to a little smaller.
            let r = Rect::from_center_size(r.center(), r.size() * (0.96 + 0.04 * opacity));
            if !r.intersects(body) {
                continue;
            }
            let mut hovered = false;
            if !card.leaving {
                let hit = f.interact(r, ("card", id));
                hovered = hit.as_ref().is_some_and(|h| h.hovered());
                if hit.as_ref().is_some_and(|h| h.clicked()) {
                    self.selected = Some(id);
                    self.requests.push(super::Request::Focus(id));
                }
                if let Some(h) = hit {
                    h.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
            let hover = ui.ctx().animate_bool_with_time(f.id(("hover", id)), hovered, 0.12);
            let selected = !card.leaving && self.selected == Some(id);
            let (border, border_width) = border_for(card.agent.status, selected, f.focused, hovered, f.c);
            card.set_colors(now, border, border_width, status_color(card.agent.status, f.c), f.animate);
            // A card changing sections lifts off the grid on its way.
            let lift = card.lift(now);
            let r = Rect::from_center_size(r.center(), r.size() * (1.0 + 0.025 * lift));
            breathing |= card.agent.status == Status::Working;
            let look = Look { hover, lift, border: card.border.get(now), border_width: card.border_width.get(now), tint: card.tint.get(now) };
            let mut p = bp.clone();
            p.multiply_opacity(opacity);
            agent_card(&f, &p, r, &card.agent, look);
        }
        motion.forget_gone(now);
        self.view.motion = motion;
        if busy {
            ui.ctx().request_repaint();
        }

        let span = ScrollSpan { offset: self.scroll, content: content_h, view: view_h };
        if let Some(offset) = overlay_scrollbar(ui, painter, body, f.id("scrollbar"), span, &mut self.view.scroll_active) {
            self.scroll = offset;
        }

        // Elapsed times tick by the second; the working dots breathe smoothly.
        let next = if breathing && f.animate { Duration::from_millis(50) } else { Duration::from_secs(1) };
        ctx.request_repaint_after(next);
    }
}

/// A group's caption: "NEEDS YOU  1", small and quiet, like a grouped list's section header.
fn section_header(p: &Painter, r: Rect, status: Status, n: usize, c: &Theme) {
    let y = r.bottom() - 11.0;
    let font = paint::bold(11.0);
    let text = status.label().to_uppercase();
    let g = p.layout_no_wrap(text, font, c.text_dim);
    let w = g.size().x;
    p.galley(Pos2::new(r.left() + 2.0, y - g.size().y / 2.0), g, c.text_dim);
    p.text(Pos2::new(r.left() + w + 9.0, y), Align2::LEFT_CENTER, n.to_string(), ui_font(11.0), c.text_dim.gamma_multiply(0.7));
}

/// How a card is drawn this frame: hover (0 to 1) brightens it, lift (0 to 1) raises it
/// while it changes sections, and its border and status tint mid-cross-fade.
struct Look {
    hover: f32,
    lift: f32,
    border: Color32,
    border_width: f32,
    tint: Color32,
}

/// The border a card heads for: the accent when it's the selected one in the focused pane,
/// amber when its agent needs you, a quiet hairline otherwise (brighter under the pointer).
fn border_for(status: Status, selected: bool, focused: bool, hovered: bool, c: &Theme) -> (Color32, f32) {
    if selected && focused {
        (ACCENT.gamma_multiply(0.85), 1.5)
    } else if status == Status::NeedsYou {
        (c.yellow.gamma_multiply(0.45), 1.0)
    } else if selected {
        (white_alpha(40), 1.0)
    } else if hovered {
        (white_alpha(26), 1.0)
    } else {
        (white_alpha(14), 1.0)
    }
}

/// One agent: app tile, its task over its folder and tab, and the status pill over how long
/// it has been in that state.
fn agent_card(f: &Frame, p: &Painter, r: Rect, agent: &Agent, look: Look) {
    let c = f.c;
    let Look { hover, lift, border, border_width, tint } = look;
    let shadow = Shadow {
        offset: [0, (2.0 + 6.0 * lift) as i8],
        blur: (12.0 + 14.0 * lift) as u8,
        spread: 0,
        color: Color32::from_black_alpha((40.0 + 30.0 * hover + 50.0 * lift) as u8),
    };
    p.add(shadow.as_shape(r, CARD_RADIUS));
    p.rect_filled(r, CARD_RADIUS, white_alpha((8.0 + 7.0 * hover.max(lift)) as u8));
    // The glass edge along the top, as on the pane cards.
    let inset = CARD_RADIUS as f32 + 2.0;
    p.line_segment(
        [Pos2::new(r.left() + inset, r.top() + 1.0), Pos2::new(r.right() - inset, r.top() + 1.0)],
        Stroke::new(1.0, white_alpha(14)),
    );
    p.rect_stroke(r, CARD_RADIUS, Stroke::new(border_width, border), StrokeKind::Inside);

    let tile = Rect::from_center_size(Pos2::new(r.left() + 14.0 + TILE / 2.0, r.center().y), Vec2::splat(TILE));
    icons::app_tile(p, tile, agent.kind.process(), agent.mark.as_deref());

    // The status pill, top right, with the time in that state under it.
    let color = tint;
    let pill_font = paint::bold(11.0);
    let label = p.layout_no_wrap(agent.status.label().to_string(), pill_font, color);
    let pill = Rect::from_min_size(Pos2::new(r.right() - 14.0 - (label.size().x + 26.0), r.top() + 14.0), Vec2::new(label.size().x + 26.0, 22.0));
    let fill = color.gamma_multiply(0.15);
    p.rect_filled(pill, 11, fill);
    let dot = Pos2::new(pill.left() + 12.0, pill.center().y);
    status_dot(f, p, dot, agent.status, color);
    p.galley(Pos2::new(dot.x + 8.0, pill.center().y - label.size().y / 2.0), label, color);
    let since = elapsed(f.now.saturating_duration_since(agent.since));
    let since_y = r.top() + 51.0;
    let since_w = p.layout_no_wrap(since.clone(), ui_font(11.5), c.text_dim).size().x;
    p.text(Pos2::new(pill.right() - 4.0, since_y), Align2::RIGHT_CENTER, &since, ui_font(11.5), c.text_dim);

    // The task, and where it runs.
    let x = tile.right() + 13.0;
    let title_room = (pill.left() - 12.0 - x).max(0.0);
    paint::faded_text(p, Pos2::new(x, r.top() + 25.0), &agent.name, paint::bold(14.0), c.text_bright, title_room, 18.0);
    let mut detail = agent.kind.label().to_string();
    let remote = match &agent.source {
        Source::Remote { host, pane, .. } => Some((host, pane.is_some())),
        Source::Local => None,
    };
    if let Some((host, _)) = remote {
        detail.push_str(" · ");
        detail.push_str(host);
    }
    if let Some(dir) = &agent.cwd {
        detail.push_str(" · ");
        // A remote folder comes already shortened with the host's own home.
        detail.push_str(&if remote.is_some() { dir.to_string_lossy().into_owned() } else { crate::terminal::tilde(dir) });
    }
    match (agent.tab, remote) {
        (Some(tab), _) => detail.push_str(&format!(" · Tab {}", tab + 1)),
        (None, Some((_, false))) => detail.push_str(" · Detached"),
        _ => {}
    }
    let detail_room = (pill.right() - 4.0 - since_w - 12.0 - x).max(0.0);
    paint::faded_text(p, Pos2::new(x, since_y), &detail, ui_font(12.0), c.text_dim, detail_room, 18.0);
}

/// The status dot. While working it breathes, and a soft ring keeps rippling out of it.
fn status_dot(f: &Frame, p: &Painter, at: Pos2, status: Status, color: Color32) {
    if status != Status::Working || !f.animate {
        p.circle_filled(at, 3.5, color);
        return;
    }
    let t = (f.time * 1.1).fract() as f32;
    p.circle_stroke(at, 3.5 + 5.0 * t, Stroke::new(1.2, color.gamma_multiply(0.55 * (1.0 - t) * (1.0 - t))));
    let breathe = 0.5 + 0.5 * (f.time as f32 * std::f32::consts::TAU * 0.55).sin();
    p.circle_filled(at, 3.2 + 0.6 * breathe, color.gamma_multiply(0.75 + 0.25 * breathe));
}

/// Nothing to show: no agents running.
/// The hosts that can't be watched, one quiet line each with an amber dot, above the cards.
/// Returns the height they took.
fn host_notes(f: &Frame, p: &Painter, area: Rect, notes: &[String]) -> f32 {
    const LINE: f32 = 20.0;
    let mut y = area.top() + 8.0 + LINE / 2.0;
    let room = (area.width() - 2.0 * SIDE - 14.0).max(0.0);
    for note in notes {
        p.circle_filled(Pos2::new(area.left() + SIDE + 3.0, y), 3.0, f.c.yellow.gamma_multiply(0.8));
        paint::faded_text(p, Pos2::new(area.left() + SIDE + 14.0, y), note, ui_font(12.0), f.c.text_dim, room, LINE);
        y += LINE;
    }
    8.0 + notes.len() as f32 * LINE
}

fn empty_state(f: &Frame, p: &Painter, area: Rect) {
    let c = f.c;
    let center = area.center() - Vec2::new(0.0, (area.height() * 0.08).min(36.0));
    let g = center - Vec2::new(0.0, 40.0);
    p.circle_filled(g, 28.0, white_alpha(10));
    icons::sparkle(p, g, 15.0, c.text_dim);
    p.text(center + Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, "No Agents Running", paint::bold(15.0), c.text_bright);
    let detail = "Start claude or codex in any pane to see it here.";
    paint::caption(p, Rect::from_center_size(center + Vec2::new(0.0, 28.0), Vec2::new((area.width() - 40.0).max(0.0), 18.0)), detail, ui_font(12.5), c.text_dim);
}
