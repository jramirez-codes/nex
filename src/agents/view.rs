//! Drawing the Agents pane, in the style of a macOS / iOS dashboard: grouped sections of
//! rounded agent cards (two or three to a row when there's room). Each card has the agent's
//! app tile, its task and folder, and a status pill whose dot breathes while the agent
//! works. Mouse handling lives here too.
//!
//! A team (see `group`) is one card the width of the pane. Closed, a row of capsules under
//! its header shows its helpers (a pipeline's joined into steps); open, they're an inset
//! list inside the card, like a grouped table. Opening is the card growing: its rows are
//! uncovered as it does, and the capsules fade as the list comes in.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use egui::epaint::Shadow;
use egui::{Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::layout::PaneId;
use crate::ui::glide::animations;
use crate::ui::touch_pan::TouchPan;
use crate::ui::theme::{ACCENT, Theme, white_alpha};
use crate::ui::widgets::{ScrollSpan, overlay_scrollbar};
use crate::ui::{icons, paint};

use super::AgentsPane;
use super::group::{Group, Member, groups};
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
/// A closed team's row of capsules, under its header.
const STRIP_H: f32 = 40.0;
const CAPSULE_H: f32 = 24.0;
/// An open team's list: its rows, the margin around it, and the "Show all" row.
const ROW_H: f32 = 46.0;
const LIST_INSET: f32 = 12.0;
const MORE_H: f32 = 34.0;
/// Helpers listed before "Show all".
const ROWS_SHOWN: usize = 6;

/// What the view keeps between frames.
pub(super) struct ViewState {
    /// Scroll so the highlighted card is in view on the next frame (after a key moved it).
    pub reveal_selection: bool,
    scroll_active: f64,
    /// Scrolling the cards with a finger.
    pan: TouchPan,
    /// Where each card and section header is on its way to.
    motion: Motion,
    /// The terminal tab shown when it was last drawn (`None` inside: the Agents / Files
    /// tab); `None` before the first frame.
    shown_tab: Option<Option<usize>>,
    /// Each team card's team as last laid out, kept while its card fades away.
    teams: HashMap<PaneId, Group>,
}

impl ViewState {
    pub(super) fn new() -> ViewState {
        ViewState { reveal_selection: false, scroll_active: -10.0, pan: TouchPan::default(), motion: Motion::new(), shown_tab: None, teams: HashMap::new() }
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

/// The tint of a status: green while working, amber when it needs you, blue when it's done
/// and you haven't looked yet, quiet grey at rest.
fn status_color(s: Status, c: &Theme) -> Color32 {
    match s {
        Status::Working => c.green,
        Status::NeedsYou => c.yellow,
        Status::Done => c.blue,
        Status::Idle => c.text_dim,
    }
}

/// How tall a team's card is, open (`Some(show all)`) or closed.
fn team_height(g: &Group, open: Option<bool>) -> f32 {
    match open {
        None => CARD_H + STRIP_H,
        Some(all) => {
            let n = g.members.len();
            let shown = if all { n } else { n.min(ROWS_SHOWN) };
            let more = if n > ROWS_SHOWN { MORE_H } else { 0.0 };
            CARD_H + 2.0 + shown as f32 * ROW_H + more + LIST_INSET
        }
    }
}

/// The rows an open team's list shows (all of them, or the first few).
fn shown_members(g: &Group, all: bool) -> &[Member] {
    if all { &g.members } else { &g.members[..g.members.len().min(ROWS_SHOWN)] }
}

/// The inset list inside an open team card `r`.
fn list_rect(r: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(r.left() + LIST_INSET, r.top() + CARD_H + 2.0), Pos2::new(r.right() - LIST_INSET, r.bottom() - LIST_INSET))
}

/// Helper row `i` of the list in card `r`.
fn row_rect(r: Rect, i: usize) -> Rect {
    let list = list_rect(r);
    Rect::from_min_size(Pos2::new(list.left(), list.top() + i as f32 * ROW_H), Vec2::new(list.width(), ROW_H))
}

/// What a helper is called in its team: its role (or pipeline stage), else its name.
fn member_label(a: &Agent) -> &str {
    a.team.role.as_deref().filter(|r| !r.is_empty()).unwrap_or(&a.name)
}

impl AgentsPane {
    /// Draw the pane into `rect` (below the pane header, edge to edge of the card), showing
    /// `agents` (all of them, sorted), under `notes` (one line per ssh host that can't be
    /// watched). `shown_tab` is the terminal tab on screen (`None` for the Agents / Files
    /// tab): switching to one bubbles the cards of the agents on it.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: u64, focused: bool, c: &Theme, agents: &[Agent], notes: &[String], shown_tab: Option<usize>) {
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

        let width = body.width() - 2.0 * SIDE;
        let teams = groups(agents);
        // Teams that broke up, or whose lead is gone, close.
        self.expanded.retain(|lead, _| teams.iter().any(|g| g.key() == *lead && g.is_team()));
        let Layout { sections, cards, content_h } = layout(&teams, width, &self.expanded);
        self.remember_targets(&teams, &cards);
        if self.selected.is_none_or(|p| !self.targets.iter().any(|(t, _)| *t == p)) {
            self.selected = self.selected.and_then(|p| self.team_of.get(&p).copied()).or_else(|| self.targets.first().map(|(t, _)| *t));
        }
        let mut motion = std::mem::take(&mut self.view.motion);
        let laid: Vec<(&Agent, Rect)> = cards.iter().map(|(g, r)| (&g.lead, *r)).collect();
        motion.update(f.time, f.animate, &sections, &laid, width);
        for (g, _) in &cards {
            self.view.teams.insert(g.key(), (*g).clone());
        }
        self.view.teams.retain(|k, _| motion.cards.contains_key(k));
        let switched = self.view.shown_tab.replace(shown_tab).is_some_and(|was| was != shown_tab);
        if switched
            && f.animate
            && let Some(tab) = shown_tab
        {
            motion.pulse_tab(f.time, tab);
        }
        // Empty once the last card has faded away.
        if motion.cards.is_empty() {
            self.view.motion = motion;
            empty_state(&f, painter, body);
            return;
        }

        self.scroll_to(&f, body, content_h);
        let (busy, breathing) = self.paint_cards(&f, painter, body, &mut motion, shown_tab);
        motion.forget_gone(f.time);
        self.view.motion = motion;
        if busy {
            ui.ctx().request_repaint();
        }

        let span = ScrollSpan { offset: self.scroll, content: content_h, view: body.height() };
        if let Some(offset) = overlay_scrollbar(ui, painter, body, f.id("scrollbar"), span, &mut self.view.scroll_active) {
            self.scroll = offset;
        }

        // Elapsed times tick by the second; the working dots breathe smoothly.
        let next = if breathing && f.animate { Duration::from_millis(50) } else { Duration::from_secs(1) };
        ctx.request_repaint_after(next);
    }

    /// What the keys move through this frame: every card, and an open team's helper rows
    /// after it; and which team each member belongs to.
    fn remember_targets(&mut self, teams: &[Group], cards: &[(&Group, Rect)]) {
        self.targets.clear();
        for (g, r) in cards {
            self.targets.push((g.key(), Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H))));
            if let Some(&all) = self.expanded.get(&g.key()) {
                for (i, m) in shown_members(g, all).iter().enumerate() {
                    self.targets.push((m.agent.pane, row_rect(*r, i)));
                }
            }
        }
        self.team_of.clear();
        for g in teams.iter().filter(|g| g.is_team()) {
            self.team_of.insert(g.key(), g.key());
            for m in &g.members {
                self.team_of.insert(m.agent.pane, g.key());
            }
        }
    }

    /// Scroll with the wheel or a finger over `body`, then keep a card moved to with the
    /// keyboard in view.
    fn scroll_to(&mut self, f: &Frame, body: Rect, content_h: f32) {
        let ui = f.ui;
        let view_h = body.height();
        if ui.rect_contains_pointer(body) && !ui.input(|i| i.modifiers.command) {
            let d = ui.input(|i| i.smooth_scroll_delta.y);
            if d != 0.0 {
                self.scroll -= d;
                self.view.scroll_active = f.time;
            }
        }
        let d = self.view.pan.delta(ui, body);
        if d != 0.0 {
            self.scroll += d;
            self.view.scroll_active = f.time;
        }
        if std::mem::take(&mut self.view.reveal_selection)
            && let Some(&(id, r)) = self.targets.iter().find(|(t, _)| Some(*t) == self.selected)
        {
            // A card's section header comes along when it's the first one.
            let top = if self.team_of.get(&id).is_none_or(|lead| *lead == id) { r.top() - SECTION_H } else { r.top() };
            if top < self.scroll {
                self.scroll = top.max(0.0);
            } else if r.bottom() + GAP > self.scroll + view_h {
                self.scroll = r.bottom() + GAP - view_h;
            }
        }
        let clamped = self.scroll.clamp(0.0, (content_h - view_h).max(0.0));
        if clamped != self.scroll {
            self.view.pan.stop();
        }
        self.scroll = clamped;
    }

    /// The section headers and cards where `motion` has them now, scrolled, with clicks on
    /// the cards. Returns whether anything is still moving, and whether an agent is working
    /// (its dots breathe).
    fn paint_cards(&mut self, f: &Frame, painter: &Painter, body: Rect, motion: &mut Motion, shown_tab: Option<usize>) -> (bool, bool) {
        let ui = f.ui;
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
        // Lifted cards are drawn last, over the ones they pass.
        let mut ids: Vec<PaneId> = motion.cards.keys().copied().collect();
        ids.sort_by_key(|id| (motion.cards[id].lift(now, shown_tab) > 0.0, *id));
        let mut breathing = false;
        for id in ids {
            let Some(card) = motion.cards.get_mut(&id) else { continue };
            busy |= card.busy(now);
            let opacity = card.opacity.get(now);
            let full = card.rect.get(now).translate(origin);
            // Appearing and leaving cards grow from / shrink to a little smaller.
            let r = Rect::from_center_size(full.center(), full.size() * (0.96 + 0.04 * opacity));
            if !r.intersects(body) {
                continue;
            }
            let team = self.view.teams.get(&id).filter(|g| g.is_team()).cloned();
            // The part of the card that's the agent itself: a team's header.
            let head = if team.is_some() { Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H.min(r.height()))) } else { r };
            let mut hovered = false;
            if !card.leaving {
                let hit = f.interact(head, ("card", id));
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
            // An agent on the shown tab lifts off the grid when you switch to it or it
            // changes sections.
            let lift = card.lift(now, shown_tab);
            let r = Rect::from_center_size(r.center(), r.size() * (1.0 + 0.025 * lift));
            let look = Look { hover, lift, border: card.border.get(now), border_width: card.border_width.get(now), tint: card.tint.get(now) };
            let mut p = bp.clone();
            p.multiply_opacity(opacity);
            match team {
                Some(g) => {
                    breathing |= g.lead.status == Status::Working;
                    let leaving = card.leaving;
                    self.team_card(f, &p, r, &g, look, leaving);
                }
                None => {
                    breathing |= card.agent.status == Status::Working;
                    card_body(&p, r, &look);
                    agent_header(f, &p, r, &card.agent, &look, None);
                }
            }
        }
        (busy, breathing)
    }

    /// A team: its lead's header (with a disclosure chevron and the team's size), then its
    /// helpers as capsules (closed) or an inset list (open), cross-fading as the card grows.
    fn team_card(&mut self, f: &Frame, p: &Painter, r: Rect, g: &Group, look: Look, leaving: bool) {
        let key = g.key();
        let open = self.expanded.get(&key).copied();
        let closed_h = team_height(g, None);
        let open_h = team_height(g, Some(open.unwrap_or(false)));
        // How open it is, from the card's height as it eases between the two.
        let t = if open_h > closed_h { ((r.height() - closed_h) / (open_h - closed_h)).clamp(0.0, 1.0) } else { 0.0 };
        card_body(p, r, &look);
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H));
        let what = if g.is_pipeline() { "stage" } else { "helper" };
        let n = g.members.len();
        let size = format!("{n} {what}{}", if n == 1 { "" } else { "s" });
        agent_header(f, p, head, &g.lead, &look, Some((&size, t)));

        // Clicking the capsules, or the chevron's corner, opens and closes it.
        let toggle_area = Rect::from_min_max(Pos2::new(r.left(), head.bottom()), Pos2::new(r.right(), head.bottom() + STRIP_H)).intersect(r);
        if !leaving && t < 0.01 && let Some(hit) = f.interact(toggle_area, ("strip", key)) {
            if hit.clicked() {
                self.selected = Some(key);
                self.toggle_team(key);
            }
            hit.on_hover_cursor(egui::CursorIcon::PointingHand);
        }

        // A hairline under the header, inset like a grouped list's separators.
        let sep_alpha = 1.0 - t;
        if sep_alpha > 0.01 {
            let y = head.bottom();
            p.line_segment([Pos2::new(r.left() + 14.0, y), Pos2::new(r.right() - 14.0, y)], Stroke::new(1.0_f32, white_alpha((12.0 * sep_alpha) as u8)));
            let mut sp = p.clone();
            sp.multiply_opacity(sep_alpha);
            capsules(f, &sp, Rect::from_min_size(Pos2::new(r.left() + 14.0, y), Vec2::new(r.width() - 28.0, STRIP_H)), g);
        }
        if t > 0.01 {
            let mut lp = p.with_clip_rect(p.clip_rect().intersect(r.shrink(1.0)));
            lp.multiply_opacity(t);
            self.helper_list(f, &lp, r, g, open.unwrap_or(false), t > 0.99 && !leaving);
        }
    }

    /// An open team's helpers: one row each (tile, label, what it is, status), hairlines
    /// between, "Show all" when there are more than fit. `live`: settled, so rows take clicks.
    fn helper_list(&mut self, f: &Frame, p: &Painter, r: Rect, g: &Group, all: bool, live: bool) {
        let list = list_rect(r);
        p.rect_filled(list, 10, white_alpha(6));
        let key = g.key();
        let rows = shown_members(g, all);
        for (i, m) in rows.iter().enumerate() {
            let row = row_rect(r, i);
            let id = m.agent.pane;
            let mut hovered = false;
            if live && let Some(hit) = f.interact(row, ("row", id)) {
                hovered = hit.hovered();
                if hit.clicked() {
                    self.selected = Some(id);
                    self.requests.push(super::Request::Focus(id));
                }
                hit.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
            let hover = f.ui.ctx().animate_bool_with_time(f.id(("row-hover", id)), hovered, 0.12);
            if hover > 0.0 {
                p.rect_filled(row.shrink2(Vec2::new(4.0, 3.0)), 8, white_alpha((8.0 * hover) as u8));
            }
            if self.selected == Some(id) && f.focused {
                p.rect_stroke(row.shrink2(Vec2::new(4.0, 3.0)), 8, Stroke::new(1.5_f32, ACCENT.gamma_multiply(0.85)), StrokeKind::Inside);
            }
            helper_row(f, p, row, m, i + 1 < rows.len() || g.members.len() > rows.len());
        }
        if g.members.len() > ROWS_SHOWN {
            let more = Rect::from_min_size(Pos2::new(list.left(), list.top() + rows.len() as f32 * ROW_H), Vec2::new(list.width(), MORE_H));
            let text = if all { "Show Fewer".to_string() } else { format!("Show All {}", g.members.len()) };
            let mut color = ACCENT;
            if live && let Some(hit) = f.interact(more, ("more", key)) {
                if hit.hovered() {
                    color = ACCENT.gamma_multiply(1.2);
                }
                if hit.clicked() {
                    self.expanded.insert(key, !all);
                }
                hit.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
            p.text(more.center(), Align2::CENTER_CENTER, text, paint::bold(12.5), color);
        }
    }
}

/// Where the sections and cards go, in content coordinates (before scrolling).
struct Layout<'a> {
    /// Each status with cards: its count and its header's rect.
    sections: Vec<(Status, usize, Rect)>,
    cards: Vec<(&'a Group, Rect)>,
    content_h: f32,
}

/// Lay `teams` out `width` wide: a header per status, then its cards in as many columns as
/// fit, each team on a row of its own across them all (open ones as tall as their list).
fn layout<'a>(teams: &'a [Group], width: f32, expanded: &HashMap<PaneId, bool>) -> Layout<'a> {
    let columns = (((width + GAP) / (MIN_CARD_W + GAP)).floor() as usize).clamp(1, MAX_COLUMNS);
    let card_w = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    let mut sections: Vec<(Status, usize, Rect)> = Vec::new();
    let mut cards: Vec<(&Group, Rect)> = Vec::new();
    let mut y = 4.0;
    for status in Status::ALL {
        let group: Vec<&Group> = teams.iter().filter(|g| g.lead.status == status).collect();
        if group.is_empty() {
            continue;
        }
        sections.push((status, group.len(), Rect::from_min_size(Pos2::new(SIDE, y), Vec2::new(width, SECTION_H))));
        y += SECTION_H;
        let mut col = 0;
        for g in group {
            if g.is_team() {
                if col > 0 {
                    y += CARD_H + GAP;
                    col = 0;
                }
                let h = team_height(g, expanded.get(&g.key()).copied());
                cards.push((g, Rect::from_min_size(Pos2::new(SIDE, y), Vec2::new(width, h))));
                y += h + GAP;
            } else {
                let at = Pos2::new(SIDE + col as f32 * (card_w + GAP), y);
                cards.push((g, Rect::from_min_size(at, Vec2::new(card_w, CARD_H))));
                col += 1;
                if col == columns {
                    y += CARD_H + GAP;
                    col = 0;
                }
            }
        }
        if col > 0 {
            y += CARD_H + GAP;
        }
        y += 6.0;
    }
    Layout { sections, cards, content_h: y + 12.0 }
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
/// amber when its agent needs you, blue when it's done, a quiet hairline otherwise (brighter under the pointer).
fn border_for(status: Status, selected: bool, focused: bool, hovered: bool, c: &Theme) -> (Color32, f32) {
    if selected && focused {
        (ACCENT.gamma_multiply(0.85), 1.5)
    } else if status == Status::NeedsYou {
        (c.yellow.gamma_multiply(0.45), 1.0)
    } else if status == Status::Done {
        (c.blue.gamma_multiply(0.4), 1.0)
    } else if selected {
        (white_alpha(40), 1.0)
    } else if hovered {
        (white_alpha(26), 1.0)
    } else {
        (white_alpha(14), 1.0)
    }
}

