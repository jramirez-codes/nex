//! The Agents view's tests: layout and text checked directly, and whole frames run
//! through egui to check what's drawn, where clicks go and how things move.

use super::*;
use crate::agents::group::{Group, Step, groups};
use crate::agents::model::{AgentKind, Source, Team};
use crate::agents::view::layout::{
    CapsuleLayout, Layout, branch_slots, capsule_layout, chevron_center, chevron_hit, layout, pill_rect, row_depth, row_indent, row_rect, row_text_x,
    shown_members,
};
use crate::agents::view::paint::{border_for, pipeline_state, team_breathes};
use crate::agents::view::reveal::Held;
use crate::agents::view::text::{card_detail, header_text, member_detail, member_tag, member_tags, member_task, member_word, numbered};
use crate::ui::icons;

fn agent(pane: PaneId, parent: Option<PaneId>) -> Agent {
    Agent {
        pane,
        tab: None,
        kind: AgentKind::CLAUDE,
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

/// A pipeline's card: its stages one step in, so each tile starts right of the column
/// the connector comes down; one icon slot per stage on the same grid, and a run on past
/// the first stage's own helper.
#[test]
fn a_pipelines_stages_hang_off_it_by_one_connector_each() {
    let mut pipe = agent(1, None);
    pipe.kind = AgentKind::Pipeline;
    let stage = |pane| Agent { team: Team { role: Some(format!("s{pane}")), ..agent(pane, Some(1)).team }, ..agent(pane, Some(1)) };
    let sub = |pane, parent| Agent { team: Team { sub: true, ..agent(pane, Some(parent)).team }, ..agent(pane, Some(parent)) };
    let teams = groups(&Agents::new(vec![pipe, stage(2), sub(4, 2), stage(3)]));
    let g = &teams[0];
    assert_eq!(g.members.iter().map(|m| (m.agent.pane, row_depth(g, m))).collect::<Vec<_>>(), [(2, 2), (4, 3), (3, 2)]);

    let list = Rect::from_min_size(Pos2::new(100.0, 0.0), Vec2::new(400.0, 3.0 * ROW_H));
    let column = list.left() + 25.0;
    let tile_left = list.left() + 12.0 + row_indent(2);
    assert!(tile_left - column >= 8.0, "room for the arm: {tile_left} vs {column}");
    let slots = branch_slots(list, g, &g.branches(), 3);
    let slot = |i: usize| Rect::from_min_max(Pos2::new(column, i as f32 * ROW_H), Pos2::new(tile_left - 3.0, (i + 1) as f32 * ROW_H));
    let run = Rect::from_min_max(Pos2::new(column, ROW_H), Pos2::new(column, 2.0 * ROW_H));
    assert_eq!(slots, [(slot(0), false), (run, false), (slot(2), true)]);
    // The separators clear the line: each starts at its row's text, right of the column.
    assert!(g.members.iter().all(|m| row_text_x(list.left(), row_depth(g, m)) > column));

    // Cut off after the first stage: its line still runs on, out of the list.
    assert_eq!(branch_slots(list, g, &g.branches(), 1), [(slot(0), false)]);
}

/// A team's sub-agents are drawn as a pipeline's stages are: as far in, off the same
/// column by the same connector, with the same tag and line under it.
#[test]
fn a_teams_sub_agents_are_laid_out_as_a_pipelines_stages() {
    let mut pipe = agent(1, None);
    pipe.kind = AgentKind::Pipeline;
    let stage = |pane| Agent { team: Team { role: Some("Explore".into()), ..agent(pane, Some(1)).team }, ..agent(pane, Some(1)) };
    let pipeline = groups(&Agents::new(vec![pipe, stage(2), stage(3)]));
    let mut team = vec![agent(1, None), sub(2, Some("Explore"), 2), sub(3, Some("Explore"), 3)];
    for a in &mut team[1..] {
        a.name = "a".into();
    }
    let team = groups(&Agents::new(team));
    let list = Rect::from_min_size(Pos2::new(100.0, 0.0), Vec2::new(400.0, 2.0 * ROW_H));
    let (p, t) = (&pipeline[0], &team[0]);
    let depths = |g: &Group| g.members.iter().map(|m| row_depth(g, m)).collect::<Vec<_>>();
    assert_eq!(depths(t), depths(p));
    assert_eq!(branch_slots(list, t, &t.branches(), 2), branch_slots(list, p, &p.branches(), 2));
    assert_eq!(member_tags(t), member_tags(p));
    assert_eq!(member_detail(&t.members[0].agent), member_detail(&p.members[0].agent));
    assert!(t.strip().iter().all(|s| matches!(s, Step::Helper(_))), "capsules side by side, as one step's stages");
}

fn pipeline(pane: PaneId, status: Status) -> Agent {
    Agent { kind: AgentKind::Pipeline, status, ..agent(pane, None) }
}

#[test]
fn only_a_running_pipelines_tile_moves_and_only_with_animations_on() {
    let t = 0.3;
    assert!(pipeline(1, Status::Working).tile_phase(true, t).is_some());
    assert_eq!(pipeline(1, Status::Working).tile_phase(false, t), None, "animations off");
    for status in [Status::Idle, Status::Done, Status::NeedsYou] {
        assert_eq!(pipeline(1, status).tile_phase(true, t), None, "{status:?}");
    }
    let claude = Agent { status: Status::Working, ..agent(1, None) };
    assert_eq!(claude.tile_phase(true, t), None, "only pipelines have a moving tile");
    // The phase goes round once every cycle.
    let p = pipeline(1, Status::Working);
    assert_eq!(p.tile_phase(true, 0.0), p.tile_phase(true, icons::PIPELINE_CYCLE));
}

#[test]
fn a_running_pipeline_keeps_its_card_breathing() {
    let working = |pane, parent| Agent { status: Status::Working, ..agent(pane, parent) };
    // A pipeline whose stage is working: its card says Working, its tile moves.
    let agents = Agents::new(vec![pipeline(1, Status::Working), working(2, Some(1)), agent(3, Some(1))]);
    let teams = groups(&agents);
    let g = teams.iter().find(|g| g.key() == 1).unwrap();
    assert!(g.is_team() && team_breathes(g));
    assert!(g.lead.tile_phase(true, 0.2).is_some());
    // A pipeline in an idle agent's team, its runner printing between stages: the card
    // is idle, but the pipeline's row shows it working, its tile moving.
    let nested = Agent { team: Team { parent: Some(1), role: None, started: 2, sub: false }, ..pipeline(2, Status::Working) };
    let agents = Agents::new(vec![agent(1, None), nested, agent(3, Some(2))]);
    let teams = groups(&agents);
    let g = teams.iter().find(|g| g.key() == 1).unwrap();
    assert!(g.lead.status == Status::Idle && team_breathes(g));
    // Nothing working: still.
    let agents = Agents::new(vec![pipeline(1, Status::Idle), agent(2, Some(1))]);
    assert!(groups(&agents).iter().all(|g| !team_breathes(g)));
}

#[test]
fn a_pipeline_tile_keeps_moving_while_a_stage_works() {
    let with = |pane, status| Agent { status, ..agent(pane, Some(1)) };
    // The pipeline's card and whether its tile's dots move.
    let state = |agents: &Agents| {
        let g = groups(agents).into_iter().find(|g| g.key() == 1).unwrap();
        let stages: Vec<Agent> = g.direct().map(|m| (*m.agent).clone()).collect();
        let (_, _, moving) = pipeline_state(&stages.iter().collect::<Vec<_>>()).unwrap();
        (g, moving)
    };
    // Build done (not looked at yet), merge working: the card says Done, but it runs on.
    let agents = Agents::new(vec![pipeline(1, Status::Idle), with(2, Status::Done), with(3, Status::Working)]);
    let (g, moving) = state(&agents);
    assert_eq!(g.lead.status, Status::Done, "Done outranks Working");
    assert!(moving && team_breathes(&g));
    // Merge needs you, nothing works: the tile is still, the badge breathes.
    let agents = Agents::new(vec![pipeline(1, Status::Idle), with(2, Status::Done), with(3, Status::NeedsYou)]);
    let (g, moving) = state(&agents);
    assert!(!moving && team_breathes(&g));
    // Every stage done: still.
    let agents = Agents::new(vec![pipeline(1, Status::Idle), with(2, Status::Done), with(3, Status::Done)]);
    let (g, moving) = state(&agents);
    assert!(!moving && !team_breathes(&g));
    let agents = Agents::new(vec![pipeline(1, Status::Idle), with(2, Status::Idle), with(3, Status::Done)]);
    let (g, moving) = state(&agents);
    assert!(!moving && !team_breathes(&g));
}

#[test]
fn a_closed_team_is_a_cell_of_the_grid_and_spans_the_row_when_open() {
    // Two single cards, two teams (8 and 2 helpers), another single: 2 columns at 700.
    let mut agents = vec![agent(1, None), agent(2, None), agent(3, None), agent(4, None), agent(9, None)];
    agents.extend((10..18).map(|p| agent(p, Some(3))));
    agents.extend((20..22).map(|p| agent(p, Some(4))));
    let teams = groups(&Agents::new(agents));
    let closed = layout(&teams, 700.0, &HashMap::new());
    let rect = |l: &Layout<Group>, key| l.cards.iter().find(|(g, _)| g.key() == key).unwrap().1;
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
fn a_viewed_card_has_an_accent_ring_that_fades_over_its_border() {
    use crate::agents::model::Seen;
    let c = Theme::from_config(&crate::config::Config::default());
    let (primary, shared) = (viewing::ring(Seen::Primary), viewing::ring(Seen::Shared));
    assert!(primary.1 > shared.1 && primary.0.a() > shared.0.a(), "the focused pane's agent is ringed more strongly");
    let base = border_for(Status::NeedsYou, false, true, false, &c);
    // All the way in it is the ring, exactly (over "needs you" too: its pill still says
    // that); all the way out, exactly the border it had.
    assert_eq!(viewing::ring_over(base, 1.0, 1.0), primary);
    assert_eq!(viewing::ring_over(base, 1.0, 0.0), shared);
    assert_eq!(viewing::ring_over(base, 0.0, 1.0), base);
    // Halfway, a blend of the two.
    let (mid, w) = viewing::ring_over(base, 0.5, 1.0);
    assert!(mid != base.0 && mid != primary.0 && w > base.1 && w < primary.1);
    assert_ne!(border_for(Status::Idle, false, true, false, &c), shared);
}

/// Paints `pane` showing `agents` in an 800×600 window, with `events`.
fn frame(ctx: &egui::Context, pane: &mut AgentsPane, agents: &Agents, time: &mut f64, events: Vec<egui::Event>) {
    frame_viewing(ctx, pane, agents, time, events, &Viewing::default());
}

/// [`frame`], with `viewing` on screen.
fn frame_viewing(ctx: &egui::Context, pane: &mut AgentsPane, agents: &Agents, time: &mut f64, events: Vec<egui::Event>, viewing: &Viewing) {
    *time += 0.05;
    let theme = Theme::from_config(&crate::config::Config::default());
    let input = egui::RawInput {
        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(800.0, 600.0))),
        time: Some(*time),
        events,
        ..Default::default()
    };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let rect = ui.max_rect();
            pane.paint(ui, ui.painter(), rect, PaintArgs { pane: 1, focused: true, c: &theme, agents, notes: &[], shown_tab: None, viewing });
        });
    });
}

