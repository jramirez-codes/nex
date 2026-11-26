//! Drawing the Agents pane, in the style of a macOS / iOS dashboard: grouped sections of
//! rounded agent cards (two or three to a row when there's room). Each card has the agent's
//! app tile, its task over the folder it works in, and a status pill whose dot breathes while the agent
//! works. A pipeline's card says how its run stands instead (`Feature · 2 building · 1 merged`)
//! over the last thing that happened, from the title its runner sets. Mouse handling lives
//! here too.
//!
//! A team (see `group`) is one card. Closed, it's a cell of the grid like any other card,
//! with a row of capsules under its header for its helpers (a pipeline's joined into steps,
//! a pipeline it asked for opened into its stages); open, it spans the row and they're an
//! inset list inside the card, like a grouped table. Opening is the card growing: its rows are
//! uncovered as it does, and the capsules fade as the list comes in. A helper goes by a tag
//! ([`member_tag`](text::member_tag): `Build-01`): its word on its capsule, the whole tag in a small capsule
//! leading its row, before the task its terminal's title says it's on. A pipeline's stages
//! hang off it by a tree connector each (`icons::icon_branch`), a step further in.
//!
//! A card whose agent is on screen says so (`viewing`): a "Viewing" pill left of its status
//! and a soft accent ring, fuller (with a glow) for the agent in the focused pane than for
//! one sharing its tab. The mark eases in and out (see `motion`'s `SeenFade`), on every card
//! and helper row alike.
//!
//! A pipeline's tile carries a progress ring (how many of its stages have finished) and an
//! iOS-style badge on its top-left corner, where a tab's attention dot sits (an amber `!`
//! while it needs you, a green tick while it's done and unseen); a stage that finishes draws
//! its tick in. The timing is `motion`'s `TileMotion`
//! and `TeamMotion::tick`; the shapes are `icons`' `tile_ring`, `badge_alert` and `badge_done`.
//!
//! A frame copies no agent: the teams are worked out when the tracker's list changes (its
//! generation) and kept, shared, until it changes again.
//!
//! Helpers are in the order `group` gives them (by status), and are drawn where `motion`
//! has them: when one changes state, its row glides to its new slot over the others (which
//! slide under it to make room) and its capsule slides along the strip. Clicks go where a
//! row is drawn; the keys go by the slots the rows are headed for.
//! A closed card's capsules open it, but a sub-agent's, which opens its tab (the one
//! following its transcript) like its row. A team's sub-agents are drawn as a pipeline's
//! stages are: each hangs off its agent by a connector, tagged by its type, over its folder.
//!
//! The work divides by role: `text` (what cards, rows and capsules say), `card` (what a card
//! shows that only changes with the list, worked out once per change of it), `layout` (where
//! everything goes), `paint` (the leaf painters), `viewing` (the "Viewing" mark), and here
//! the state kept between frames ([`ViewState`]), what a frame shares (`Frame`), and the
//! orchestration: `paint`, scrolling, the cards, a team's card, its helper list and its
//! capsules. Tests are in `tests`.
//!
//! A frame does as little as it can: the cards (`card::Cards`) are made again only when the
//! list's generation changes, the layout and what the keys move through only when the
//! cards, the width or the open teams do (`LaidOut`); what's left per frame is what moves
//! or ticks (the motion, the time in a state, what's on screen, the pointer).

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::{Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::agents::AgentsPane;
use crate::agents::group::Member;
use crate::agents::model::{Agent, Agents, Status, Viewing};
use crate::agents::motion::{Motion, TeamMotion};
use crate::agents::view::card::{Card, Cards, Header};
use crate::agents::view::layout::{
    Layout, capsule_layout, chevron_hit, layout, list_rect, row_depth, row_rect, row_text_x, shown_members, team_height,
};
use crate::agents::view::paint::{
    Look, TileLook, agent_header, border_for, branch_lines, card_body, empty_state, helper_row, host_notes, member_glyph, needs_you_tint,
    section_header, tile_look,
};
use crate::agents::view::text::{header_text, member_detail};
use crate::layout::PaneId;
use crate::ui;
use crate::ui::glide::animations;
use crate::ui::theme::{ACCENT, Theme, lighten, white_alpha};
use crate::ui::touch_pan::TouchPan;
use crate::ui::widgets::{ScrollSpan, overlay_scrollbar};

mod card;
mod layout;
mod paint;
#[cfg(test)]
mod tests;
mod text;
mod viewing;

const SIDE: f32 = 18.0;
const CARD_H: f32 = 74.0;
const CARD_RADIUS: u8 = 14;
const GAP: f32 = 10.0;
const SECTION_H: f32 = 34.0;
const MIN_CARD_W: f32 = 300.0;
const MAX_COLUMNS: usize = 3;
const TILE: f32 = 40.0;
/// A card's title and the quiet line under it, centred together on the tile.
const TITLE_Y: f32 = 28.0;
const DETAIL_Y: f32 = 47.0;
/// A closed team's row of capsules, under its header.
const STRIP_H: f32 = 40.0;
const CAPSULE_H: f32 = 24.0;
/// The `+N` capsule for the helpers that don't fit.
const MORE_W: f32 = 44.0;
/// The capsule a helper's tag sits in, leading its row.
const TAG_H: f32 = 18.0;
/// An open team's list: its rows, the margin around it, and the "Show all" row.
const ROW_H: f32 = 46.0;
/// How much further in a helper's own helpers sit in an open team's list.
const INDENT_STEP: f32 = 22.0;
const LIST_INSET: f32 = 12.0;
const MORE_H: f32 = 34.0;
/// Helpers listed before "Show all".
const ROWS_SHOWN: usize = 6;

/// What the view keeps between frames.
pub(super) struct ViewState {
    /// Scroll so the highlighted card is in view on the next frame (after a key moved it).
    pub reveal_selection: bool,
    /// A card (or helper row) jumped to its agent since a key last moved the highlight: the
    /// highlight isn't drawn, so coming back from that agent's tab no card is ringed for
    /// it. It's still where Enter goes.
    pub(super) jumped: bool,
    scroll_active: f64,
    /// Scrolling the cards with a finger.
    pan: TouchPan,
    /// Where each card and section header is on its way to.
    motion: Motion,
    /// The terminal tab shown when it was last drawn (`None` inside: the Agents / Files
    /// tab); `None` before the first frame.
    shown_tab: Option<Option<usize>>,
    /// Each card as last laid out, kept while it fades away.
    cards: HashMap<PaneId, Arc<Card>>,
    /// The agents as cards, made again only when the list's generation changes.
    grouped: Option<Cards>,
    /// Where the cards go: kept until the cards, the width or the open teams change.
    laid: Option<Arc<LaidOut>>,
    /// What was on screen last frame: a helper newly in view opens its team.
    viewing: Viewing,
    /// The order the cards are drawn in this frame (kept to reuse its room).
    draw_order: Vec<(PaneId, f32)>,
}

impl ViewState {
    pub(super) fn new() -> ViewState {
        ViewState {
            reveal_selection: false,
            jumped: false,
            scroll_active: -10.0,
            pan: TouchPan::default(),
            motion: Motion::new(),
            shown_tab: None,
            cards: HashMap::new(),
            grouped: None,
            laid: None,
            viewing: Viewing::default(),
            draw_order: Vec::new(),
        }
    }

    /// `agents` as cards: the ones made last time, unless the list changed since. Says
    /// whether they were made again.
    fn grouped(&mut self, agents: &Agents) -> (Arc<[Arc<Card>]>, bool) {
        if let Some(cards) = self.grouped.as_ref().filter(|c| c.generation == agents.generation()) {
            return (Arc::clone(&cards.cards), false);
        }
        let cards = Cards::new(agents);
        let shared = Arc::clone(&cards.cards);
        self.grouped = Some(cards);
        (shared, true)
    }

    /// Where `cards` go `width` wide with `expanded` open: last frame's layout while none of
    /// them changed, else a new one. Says whether it's new.
    fn laid_out(&mut self, cards: &Arc<[Arc<Card>]>, width: f32, expanded: &HashMap<PaneId, bool>) -> (Arc<LaidOut>, bool) {
        if let Some(laid) = self.laid.as_ref().filter(|l| l.fits(cards, width, expanded)) {
            return (Arc::clone(laid), false);
        }
        let laid = Arc::new(LaidOut::new(cards, width, expanded));
        self.laid = Some(Arc::clone(&laid));
        (laid, true)
    }
}

/// A layout of the cards, and what it was made from (see [`ViewState::laid_out`]).
struct LaidOut {
    cards: Arc<[Arc<Card>]>,
    width: f32,
    expanded: HashMap<PaneId, bool>,
    layout: Layout<Card>,
    /// Each laid card's lead, where it goes: what `Motion` heads for.
    leads: Vec<(Arc<Agent>, Rect)>,
}

impl LaidOut {
    fn new(cards: &Arc<[Arc<Card>]>, width: f32, expanded: &HashMap<PaneId, bool>) -> LaidOut {
        let layout = layout(cards, width, expanded);
        let leads = layout.cards.iter().map(|(c, r)| (Arc::clone(&c.group.lead), *r)).collect();
        LaidOut { cards: Arc::clone(cards), width, expanded: expanded.clone(), layout, leads }
    }

    /// Whether it is the layout of `cards`, `width` wide with `expanded` open.
    fn fits(&self, cards: &Arc<[Arc<Card>]>, width: f32, expanded: &HashMap<PaneId, bool>) -> bool {
        Arc::ptr_eq(&self.cards, cards) && self.width == width && self.expanded == *expanded
    }
}

/// What [`AgentsPane::paint`] draws, besides where.
pub struct PaintArgs<'a> {
    /// The pane's id, for its widgets' ids.
    pub pane: u64,
    pub focused: bool,
    pub c: &'a Theme,
    /// All the agents, sorted.
    pub agents: &'a Agents,
    /// One line per ssh host that can't be watched, shown above the cards.
    pub notes: &'a [String],
    /// The terminal tab on screen (`None` for the Agents / Files tab): switching to one
    /// bubbles the cards of the agents on it.
    pub shown_tab: Option<usize>,
    /// Which agents are on screen, whose cards say so.
    pub viewing: &'a Viewing,
}