/// A card's glass: its shadow (deeper when lifted), fill, top edge and border.
fn card_body(p: &Painter, r: Rect, look: &Look) {
    let Look { hover, lift, border, border_width, .. } = *look;
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
        Stroke::new(1.0_f32, white_alpha(14)),
    );
    p.rect_stroke(r, CARD_RADIUS, Stroke::new(border_width, border), StrokeKind::Inside);
}

/// An agent's header, `r` being its 74-point-tall card (or a team card's top): app tile, its
/// task over its folder and tab, and the status pill over how long it has been in that
/// state. A team's (`team`: its size, and how open it is) also says how big it is and has
/// a disclosure chevron.
fn agent_header(f: &Frame, p: &Painter, r: Rect, agent: &Agent, look: &Look, team: Option<(&str, f32)>) {
    let c = f.c;
    let tile = Rect::from_center_size(Pos2::new(r.left() + 14.0 + TILE / 2.0, r.top() + CARD_H / 2.0), Vec2::splat(TILE));
    icons::app_tile(p, tile, agent.kind.process(), agent.mark.as_deref());

    // The status pill, top right, with the time in that state under it.
    let color = look.tint;
    let pill_font = paint::bold(11.0);
    let label = p.layout_no_wrap(agent.status.label().to_string(), pill_font, color);
    let pill = Rect::from_min_size(Pos2::new(r.right() - 14.0 - (label.size().x + 26.0), r.top() + 14.0), Vec2::new(label.size().x + 26.0, 22.0));
    p.rect_filled(pill, 11, color.gamma_multiply(0.15));
    let dot = Pos2::new(pill.left() + 12.0, pill.center().y);
    status_dot(f, p, dot, agent.status, color);
    p.galley(Pos2::new(dot.x + 8.0, pill.center().y - label.size().y / 2.0), label, color);
    let since = elapsed(f.now.saturating_duration_since(agent.since));
    let since_y = r.top() + 51.0;
    let since_w = p.layout_no_wrap(since.clone(), ui_font(11.5), c.text_dim).size().x;
    p.text(Pos2::new(pill.right() - 4.0, since_y), Align2::RIGHT_CENTER, &since, ui_font(11.5), c.text_dim);

    // A team's chevron, left of the pill: pointing right while closed, down when open.
    let mut title_right = pill.left() - 12.0;
    if let Some((_, open)) = team {
        let at = Pos2::new(pill.left() - 16.0, pill.center().y);
        chevron(p, at, open, c.text_dim);
        title_right = at.x - 12.0;
    }

    // The task, and where it runs.
    let x = tile.right() + 13.0;
    let title_room = (title_right - x).max(0.0);
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
    if let Some((size, _)) = team {
        detail.push_str(" · ");
        detail.push_str(size);
    }
    if let Some(dir) = &agent.cwd {
        detail.push_str(" · ");
        // A remote folder comes already shortened with the host's own home.
        detail.push_str(&if remote.is_some() { dir.to_string_lossy().into_owned() } else { crate::terminal::tilde(dir) });
    }
    match (agent.tab, remote) {
        (Some(tab), _) => detail.push_str(&format!(" · Tab {}", tab + 1)),
        (None, Some((_, false))) if team.is_none() => detail.push_str(" · Detached"),
        _ => {}
    }
    let detail_room = (pill.right() - 4.0 - since_w - 12.0 - x).max(0.0);
    paint::faded_text(p, Pos2::new(x, since_y), &detail, ui_font(12.0), c.text_dim, detail_room, 18.0);
}

