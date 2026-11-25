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
//! ([`member_tag`]: `Build-01`): its word on its capsule, the whole tag in a small capsule
//! leading its row, before the task its terminal's title says it's on.
//!
//! A card whose agent is on screen says so (`viewing`): a "Viewing" pill left of its status
//! and a soft accent ring, fuller (with a glow) for the agent in the focused pane than for
//! one sharing its tab.
//!
//! A frame copies no agent: the teams are worked out when the tracker's list changes (its
//! generation) and kept, shared, until it changes again.
//!
//! Helpers are in the order `group` gives them (by status), and are drawn where `motion`
//! has them: when one changes state, its row glides to its new slot over the others (which
//! slide under it to make room) and its capsule slides along the strip. Clicks go where a
//! row is drawn; the keys go by the slots the rows are headed for.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};

use egui::epaint::Shadow;
use egui::{Align2, Color32, FontId, Id, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, Vec2};

use crate::layout::PaneId;
use crate::ui::glide::animations;
use crate::ui::touch_pan::TouchPan;
use crate::ui::theme::{ACCENT, Theme, lighten, white_alpha};
use crate::ui::widgets::{ScrollSpan, overlay_scrollbar};
use crate::ui::{icons, paint};

use super::AgentsPane;
use super::group::{Group, Member, Step, groups, same_step};
use super::model::{Agent, AgentKind, Agents, Seen, Source, Status, Viewing, elapsed};
use super::motion::{Motion, TeamMotion};

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
    /// Each team card's team as last laid out, kept while its card fades away.
    teams: HashMap<PaneId, Arc<Group>>,
    /// The agents as teams, and the generation of the list they were made from: made again
    /// only when that changes.
    grouped: Option<(u64, Arc<[Arc<Group>]>)>,
    /// How each card was last on screen, for its "Viewing" pill to fade out as.
    seen: HashMap<PaneId, Seen>,
}

impl ViewState {
    pub(super) fn new() -> ViewState {
        ViewState { reveal_selection: false, jumped: false, scroll_active: -10.0, pan: TouchPan::default(), motion: Motion::new(), shown_tab: None, teams: HashMap::new(), grouped: None, seen: HashMap::new() }
    }

