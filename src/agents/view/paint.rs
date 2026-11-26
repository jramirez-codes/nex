//! The leaf painters of the Agents pane, each drawing one thing where `layout` put it: a
//! section header, a card's body and header, a pipeline's tile, the chevron, a helper row
//! and its connectors, status dots and glyphs, the host notes, the empty state. Also the
//! per-frame looks they take (`Look`, `TileLook`) and the colors behind them. The view's
//! root decides what goes where and in what order; these only paint.

use egui::epaint::Shadow;
use egui::{Align2, Color32, Painter, Pos2, Rect, Stroke, StrokeKind, Vec2};

use crate::ui::theme::{ACCENT, Theme, white_alpha};
use crate::ui::{icons, paint};
use crate::agents::group::{Group, Member};
use crate::agents::model::{Agent, Status, elapsed};
use crate::agents::motion::{Badge, TileMotion, breath, stage_progress};
use crate::agents::view::viewing;
use crate::agents::view::{CARD_H, CARD_RADIUS, DETAIL_Y, Frame, SIDE, TAG_H, TILE, TITLE_Y, status_color, ui_font};
use crate::agents::view::text::{HeaderText, header_text, member_detail, member_task};
use crate::agents::view::layout::{branch_slots, chevron_center, pill_rect, row_indent, row_text_x};