#[test]
fn a_card_and_a_helper_row_fade_their_viewing_mark_in_and_out() {
    use crate::agents::motion::VIEW;
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, true);
    let list = Agents::new(vec![agent(1, None), agent(2, Some(1)), agent(9, None)]);
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    let none = Viewing::default();
    // Open the team and let everything settle, nothing on screen.
    for _ in 0..2 {
        frame(&ctx, &mut pane, &list, &mut time, vec![]);
    }
    pane.expanded.insert(1, false);
    for _ in 0..14 {
        frame(&ctx, &mut pane, &list, &mut time, vec![]);
    }
    let card = |pane: &AgentsPane, id, t| pane.view.motion.cards[&id].seen.shown(t);
    let row = |pane: &AgentsPane, t| pane.view.motion.teams[&1].rows[&2].seen.shown(t);
    assert_eq!((card(&pane, 9, time), row(&pane, time)), (0.0, 0.0));

    // Card 9 is in the focused pane, helper 2 shares the tab: both ease in, partway after
    // one frame (shorter than VIEW), all the way once VIEW has passed.
    let on = Viewing { panes: vec![2, 9], primary: Some(9) };
    frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &on);
    let started = time;
    frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &on);
    let (c, r) = (card(&pane, 9, time), row(&pane, time));
    assert!(c > 0.0 && c < 1.0 && r > 0.0 && r < 1.0, "fading in: {c} {r}");
    assert!(pane.view.motion.cards[&9].busy(time));
    while time < started + VIEW {
        frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &on);
    }
    assert_eq!((card(&pane, 9, time), row(&pane, time)), (1.0, 1.0));
    assert_eq!(pane.view.motion.cards[&9].seen.primary(time), 1.0);
    assert!(!pane.view.motion.cards[&9].busy(time) && !pane.view.motion.teams[&1].busy(time), "settled: nothing left to redraw");

    // Off screen again: they ease back out to exactly none.
    frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &none);
    let left = time;
    frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &none);
    assert!(card(&pane, 9, time) > 0.0 && card(&pane, 9, time) < 1.0);
    while time < left + VIEW {
        frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &none);
    }
    assert_eq!((card(&pane, 9, time), row(&pane, time)), (0.0, 0.0));
    assert!(!pane.view.motion.cards[&9].busy(time));
}