    /// `agents` as teams: the ones made last time, unless the list changed since.
    fn grouped(&mut self, agents: &Agents) -> Arc<[Arc<Group>]> {
        match &self.grouped {
            Some((generation, teams)) if *generation == agents.generation() => Arc::clone(teams),
            _ => {
                let teams: Arc<[Arc<Group>]> = groups(agents).into();
                self.grouped = Some((agents.generation(), Arc::clone(&teams)));
                teams
            }
        }
    }
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

/// `text` with its first letter a capital: `build` → `Build`.
fn capitalized(text: &str) -> String {
    let mut chars = text.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

/// A session name without the number `nex-host` gives a second agent of the same name, and
/// that number: `build-2` → (`build`, 2).
fn numbered(name: &str) -> (&str, Option<u32>) {
    match name.rsplit_once('-') {
        Some((stem, n)) if !stem.is_empty() && !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => (stem, n.parse().ok()),
        _ => (name, None),
    }
}

/// The name a helper's session goes by (its card's name while its title holds a task
/// instead, or the program when nothing else is known).
fn session_name(a: &Agent) -> &str {
    match &a.session {
        Some(name) if !name.is_empty() => name,
        _ if a.task.is_none() => &a.name,
        _ => a.kind.process(),
    }
}

/// What kind of helper it is, as its capsule says and its tag starts: its role (or pipeline
/// stage), else its session's name without its number, capitalized (`Build`). A sub-agent's
/// is its type (`Explore`), else `Sub`.
fn member_word(a: &Agent) -> String {
    let role = a.team.role.as_deref().map(str::trim).filter(|r| !r.is_empty());
    capitalized(match role {
        Some(role) => role,
        None if a.team.sub => "Sub",
        None => numbered(session_name(a)).0,
    })
}

/// What a helper goes by in its team: its word and a number (`Build-01`). The number is its
/// session name's (`build-2` is 02), else its place among the `siblings` (the helpers its
/// parent started, itself among them or not) with the same word, in the order they started.
fn member_tag(a: &Agent, siblings: &[&Agent]) -> String {
    let word = member_word(a);
    let own = if a.team.sub { None } else { numbered(session_name(a)).1 };
    let n = own.unwrap_or_else(|| {
        let order = |x: &Agent| (x.team.started, x.pane);
        let before = siblings.iter().filter(|s| s.pane != a.pane && order(s) < order(a) && member_word(s) == word).count();
        before as u32 + 1
    });
    format!("{word}-{n:02}")
}

/// What a helper is doing, for its row: the task in its terminal's title, live. A pipeline's
/// is how its run stands.
fn member_task(a: &Agent) -> Option<&str> {
    let task = a.task.as_deref()?;
    let task = if a.kind == AgentKind::Pipeline { nex_host::proto::split_pipeline_title(task).0 } else { task.trim() };
    (!task.is_empty()).then_some(task)
}

/// Each member's tag, in `g.members`' order.
fn member_tags(g: &Group) -> Vec<String> {
    g.members
        .iter()
        .map(|m| {
            let siblings: Vec<&Agent> = g.members.iter().map(|s| &*s.agent).filter(|s| s.team.parent == m.agent.team.parent).collect();
            member_tag(&m.agent, &siblings)
        })
        .collect()
}

impl AgentsPane {
    /// Draw the pane into `rect` (below the pane header, edge to edge of the card), showing
    /// `agents` (all of them, sorted), under `notes` (one line per ssh host that can't be
    /// watched). `shown_tab` is the terminal tab on screen (`None` for the Agents / Files
    /// tab): switching to one bubbles the cards of the agents on it. `viewing`: which agents
    /// are on screen, whose cards say so.
    #[allow(clippy::too_many_arguments)]
    pub fn paint(&mut self, ui: &Ui, painter: &Painter, rect: Rect, pane: u64, focused: bool, c: &Theme, agents: &Agents, notes: &[String], shown_tab: Option<usize>, viewing: &Viewing) {
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
        let teams = self.view.grouped(agents);
        // Teams that broke up, or whose lead is gone, close.
        self.expanded.retain(|lead, _| teams.iter().any(|g| g.key() == *lead && g.is_team()));
        let Layout { sections, cards, content_h } = layout(&teams, width, &self.expanded);
        self.remember_targets(&teams, &cards);
        if self.selected.is_none_or(|p| !self.targets.iter().any(|(t, _)| *t == p)) {
            self.selected = self.selected.and_then(|p| self.team_of.get(&p).copied()).or_else(|| self.targets.first().map(|(t, _)| *t));
        }
        let mut motion = std::mem::take(&mut self.view.motion);
        let laid: Vec<(&Arc<Agent>, Rect)> = cards.iter().map(|(g, r)| (&g.lead, *r)).collect();
        motion.update(f.time, f.animate, &sections, &laid, width);
        for (g, _) in &cards {
            self.view.teams.insert(g.key(), Arc::clone(g));
        }
        self.view.teams.retain(|k, _| motion.cards.contains_key(k));
        self.view.seen.retain(|k, _| motion.cards.contains_key(k));
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
    fn remember_targets(&mut self, teams: &[Arc<Group>], cards: &[(&Arc<Group>, Rect)]) {
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
        let mut ids: Vec<(PaneId, f32)> = motion.cards.iter().map(|(id, card)| (*id, card.lift(now, shown_tab))).collect();
        ids.sort_by_key(|&(id, lift)| (lift > 0.0, id));
        let mut breathing = false;
        for (id, lift) in ids {
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
            let team = self.view.teams.get(&id).filter(|g| g.is_team()).cloned();
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
                match &team {
                    Some(g) => viewing::team_seen(f.viewing, g, self.expanded.contains_key(&id)),
                    None => f.viewing.of(&card.agent),
                }
            };
            let (border, border_width) = border_for(card.agent.status, selected, f.focused, hovered, seen, f.c);
            card.set_colors(now, border, border_width, status_color(card.agent.status, f.c), f.animate);
            let fade = |what: &str, on: bool| if f.animate { ui.ctx().animate_bool_with_time(f.id((what, id)), on, 0.3) } else { f32::from(u8::from(on)) };
            let glow = fade("viewing-glow", seen == Some(Seen::Primary));
            // The pill fades out as the kind it was when last on screen.
            let kind = seen.unwrap_or_else(|| self.view.seen.get(&id).copied().unwrap_or(Seen::Shared));
            if let Some(s) = seen {
                self.view.seen.insert(id, s);
            }
            let look = Look {
                hover,
                lift,
                border: card.border.get(now),
                border_width: card.border_width.get(now),
                tint: card.tint.get(now),
                glow,
                seen: (kind, fade("viewing-pill", seen.is_some())),
            };
            let mut p = bp.clone();
            p.multiply_opacity(opacity);
            match team {
                Some(g) => {
                    breathing |= g.lead.status == Status::Working;
                    let leaving = card.leaving;
                    // Its rows and capsules slide, unless the card itself was resized.
                    let inside = motion.teams.entry(id).or_default();
                    let slide = f.animate && !inside.resized(laid_w);
                    self.team_card(f, &p, r, &g, look, leaving, inside, slide);
                    busy |= inside.busy(now);
                }
                None => {
                    motion.teams.remove(&id);
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
    /// `inside`: where its rows and capsules are; `slide`: whether they glide to new places.
    #[allow(clippy::too_many_arguments)]
    fn team_card(&mut self, f: &Frame, p: &Painter, r: Rect, g: &Group, look: Look, leaving: bool, inside: &mut TeamMotion, slide: bool) {
        let key = g.key();
        let open = self.expanded.get(&key).copied();
        let closed_h = team_height(g, None);
        let open_h = team_height(g, Some(open.unwrap_or(false)));
        // How open it is, from the card's height as it eases between the two.
        let t = if open_h > closed_h { ((r.height() - closed_h) / (open_h - closed_h)).clamp(0.0, 1.0) } else { 0.0 };
        card_body(p, r, &look);
        let head = Rect::from_min_size(r.min, Vec2::new(r.width(), CARD_H));
        let size = g.size();
        let pill = agent_header(f, p, head, &g.lead, &look, Some((&size, t)));

        // Clicking the chevron opens and closes it (over the header's own click, which
        // jumps to the lead); so do the capsules while it's closed.
        if !leaving && let Some(hit) = f.interact(chevron_hit(pill), ("chevron", key)) {
            if hit.clicked() {
                self.selected = Some(key);
                self.toggle_team(key);
            }
            hit.on_hover_cursor(egui::CursorIcon::PointingHand);
        }
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
            capsules(f, &sp, Rect::from_min_size(Pos2::new(r.left() + 14.0, y), Vec2::new(r.width() - 28.0, STRIP_H)), g, inside, slide);
        } else {
            inside.close_capsules();
        }
        if t > 0.01 {
            let mut lp = p.with_clip_rect(p.clip_rect().intersect(r.shrink(1.0)));
            lp.multiply_opacity(t);
            self.helper_list(f, &lp, r, g, open.unwrap_or(false), t > 0.99 && !leaving, inside, slide);
        } else {
            inside.close_rows();
        }
    }

    /// An open team's helpers: one row each (tile, tag and task, folder, status), hairlines
    /// between, "Show all" when there are more than fit. `live`: settled, so rows take clicks.
    /// Each row is drawn where `inside` has it on the way to its slot (`slide`: gliding
    /// there), the one whose helper just changed state over the others.
    #[allow(clippy::too_many_arguments)]
    fn helper_list(&mut self, f: &Frame, p: &Painter, r: Rect, g: &Group, all: bool, live: bool, inside: &mut TeamMotion, slide: bool) {
        let list = list_rect(r);
        p.rect_filled(list, 10, white_alpha(6));
        let key = g.key();
        let now = f.time;
        let rows = shown_members(g, all);
        let tags = member_tags(g);
        let slots: Vec<(&Member, &str, f32)> = rows.iter().zip(&tags).enumerate().map(|(i, (m, tag))| (m, tag.as_str(), i as f32 * ROW_H)).collect();
        inside.update_rows(now, slide, &slots);

        // The hairlines stay where the slots are, each starting at its slot's text.
        for (i, m) in rows.iter().enumerate() {
            if i + 1 < rows.len() || g.members.len() > rows.len() {
                let y = list.top() + (i + 1) as f32 * ROW_H;
                p.line_segment([Pos2::new(row_text_x(list.left(), m.depth), y), Pos2::new(list.right(), y)], Stroke::new(1.0_f32, white_alpha(10)));
            }
        }

        // Settled rows first, then the ones making room, then the ones whose helper changed
        // state (the latest on top), each on a backing so it reads as it passes over.
        let over = |id: &PaneId| inside.rows[id].status_change && inside.rows[id].sliding(now);
        let mut order: Vec<PaneId> = inside.rows.keys().copied().collect();
        order.sort_by(|a, b| {
            let (ra, rb) = (&inside.rows[a], &inside.rows[b]);
            (over(a), ra.sliding(now)).cmp(&(over(b), rb.sliding(now))).then(ra.changed.total_cmp(&rb.changed)).then(a.cmp(b))
        });
        let backing = lighten(f.c.card.to_opaque(), 0.07);
        for id in order {
            let m = &inside.rows[&id];
            let row = Rect::from_min_size(Pos2::new(list.left(), list.top() + m.y.get(now)), Vec2::new(list.width(), ROW_H));
            let mut rp = p.clone();
            rp.multiply_opacity(m.opacity.get(now));
            let inner = row.shrink2(Vec2::new(4.0, 3.0));
            if over(&id) {
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
            let seen = (!m.leaving).then(|| f.viewing.of(&m.member.agent)).flatten();
            if let Some(seen) = seen {
                viewing::row_fill(&rp, inner, seen);
            }
            let hover = f.ui.ctx().animate_bool_with_time(f.id(("row-hover", id)), hovered, 0.12);
            if hover > 0.0 {
                rp.rect_filled(inner, 8, white_alpha((8.0 * hover) as u8));
            }
            if self.highlighted() == Some(id) && f.focused && !m.leaving {
                rp.rect_stroke(inner, 8, Stroke::new(1.5_f32, ACCENT.gamma_multiply(0.85)), StrokeKind::Inside);
            }
            helper_row(f, &rp, row, &m.member, &m.tag, seen);
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
    cards: Vec<(&'a Arc<Group>, Rect)>,
    content_h: f32,
}

/// Lay `teams` (sorted by status, as [`groups`] leaves them) out `width` wide: a header per status, then its cards in as many columns as
/// fit. A closed team is a cell of the grid like any card, only taller, so teams fill rows
/// of their own first and the single cards follow (no row mixes heights); an open team
/// spans the row, as tall as its list.
fn layout<'a>(teams: &'a [Arc<Group>], width: f32, expanded: &HashMap<PaneId, bool>) -> Layout<'a> {
    let columns = (((width + GAP) / (MIN_CARD_W + GAP)).floor() as usize).clamp(1, MAX_COLUMNS);
    let card_w = (width - GAP * (columns as f32 - 1.0)) / columns as f32;
    let mut sections: Vec<(Status, usize, Rect)> = Vec::new();
    let mut cards: Vec<(&Arc<Group>, Rect)> = Vec::with_capacity(teams.len());
    let mut y = 4.0;
    let mut rest = teams;
    for status in Status::ALL {
        // Sorted by status: this one's teams are the next ones.
        let (group, after) = rest.split_at(rest.iter().take_while(|g| g.lead.status == status).count());
        rest = after;
        if group.is_empty() {
            continue;
        }
        sections.push((status, group.len(), Rect::from_min_size(Pos2::new(SIDE, y), Vec2::new(width, SECTION_H))));
        y += SECTION_H;
        // The row being filled: how many cards it has, and how tall they are.
        let mut col = 0;
        let mut row_h = 0.0;
        for g in group.iter().filter(|g| g.is_team()).chain(group.iter().filter(|g| !g.is_team())) {
            let open = g.is_team().then(|| expanded.get(&g.key()).copied()).flatten();
            let h = if g.is_team() { team_height(g, open) } else { CARD_H };
            if col > 0 && (open.is_some() || h != row_h) {
                y += row_h + GAP;
                col = 0;
            }
            if open.is_some() {
                cards.push((g, Rect::from_min_size(Pos2::new(SIDE, y), Vec2::new(width, h))));
                y += h + GAP;
                continue;
            }
            cards.push((g, Rect::from_min_size(Pos2::new(SIDE + col as f32 * (card_w + GAP), y), Vec2::new(card_w, h))));
            row_h = h;
            col += 1;
            if col == columns {
                y += row_h + GAP;
                col = 0;
            }
        }
        if col > 0 {
            y += row_h + GAP;
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
    /// The accent glow of the agent in the focused pane (0 to 1).
    glow: f32,
    /// How it's on screen, and how far its "Viewing" pill is in (0 to 1).
    seen: (Seen, f32),
}

/// The border a card heads for: the accent when it's the selected one in the focused pane,
/// a softer accent ring when its agent is on screen (`seen`, see [`viewing::ring`]), amber
/// when its agent needs you, blue when it's done, a quiet hairline otherwise (brighter under
/// the pointer).
fn border_for(status: Status, selected: bool, focused: bool, hovered: bool, seen: Option<Seen>, c: &Theme) -> (Color32, f32) {
    if selected && focused {
        (ACCENT.gamma_multiply(0.85), 1.5)
    } else if let Some(seen) = seen {
        viewing::ring(seen)
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
    let Look { hover, lift, border, border_width, glow, .. } = *look;
    viewing::glow(p, r, glow);
    let shadow = Shadow {
        offset: [0, (2.0 + 6.0 * lift) as i8],
        blur: (12.0 + 14.0 * lift) as u8,
        spread: 0,
        color: Color32::from_black_alpha((28.0 + 24.0 * hover + 50.0 * lift) as u8),
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
/// Returns the pill's rect (a team's chevron sits left of it: [`chevron_hit`]).
fn agent_header(f: &Frame, p: &Painter, r: Rect, agent: &Agent, look: &Look, team: Option<(&str, f32)>) -> Rect {
    let c = f.c;
    let tile = Rect::from_center_size(Pos2::new(r.left() + 14.0 + TILE / 2.0, r.top() + CARD_H / 2.0), Vec2::splat(TILE));
    icons::app_tile(p, tile, agent.kind.process(), agent.mark.as_deref());

    // The status pill, top right, with the time in that state under it.
    let color = look.tint;
    let pill_font = paint::bold(11.0);
    let label = p.layout_no_wrap(agent.status.label().to_string(), pill_font, color);
    let pill = pill_rect(r, label.size().x);
    p.rect_filled(pill, 10, color.gamma_multiply(0.12));
    let dot = Pos2::new(pill.left() + 11.0, pill.center().y);
    status_dot(f, p, dot, agent.status, color);
    p.galley(Pos2::new(dot.x + 7.5, pill.center().y - label.size().y / 2.0), label, color);
    let since = p.layout_no_wrap(elapsed(f.now.saturating_duration_since(agent.since)), ui_font(11.5), c.text_dim);
    let since_y = r.top() + DETAIL_Y;
    let since_w = since.size().x;
    p.galley(Align2::RIGHT_CENTER.anchor_size(Pos2::new(pill.right() - 4.0, since_y), since.size()).min, since, c.text_dim);

    // A team's chevron, left of the pill: pointing right while closed, down when open.
    let mut title_right = pill.left() - 12.0;
    if let Some((_, open)) = team {
        let at = chevron_center(pill);
        chevron(p, at, open, c.text_dim);
        title_right = at.x - 12.0;
    }

    // "Viewing", left of them, while the agent is on screen (only its eye on a narrow card).
    let x = tile.right() + 13.0;
    let (seen, shown) = look.seen;
    if shown > 0.01 {
        let right = title_right + 4.0;
        let left = viewing::pill(p, right, pill.center().y, seen, shown, viewing::compact(p, x, right));
        title_right = title_right + (left - 8.0 - title_right) * shown;
    }

    // The task, and the checkout it's working in. A pipeline's counts follow its name in
    // lighter type.
    let title_room = (title_right - x).max(0.0);
    let HeaderText { title, counts, detail } = header_text(agent, team.map(|(size, _)| size));
    paint::faded_text(p, Pos2::new(x, r.top() + TITLE_Y), &title, paint::bold(14.0), c.text_bright, title_room, 18.0);
    if let Some(counts) = counts {
        let used = p.layout_no_wrap(title, paint::bold(14.0), c.text_bright).size().x;
        let room = title_room - used - 18.0;
        if room > 0.0 {
            paint::faded_text(p, Pos2::new(x + used, r.top() + TITLE_Y), &format!(" \u{00b7} {counts}"), ui_font(13.5), c.text, room + 18.0, 18.0);
        }
    }
    let detail_room = (pill.right() - 4.0 - since_w - 12.0 - x).max(0.0);
    paint::faded_text(p, Pos2::new(x, since_y), &detail, ui_font(12.0), c.text_dim, detail_room, 18.0);
    pill
}

/// The last part of the folder an agent works in (`nex`, or a worktree's own folder), so
/// which checkout each one is in reads at a glance.
fn folder(a: &Agent) -> Option<String> {
    let dir = a.cwd.as_ref()?;
    let name = dir.file_name().map_or_else(|| dir.to_string_lossy().into_owned(), |n| n.to_string_lossy().into_owned());
    (!name.is_empty()).then_some(name)
}

/// The quiet line under a card's title: its folder (or, without one, what kind of agent it
/// is), a team's size, and its tab, or `Detached` for an agent on an ssh host no pane shows.
fn card_detail(a: &Agent, team_size: Option<&str>) -> String {
    let mut parts = vec![folder(a).unwrap_or_else(|| a.kind.label().to_string())];
    parts.extend(team_size.map(str::to_string));
    match (a.tab, &a.source) {
        (Some(tab), _) => parts.push(format!("Tab {}", tab + 1)),
        (None, Source::Remote { pane: None, .. }) if team_size.is_none() => parts.push("Detached".into()),
        _ => {}
    }
    parts.join(" · ")
}

/// What a card's header says.
struct HeaderText {
    /// In bold: the agent's task (or name).
    title: String,
    /// After the title, lighter: how a pipeline's run stands (`2 building · 1 merged`).
    counts: Option<String>,
    /// The quiet line under them.
    detail: String,
}

/// A card's header: its agent's name over [`card_detail`]. A pipeline whose runner says how
/// it stands (its title; one from an older `nex-host` has none) is its name, capitalized,
/// then the counts, over the last thing that happened and then the detail.
fn header_text(a: &Agent, team_size: Option<&str>) -> HeaderText {
    let detail = card_detail(a, team_size);
    let stands = a.task.as_deref().filter(|_| a.kind == AgentKind::Pipeline).map(nex_host::proto::split_pipeline_title);
    match stands {
        Some((counts, event)) if !counts.is_empty() || !event.is_empty() => HeaderText {
            title: capitalized(&a.name),
            counts: (!counts.is_empty()).then(|| counts.to_string()),
            detail: if event.is_empty() { detail } else { format!("{event} \u{00b7} {detail}") },
        },
        _ => HeaderText { title: a.name.clone(), counts: None, detail },
    }
}

/// The quiet line under a helper's tag and task: its folder (or, without one, what kind of
/// agent it is).
fn member_detail(a: &Agent) -> String {
    // A sub-agent works in its agent's folder: what kind it is says more.
    if a.team.sub {
        let kind = a.team.role.clone().filter(|r| !r.is_empty()).unwrap_or_else(|| "Sub-agent".into());
        return format!("{kind} \u{00b7} {}", a.kind.label());
    }
    folder(a).unwrap_or_else(|| a.kind.label().to_string())
}

/// The status pill in header `r`, for a label `label_w` wide: top right.
fn pill_rect(r: Rect, label_w: f32) -> Rect {
    Rect::from_min_size(Pos2::new(r.right() - 16.0 - (label_w + 26.0), r.top() + TITLE_Y - 10.0), Vec2::new(label_w + 26.0, 20.0))
}

/// Where a team's chevron is drawn: just left of its status `pill`.
fn chevron_center(pill: Rect) -> Pos2 {
    Pos2::new(pill.left() - 16.0, pill.center().y)
}

/// The chevron's click box: a finger-sized square around it, not past the pill.
fn chevron_hit(pill: Rect) -> Rect {
    let at = chevron_center(pill);
    Rect::from_min_max(Pos2::new(at.x - 16.0, at.y - 16.0), Pos2::new(pill.left() - 2.0, at.y + 16.0))
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

/// Where a closed team's strip puts its steps, from the strip's left edge.
#[derive(Debug, PartialEq)]
struct CapsuleLayout {
    /// The left edge of each step that fits (the first so many of them).
    xs: Vec<f32>,
    /// The hairlines joining a pipeline's stage to the one before: from and to.
    joins: Vec<(f32, f32)>,
    /// The `+N` capsule's left edge and its N: the agents that don't fit.
    more: Option<(f32, usize)>,
}

/// Lay `steps`, each `widths` wide, along a strip `strip_w` wide: a pipeline's stages joined
/// by a hairline to the stage before, side by side with another of the same stage (`build`,
/// `build-2`), a nested pipeline's name before its stages. `+N` for what doesn't fit.
fn capsule_layout(steps: &[Step], widths: &[f32], strip_w: f32) -> CapsuleLayout {
    // The agents from step `i` on (what a `+N` there stands for).
    let left = |i: usize| steps[i..].iter().filter(|s| s.agent().is_some()).count();
    let mut laid = CapsuleLayout { xs: Vec::new(), joins: Vec::new(), more: None };
    let mut x = 0.0;
    let mut prev: Option<Step> = None;
    for (i, (&step, &w)) in steps.iter().zip(widths).enumerate() {
        let (gap, joined) = match (prev, step) {
            (None, _) => (0.0, false),
            (_, Step::Heading(_)) => (14.0, false),
            (Some(Step::Heading(_)), _) => (8.0, false),
            (Some(Step::Stage(a)), Step::Stage(b)) if !same_step(a, b) => (16.0, true),
            _ => (6.0, false),
        };
        let room = strip_w - x - gap - if left(i + 1) > 0 { MORE_W + 6.0 } else { 0.0 };
        if w > room {
            laid.more = Some((x + if prev.is_some() { 6.0 } else { 0.0 }, left(i)));
            break;
        }
        if joined {
            laid.joins.push((x + 3.0, x + gap - 3.0));
        }
        x += gap;
        laid.xs.push(x);
        x += w;
        prev = Some(step);
    }
    laid
}

/// A closed team's helpers in `strip`: a capsule each (status and its tag's word), the lead's own
/// helpers only, with a pipeline among them opened into its stages after its name (see
/// [`capsule_layout`]). A capsule is drawn where `inside` has it on the way to its place
/// (`slide`: gliding there when the order changes); one going into the `+N` fades.
fn capsules(f: &Frame, p: &Painter, strip: Rect, g: &Group, inside: &mut TeamMotion, slide: bool) {
    let c = f.c;
    let now = f.time;
    let steps = g.strip();
    let y = strip.center().y;
    let font = paint::bold(11.5);
    let galleys: Vec<_> = steps
        .iter()
        .map(|step| match step {
            Step::Heading(name) => p.layout_no_wrap(format!("{name}:"), font.clone(), c.text_dim),
            Step::Stage(a) | Step::Helper(a) => {
                // A stage or helper running sub-agents says how many.
                let label = match g.subs_of(a.pane) {
                    0 => member_word(a),
                    n => format!("{} \u{00b7}{n}", member_word(a)),
                };
                p.layout_no_wrap(label, font.clone(), c.text_bright)
            }
        })
        .collect();
    let widths: Vec<f32> = steps.iter().zip(&galleys).map(|(s, g)| g.size().x + if s.agent().is_some() { 30.0 } else { 0.0 }).collect();
    let laid = capsule_layout(&steps, &widths, strip.width());

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
    for (i, a, _) in shown {
        let m = &inside.capsules[&a.pane];
        let mut cp = p.clone();
        cp.multiply_opacity(m.opacity.get(now));
        let cap = Rect::from_min_size(Pos2::new(strip.left() + m.x.get(now), y - CAPSULE_H / 2.0), Vec2::new(widths[i], CAPSULE_H));
        let color = status_color(a.status, c);
        let fill = if a.status == Status::NeedsYou { c.yellow.gamma_multiply(0.14) } else { white_alpha(9) };
        cp.rect_filled(cap, 12, fill);
        member_glyph(f, &cp, Pos2::new(cap.left() + 12.0, y), a.status, color);
        cp.galley(Pos2::new(cap.left() + 21.0, y - galleys[i].size().y / 2.0), galleys[i].clone(), c.text_bright);
    }
}

/// Where a helper row's text starts, for a row whose left edge is `left`: after its tile,
/// indented for a helper's own helpers.
fn row_text_x(left: f32, depth: usize) -> f32 {
    left + 12.0 + row_indent(depth) + 26.0 + 10.0
}

fn row_indent(depth: usize) -> f32 {
    16.0 * (depth.saturating_sub(1).min(3)) as f32
}

/// One helper's row in an open team: tile (indented for a helper's own helpers), its `tag`
/// in a small capsule and the task it's on (see [`member_task`]) over the folder it works
/// in, and its status with how long it has been in it.
fn helper_row(f: &Frame, p: &Painter, row: Rect, m: &Member, tag: &str, seen: Option<Seen>) {
    let c = f.c;
    let a = &m.agent;
    let tile = Rect::from_center_size(Pos2::new(row.left() + 12.0 + row_indent(m.depth) + 13.0, row.center().y), Vec2::splat(26.0));
    icons::app_tile(p, tile, a.kind.process(), a.mark.as_deref());
    let x = row_text_x(row.left(), m.depth);

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

    // An eye left of the status while it's on screen.
    let mut end = dot.x;
    if let Some(seen) = seen {
        end -= 18.0;
        viewing::row_eye(p, Pos2::new(end, row.center().y), seen);
    }
    let room = (end - 14.0 - x).max(0.0);
    // The tag: a soft capsule, quieter than the task beside it. In a narrow row it gives way
    // to the fade like the text does.
    let line_y = row.top() + 16.0;
    let tag_g = p.layout_no_wrap(tag.to_string(), paint::bold(10.5), c.text);
    let cap = Rect::from_min_size(Pos2::new(x, line_y - TAG_H / 2.0), Vec2::new(tag_g.size().x + 16.0, TAG_H));
    let tp = p.with_clip_rect(p.clip_rect().intersect(Rect::from_min_max(row.min, Pos2::new(x + room, row.bottom()))));
    tp.rect_filled(cap, TAG_H / 2.0, white_alpha(13));
    tp.galley(Pos2::new(cap.left() + 8.0, line_y - tag_g.size().y / 2.0), tag_g, c.text);
    if let Some(task) = member_task(a) {
        let task_x = cap.right() + 8.0;
        paint::faded_text(p, Pos2::new(task_x, line_y), task, paint::bold(13.0), c.text_bright, (x + room - task_x).max(0.0), 16.0);
    }
    let detail = member_detail(a);
    paint::faded_text(p, Pos2::new(x, row.top() + 32.0), &detail, ui_font(11.5), c.text_dim, room, 16.0);
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
            task: None,
            session: None,
            mark: None,
            cwd: None,
            status: Status::Idle,
            since: Instant::now(),
            source: Source::Local,
            team: Team { parent, role: None, started: pane, sub: false },
        }
    }

    #[test]
    fn a_closed_team_is_a_cell_of_the_grid_and_spans_the_row_when_open() {
        // Two single cards, two teams (8 and 2 helpers), another single: 2 columns at 700.
        let mut agents = vec![agent(1, None), agent(2, None), agent(3, None), agent(4, None), agent(9, None)];
        agents.extend((10..18).map(|p| agent(p, Some(3))));
        agents.extend((20..22).map(|p| agent(p, Some(4))));
        let teams = groups(&Agents::new(agents));
        let closed = layout(&teams, 700.0, &HashMap::new());
        let rect = |l: &Layout, key| l.cards.iter().find(|(g, _)| g.key() == key).unwrap().1;
        assert_eq!(closed.cards.len(), 5, "helpers are inside their team's card");
        let (t3, t4) = (rect(&closed, 3), rect(&closed, 4));
        assert_eq!((t3.width(), t3.height()), (rect(&closed, 1).width(), CARD_H + STRIP_H), "as wide as a card, taller");
        assert!(t3.width() < 400.0 && t3.top() == t4.top(), "the teams share a row");
        let singles = [1, 2, 9].map(|k| rect(&closed, k));
        assert!(singles.iter().all(|r| r.top() > t3.bottom()), "singles come after, on rows of their own");
        assert!(singles[0].top() == singles[1].top() && singles[2].top() > singles[0].bottom());

        let open = layout(&teams, 700.0, &HashMap::from([(3, false)]));
        let o3 = rect(&open, 3);
        assert_eq!((o3.width(), o3.height()), (700.0, CARD_H + 2.0 + ROWS_SHOWN as f32 * ROW_H + MORE_H + LIST_INSET), "open, it spans the row");
        assert!(rect(&open, 4).top() > o3.bottom(), "nothing beside an open team");
        let all = layout(&teams, 700.0, &HashMap::from([(3, true)]));
        assert_eq!(rect(&all, 3).height(), CARD_H + 2.0 + 8.0 * ROW_H + MORE_H + LIST_INSET);
        assert!(all.content_h > open.content_h && open.content_h > closed.content_h);
        assert_eq!(row_rect(o3, 1).top() - row_rect(o3, 0).top(), ROW_H);

        // One column (a phone): every card is the full width, as before.
        let narrow = layout(&teams, 320.0, &HashMap::new());
        assert!(narrow.cards.iter().all(|(_, r)| r.width() == 320.0));
    }

    #[test]
    fn a_viewed_card_has_an_accent_ring_under_the_keyboards_own() {
        let c = Theme::from_config(&crate::config::Config::default());
        let (primary, shared) = (viewing::ring(Seen::Primary), viewing::ring(Seen::Shared));
        assert_eq!(border_for(Status::Working, false, true, false, Some(Seen::Primary), &c), primary);
        assert_eq!(border_for(Status::Working, false, true, false, Some(Seen::Shared), &c), shared);
        assert!(primary.1 > shared.1 && primary.0.a() > shared.0.a(), "the focused pane's agent is ringed more strongly");
        // It says viewed over "needs you" (its pill still says that), but the selected card in
        // the focused Agents pane keeps its own ring.
        assert_eq!(border_for(Status::NeedsYou, false, true, false, Some(Seen::Shared), &c), shared);
        assert_eq!(border_for(Status::Working, true, true, false, Some(Seen::Primary), &c), border_for(Status::Working, true, true, false, None, &c));
        assert_ne!(border_for(Status::Idle, false, true, false, None, &c), shared);
    }

    /// Paints `pane` showing `agents` in an 800×600 window, with `events`.
    fn frame(ctx: &egui::Context, pane: &mut AgentsPane, agents: &Agents, time: &mut f64, events: Vec<egui::Event>) {
        *time += 0.05;
        let theme = Theme::from_config(&crate::config::Config::default());
        let input = egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))), time: Some(*time), events, ..Default::default() };
        let _ = ctx.run(input, |ctx| {
            egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
                let rect = ui.max_rect();
                pane.paint(ui, ui.painter(), rect, 1, true, &theme, agents, &[], None, &Viewing::default());
            });
        });
    }

    fn click(ctx: &egui::Context, pane: &mut AgentsPane, agents: &Agents, time: &mut f64, at: Pos2) {
        let press = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Primary, pressed, modifiers: egui::Modifiers::NONE };
        for events in [vec![egui::Event::PointerMoved(at)], vec![press(true)], vec![press(false)], vec![]] {
            frame(ctx, pane, agents, time, events);
        }
    }

    #[test]
    fn the_chevron_drawn_on_a_pipelines_card_opens_and_closes_it() {
        let ctx = egui::Context::default();
        crate::render::fonts::install(&ctx, &crate::config::Config::default());
        crate::ui::glide::set_animations(&ctx, false);
        let mut lead = agent(1, None);
        lead.kind = AgentKind::Pipeline;
        let agents = Agents::new(vec![lead, agent(2, Some(1)), agent(3, Some(1))]);
        let mut pane = AgentsPane::new();
        let mut time = 0.0;
        for _ in 0..3 {
            frame(&ctx, &mut pane, &agents, &mut time, vec![]);
        }

        // Where the card and its chevron are drawn: under its section header, the chevron
        // left of the status pill.
        let body = Rect::from_min_max(Pos2::new(0.0, 6.0), Pos2::new(800.0, 600.0));
        let teams = groups(&agents);
        let card = layout(&teams, body.width() - 2.0 * SIDE, &HashMap::new()).cards[0].1.translate(body.min.to_vec2());
        let head = Rect::from_min_size(card.min, Vec2::new(card.width(), CARD_H));
        let painter = egui::Painter::new(ctx.clone(), egui::LayerId::background(), body);
        let label_w = painter.layout_no_wrap(Status::Idle.label().to_string(), paint::bold(11.0), Color32::WHITE).size().x;
        let pill = pill_rect(head, label_w);
        let arrow = chevron_center(pill);
        assert!(chevron_hit(pill).contains(arrow) && !chevron_hit(pill).intersects(pill), "the box is around the arrow, short of the pill");

        click(&ctx, &mut pane, &agents, &mut time, arrow);
        assert_eq!(pane.expanded.get(&1), Some(&false), "the arrow opens the list");
        assert!(pane.requests.is_empty(), "rather than jumping to the pipeline");
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
        // Open, it spans the row, so its arrow moved with the pill. Its edge closes it too,
        // a few points off the arrow.
        let wide = layout(&teams, body.width() - 2.0 * SIDE, &pane.expanded).cards[0].1.translate(body.min.to_vec2());
        let arrow = chevron_center(pill_rect(Rect::from_min_size(wide.min, Vec2::new(wide.width(), CARD_H)), label_w));
        click(&ctx, &mut pane, &agents, &mut time, arrow + Vec2::new(-10.0, 8.0));
        assert!(pane.expanded.is_empty(), "and closes it again");
        assert!(pane.requests.is_empty());

        // The capsules under a closed card open it; the rest of the header jumps.
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
        click(&ctx, &mut pane, &agents, &mut time, Pos2::new(card.center().x, head.bottom() + STRIP_H / 2.0));
        assert_eq!(pane.expanded.get(&1), Some(&false));
        // Open, its rows are where they're drawn: the second stage's.
        let open = layout(&teams, body.width() - 2.0 * SIDE, &pane.expanded).cards[0].1.translate(body.min.to_vec2());
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
        click(&ctx, &mut pane, &agents, &mut time, row_rect(open, 1).center());
        assert_eq!(pane.requests, vec![super::super::Request::Focus(3)]);
        assert_eq!((pane.selected, pane.highlighted()), (Some(3), None), "a jump leaves no highlight drawn");
        pane.requests.clear();
        click(&ctx, &mut pane, &agents, &mut time, head.left_center() + Vec2::new(30.0, 0.0));
        assert_eq!(pane.requests, vec![super::super::Request::Focus(1)]);
    }

    #[test]
    fn a_frame_shares_the_trackers_agents_and_regroups_only_when_they_change() {
        use crate::agents::model::{Probe, Tracker};
        let ctx = egui::Context::default();
        crate::render::fonts::install(&ctx, &crate::config::Config::default());
        crate::ui::glide::set_animations(&ctx, false);
        let at = Instant::now();
        let probe = |pane: PaneId, parent: Option<PaneId>, status: Status| Probe {
            pane,
            tab: Some(0),
            kind: AgentKind::Claude,
            title: format!("task {pane}"),
            mark: None,
            cwd: None,
            session_name: None,
            session_status: None,
            attention: false,
            last_output: at,
            last_input: at,
            source: Source::Local,
            reported: Some(status),
            team: Team { parent, role: None, started: pane, sub: false },
        };
        let probes = |second: Status| vec![probe(1, None, Status::Idle), probe(2, Some(1), Status::Idle), probe(3, None, second)];
        let mut tracker = Tracker::default();
        tracker.update(probes(Status::Idle), at);
        let mut pane = AgentsPane::new();
        let mut time = 0.0;
        let teams = |pane: &AgentsPane| Arc::clone(&pane.view.grouped.as_ref().expect("painted").1);

        frame(&ctx, &mut pane, tracker.agents(), &mut time, vec![]);
        let first = teams(&pane);
        assert_eq!(first.iter().map(|g| (g.key(), g.members.len())).collect::<Vec<_>>(), [(1, 1), (3, 0)]);
        // Nothing is copied on the way to the screen: the cards hold the tracker's agents.
        let tracked = |tracker: &Tracker, pane: PaneId| Arc::clone(tracker.agents().iter().find(|a| a.pane == pane).unwrap());
        assert!(Arc::ptr_eq(&first[0].lead, &tracked(&tracker, 1)) && Arc::ptr_eq(&first[0].members[0].agent, &tracked(&tracker, 2)));
        assert!(Arc::ptr_eq(&pane.view.motion.cards[&3].agent, &tracked(&tracker, 3)));
        assert!(Arc::ptr_eq(&pane.view.teams[&1], &first[0]));

        // More frames, and polls that find the agents as they were: the same teams.
        for _ in 0..3 {
            frame(&ctx, &mut pane, tracker.agents(), &mut time, vec![]);
            tracker.update(probes(Status::Idle), at + Duration::from_secs(1));
        }
        frame(&ctx, &mut pane, tracker.agents(), &mut time, vec![]);
        assert!(Arc::ptr_eq(&first, &teams(&pane)), "not made again while nothing changed");
        assert!(Arc::ptr_eq(&pane.view.motion.cards[&3].agent, &tracked(&tracker, 3)));

        // An agent starts working: made again, from the new list.
        tracker.update(probes(Status::Working), at + Duration::from_secs(2));
        frame(&ctx, &mut pane, tracker.agents(), &mut time, vec![]);
        let second = teams(&pane);
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(second.iter().map(|g| (g.key(), g.lead.status)).collect::<Vec<_>>(), [(3, Status::Working), (1, Status::Idle)]);
    }

    #[test]
    fn a_helper_changing_state_glides_its_row_to_its_new_slot() {
        use crate::agents::motion::MOVE;
        let ctx = egui::Context::default();
        crate::render::fonts::install(&ctx, &crate::config::Config::default());
        crate::ui::glide::set_animations(&ctx, true);
        let mut agents = vec![agent(1, None), agent(2, Some(1)), agent(3, Some(1))];
        // A poll that finds an agent changed makes a new list (a new generation).
        let mut list = Agents::new(agents.clone());
        let mut pane = AgentsPane::new();
        let mut time = 0.0;
        let frames = |pane: &mut AgentsPane, agents: &Agents, time: &mut f64, n: usize| {
            for _ in 0..n {
                frame(&ctx, pane, agents, time, vec![]);
            }
        };
        // Open the team and let its card grow: the rows are simply there, in start order.
        frames(&mut pane, &list, &mut time, 2);
        pane.expanded.insert(1, false);
        frames(&mut pane, &list, &mut time, 14);
        let ys = |pane: &AgentsPane, time: f64| {
            let rows = &pane.view.motion.teams[&1].rows;
            (rows[&2].y.get(time), rows[&3].y.get(time))
        };
        assert_eq!(ys(&pane, time), (0.0, ROW_H));
        assert!(!pane.view.motion.teams[&1].busy(time));
        let order = |pane: &AgentsPane| pane.targets.iter().map(|(id, _)| *id).collect::<Vec<_>>();
        assert_eq!(order(&pane), [1, 2, 3]);

        // Helper 3 finishes: it belongs above 2. The keys go by the new order at once...
        agents[2].status = Status::Done;
        list = Agents::new(agents.clone());
        frames(&mut pane, &list, &mut time, 1);
        let started = time;
        assert_eq!(order(&pane), [1, 3, 2]);
        assert_eq!(ys(&pane, time), (0.0, ROW_H), "...while the rows set off from where they were");
        // ...halfway, the rows are between slots and the pane keeps repainting...
        frames(&mut pane, &list, &mut time, 4);
        assert!(time < started + MOVE);
        let (two, three) = ys(&pane, time);
        assert!(0.0 < two && two < ROW_H && 0.0 < three && three < ROW_H, "{two} {three}");
        let team = &pane.view.motion.teams[&1];
        assert!(team.busy(time) && team.rows[&3].status_change && !team.rows[&2].status_change);
        // ...and after MOVE they have settled where the keys already had them.
        frames(&mut pane, &list, &mut time, 6);
        assert!(time > started + MOVE);
        assert_eq!(ys(&pane, time), (ROW_H, 0.0));
        assert!(!pane.view.motion.teams[&1].busy(time));
        assert_eq!(order(&pane), [1, 3, 2]);
        assert!(pane.targets[1].1.top() < pane.targets[2].1.top());

        // With animations off the next change jumps.
        crate::ui::glide::set_animations(&ctx, false);
        agents[1].status = Status::NeedsYou;
        list = Agents::new(agents.clone());
        frames(&mut pane, &list, &mut time, 1);
        assert_eq!(ys(&pane, time), (0.0, ROW_H));
        assert!(!pane.view.motion.teams[&1].busy(time));
    }

    #[test]
    fn capsules_are_laid_along_the_strip_with_a_count_for_the_rest() {
        let stage = |pane, role: &str| {
            let mut a = agent(pane, Some(1));
            a.team.role = Some(role.into());
            a
        };
        let (b1, b2, merge, other) = (stage(2, "build"), stage(3, "build"), stage(4, "merge"), agent(5, Some(1)));
        let stages = [Step::Stage(&b1), Step::Stage(&b2), Step::Stage(&merge)];
        // The builders side by side, a hairline on to the merge stage.
        let laid = capsule_layout(&stages, &[60.0, 60.0, 60.0], 300.0);
        assert_eq!(laid, CapsuleLayout { xs: vec![0.0, 66.0, 142.0], joins: vec![(129.0, 139.0)], more: None });
        // The last one fits exactly; a point less and it's a +1 after the builders.
        assert_eq!(capsule_layout(&stages, &[60.0, 60.0, 60.0], 202.0).more, None);
        let tight = capsule_layout(&stages, &[60.0, 60.0, 60.0], 201.0);
        assert_eq!(tight, CapsuleLayout { xs: vec![0.0, 66.0], joins: vec![], more: Some((132.0, 1)) });
        // Room is kept for the +N while agents are left: here only the first fits beside it.
        let narrow = capsule_layout(&stages, &[60.0, 60.0, 60.0], 170.0);
        assert_eq!((narrow.xs, narrow.more), (vec![0.0], Some((66.0, 2))));
        assert_eq!(capsule_layout(&stages, &[60.0, 60.0, 60.0], 40.0).more, Some((0.0, 3)), "nothing fits");

        // A helper, then a pipeline's name before its stages (a name is no agent).
        let nested = [Step::Helper(&other), Step::Heading("feature"), Step::Stage(&b1), Step::Stage(&merge)];
        let laid = capsule_layout(&nested, &[50.0, 40.0, 60.0, 60.0], 400.0);
        assert_eq!(laid, CapsuleLayout { xs: vec![0.0, 64.0, 112.0, 188.0], joins: vec![(175.0, 185.0)], more: None });
        assert_eq!(capsule_layout(&nested, &[50.0, 40.0, 60.0, 60.0], 150.0).more, Some((56.0, 2)));
    }

    #[test]
    fn a_reorder_leaves_the_helpers_tags_as_they_were() {
        let mut agents = vec![agent(1, None), helper(2, "anna", Some("build"), 10), helper(3, "ben", Some("build"), 20), helper(4, "cy", Some("build"), 30), helper(5, "merge", Some("merge"), 40)];
        let tags = |agents: &[Agent]| {
            let g = &groups(&Agents::new(agents.to_vec()))[0];
            g.members.iter().zip(member_tags(g)).map(|(m, tag)| (m.agent.pane, tag)).collect::<Vec<_>>()
        };
        let before = tags(&agents);
        assert_eq!(before, [(2, "Build-01".to_string()), (3, "Build-02".into()), (4, "Build-03".into()), (5, "Merge-01".into())]);
        // The last builder finishes and the first goes to work: the rows change places,
        // each helper keeps its tag.
        agents[3].status = Status::Done;
        agents[1].status = Status::Working;
        let after = tags(&agents);
        assert_eq!(after.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [4, 2, 3, 5]);
        let mut sorted = after.clone();
        sorted.sort();
        assert_eq!(sorted, before);
    }

    #[test]
    fn a_card_says_which_folder_its_agent_works_in_not_which_host() {
        let mut a = agent(1, None);
        assert_eq!(card_detail(&a, None), "Claude Code", "no folder: what it is");
        a.cwd = Some("/home/me/.nex/worktrees/nex-feature-build-17".into());
        a.tab = Some(1);
        assert_eq!(card_detail(&a, None), "nex-feature-build-17 · Tab 2");
        assert_eq!(card_detail(&a, Some("2 helpers")), "nex-feature-build-17 · 2 helpers · Tab 2");

        // On an ssh host: its folder (already shortened with the host's home), not the host.
        let target = crate::remote::SshTarget { opts: vec![], destination: "build-box".into() };
        a.source = Source::Remote { host: "build-box".into(), target, session: "s1".into(), pane: None };
        a.tab = None;
        a.cwd = Some("~/Documents/nex".into());
        let detail = card_detail(&a, None);
        assert_eq!(detail, "nex · Detached");
        assert!(!detail.contains("build-box"));
        a.cwd = Some("~".into());
        assert_eq!(card_detail(&a, Some("1 helper")), "~ · 1 helper");
    }

    #[test]
    fn a_helper_row_says_which_folder_it_works_in() {
        let mut a = agent(2, Some(1));
        assert_eq!(member_detail(&a), "Claude Code");
        a.cwd = Some("/srv/nex-wt-review".into());
        assert_eq!(member_detail(&a), "nex-wt-review");
        a.team.role = Some("review".into());
        a.task = Some("Read the diff".into());
        assert_eq!(member_detail(&a), "nex-wt-review", "the folder alone: its task is on the line above");
        // A sub-agent works in its agent's folder: what kind it is.
        a.team.sub = true;
        a.team.role = Some("Explore".into());
        assert_eq!(member_detail(&a), "Explore · Claude Code");
        a.team.role = None;
        assert_eq!(member_detail(&a), "Sub-agent · Claude Code");
    }

    /// A helper of card 1 that `nex-host` started as `session`, at `started`.
    fn helper(pane: PaneId, session: &str, role: Option<&str>, started: u64) -> Agent {
        let mut a = agent(pane, Some(1));
        a.name = session.into();
        a.session = Some(session.into());
        a.team.role = role.map(Into::into);
        a.team.started = started;
        a
    }

    fn sub(pane: PaneId, kind: Option<&str>, started: u64) -> Agent {
        let mut a = agent(pane, Some(1));
        a.name = "Find where titles are set".into();
        a.task = Some(a.name.clone());
        a.team = Team { parent: Some(1), role: kind.map(Into::into), started, sub: true };
        a
    }

    #[test]
    fn helpers_go_by_a_tag_their_role_or_name_and_a_number() {
        // Its role (a pipeline's stage) is the word; without one, its session's name.
        let build = helper(2, "build", Some("build"), 10);
        assert_eq!((member_word(&build), member_tag(&build, &[])), ("Build".into(), "Build-01".into()));
        let checker = helper(3, "checker", None, 20);
        assert_eq!(member_tag(&checker, &[&build, &checker]), "Checker-01");
        let reviewer = helper(4, "rev", Some("reviewer"), 30);
        assert_eq!(member_tag(&reviewer, &[&build, &checker, &reviewer]), "Reviewer-01", "the role, not the name");
        let mut blank = helper(5, "docs", Some(" "), 40);
        assert_eq!(member_tag(&blank, &[]), "Docs-01", "an empty role is no role");

        // Once its title holds a task its card goes by that, but it's still the same helper.
        blank.name = "Write the changelog".into();
        blank.task = Some(blank.name.clone());
        assert_eq!(member_tag(&blank, &[]), "Docs-01");

        // The number nex-host gave a second one of the same name is its number.
        let second = helper(6, "build-2", Some("build"), 5);
        assert_eq!(member_tag(&second, &[&build, &second]), "Build-02", "though it started first");
        assert_eq!(member_tag(&helper(7, "build-12", None, 0), &[]), "Build-12");
        assert_eq!(member_tag(&helper(8, "build-123", None, 0), &[]), "Build-123", "two digits at least");
        assert_eq!(member_tag(&helper(9, "build-a", None, 0), &[]), "Build-a-01", "only a number is one");
        assert_eq!(numbered("-2"), ("-2", None));

        // Without one: its place among the siblings with the same word, as they started
        // (the same moment: by card).
        let revs = [helper(10, "anna", Some("reviewer"), 30), helper(11, "ben", Some("reviewer"), 10), helper(12, "cy", Some("reviewer"), 30)];
        let all: Vec<&Agent> = revs.iter().chain([&build, &checker]).collect();
        let tags: Vec<String> = revs.iter().map(|a| member_tag(a, &all)).collect();
        assert_eq!(tags, ["Reviewer-02", "Reviewer-01", "Reviewer-03"]);
        assert_eq!(member_tag(&checker, &all), "Checker-01", "other words don't count");
    }

    #[test]
    fn sub_agents_are_tagged_by_their_type() {
        let lead_session = {
            // A sub-agent has no session of its own, whatever its agent's is called.
            let mut s = sub(20, Some("Explore"), 10);
            s.session = None;
            s
        };
        let (second, plain, other) = (sub(21, Some("Explore"), 20), sub(22, None, 30), sub(23, Some("general-purpose"), 5));
        let all = [&lead_session, &second, &plain, &other];
        assert_eq!(all.map(|a| member_tag(a, &all)), ["Explore-01", "Explore-02", "Sub-01", "General-purpose-01"]);
        assert_eq!(member_word(&plain), "Sub");
    }

    #[test]
    fn a_helper_row_is_its_tag_then_the_task_in_its_title() {
        let mut lead = agent(1, None);
        lead.kind = AgentKind::Pipeline;
        let mut build = helper(2, "build", Some("build"), 10);
        let mut second = helper(3, "build-2", Some("build"), 20);
        let merge = helper(4, "merge", Some("merge"), 30);
        // A stage's own sub-agents are numbered among themselves.
        let mut explore = sub(5, Some("Explore"), 15);
        explore.team.parent = Some(2);
        build.cwd = Some("/home/me/.nex/worktrees/nex-feature-build-17".into());
        second.name = "Fix the login redirect".into();
        second.task = Some(second.name.clone());
        let agents = vec![lead, build, second, merge, explore];
        let g = &groups(&Agents::new(agents))[0];
        let rows: Vec<(String, Option<&str>, String)> =
            g.members.iter().zip(member_tags(g)).map(|(m, tag)| (tag, member_task(&m.agent), member_detail(&m.agent))).collect();
        let row = |tag: &str, task: Option<&'static str>, detail: &str| (tag.to_string(), task, detail.to_string());
        assert_eq!(
            rows,
            [
                row("Build-01", None, "nex-feature-build-17"),
                row("Explore-01", Some("Find where titles are set"), "Explore · Claude Code"),
                row("Build-02", Some("Fix the login redirect"), "Claude Code"),
                row("Merge-01", None, "Claude Code"),
            ],
            "the tag alone until it has a task"
        );
        // The closed card's capsules say the same words.
        let words: Vec<String> = g.strip().iter().filter_map(|s| s.agent()).map(member_word).collect();
        assert_eq!(words, ["Build", "Build", "Merge"]);

        // The task follows the title as the agent retitles its terminal.
        let mut a = helper(2, "build", Some("build"), 10);
        a.task = Some("Run the tests".into());
        assert_eq!(member_task(&a), Some("Run the tests"));
        // A pipeline among an agent's helpers: how its run stands.
        a.kind = AgentKind::Pipeline;
        a.task = Some("2 building · 1 merged — ✓ build approved".into());
        assert_eq!(member_task(&a), Some("2 building · 1 merged"));
        a.task = Some(" — ✗ stopped".into());
        assert_eq!(member_task(&a), None);
    }

    #[test]
    fn a_pipelines_header_says_how_its_run_stands() {
        let mut a = agent(1, None);
        a.kind = AgentKind::Pipeline;
        a.name = "feature".into();
        a.cwd = Some("~/Documents/nex".into());
        let text = |a: &Agent| {
            let h = header_text(a, Some("2 stages"));
            (h.title, h.counts, h.detail)
        };
        // An older nex-host sets no title: the card as it was.
        assert_eq!(text(&a), ("feature".into(), None, "nex · 2 stages".into()));

        a.task = Some("2 building · 1 in review · 3 merged — ✓ build (PR 7) approved: merged".into());
        assert_eq!(text(&a), ("Feature".into(), Some("2 building · 1 in review · 3 merged".into()), "✓ build (PR 7) approved: merged · nex · 2 stages".into()));
        a.task = Some("stage 2 of 3: review".into());
        assert_eq!(text(&a), ("Feature".into(), Some("stage 2 of 3: review".into()), "nex · 2 stages".into()), "nothing has happened yet");
        a.task = Some(" — ✗ stopped: no plan".into());
        assert_eq!(text(&a), ("Feature".into(), None, "✗ stopped: no plan · nex · 2 stages".into()));

        // Any other agent's title is its task, and already its name.
        a.kind = AgentKind::Claude;
        a.name = "Fix it — properly".into();
        a.task = Some(a.name.clone());
        assert_eq!(text(&a), ("Fix it — properly".into(), None, "nex · 2 stages".into()));
    }
}