/// Everything a frame's drawing shares.
struct Frame<'a> {
    ui: &'a Ui,
    c: &'a Theme,
    focused: bool,
    pane: u64,
    /// Which agents are on screen, for their cards' "Viewing" mark.
    viewing: &'a Viewing,
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

impl AgentsPane {
    /// Draw the pane into `rect` (below the pane header, edge to edge of the card), showing
    /// what `args` says (see [`PaintArgs`]).
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, args: PaintArgs) {
        let PaintArgs { pane, focused, c, agents, notes, shown_tab, viewing } = args;
        let ctx = ui.ctx().clone();
        let f = Frame {
            ui,
            c,
            focused,
            pane,
            viewing,
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
        let (cards, regrouped) = self.view.grouped(agents);
        if regrouped {
            self.regrouped(&cards);
        }
        // A helper coming into view opens its team, so its row is there to be marked.
        if self.view.viewing != *viewing {
            let was = std::mem::replace(&mut self.view.viewing, viewing.clone());
            for (lead, all) in viewing::teams_to_open(viewing, &was, &cards, ROWS_SHOWN) {
                let shown_all = self.expanded.entry(lead).or_insert(false);
                *shown_all |= all;
            }
        }
        let (laid, relaid) = self.view.laid_out(&cards, width, &self.expanded);
        if relaid {
            self.note_layout(&laid.layout);
        }
        if self.selected.is_none_or(|p| !self.targets.iter().any(|(t, _)| *t == p)) {
            self.selected = self.selected.and_then(|p| self.team_of.get(&p).copied()).or_else(|| self.targets.first().map(|(t, _)| *t));
        }
        let content_h = laid.layout.content_h;
        let mut motion = std::mem::take(&mut self.view.motion);
        motion.update(f.time, f.animate, &laid.layout.sections, &laid.leads, width);
        // The cards kept are the ones `motion` still draws: the laid ones, and those fading.
        if self.view.cards.len() > motion.cards.len() {
            self.view.cards.retain(|k, _| motion.cards.contains_key(k));
        }
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

    /// The cards were made again (the list changed): teams that broke up, or whose lead is
    /// gone, close, and each member's team is noted for the keys.
    fn regrouped(&mut self, cards: &[Arc<Card>]) {
        self.team_of.clear();
        for c in cards.iter().filter(|c| c.group.is_team()) {
            self.team_of.insert(c.key(), c.key());
            for m in &c.group.members {
                self.team_of.insert(m.agent.pane, c.key());
            }
        }
        // A team's lead is its own team: what's open is what's still a team.
        self.expanded.retain(|lead, _| self.team_of.get(lead) == Some(lead));
    }

    /// The layout changed: what the keys move through (every card, and an open team's
    /// helper rows after it), and the cards to draw.
    fn note_layout(&mut self, laid: &Layout<Card>) {
        self.targets.clear();
        for (c, r) in &laid.cards {
            self.targets.push((c.key(), Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H))));
            if let Some(&all) = self.expanded.get(&c.key()) {
                for (i, m) in shown_members(&c.group, all).iter().enumerate() {
                    self.targets.push((m.agent.pane, row_rect(*r, i)));
                }
            }
            self.view.cards.insert(c.key(), Arc::clone(c));
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
        let mut ids = std::mem::take(&mut self.view.draw_order);
        ids.clear();
        ids.extend(motion.cards.iter().map(|(id, card)| (*id, card.lift(now, shown_tab))));
        ids.sort_by_key(|&(id, lift)| (lift > 0.0, id));
        let mut breathing = false;
        for &(id, lift) in &ids {
            let Some(card) = motion.cards.get_mut(&id) else { continue };
            busy |= card.busy(now);
            let opacity = card.opacity.get(now);
            let full = card.rect.get(now).translate(origin);
            // Appearing and leaving cards grow from / shrink to a little smaller.
            let r = Rect::from_center_size(full.center(), full.size() * (0.96 + 0.04 * opacity));
            if !r.intersects(body) {
                continue;
            }
            // An agent on the shown tab lifts off the grid when you switch to it or it
            // changes sections. Clicks go where it's drawn, lifted.
            let r = Rect::from_center_size(r.center(), r.size() * (1.0 + 0.025 * lift));
            let Some(drawn) = self.view.cards.get(&id).cloned() else { continue };
            let team = drawn.group.is_team().then_some(&*drawn);
            // The width the card is laid out at (not the one it's drawn at while it lifts).
            let laid_w = card.rect.target().width();
            // The part of the card that's the agent itself: a team's header.
            let head = if team.is_some() { Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H.min(r.height()))) } else { r };
            let mut hovered = false;
            if !card.leaving {
                let hit = f.interact(head, ("card", id));
                hovered = hit.as_ref().is_some_and(|h| h.hovered());
                if hit.as_ref().is_some_and(|h| h.clicked()) {
                    self.jump(id);
                }
                if let Some(h) = hit {
                    h.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
            let hover = ui.ctx().animate_bool_with_time(f.id(("hover", id)), hovered, 0.12);
            let selected = !card.leaving && self.highlighted() == Some(id);
            // On screen: the agent itself, or for a team its lead or one of its helpers.
            let seen = if card.leaving {
                None
            } else {
                match team {
                    Some(t) => viewing::team_seen(f.viewing, &t.group),
                    None => f.viewing.of(&card.agent),
                }
            };
            // The selected card in the focused pane keeps the keyboard's ring over the viewed one.
            let ring = if selected && f.focused { None } else { seen };
            let (border, border_width) = border_for(card.agent.status, selected, f.focused, hovered, f.c);
            card.set_colors(now, border, border_width, status_color(card.agent.status, f.c), seen, ring, f.animate);
            let (border, border_width) =
                viewing::ring_over((card.border.get(now), card.border_width.get(now)), card.ring.shown(now), card.ring.primary(now));
            let (shown, primary) = (card.seen.shown(now), card.seen.primary(now));
            // A pipeline's tile: a ring for how far its stages are, a badge for how it stands.
            let tile = team.and_then(|t| t.pipeline).map(|(progress, _, working)| {
                card.tile.update(now, progress, drawn.group.lead.status, seen.is_some(), f.animate);
                busy |= card.tile.busy(now);
                breathing |= card.tile.breathing();
                tile_look(f, &card.tile, working)
            });
            let look = Look {
                hover,
                lift,
                border,
                border_width,
                tint: card.tint.get(now),
                glow: viewing::glow_strength(shown, primary),
                seen: (shown, primary),
                tile,
            };
            let mut p = bp.clone();
            p.multiply_opacity(opacity);
            match team {
                Some(t) => {
                    breathing |= t.breathes;
                    let leaving = card.leaving;
                    // Its rows and capsules slide, unless the card itself was resized.
                    let inside = motion.teams.entry(id).or_default();
                    let slide = f.animate && !inside.resized(laid_w);
                    self.team_card(f, &p, r, t, look, leaving, inside, slide);
                    busy |= inside.busy(now);
                }
                None => {
                    motion.teams.remove(&id);
                    breathing |= card.agent.status == Status::Working;
                    card_body(&p, r, &look);
                    // Its words were worked out with the card, for the lead it was laid out
                    // with: `motion`'s agent is that one.
                    let header = if Arc::ptr_eq(&drawn.group.lead, &card.agent) { None } else { Some(Header::from(header_text(&card.agent, None))) };
                    agent_header(f, &p, r, &card.agent, &look, header.as_ref().unwrap_or(&drawn.header), None);
                }
            }
        }
        self.view.draw_order = ids;
        (busy, breathing)
    }

    /// A team: its lead's header (with a disclosure chevron and the team's size), then its
    /// helpers as capsules (closed) or an inset list (open), cross-fading as the card grows.
    /// `inside`: where its rows and capsules are; `slide`: whether they glide to new places.
    #[allow(clippy::too_many_arguments)]
    fn team_card(&mut self, f: &Frame, p: &Painter, r: Rect, card: &Card, look: Look, leaving: bool, inside: &mut TeamMotion, slide: bool) {
        let g = &*card.group;
        let key = g.key();
        // A helper that has just finished draws its tick in, in its capsule and its row.
        inside.note_statuses(f.time, f.animate, g.members.iter().map(|m| &*m.agent));
        let open = self.expanded.get(&key).copied();
        let closed_h = team_height(g, None);
        let open_h = team_height(g, Some(open.unwrap_or(false)));
        // How open it is, from the card's height as it eases between the two.
        let t = if open_h > closed_h { ((r.height() - closed_h) / (open_h - closed_h)).clamp(0.0, 1.0) } else { 0.0 };
        card_body(p, r, &look);
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H));
        let pill = agent_header(f, p, head, &g.lead, &look, &card.header, Some(t));

        // Clicking the chevron opens and closes it (over the header's own click, which
        // jumps to the lead); so do the capsules while it's closed, but a sub-agent's, which
        // jumps to the agent running it.
        if !leaving && let Some(hit) = f.interact(chevron_hit(pill), ("chevron", key)) {
            if hit.clicked() {
                self.selected = Some(key);
                self.toggle_team(key);
            }
            hit.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
        let toggle_area = Rect::from_min_max(Pos2::new(r.left(), head.bottom()), Pos2::new(r.right(), head.bottom() + STRIP_H)).intersect(r);
        if !leaving
            && t < 0.01
            && let Some(hit) = f.interact(toggle_area, ("strip", key))
        {
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
            p.line_segment(
                [Pos2::new(r.left() + 14.0, y), Pos2::new(r.right() - 14.0, y)],
                Stroke::new(1.0_f32, white_alpha((12.0 * sep_alpha) as u8)),
            );
            let mut sp = p.clone();
            sp.multiply_opacity(sep_alpha);
            let subs =
                capsules(f, &sp, Rect::from_min_size(Pos2::new(r.left() + 14.0, y), Vec2::new(r.width() - 28.0, STRIP_H)), card, inside, slide);
            // Over the strip's click (the later one takes it).
            for (pane, cap) in subs {
                if !leaving
                    && t < 0.01
                    && let Some(hit) = f.interact(cap, ("capsule", pane))
                {
                    if hit.clicked() {
                        self.jump(pane);
                    }
                    hit.on_hover_cursor(egui::CursorIcon::PointingHand);
                }
            }
        } else {
            inside.close_capsules();
        }
        if t > 0.01 {
            let mut lp = p.with_clip_rect(p.clip_rect().intersect(r.shrink(1.0)));
            lp.multiply_opacity(t);
            self.helper_list(f, &lp, r, card, open.unwrap_or(false), t > 0.99 && !leaving, inside, slide);
        } else {
            inside.close_rows();
        }
    }

    /// An open team's helpers: one row each (tile, tag and task, folder, status), hairlines
    /// between, "Show all" when there are more than fit. `live`: settled, so rows take clicks.
    /// Each row is drawn where `inside` has it on the way to its slot (`slide`: gliding
    /// there), the one whose helper just changed state over the others.
    #[allow(clippy::too_many_arguments)]
    fn helper_list(&mut self, f: &Frame, p: &Painter, r: Rect, card: &Card, all: bool, live: bool, inside: &mut TeamMotion, slide: bool) {
        let g = &*card.group;
        let list = list_rect(r);
        p.rect_filled(list, 10, white_alpha(6));
        let key = g.key();
        let now = f.time;
        let rows = shown_members(g, all);
        let slots: Vec<(&Member, &str, f32)> =
            rows.iter().zip(&card.members).enumerate().map(|(i, (m, t))| (m, t.tag.as_str(), i as f32 * ROW_H)).collect();
        inside.update_rows(now, slide, &slots);
        // Each row's "Viewing" mark fades in and out with its helper on screen.
        for m in inside.rows.values_mut() {
            let seen = if m.leaving {
                None
            } else {
                match card.member_index(m.member.agent.pane) {
                    Some(i) => viewing::seen_under(f.viewing, &m.member.agent, card.parent(i)),
                    None => viewing::member_seen(f.viewing, g, &m.member),
                }
            };
            m.seen.set(now, seen, f.animate);
        }
        // A pipeline among the helpers: its tile's ring and badge, from its own stages.
        let mut tiles: Vec<(PaneId, TileLook)> = Vec::new();
        for n in &card.nested {
            let member = &g.members[n.member];
            let Some(row) = inside.rows.get_mut(&member.agent.pane) else { continue };
            let (progress, status, working) = n.state;
            let viewed = |a: &Agent| f.viewing.of(a).is_some();
            let seen = !row.leaving && (viewed(&member.agent) || n.stages.iter().any(|&j| viewed(&g.members[j].agent)));
            row.tile.update(now, progress, status, seen, f.animate);
            tiles.push((member.agent.pane, tile_look(f, &row.tile, working)));
        }

        // The hairlines stay where the slots are, each starting at its slot's text.
        for (i, m) in rows.iter().enumerate() {
            if i + 1 < rows.len() || g.members.len() > rows.len() {
                let y = list.top() + (i + 1) as f32 * ROW_H;
                p.line_segment(
                    [Pos2::new(row_text_x(list.left(), row_depth(g, m)), y), Pos2::new(list.right(), y)],
                    Stroke::new(1.0_f32, white_alpha(10)),
                );
            }
        }
        branch_lines(p, list, g, &card.branches, rows.len());

        // Settled rows first, then the ones making room, then the ones whose helper changed
        // state (the latest on top), each on a backing so it reads as it passes over.
        // Each row's place in that order worked out once, not at every comparison.
        let mut order: Vec<((bool, bool), f64, PaneId)> = inside
            .rows
            .iter()
            .map(|(id, m)| {
                let sliding = m.sliding(now);
                ((m.status_change && sliding, sliding), m.changed, *id)
            })
            .collect();
        order.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)).then(a.2.cmp(&b.2)));
        let backing = lighten(f.c.card.to_opaque(), 0.07);
        for ((over, _), _, id) in order {
            let m = &inside.rows[&id];
            let row = Rect::from_min_size(Pos2::new(list.left(), list.top() + m.y.get(now)), Vec2::new(list.width(), ROW_H));
            let mut rp = p.clone();
            rp.multiply_opacity(m.opacity.get(now));
            let inner = row.shrink2(Vec2::new(4.0, 3.0));
            if over {
                // In as it sets off, out as it lands.
                let arc = (m.y.t(now) * std::f32::consts::PI).sin();
                rp.rect_filled(inner, 8, backing.gamma_multiply((arc * 4.0).min(1.0)));
            }
            let mut hovered = false;
            if live
                && !m.leaving
                && let Some(hit) = f.interact(row, ("row", id))
            {
                hovered = hit.hovered();
                if hit.clicked() {
                    self.jump(id);
                }
                hit.on_hover_cursor(egui::CursorIcon::PointingHand);
            }
            let seen = (m.seen.shown(now), m.seen.primary(now));
            let hover = f.ui.ctx().animate_bool_with_time(f.id(("row-hover", id)), hovered, 0.12);
            if hover > 0.0 {
                rp.rect_filled(inner, 8, white_alpha((8.0 * hover) as u8));
            }
            // As on a card, the keyboard's ring is drawn over the viewing one.
            let keyed = self.highlighted() == Some(id) && f.focused && !m.leaving;
            viewing::row_mark(&rp, inner, 8, seen.0, seen.1, !keyed);
            if keyed {
                rp.rect_stroke(inner, 8, Stroke::new(1.5_f32, ACCENT.gamma_multiply(0.85)), StrokeKind::Inside);
            }
            let glyph = (inside.tick(now, id), tiles.iter().find(|(p, _)| *p == id).map(|(_, t)| *t));
            // Its folder as worked out with the card, unless the row still shows an agent the
            // card has moved on from (fading out).
            let own = card.member_index(id).filter(|&i| Arc::ptr_eq(&g.members[i].agent, &m.member.agent));
            let detail = match own {
                Some(i) => std::borrow::Cow::Borrowed(card.members[i].detail.as_str()),
                None => std::borrow::Cow::Owned(member_detail(&m.member.agent)),
            };
            helper_row(f, &rp, row, &m.member, row_depth(g, &m.member), (&m.tag, &detail), seen, glyph);
        }
        if g.members.len() > ROWS_SHOWN {
            let more = Rect::from_min_size(Pos2::new(list.left(), list.top() + rows.len() as f32 * ROW_H), Vec2::new(list.width(), MORE_H));
            let text = if all { "Show Fewer" } else { card.show_all.as_str() };
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
            p.text(more.center(), Align2::CENTER_CENTER, text, ui::paint::bold(12.5), color);
        }
    }
}