/// Viewing a pipeline's stage moves the card's ring and glow onto that stage's row
/// (opening the closed card to show it); viewing the pipeline itself puts them back on
/// the card and off every row.
#[test]
fn a_viewed_stage_takes_the_ring_and_glow_off_its_pipelines_card() {
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, false);
    let stage = |pane| Agent { team: Team { role: Some(format!("s{pane}")), ..agent(pane, Some(1)).team }, ..agent(pane, Some(1)) };
    let list = Agents::new(vec![pipeline(1, Status::Working), stage(2), stage(3)]);
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    frame(&ctx, &mut pane, &list, &mut time, vec![]);
    assert!(!pane.expanded.contains_key(&1), "closed to begin with");
    // No keyboard highlight, whose ring would stand in for the viewing one.
    pane.view.jumped = true;

    let card = |pane: &AgentsPane, time| {
        let c = &pane.view.motion.cards[&1];
        (c.seen.shown(time), c.ring.shown(time))
    };
    let row =
        |pane: &AgentsPane, id, time| pane.view.motion.teams[&1].rows.get(&id).map_or((0.0, 0.0), |r| (r.seen.shown(time), r.seen.primary(time)));

    let child = Viewing { panes: vec![2], primary: Some(2) };
    for _ in 0..3 {
        frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &child);
    }
    assert!(pane.expanded.contains_key(&1), "the card opens to show the stage");
    assert_eq!(card(&pane, time), (0.0, 0.0), "no ring or glow on the pipeline's card");
    assert_eq!(row(&pane, 2, time), (1.0, 1.0), "the stage's row has them, at full strength");
    assert_eq!(row(&pane, 3, time), (0.0, 0.0));
    assert_eq!(viewing::glow_strength(row(&pane, 2, time).0, row(&pane, 2, time).1), 1.0);

    let lead = Viewing { panes: vec![1], primary: Some(1) };
    for _ in 0..3 {
        frame_viewing(&ctx, &mut pane, &list, &mut time, vec![], &lead);
    }
    assert_eq!(card(&pane, time), (1.0, 1.0), "the pipeline in view rings its card");
    assert_eq!((row(&pane, 2, time).0, row(&pane, 3, time).0), (0.0, 0.0), "and no row");
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
    let label_w = painter.layout_no_wrap(Status::Idle.label().to_string(), ui::paint::bold(11.0), Color32::WHITE).size().x;
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
fn clicking_a_sub_agent_opens_its_tab_from_its_capsule_and_its_row() {
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, false);
    // An agent running two sub-agents.
    let agents = Agents::new(vec![agent(1, None), sub(2, Some("Explore"), 10), sub(3, Some("Plan"), 20)]);
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    for _ in 0..3 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    let body = Rect::from_min_max(Pos2::new(0.0, 6.0), Pos2::new(800.0, 600.0));
    let teams = groups(&agents);
    let card = layout(&teams, body.width() - 2.0 * SIDE, &HashMap::new()).cards[0].1.translate(body.min.to_vec2());
    let strip_y = card.top() + CARD_H + STRIP_H / 2.0;

    // The first capsule, at the start of the strip, is a sub-agent's: it asks for its tab
    // (the app opens or focuses it), and the card stays closed.
    let first = teams[0].strip()[0].agent().unwrap().pane;
    click(&ctx, &mut pane, &agents, &mut time, Pos2::new(card.left() + 14.0 + 12.0, strip_y));
    assert_eq!(pane.requests, vec![super::super::Request::Focus(first)]);
    assert!(pane.expanded.is_empty(), "rather than opening the card");
    pane.requests.clear();

    // The strip past the capsules still opens it.
    click(&ctx, &mut pane, &agents, &mut time, Pos2::new(card.right() - 20.0, strip_y));
    assert_eq!(pane.expanded.get(&1), Some(&false));
    assert!(pane.requests.is_empty());

    // Open, a sub-agent's row asks for it too.
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    let open = layout(&teams, body.width() - 2.0 * SIDE, &pane.expanded).cards[0].1.translate(body.min.to_vec2());
    let second = shown_members(&teams[0], false)[1].agent.pane;
    click(&ctx, &mut pane, &agents, &mut time, row_rect(open, 1).center());
    assert_eq!(pane.requests, vec![super::super::Request::Focus(second)]);

    // So does one under a pipeline's stage, in the pipeline's card.
    let mut pipe = agent(1, None);
    pipe.kind = AgentKind::Pipeline;
    let mut stage = helper(2, "build", Some("build"), 5);
    stage.team.parent = Some(1);
    let mut under = sub(3, Some("Explore"), 10);
    under.team.parent = Some(2);
    let agents = Agents::new(vec![pipe, stage, under]);
    let mut pane = AgentsPane::new();
    pane.expanded.insert(1, false);
    for _ in 0..3 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    let teams = groups(&agents);
    let open = layout(&teams, body.width() - 2.0 * SIDE, &pane.expanded).cards[0].1.translate(body.min.to_vec2());
    assert_eq!(shown_members(&teams[0], false)[1].agent.pane, 3);
    click(&ctx, &mut pane, &agents, &mut time, row_rect(open, 1).center());
    assert_eq!(pane.requests, vec![super::super::Request::Focus(3)]);
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
        kind: AgentKind::CLAUDE,
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
    let teams = |pane: &AgentsPane| Arc::clone(&pane.view.grouped.as_ref().expect("painted").cards);

    frame(&ctx, &mut pane, tracker.agents(), &mut time, vec![]);
    let first = teams(&pane);
    assert_eq!(first.iter().map(|c| (c.key(), c.group.members.len())).collect::<Vec<_>>(), [(1, 1), (3, 0)]);
    // Nothing is copied on the way to the screen: the cards hold the tracker's agents.
    let tracked = |tracker: &Tracker, pane: PaneId| Arc::clone(tracker.agents().iter().find(|a| a.pane == pane).unwrap());
    assert!(Arc::ptr_eq(&first[0].group.lead, &tracked(&tracker, 1)) && Arc::ptr_eq(&first[0].group.members[0].agent, &tracked(&tracker, 2)));
    assert!(Arc::ptr_eq(&pane.view.motion.cards[&3].agent, &tracked(&tracker, 3)));
    assert!(Arc::ptr_eq(&pane.view.cards[&1], &first[0]));

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
    assert_eq!(second.iter().map(|c| (c.key(), c.group.lead.status)).collect::<Vec<_>>(), [(3, Status::Working), (1, Status::Idle)]);
}