/// A disclosure chevron at `at`, turning from › (`open` 0) to ⌄ (`open` 1).
fn chevron(p: &Painter, at: Pos2, open: f32, color: Color32) {
    let angle = open * std::f32::consts::FRAC_PI_2;
    let (s, c) = angle.sin_cos();
    let rot = |v: Vec2| at + Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c);
    let stroke = Stroke::new(1.6_f32, color);
    let tip = rot(Vec2::new(2.5, 0.0));
    p.line_segment([rot(Vec2::new(-1.5, -4.0)), tip], stroke);
    p.line_segment([tip, rot(Vec2::new(-1.5, 4.0))], stroke);
}

/// A closed team's helpers in `strip`: a capsule each (status and label), the lead's own
/// helpers only, joined by hairlines into steps for a pipeline, `+N` for what doesn't fit.
fn capsules(f: &Frame, p: &Painter, strip: Rect, g: &Group) {
    let c = f.c;
    let direct: Vec<&Member> = g.direct().collect();
    let y = strip.center().y;
    let font = paint::bold(11.5);
    let join = if g.is_pipeline() { 16.0 } else { 6.0 };
    let more_w = 44.0;
    let mut x = strip.left();
    for (i, m) in direct.iter().enumerate() {
        let label = p.layout_no_wrap(member_label(&m.agent).to_string(), font.clone(), c.text_bright);
        let w = label.size().x + 30.0;
        let room_left = strip.right() - x - if i + 1 < direct.len() { more_w + join } else { 0.0 };
        if w > room_left {
            let rest = direct.len() - i;
            let more = Rect::from_min_size(Pos2::new(x, y - CAPSULE_H / 2.0), Vec2::new(more_w, CAPSULE_H));
            p.rect_filled(more, 12, white_alpha(9));
            p.text(more.center(), Align2::CENTER_CENTER, format!("+{rest}"), font.clone(), c.text_dim);
            return;
        }
        if i > 0 && g.is_pipeline() {
            p.line_segment([Pos2::new(x - join + 3.0, y), Pos2::new(x - 3.0, y)], Stroke::new(1.0_f32, white_alpha(40)));
        }
        let cap = Rect::from_min_size(Pos2::new(x, y - CAPSULE_H / 2.0), Vec2::new(w, CAPSULE_H));
        let color = status_color(m.agent.status, c);
        let fill = if m.agent.status == Status::NeedsYou { c.yellow.gamma_multiply(0.14) } else { white_alpha(9) };
        p.rect_filled(cap, 12, fill);
        member_glyph(f, p, Pos2::new(cap.left() + 12.0, y), m.agent.status, color);
        p.galley(Pos2::new(cap.left() + 21.0, y - label.size().y / 2.0), label, c.text_bright);
        x = cap.right() + join;
    }
}