/// A group's caption: "NEEDS YOU  1", small and quiet, like a grouped list's section header.
pub(super) fn section_header(p: &Painter, r: Rect, status: Status, n: usize, c: &Theme) {
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
pub(super) struct Look {
    pub(super) hover: f32,
    pub(super) lift: f32,
    pub(super) border: Color32,
    pub(super) border_width: f32,
    pub(super) tint: Color32,
    /// The accent glow of the agent in the focused pane (0 to 1).
    pub(super) glow: f32,
    /// How far its "Viewing" pill is in, and how far toward the focused pane's look (0 to 1
    /// each).
    pub(super) seen: (f32, f32),
    /// A pipeline's tile: its ring and badge.
    pub(super) tile: Option<TileLook>,
}

/// A pipeline tile's progress ring and corner badge, as drawn this frame (see [`TileMotion`]).
#[derive(Clone, Copy)]
pub(super) struct TileLook {
    /// How far round the ring is, 0 to 1.
    pub(super) ring: f32,
    /// How bright the ring's leading end glows (a stage is working), 0 to 1.
    pub(super) glow: f32,
    /// The badge, its scale and its opacity.
    pub(super) badge: Option<(Badge, f32, f32)>,
    /// Where a Needs-you badge's breath is, 0 to 1 (0: still).
    pub(super) breath: f32,
    /// Whether the tile's dots move: a stage is at work. Not from the pipeline's status,
    /// which is its most urgent stage's, so a finished stage nobody has looked at yet
    /// (Done) hides one that still works.
    pub(super) moving: bool,
}

/// How a pipeline stands from its `stages`: the fraction done, the most urgent status among
/// them, and whether one is at work. `None` before it has any.
pub(super) fn pipeline_state(stages: &[&Agent]) -> Option<(f32, Status, bool)> {
    let progress = stage_progress(stages.iter().map(|a| a.status))?;
    let status = stages.iter().map(|a| a.status).min()?;
    Some((progress, status, stages.iter().any(|a| a.status == Status::Working)))
}

/// `t` as drawn this frame: the working glow pulses with the status dots, the Needs-you
/// badge breathes until looked at. With animations off, both hold still at full. The dots
/// move while a stage is `working`.
pub(super) fn tile_look(f: &Frame, t: &TileMotion, working: bool) -> TileLook {
    let glow = match (working, f.animate) {
        (false, _) => 0.0,
        (true, false) => 1.0,
        (true, true) => 0.65 + 0.35 * (0.5 + 0.5 * (f.time as f32 * std::f32::consts::TAU * 0.55).sin()),
    };
    TileLook { ring: t.ring(f.time), glow, badge: t.badge(f.time), breath: if t.breathing() { breath(f.time) } else { 0.0 }, moving: working }
}

/// A pipeline's app tile `tile` with its ring (under) and badge (over).
pub(super) fn pipeline_tile(f: &Frame, p: &Painter, tile: Rect, agent: &Agent, look: Option<&TileLook>) {
    let phase = tile_phase(f, agent, look);
    let Some(look) = look else {
        icons::app_tile(p, tile, agent.kind.process(), agent.mark.as_deref(), phase);
        return;
    };
    icons::tile_ring(p, tile, look.ring, ACCENT, look.glow);
    icons::app_tile(p, tile, agent.kind.process(), agent.mark.as_deref(), phase);
    match look.badge {
        Some((Badge::NeedsYou, scale, opacity)) => icons::badge_alert(p, tile, f.c.yellow, scale, opacity, look.breath),
        Some((Badge::Done, scale, opacity)) => icons::badge_done(p, tile, f.c.green, scale, opacity),
        None => {}
    }
}

/// Where `agent`'s tile is in its motion this frame, or `None` when it's still. A pipeline
/// with stages (`look`) moves while one of them works, or while it is itself at work between
/// two; without, while its status says it works.
pub(super) fn tile_phase(f: &Frame, agent: &Agent, look: Option<&TileLook>) -> Option<f32> {
    match look {
        Some(l) if l.moving && f.animate => Some(icons::pipeline_phase(f.time)),
        _ => agent.tile_phase(f.animate, f.time),
    }
}

/// The border a card heads for: the accent when it's the selected one in the focused pane,
/// amber
/// when its agent needs you, blue when it's done, a quiet hairline otherwise (brighter under
/// the pointer). A viewed card's accent ring fades in over it ([`viewing::ring_over`]).
pub(super) fn border_for(status: Status, selected: bool, focused: bool, hovered: bool, c: &Theme) -> (Color32, f32) {
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
pub(super) fn card_body(p: &Painter, r: Rect, look: &Look) {
    let Look { hover, lift, border, border_width, glow, .. } = *look;
    viewing::glow(p, r, CARD_RADIUS, glow);
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
/// Returns the pill's rect (a team's chevron sits left of it: [`chevron_hit`](crate::agents::view::layout::chevron_hit)).
pub(super) fn agent_header(f: &Frame, p: &Painter, r: Rect, agent: &Agent, look: &Look, team: Option<(&str, f32)>) -> Rect {
    let c = f.c;
    let tile = Rect::from_center_size(Pos2::new(r.left() + 14.0 + TILE / 2.0, r.top() + CARD_H / 2.0), Vec2::splat(TILE));
    pipeline_tile(f, p, tile, agent, look.tile.as_ref());

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
    let (shown, primary) = look.seen;
    if shown > 0.01 {
        let right = title_right + 4.0;
        let left = viewing::pill(p, right, pill.center().y, primary, shown, viewing::compact(p, x, right));
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

/// A disclosure chevron at `at`, turning from › (`open` 0) to ⌄ (`open` 1).
pub(super) fn chevron(p: &Painter, at: Pos2, open: f32, color: Color32) {
    let angle = open * std::f32::consts::FRAC_PI_2;
    let (s, c) = angle.sin_cos();
    let rot = |v: Vec2| at + Vec2::new(v.x * c - v.y * s, v.x * s + v.y * c);
    let stroke = Stroke::new(1.6_f32, color);
    let tip = rot(Vec2::new(2.5, 0.0));
    p.line_segment([rot(Vec2::new(-1.5, -4.0)), tip], stroke);
    p.line_segment([tip, rot(Vec2::new(-1.5, 4.0))], stroke);
}

/// Paints [`branch_slots`]: the connector icon in each stage's slot, a plain line where it
/// only runs on (a zero-width slot). A quiet hairline, a step over the separators, starting
/// a little below the list's top.
pub(super) fn branch_lines(p: &Painter, list: Rect, g: &Group, shown: usize) {
    let c = white_alpha(22);
    let p = p.with_clip_rect(p.clip_rect().intersect(Rect::from_min_max(list.min + Vec2::new(0.0, 6.0), list.max)));
    for (r, last) in branch_slots(list, g, shown) {
        if r.width() <= 0.0 {
            p.line_segment([r.left_top(), r.left_bottom()], Stroke::new(1.2_f32, c));
        } else {
            icons::icon_branch(&p, r, c, last);
        }
    }
}

/// One helper's row in an open team: tile (indented for a helper's own helpers), its `tag`
/// in a small capsule and the task it's on (see [`member_task`]) over the folder it works
/// in, and its status with how long it has been in it.
#[allow(clippy::too_many_arguments)]
pub(super) fn helper_row(f: &Frame, p: &Painter, row: Rect, m: &Member, depth: usize, tag: &str, seen: (f32, f32), (tick, tile_look): (f32, Option<TileLook>)) {
    let c = f.c;
    let a = &m.agent;
    let tile = Rect::from_center_size(Pos2::new(row.left() + 12.0 + row_indent(depth) + 13.0, row.center().y), Vec2::splat(26.0));
    pipeline_tile(f, p, tile, a, tile_look.as_ref());
    let x = row_text_x(row.left(), depth);

    let color = status_color(a.status, c);
    let since = elapsed(f.now.saturating_duration_since(a.since));
    let since_g = p.layout_no_wrap(since, ui_font(11.5), c.text_dim);
    let since_x = row.right() - 12.0 - since_g.size().x;
    let label_g = p.layout_no_wrap(a.status.label().to_string(), paint::bold(11.0), color);
    let label_x = since_x - 12.0 - label_g.size().x;
    let dot = Pos2::new(label_x - 9.0, row.center().y);
    member_glyph(f, p, dot, a.status, color, tick);
    p.galley(Pos2::new(label_x, row.center().y - label_g.size().y / 2.0), label_g, color);
    p.galley(Pos2::new(since_x, row.center().y - since_g.size().y / 2.0), since_g, c.text_dim);

    // An eye left of the status while it's on screen (`seen`: how far in, and how far
    // toward the focused pane's look), making room as it fades in.
    let (shown, primary) = seen;
    let mut end = dot.x;
    if shown > 0.01 {
        end -= 18.0 * shown;
        viewing::row_eye(p, Pos2::new(end, row.center().y), shown, primary);
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

/// A helper's status in a capsule or row: a tick once it's done, else its dot. `drawn` (0 to
/// 1): how far a tick that has just appeared is drawn in; the dot it replaces shrinks away
/// as the stroke goes.
pub(super) fn member_glyph(f: &Frame, p: &Painter, at: Pos2, status: Status, color: Color32, drawn: f32) {
    if status == Status::Done {
        if drawn < 1.0 {
            let gone = (drawn * 2.0).min(1.0);
            p.circle_filled(at, 3.5 * (1.0 - 0.6 * gone), color.gamma_multiply(1.0 - gone));
        }
        icons::check(p, at, 1.0, Stroke::new(1.6_f32, color), drawn);
    } else {
        status_dot(f, p, at, status, color);
    }
}

/// A Needs-you capsule's fill: amber, breathing slowly (steady with animations off).
pub(super) fn needs_you_tint(f: &Frame) -> Color32 {
    let strength = if f.animate { 0.09 + 0.1 * breath(f.time) } else { 0.14 };
    f.c.yellow.gamma_multiply(strength)
}

/// Whether anything on a team's card moves, so the pane keeps repainting: its lead's dot, a
/// stage's glowing ring end (and a pipeline's tile) while it works, a Needs-you capsule's
/// tint, or a running pipeline's tile among its helpers. A member at work counts even when
/// the lead's status doesn't say so (a pipeline's Done stage outranks its Working one).
pub(super) fn team_breathes(g: &Group) -> bool {
    g.lead.status == Status::Working || g.members.iter().any(|m| matches!(m.agent.status, Status::Working | Status::NeedsYou) || m.agent.tile_animates())
}

/// The status dot. While working it breathes, and a soft ring keeps rippling out of it.
pub(super) fn status_dot(f: &Frame, p: &Painter, at: Pos2, status: Status, color: Color32) {
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
pub(super) fn host_notes(f: &Frame, p: &Painter, area: Rect, notes: &[String]) -> f32 {
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
pub(super) fn empty_state(f: &Frame, p: &Painter, area: Rect) {
    let c = f.c;
    let center = area.center() - Vec2::new(0.0, (area.height() * 0.08).min(36.0));
    let g = center - Vec2::new(0.0, 40.0);
    p.circle_filled(g, 28.0, white_alpha(10));
    icons::sparkle(p, g, 15.0, c.text_dim);
    p.text(center + Vec2::new(0.0, 6.0), Align2::CENTER_CENTER, "No Agents Running", paint::bold(15.0), c.text_bright);
    let detail = "Start claude or codex in any pane to see it here.";
    paint::caption(p, Rect::from_center_size(center + Vec2::new(0.0, 28.0), Vec2::new((area.width() - 40.0).max(0.0), 18.0)), detail, ui_font(12.5), c.text_dim);
}