#[test]
fn a_pipelines_tile_rings_its_finished_stages_and_a_finished_stage_draws_its_tick() {
    use crate::agents::motion::{Badge, MOVE, POP, TICK};
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, true);
    let stage = |pane, status| Agent { status, ..agent(pane, Some(1)) };
    let mut agents =
        vec![Agent { kind: AgentKind::Pipeline, ..agent(1, None) }, stage(2, Status::Idle), stage(3, Status::Working), stage(4, Status::Working)];
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    for _ in 0..2 {
        frame(&ctx, &mut pane, &Agents::new(agents.clone()), &mut time, vec![]);
    }
    fn card(pane: &AgentsPane) -> &crate::agents::motion::CardMotion {
        &pane.view.motion.cards[&1]
    }
    assert!((card(&pane).tile.ring(time) - 1.0 / 3.0).abs() < 1e-6, "1 of 3 stages finished");
    assert!(card(&pane).tile.badge(time).is_none());

    // Stage 3 finishes: the ring eases on to 2 of 3 and the stage's capsule draws its tick.
    agents[2].status = Status::Done;
    let list = Agents::new(agents.clone());
    frame(&ctx, &mut pane, &list, &mut time, vec![]);
    let started = time;
    assert_eq!(pane.view.motion.teams[&1].tick(time, 3), 0.0);
    frame(&ctx, &mut pane, &list, &mut time, vec![]);
    let ring = card(&pane).tile.ring(time);
    assert!(1.0 / 3.0 < ring && ring < 2.0 / 3.0, "on its way: {ring}");
    assert!(card(&pane).busy(time) && pane.view.motion.teams[&1].busy(time));
    while time < started + MOVE.max(TICK) {
        frame(&ctx, &mut pane, &list, &mut time, vec![]);
    }
    assert!((card(&pane).tile.ring(time) - 2.0 / 3.0).abs() < 1e-6);
    assert_eq!(pane.view.motion.teams[&1].tick(time, 3), 1.0);
    // The team is Done (an unseen stage finished), but stage 4 still works: no tick yet.
    assert_eq!(pane.view.cards[&1].group.lead.status, Status::Done);
    assert_eq!(card(&pane).tile.badge(time), None, "no Done badge mid-run");

    // The last stage finishes: the ring closes and the Done badge pops in.
    agents[3].status = Status::Done;
    let list = Agents::new(agents.clone());
    frame(&ctx, &mut pane, &list, &mut time, vec![]);
    let started = time;
    assert_eq!(card(&pane).tile.badge(time).map(|b| b.0), Some(Badge::Done));
    while time < started + MOVE.max(POP).max(TICK) {
        frame(&ctx, &mut pane, &list, &mut time, vec![]);
    }
    assert!((card(&pane).tile.ring(time) - 1.0).abs() < 1e-6);
    assert_eq!(card(&pane).tile.badge(time), Some((Badge::Done, 1.0, 1.0)));
    assert!(!card(&pane).busy(time) && !pane.view.motion.teams[&1].busy(time), "settled: no more redraws");
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
    let (b1, b2, merge, other) = (Arc::new(b1), Arc::new(b2), Arc::new(merge), Arc::new(other));
    let stages = [Step::Stage(Arc::clone(&b1)), Step::Stage(b2), Step::Stage(Arc::clone(&merge))];
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
    let nested = [Step::Helper(other), Step::Heading("feature".into()), Step::Stage(b1), Step::Stage(merge)];
    let laid = capsule_layout(&nested, &[50.0, 40.0, 60.0, 60.0], 400.0);
    assert_eq!(laid, CapsuleLayout { xs: vec![0.0, 64.0, 112.0, 188.0], joins: vec![(175.0, 185.0)], more: None });
    assert_eq!(capsule_layout(&nested, &[50.0, 40.0, 60.0, 60.0], 150.0).more, Some((56.0, 2)));
}