/// One helper's row in an open team: tile (indented for a helper's own helpers), its label
/// over what it is, and its status with how long it has been in it. `rule`: a hairline
/// under it.
fn helper_row(f: &Frame, p: &Painter, row: Rect, m: &Member, rule: bool) {
    let c = f.c;
    let a = &m.agent;
    let indent = 16.0 * (m.depth.saturating_sub(1).min(3)) as f32;
    let tile = Rect::from_center_size(Pos2::new(row.left() + 12.0 + indent + 13.0, row.center().y), Vec2::splat(26.0));
    icons::app_tile(p, tile, a.kind.process(), a.mark.as_deref());
    let x = tile.right() + 10.0;

    let color = status_color(a.status, c);
    let since = elapsed(f.now.saturating_duration_since(a.since));
    let since_g = p.layout_no_wrap(since, ui_font(11.5), c.text_dim);
    let since_x = row.right() - 12.0 - since_g.size().x;
    let label_g = p.layout_no_wrap(a.status.label().to_string(), paint::bold(11.0), color);
    let label_x = since_x - 12.0 - label_g.size().x;
    let dot = Pos2::new(label_x - 9.0, row.center().y);
    status_dot(f, p, dot, a.status, color);
    p.galley(Pos2::new(label_x, row.center().y - label_g.size().y / 2.0), label_g, color);
    p.galley(Pos2::new(since_x, row.center().y - since_g.size().y / 2.0), since_g, c.text_dim);

    let room = (dot.x - 14.0 - x).max(0.0);
    let label = member_label(a);
    paint::faded_text(p, Pos2::new(x, row.top() + 16.0), label, paint::bold(13.0), c.text_bright, room, 16.0);
    let mut detail = a.kind.label().to_string();
    if a.name != label {
        detail = format!("{} · {}", a.name, detail);
    }
    paint::faded_text(p, Pos2::new(x, row.top() + 32.0), &detail, ui_font(11.5), c.text_dim, room, 16.0);
    if rule {
        p.line_segment([Pos2::new(x, row.bottom()), Pos2::new(row.right(), row.bottom())], Stroke::new(1.0_f32, white_alpha(10)));
    }
}