/// A closed team's helpers in `strip`: a capsule each (status and its tag's word), the lead's own
/// helpers only, with a pipeline among them opened into its stages after its name (see
/// [`capsule_layout`]). A capsule is drawn where `inside` has it on the way to its place
/// (`slide`: gliding there when the order changes); one going into the `+N` fades.
/// Returns where the sub-agents' capsules are, which take clicks of their own.
fn capsules(f: &Frame, p: &Painter, strip: Rect, card: &Card, inside: &mut TeamMotion, slide: bool) -> Vec<(PaneId, Rect)> {
    let c = f.c;
    let now = f.time;
    let steps = &card.strip;
    let y = strip.center().y;
    let font = ui::paint::bold(11.5);
    // The labels were worked out with the card; egui keeps their shaped text between frames.
    let galleys: Vec<_> = steps
        .iter()
        .zip(&card.labels)
        .map(|(step, label)| p.layout_no_wrap(label.clone(), font.clone(), if step.agent().is_some() { c.text_bright } else { c.text_dim }))
        .collect();
    let widths: Vec<f32> = steps.iter().zip(&galleys).map(|(s, g)| g.size().x + if s.agent().is_some() { 30.0 } else { 0.0 }).collect();
    let laid = capsule_layout(steps, &widths, strip.width());

    for &(from, to) in &laid.joins {
        p.line_segment([Pos2::new(strip.left() + from, y), Pos2::new(strip.left() + to, y)], Stroke::new(1.0_f32, white_alpha(40)));
    }
    if let Some((x, n)) = laid.more {
        let more = Rect::from_min_size(Pos2::new(strip.left() + x, y - CAPSULE_H / 2.0), Vec2::new(MORE_W, CAPSULE_H));
        p.rect_filled(more, 12, white_alpha(9));
        p.text(more.center(), Align2::CENTER_CENTER, format!("+{n}"), font, c.text_dim);
    }
    let mut placed: Vec<(PaneId, f32)> = Vec::new();
    for (i, &x) in laid.xs.iter().enumerate() {
        match steps[i].agent() {
            Some(a) => placed.push((a.pane, x)),
            None => p.galley(Pos2::new(strip.left() + x, y - galleys[i].size().y / 2.0), galleys[i].clone(), c.text_dim),
        }
    }
    inside.update_capsules(now, slide, &placed);

    // The capsules on their way are drawn over the ones in place.
    let mut shown: Vec<(usize, &Agent, bool)> = Vec::new();
    for (i, step) in steps.iter().enumerate() {
        if let Some(a) = step.agent()
            && let Some(m) = inside.capsules.get(&a.pane)
        {
            shown.push((i, a, m.x.moving(now)));
        }
    }
    shown.sort_by_key(|&(_, _, moving)| moving);
    let mut subs = Vec::new();
    for (i, a, _) in shown {
        let m = &inside.capsules[&a.pane];
        let mut cp = p.clone();
        let opacity = m.opacity.get(now);
        cp.multiply_opacity(opacity);
        let cap = Rect::from_min_size(Pos2::new(strip.left() + m.x.get(now), y - CAPSULE_H / 2.0), Vec2::new(widths[i], CAPSULE_H));
        if a.team.sub && opacity > 0.5 {
            subs.push((a.pane, cap));
        }
        let color = status_color(a.status, c);
        let fill = if a.status == Status::NeedsYou { needs_you_tint(f) } else { white_alpha(9) };
        cp.rect_filled(cap, 12, fill);
        member_glyph(f, &cp, Pos2::new(cap.left() + 12.0, y), a.status, color, inside.tick(now, a.pane));
        cp.galley(Pos2::new(cap.left() + 21.0, y - galleys[i].size().y / 2.0), galleys[i].clone(), c.text_bright);
    }
    subs
}