#[test]
fn a_reorder_leaves_the_helpers_tags_as_they_were() {
    let mut agents = vec![
        agent(1, None),
        helper(2, "anna", Some("build"), 10),
        helper(3, "ben", Some("build"), 20),
        helper(4, "cy", Some("build"), 30),
        helper(5, "merge", Some("merge"), 40),
    ];
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
    // A sub-agent's is its agent's folder, as a stage's is: its tag says its type.
    a.team.sub = true;
    a.team.role = Some("Explore".into());
    assert_eq!(member_detail(&a), "nex-wt-review");
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
            row("Explore-01", Some("Find where titles are set"), "Claude Code"),
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
    assert_eq!(
        text(&a),
        ("Feature".into(), Some("2 building · 1 in review · 3 merged".into()), "✓ build (PR 7) approved: merged · nex · 2 stages".into())
    );
    a.task = Some("stage 2 of 3: review".into());
    assert_eq!(text(&a), ("Feature".into(), Some("stage 2 of 3: review".into()), "nex · 2 stages".into()), "nothing has happened yet");
    a.task = Some(" — ✗ stopped: no plan".into());
    assert_eq!(text(&a), ("Feature".into(), None, "✗ stopped: no plan · nex · 2 stages".into()));

    // Any other agent's title is its task, and already its name.
    a.kind = AgentKind::CLAUDE;
    a.name = "Fix it — properly".into();
    a.task = Some(a.name.clone());
    assert_eq!(text(&a), ("Fix it — properly".into(), None, "nex · 2 stages".into()));
}

