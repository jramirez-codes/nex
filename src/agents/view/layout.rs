//! Where things go on the Agents pane, without drawing them: the sections and the grid of
//! cards (`layout`), a team card's height and its inset list's rows, the status pill and
//! its chevron, the closed card's strip of capsules (`capsule_layout`), and how far in a
//! helper row and its tree connectors sit. Pure geometry, so the tests check it directly.

use std::collections::HashMap;
use std::sync::Arc;

use egui::{Pos2, Rect, Vec2};

use crate::agents::group::{Group, Member, Step, same_step};
use crate::agents::model::Status;
use crate::agents::view::{
    CARD_H, GAP, INDENT_STEP, LIST_INSET, MAX_COLUMNS, MIN_CARD_W, MORE_H, MORE_W, ROW_H, ROWS_SHOWN, SECTION_H, SIDE, STRIP_H, TITLE_Y,
};
use crate::layout::PaneId;

/// How tall a team's card is, open (`Some(show all)`) or closed.
pub(super) fn team_height(g: &Group, open: Option<bool>) -> f32 {
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
pub(super) fn shown_members(g: &Group, all: bool) -> &[Member] {
    if all { &g.members } else { &g.members[..g.members.len().min(ROWS_SHOWN)] }
}

/// The inset list inside an open team card `r`.
pub(super) fn list_rect(r: Rect) -> Rect {
    Rect::from_min_max(Pos2::new(r.left() + LIST_INSET, r.top() + CARD_H + 2.0), Pos2::new(r.right() - LIST_INSET, r.bottom() - LIST_INSET))
}

/// Helper row `i` of the list in card `r`.
pub(super) fn row_rect(r: Rect, i: usize) -> Rect {
    let list = list_rect(r);
    Rect::from_min_size(Pos2::new(list.left(), list.top() + i as f32 * ROW_H), Vec2::new(list.width(), ROW_H))
}

/// Where the sections and cards go, in content coordinates (before scrolling).
pub(super) struct Layout<'a> {
    /// Each status with cards: its count and its header's rect.
    pub(super) sections: Vec<(Status, usize, Rect)>,
    pub(super) cards: Vec<(&'a Arc<Group>, Rect)>,
    pub(super) content_h: f32,
}

/// Lay `teams` (sorted by status, as [`groups`](crate::agents::group::groups) leaves them) out `width` wide: a header per status, then its cards in as many columns as
/// fit. A closed team is a cell of the grid like any card, only taller, so teams fill rows
/// of their own first and the single cards follow (no row mixes heights); an open team
/// spans the row, as tall as its list.
pub(super) fn layout<'a>(teams: &'a [Arc<Group>], width: f32, expanded: &HashMap<PaneId, bool>) -> Layout<'a> {
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

/// The status pill in header `r`, for a label `label_w` wide: top right.
pub(super) fn pill_rect(r: Rect, label_w: f32) -> Rect {
    Rect::from_min_size(Pos2::new(r.right() - 16.0 - (label_w + 26.0), r.top() + TITLE_Y - 10.0), Vec2::new(label_w + 26.0, 20.0))
}

/// Where a team's chevron is drawn: just left of its status `pill`.
pub(super) fn chevron_center(pill: Rect) -> Pos2 {
    Pos2::new(pill.left() - 16.0, pill.center().y)
}

/// The chevron's click box: a finger-sized square around it, not past the pill.
pub(super) fn chevron_hit(pill: Rect) -> Rect {
    let at = chevron_center(pill);
    Rect::from_min_max(Pos2::new(at.x - 16.0, at.y - 16.0), Pos2::new(pill.left() - 2.0, at.y + 16.0))
}

/// Where a closed team's strip puts its steps, from the strip's left edge.
#[derive(Debug, PartialEq)]
pub(super) struct CapsuleLayout {
    /// The left edge of each step that fits (the first so many of them).
    pub(super) xs: Vec<f32>,
    /// The hairlines joining a pipeline's stage to the one before: from and to.
    pub(super) joins: Vec<(f32, f32)>,
    /// The `+N` capsule's left edge and its N: the agents that don't fit.
    pub(super) more: Option<(f32, usize)>,
}

/// Lay `steps`, each `widths` wide, along a strip `strip_w` wide: a pipeline's stages joined
/// by a hairline to the stage before, side by side with another of the same stage (`build`,
/// `build-2`), a nested pipeline's name before its stages. `+N` for what doesn't fit.
pub(super) fn capsule_layout(steps: &[Step], widths: &[f32], strip_w: f32) -> CapsuleLayout {
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

/// Where a helper row's text starts, for a row whose left edge is `left`: after its tile,
/// indented for a helper's own helpers.
pub(super) fn row_text_x(left: f32, depth: usize) -> f32 {
    left + 12.0 + row_indent(depth) + 26.0 + 10.0
}

/// How far a row at `depth` ([`row_depth`]) has its tile in: a step a level, so a child's
/// tile starts a little right of its parent's centre, room for the connector's arm.
pub(super) fn row_indent(depth: usize) -> f32 {
    INDENT_STEP * (depth.saturating_sub(1).min(3)) as f32
}

/// How far in member `m` of `g`'s list sits: its depth, one more in a pipeline's card,
/// whose stages hang off the card's own (unlisted) row by a connector, and one more for a
/// sub-agent of a team's lead, which hangs off it the same way.
pub(super) fn row_depth(g: &Group, m: &Member) -> usize {
    m.depth + (g.is_pipeline() || (m.depth == 1 && m.agent.team.sub)) as usize
}

/// Where the connectors to `g`'s stages go in its list `list` (its first `shown` rows): for
/// each, its [`icons::icon_branch`](crate::ui::icons::icon_branch) slot, from its pipeline's tile centre to just short of the
/// stage's tile, plus the line's lead-in from a nested pipeline's tile to its first stage and
/// its run past a stage's own helpers to the next stage (zero-width slots). By slot, so they
/// stay while rows glide.
pub(super) fn branch_slots(list: Rect, g: &Group, shown: usize) -> Vec<(Rect, bool)> {
    let tile_left = |m: &Member| list.left() + 12.0 + row_indent(row_depth(g, m));
    let slot_top = |i: usize| list.top() + i as f32 * ROW_H;
    let mut out = Vec::new();
    for (i, b) in g.branches().into_iter().enumerate().take(shown) {
        let Some(b) = b else { continue };
        let m = &g.members[i];
        let x = b.parent.map_or(list.left() + 12.0, |j| tile_left(&g.members[j])) + 13.0;
        let right = tile_left(m) - 3.0;
        if right <= x {
            continue; // As deep as rows go: no room for an arm.
        }
        let row = Rect::from_min_max(Pos2::new(x, slot_top(i)), Pos2::new(right, slot_top(i + 1)));
        // A nested pipeline's first stage: down from just under the pipeline's tile. (The
        // card's own comes down from the list's top: [`branch_lines`] clips it there.)
        if let Some(j) = b.parent
            && j + 1 == i
        {
            let top = slot_top(i) - (ROW_H / 2.0 - 13.0) + 3.0;
            out.push((Rect::from_min_max(Pos2::new(x, top), Pos2::new(x, row.top())), false));
        }
        out.push((row, b.last));
        if !b.last {
            // On past this stage's own helpers to the next stage.
            let past = g.members[i + 1..].iter().take_while(|n| n.depth > m.depth).count().min(shown.saturating_sub(i + 1));
            if past > 0 {
                out.push((Rect::from_min_max(Pos2::new(x, slot_top(i + 1)), Pos2::new(x, slot_top(i + 1 + past))), false));
            }
        }
    }
    out
}