/// A helper's status in a capsule: a tick once it's done, else its dot.
fn member_glyph(f: &Frame, p: &Painter, at: Pos2, status: Status, color: Color32) {
    if status == Status::Done {
        let stroke = Stroke::new(1.6_f32, color);
        p.line_segment([at + Vec2::new(-3.5, 0.2), at + Vec2::new(-1.0, 2.8)], stroke);
        p.line_segment([at + Vec2::new(-1.0, 2.8), at + Vec2::new(3.8, -2.6)], stroke);
    } else {
        status_dot(f, p, at, status, color);
    }
}

/// The status dot. While working it breathes, and a soft ring keeps rippling out of it.
fn status_dot(f: &Frame, p: &Painter, at: Pos2, status: Status, color: Color32) {
    if status != Status::Working || !f.animate {
        p.circle_filled(at, 3.5, color);
        return;
    }
    let t = (f.time * 1.1).fract() as f32;
    p.circle_stroke(at, 3.5 + 5.0 * t, Stroke::new(1.2_f32, color.gamma_multiply(0.55 * (1.0 - t) * (1.0 - t))));
    let breathe = 0.5 + 0.5 * (f.time as f32 * std::f32::consts::TAU * 0.55).sin();
    p.circle_filled(at, 3.2 + 0.6 * breathe, color.gamma_multiply(0.75 + 0.25 * breathe));
}

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