/// What a frame leaves for the keys: the cards in reading order (by section, a team's card
/// before the single ones), an open team's rows after its card, and the highlight on the
/// first card until a key moves it. A highlighted helper whose row is no longer drawn hands
/// the highlight to its team's card; one that's gone, to the first card.
#[test]
fn a_frame_leaves_the_cards_and_an_open_teams_rows_for_the_keys_in_reading_order() {
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, false);
    let mut working = agent(5, None);
    working.status = Status::Working;
    let list = |helpers: &[PaneId]| {
        let mut agents = vec![agent(1, None), agent(4, None), working.clone()];
        agents.extend(helpers.iter().map(|&p| agent(p, Some(1))));
        Agents::new(agents)
    };
    let agents = list(&[2, 3]);
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    let targets = |pane: &AgentsPane| pane.targets.iter().map(|(p, _)| *p).collect::<Vec<_>>();
    assert_eq!(targets(&pane), [5, 1, 4], "working first; idle: the team, then the single card");
    assert_eq!(pane.selected, Some(5), "the first card is highlighted");
    assert_eq!(pane.team_of, HashMap::from([(1, 1), (2, 1), (3, 1)]));

    pane.selected = Some(1);
    assert!(pane.key(egui::Key::Space, egui::Modifiers::NONE), "space opens the team");
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert_eq!(targets(&pane), [5, 1, 2, 3, 4], "its rows follow its card");
    pane.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    pane.key(egui::Key::ArrowDown, egui::Modifiers::NONE);
    assert_eq!(pane.selected, Some(3));
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert_eq!(pane.selected, Some(3), "a frame that changes nothing keeps the highlight");

    // The team is closed with 3 highlighted: the highlight goes up to its card.
    pane.expanded.clear();
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert_eq!((targets(&pane), pane.selected), (vec![5, 1, 4], Some(1)));

    // Open again, helper 3 highlighted, and then gone: the first card is highlighted.
    pane.expanded.insert(1, false);
    pane.selected = Some(3);
    let fewer = list(&[2]);
    frame(&ctx, &mut pane, &fewer, &mut time, vec![]);
    assert_eq!(targets(&pane), [5, 1, 2, 4]);
    assert_eq!(pane.selected, Some(5));

    // The team breaks up: it closes, and opens closed should it come back.
    frame(&ctx, &mut pane, &list(&[]), &mut time, vec![]);
    assert!(pane.expanded.is_empty() && pane.team_of.is_empty());
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert_eq!(targets(&pane), [5, 1, 4]);
}

/// The cards are made once per list and the layout once per list, width and set of open
/// teams: frames that change none of them reuse both, and each change makes them again.
#[test]
fn the_layout_is_kept_until_the_list_the_width_or_an_open_team_changes() {
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, false);
    let agents = Agents::new(vec![agent(1, None), agent(2, Some(1)), agent(3, None)]);
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    let laid = |pane: &AgentsPane| Arc::clone(pane.view.laid.as_ref().expect("painted"));
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    let first = laid(&pane);
    for _ in 0..3 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    assert!(Arc::ptr_eq(&first, &laid(&pane)), "nothing changed: the same layout");

    // A team opens: laid out again, the team spanning the row.
    pane.expanded.insert(1, false);
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    let open = laid(&pane);
    assert!(!Arc::ptr_eq(&first, &open));
    assert!(Arc::ptr_eq(&open.cards, &first.cards), "the cards themselves are kept");
    assert_eq!(open.layout.cards.iter().find(|(c, _)| c.key() == 1).unwrap().1.width(), 800.0 - 2.0 * SIDE);
    assert_eq!(pane.targets.iter().map(|(p, _)| *p).collect::<Vec<_>>(), [1, 2, 3]);

    // A narrower pane: laid out again.
    let theme = Theme::from_config(&crate::config::Config::default());
    time += 0.05;
    let input =
        egui::RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(400.0, 600.0))), time: Some(time), ..Default::default() };
    let _ = ctx.run(input, |ctx| {
        egui::CentralPanel::default().frame(egui::Frame::NONE).show(ctx, |ui| {
            let rect = ui.max_rect();
            let viewing = Viewing::default();
            pane.paint(
                ui,
                ui.painter(),
                rect,
                PaintArgs { pane: 1, focused: true, c: &theme, agents: &agents, notes: &[], shown_tab: None, viewing: &viewing },
            );
        });
    });
    let narrow = laid(&pane);
    assert!(!Arc::ptr_eq(&open, &narrow) && narrow.width == 400.0 - 2.0 * SIDE);

    // A new list (even of the same agents): new cards, laid out again.
    let again = Agents::new(vec![agent(1, None), agent(2, Some(1)), agent(3, None)]);
    frame(&ctx, &mut pane, &again, &mut time, vec![]);
    let relisted = laid(&pane);
    assert!(!Arc::ptr_eq(&relisted.cards, &narrow.cards));
    assert!(Arc::ptr_eq(&pane.view.cards[&1], &relisted.cards[0]), "the card drawn is the new one");
}

/// A card's words, worked out once with it, are what the text functions say: its header,
/// a closed team's capsules, each helper's tag, folder and parent, and how a pipeline stands.
#[test]
fn a_card_works_out_what_it_shows_once_as_the_text_says() {
    use crate::agents::view::card::{Card, Parent};
    let mut agents = vec![
        agent(1, None),
        helper(2, "build", Some("build"), 10),
        helper(3, "build-2", Some("build"), 20),
        helper(4, "rev", Some("review"), 30),
        sub(5, Some("Explore"), 40),
    ];
    let mut pipe = agent(6, Some(1));
    pipe.kind = AgentKind::Pipeline;
    pipe.name = "feature".into();
    let mut stage = agent(7, Some(6));
    stage.team.role = Some("build".into());
    stage.status = Status::Working;
    let mut done = agent(8, Some(6));
    done.team.role = Some("merge".into());
    done.status = Status::Done;
    agents.extend([pipe, stage, done]);
    agents[1].cwd = Some("/w/nex-build".into());
    let teams = groups(&Agents::new(agents));
    let g = &teams[0];
    let card = Card::new(Arc::clone(g));

    let direct = header_text(&g.lead, Some(&g.size()));
    assert_eq!(
        (card.header.title.as_str(), card.header.detail.as_str(), card.header.counts.is_none()),
        (direct.title.as_str(), direct.detail.as_str(), true)
    );
    let tags = member_tags(g);
    let siblings: Vec<&Agent> = g.members.iter().map(|m| &*m.agent).filter(|a| a.team.parent == Some(1)).collect();
    for (i, m) in g.members.iter().enumerate() {
        assert_eq!(card.members[i].tag, tags[i]);
        assert_eq!(card.members[i].detail, member_detail(&m.agent));
        assert_eq!(card.member_index(m.agent.pane), Some(i));
        if m.agent.team.parent == Some(1) {
            assert_eq!(tags[i], member_tag(&m.agent, &siblings), "a whole team tagged at once tags each as alone");
            assert_eq!(card.members[i].parent, Some(Parent::Lead));
        }
    }
    let at = |pane| card.member_index(pane).unwrap();
    assert_eq!(card.members[at(7)].parent, Some(Parent::Member(at(6))));
    assert_eq!(card.parent(at(7)).map(|a| a.pane), Some(6));
    assert_eq!(card.members[at(2)].detail, "nex-build");
    let words: Vec<&str> = card.labels.iter().map(String::as_str).collect();
    assert_eq!(words.len(), g.strip().len());
    assert!(words.contains(&"feature:") && words.contains(&"Build") && words.contains(&"Explore"));
    assert_eq!(card.branches, g.branches());
    assert_eq!(card.pipeline, None, "the lead is no pipeline");
    let nested: Vec<_> = card.nested.iter().map(|n| (n.member, n.stages.clone(), n.state)).collect();
    assert_eq!(nested, [(at(6), vec![at(8), at(7)], (0.5, Status::Done, true))], "its stages by status, the done one first");
    assert!(card.breathes, "a stage is at work");
    assert_eq!(card.show_all, "Show All 7");

    // A pipeline's own card: its counts after its name, and how its stages stand.
    let mut p = agent(10, None);
    p.kind = AgentKind::Pipeline;
    p.name = "feature".into();
    p.task = Some(nex_host::proto::pipeline_title("2 building", "build-2 started"));
    let stages = (11..13).map(|pane| Agent { status: Status::Done, ..agent(pane, Some(10)) });
    let teams = groups(&Agents::new(std::iter::once(p).chain(stages).collect()));
    let card = Card::new(Arc::clone(&teams[0]));
    assert_eq!(card.header.counts.as_deref(), Some(" \u{00b7} 2 building"));
    assert_eq!(card.pipeline, Some((1.0, Status::Done, false)));
    assert!(!card.breathes);
}

/// A held line too long for its room scrolls to its end and back, pausing at each; one that
/// fits never moves; with animations off a held line shows its end.
#[test]
fn a_held_line_scrolls_to_its_end_and_back_and_one_that_fits_stays_put() {
    use crate::agents::view::reveal::offset;
    for t in [0.0, 0.5, 1.0, 3.0, 10.0] {
        assert_eq!(offset(t, 0.0, true), 0.0, "fits, at {t}");
        assert_eq!(offset(t, -20.0, true), 0.0, "room to spare, at {t}");
    }
    assert_eq!(offset(0.3, 120.0, true), 0.0, "a pause before it moves");
    let mid = offset(1.6, 120.0, true);
    assert!(mid > 0.0 && mid < 120.0, "on its way: {mid}");
    // 120 points at 60 a second: at its end from 2.6 s, held there, then back.
    assert_eq!(offset(2.8, 120.0, true), 120.0);
    let back = offset(4.2, 120.0, true);
    assert!(back > 0.0 && back < 120.0, "on its way back: {back}");
    assert_eq!(offset(5.3, 120.0, true), 0.0, "home, and round again");
    assert_eq!(offset(0.0, 120.0, false), 120.0, "animations off: its end, still");
}

/// A finger on `pane` at `at` (`phase`), as egui-winit sends it: a touch with the pointer.
fn touch(ctx: &egui::Context, pane: &mut AgentsPane, agents: &Agents, time: &mut f64, phase: egui::TouchPhase, at: Pos2) {
    use egui::{Event, PointerButton};
    let mut ev = vec![Event::Touch { device_id: egui::TouchDeviceId(1), id: egui::TouchId(0), phase, pos: at, force: None }];
    match phase {
        egui::TouchPhase::Start => {
            ev.push(Event::PointerMoved(at));
            ev.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: true, modifiers: Default::default() });
        }
        egui::TouchPhase::Move => ev.push(Event::PointerMoved(at)),
        _ => {
            ev.push(Event::PointerButton { pos: at, button: PointerButton::Primary, pressed: false, modifiers: Default::default() });
            ev.push(Event::PointerGone);
        }
    }
    frame(ctx, pane, agents, time, ev);
}

/// Where card `key` is drawn in [`frame`]'s window, before any scrolling.
fn card_rect(agents: &Agents, expanded: &HashMap<PaneId, bool>, key: PaneId) -> Rect {
    let body = Rect::from_min_max(Pos2::new(0.0, 6.0), Pos2::new(800.0, 600.0));
    let teams = groups(agents);
    let laid = layout(&teams, body.width() - 2.0 * SIDE, expanded);
    laid.cards.iter().find(|(c, _)| c.key() == key).unwrap().1.translate(body.min.to_vec2())
}