/// Nothing to show: no agents running.
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agents::model::{AgentKind, Team};

    fn agent(pane: PaneId, parent: Option<PaneId>) -> Agent {
        Agent {
            pane,
            tab: None,
            kind: AgentKind::Claude,
            name: format!("a{pane}"),
            mark: None,
            cwd: None,
            status: Status::Idle,
            since: Instant::now(),
            source: Source::Local,
            team: Team { parent, role: None, started: pane },
        }
    }

    #[test]
    fn a_team_takes_a_row_of_its_own_and_grows_when_open() {
        // Two single cards, then a team of 8, then another single: 2 columns at 700.
        let mut agents = vec![agent(1, None), agent(2, None), agent(3, None), agent(9, None)];
        agents.extend((10..18).map(|p| agent(p, Some(3))));
        let teams = groups(&agents);
        let closed = layout(&teams, 700.0, &HashMap::new());
        let rect = |l: &Layout, key| l.cards.iter().find(|(g, _)| g.key() == key).unwrap().1;
        assert_eq!(closed.cards.len(), 4, "helpers are inside their team's card");
        let team = rect(&closed, 3);
        assert_eq!((team.width(), team.height()), (700.0, CARD_H + STRIP_H));
        assert!(rect(&closed, 1).top() == rect(&closed, 2).top() && rect(&closed, 1).width() < 400.0, "singles share a row");
        assert!(rect(&closed, 9).top() > team.bottom() || rect(&closed, 9).bottom() < team.top(), "nothing beside a team");

        let open = layout(&teams, 700.0, &HashMap::from([(3, false)]));
        assert_eq!(rect(&open, 3).height(), CARD_H + 2.0 + ROWS_SHOWN as f32 * ROW_H + MORE_H + LIST_INSET);
        let all = layout(&teams, 700.0, &HashMap::from([(3, true)]));
        assert_eq!(rect(&all, 3).height(), CARD_H + 2.0 + 8.0 * ROW_H + MORE_H + LIST_INSET);
        assert!(all.content_h > open.content_h && open.content_h > closed.content_h);
        assert_eq!(row_rect(rect(&open, 3), 1).top() - row_rect(rect(&open, 3), 0).top(), ROW_H);
    }

    #[test]
    fn helpers_are_called_by_their_role() {
        let mut a = agent(1, None);
        assert_eq!(member_label(&a), "a1");
        a.team.role = Some("review".into());
        assert_eq!(member_label(&a), "review");
    }
}