fn reveal_ctx() -> egui::Context {
    let ctx = egui::Context::default();
    crate::render::fonts::install(&ctx, &crate::config::Config::default());
    crate::ui::glide::set_animations(&ctx, true);
    ctx
}

/// Holding a card whose task is cut off scrolls it into view for as long as the finger
/// stays; lifting it snaps the line back, opens nothing and scrolls nothing, even after the
/// finger wandered. A card whose words fit doesn't move while held, and a tap still opens.
#[test]
fn holding_a_card_scrolls_its_cut_off_task_until_the_finger_lifts() {
    let ctx = reveal_ctx();
    let mut long = agent(1, None);
    long.name = "Rework how the tracker reconciles remote panes with the transcripts they follow".into();
    // Enough cards that the list could scroll.
    let agents = Agents::new(std::iter::once(long).chain((2..30).map(|p| agent(p, None))).collect());
    let mut pane = AgentsPane::new();
    let mut time = 0.0;
    for _ in 0..3 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    let at = card_rect(&agents, &HashMap::new(), 1).left_center() + Vec2::new(80.0, 0.0);

    touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Start, at);
    for _ in 0..20 {
        touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Move, at);
    }
    let r = pane.view.reveal.expect("a hold past the click time reveals");
    assert_eq!((r.held, r.touch), (Held::Card(1), true));
    for _ in 0..30 {
        touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Move, at);
    }
    assert!(pane.view.reveal.unwrap().shifted > 0.0, "the cut-off task scrolls along");
    // The finger drifts up the list while it holds: the list stays where it is.
    for k in 1..=10 {
        touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Move, at - Vec2::new(0.0, 20.0 * k as f32));
    }
    assert!(pane.view.reveal.is_some(), "still held");
    touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::End, at - Vec2::new(0.0, 200.0));
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert_eq!(pane.view.reveal, None, "lifting the finger snaps it back");
    assert!(pane.requests.is_empty(), "and the hold opened nothing");
    assert_eq!(pane.scroll, 0.0, "nor scrolled the list");

    // A short name held: revealed, but nothing moves.
    let at2 = card_rect(&agents, &HashMap::new(), 2).left_center() + Vec2::new(80.0, 0.0);
    touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Start, at2);
    for _ in 0..50 {
        touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Move, at2);
    }
    let r = pane.view.reveal.expect("held");
    assert_eq!((r.held, r.shifted), (Held::Card(2), 0.0), "its name fits: it stays put");
    touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::End, at2);
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert!(pane.requests.is_empty() && pane.view.reveal.is_none());

    // A tap still opens the agent.
    touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::Start, at);
    touch(&ctx, &mut pane, &agents, &mut time, egui::TouchPhase::End, at);
    frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    assert_eq!(pane.requests, vec![super::super::Request::Focus(1)]);
    assert_eq!(pane.view.reveal, None);
}

/// On a desktop a right click on an open team's helper row reveals its cut-off task, until
/// the next press; it opens nothing, and a click after it opens the helper as ever.
#[test]
fn a_right_click_reveals_a_helper_rows_cut_off_task_until_the_next_press() {
    let ctx = reveal_ctx();
    let mut lead = agent(1, None);
    lead.kind = AgentKind::Pipeline;
    let mut build = helper(2, "build", Some("build"), 10);
    build.name = "Teach the agents view to scroll a held card's cut-off task and folder into view, then snap it back when the finger lifts off the glass again".into();
    build.task = Some(build.name.clone());
    let short = helper(3, "merge", Some("merge"), 20);
    let agents = Agents::new(vec![lead, build, short]);
    let mut pane = AgentsPane::new();
    pane.expanded.insert(1, false);
    let mut time = 0.0;
    for _ in 0..3 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    let open = card_rect(&agents, &pane.expanded, 1);
    let teams = groups(&agents);
    let index = |pane: PaneId| shown_members(&teams[0], false).iter().position(|m| m.agent.pane == pane).unwrap();
    let at = row_rect(open, index(2)).center();
    let right = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    for events in [vec![egui::Event::PointerMoved(at)], vec![right(true)], vec![right(false)]] {
        frame(&ctx, &mut pane, &agents, &mut time, events);
    }
    assert_eq!(pane.view.reveal.map(|r| (r.held, r.touch)), Some((Held::Row(2), false)));
    for _ in 0..30 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    assert!(pane.view.reveal.unwrap().shifted > 0.0, "its task scrolls along");
    assert!(pane.requests.is_empty(), "a right click opens nothing");

    // The short one's row fits: it stays put.
    let at3 = row_rect(open, index(3)).center();
    let right3 = |pressed| egui::Event::PointerButton { pos: at3, button: egui::PointerButton::Secondary, pressed, modifiers: egui::Modifiers::NONE };
    for events in [vec![egui::Event::PointerMoved(at3)], vec![right3(true)], vec![right3(false)]] {
        frame(&ctx, &mut pane, &agents, &mut time, events);
    }
    for _ in 0..30 {
        frame(&ctx, &mut pane, &agents, &mut time, vec![]);
    }
    assert_eq!(pane.view.reveal.map(|r| (r.held, r.shifted)), Some((Held::Row(3), 0.0)));

    // A click ends it, and opens the helper as ever.
    click(&ctx, &mut pane, &agents, &mut time, at);
    assert_eq!(pane.view.reveal, None);
    assert_eq!(pane.requests, vec![super::super::Request::Focus(2)]);
}
