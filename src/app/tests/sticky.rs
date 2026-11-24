//! The sticky pane: pinned into every tab (each keeping its own size for it), surviving the
//! tabs it outlives, and closing out of all of them at once.

use egui::{Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::app::actions::Action;
use crate::layout::{Axis, Node, PaneId};

struct Sim {
    ctx: egui::Context,
    app: App,
    t: f64,
}

impl Sim {
    fn new() -> Self {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let args = crate::cli::Args { no_restore: true, ..Default::default() };
        let app = App::new(&cc, crate::config::Config::default(), args, None, None, None);
        let mut s = Sim { ctx, app, t: 0.0 };
        s.frames(3);
        s
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.t += 0.016;
            let input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(self.t),
                ..Default::default()
            };
            let app = &mut self.app;
            let _ = self.ctx.run(input, |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()));
        }
    }

    fn act(&mut self, a: Action) {
        self.app.pending.push(a);
        self.frames(2);
    }

    fn focused(&self) -> PaneId {
        self.app.focused_pane().unwrap()
    }

    /// Every tab (terminal tabs, then Files) holds `id`.
    fn everywhere(&self, id: PaneId) -> bool {
        self.app.tabs.iter().chain(self.app.home_tab.iter()).all(|t| t.contains(id))
    }

    /// A tab split in two with its right pane pinned; returns `(left, sticky)`.
    fn pinned() -> (Self, PaneId, PaneId) {
        let mut s = Sim::new();
        let left = s.focused();
        s.act(Action::Split(Axis::Horizontal));
        let sticky = s.focused();
        s.act(Action::ToggleSticky(sticky));
        assert_eq!(s.app.sticky, Some(sticky));
        (s, left, sticky)
    }
}

#[test]
fn pinned_pane_joins_every_tab_on_its_side_with_its_own_size_in_each() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    s.act(Action::NewTab);
    s.act(Action::ToggleExplorer);
    assert_eq!(s.app.tabs.len(), 3);
    assert!(s.everywhere(sticky));
    // The right half where it was pinned, so the right half of the new tabs.
    let Node::Split { axis, ratio, b, .. } = &s.app.tabs[1].root else { panic!("not split") };
    assert_eq!(*axis, Axis::Horizontal);
    assert!((ratio - 0.5).abs() < 0.02, "ratio {ratio}");
    assert!(matches!(**b, Node::Leaf(id) if id == sticky));

    // Sizing it in one tab leaves the others alone.
    *s.app.tabs[1].root.ratio_mut(&[]).unwrap() = 0.8;
    s.act(Action::GotoTab(0));
    let r = |s: &Sim, i: usize| match s.app.tabs[i].root {
        Node::Split { ratio, .. } => ratio,
        _ => panic!("not split"),
    };
    assert!((r(&s, 0) - 0.5).abs() < 0.02 && r(&s, 1) == 0.8);

    // There's one terminal for it, sized for the tab shown.
    assert_eq!(s.app.panes.keys().filter(|&&id| id == sticky).count(), 1);
}

#[test]
fn a_tab_down_to_the_sticky_pane_closes_and_the_pane_lives_on() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    let other = s.app.tabs[1].root.panes().into_iter().find(|&id| id != sticky).unwrap();
    s.act(Action::ClosePane(other));
    assert_eq!(s.app.tabs.len(), 1);
    assert!(s.app.panes.contains_key(&sticky));
    assert_eq!(s.app.sticky, Some(sticky));
}

#[test]
fn closing_the_sticky_pane_takes_it_out_of_every_tab() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    s.act(Action::ClosePane(sticky));
    assert_eq!(s.app.sticky, None);
    assert_eq!(s.app.tabs.len(), 2);
    assert!(s.app.tabs.iter().all(|t| !t.contains(sticky)));
    assert!(!s.app.panes.contains_key(&sticky));
}

#[test]
fn unpinning_keeps_it_in_the_shown_tab_only() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    s.act(Action::ToggleSticky(sticky));
    assert_eq!(s.app.sticky, None);
    assert!(s.app.tabs[1].contains(sticky) && !s.app.tabs[0].contains(sticky));
}

#[test]
fn the_last_tab_down_to_it_keeps_it_as_a_plain_pane() {
    let (mut s, left, sticky) = Sim::pinned();
    s.act(Action::ClosePane(left));
    assert_eq!(s.app.sticky, None);
    assert_eq!(s.app.tabs.len(), 1);
    assert!(matches!(s.app.tabs[0].root, Node::Leaf(id) if id == sticky));
}

#[test]
fn it_survives_a_restart_as_one_pane() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    let snap = s.app.snapshot_session().unwrap();
    let ctx = egui::Context::default();
    let cc = eframe::CreationContext::_new_kittest(ctx.clone());
    let args = crate::cli::Args { no_restore: true, ..Default::default() };
    let app = App::new(&cc, crate::config::Config::default(), args, None, Some(snap), None);
    let mut r = Sim { ctx, app, t: 0.0 };
    r.frames(3);
    let pinned = r.app.sticky.expect("still sticky");
    assert_eq!(r.app.tabs.len(), 2);
    assert!(r.everywhere(pinned));
    // Two tabs of two panes each, three panes in all.
    assert_eq!(r.app.panes.len(), 3);
    let _ = sticky;
}

/// The header glyph `ch` that `cfg` draws in the focused pane's header, with Shift held or
/// not: its font and where its ink is.
fn header_glyph(cfg: crate::config::Config, shift: bool, ch: char) -> (egui::FontId, Rect) {
    let ctx = egui::Context::default();
    let mut app = super::new_app(&ctx, cfg);
    let mut shapes = Vec::new();
    let modifiers = if shift { egui::Modifiers::SHIFT } else { egui::Modifiers::NONE };
    for i in 0..3 {
        let input = RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
            time: Some(i as f64 * 0.016),
            modifiers,
            ..Default::default()
        };
        shapes = ctx.run(input, |ctx| eframe::App::update(&mut app, ctx, &mut eframe::Frame::_new_kittest())).shapes;
    }
    shapes
        .into_iter()
        .find_map(|s| match s.shape {
            egui::Shape::Text(t) if t.galley.text() == ch.to_string() => {
                Some((t.galley.job.sections[0].format.font_id.clone(), t.galley.mesh_bounds.translate(t.pos.to_vec2())))
            }
            _ => None,
        })
        .unwrap_or_else(|| panic!("no U+{:x} in the header", ch as u32))
}

/// Shift turns the × into a pin of the same height in the same place, from the same (embedded)
/// font on desktop and in mobile mode, where both are bigger with the bigger buttons.
#[test]
fn the_pin_is_drawn_like_the_close_button() {
    use crate::config::{Config, TabBar};
    let mut pins = Vec::new();
    for cfg in [Config::default(), Config { tab_bar: TabBar::Sidebar, ..Config::default() }] {
        let (pin_font, pin) = header_glyph(cfg.clone(), true, '\u{eba0}');
        let (close_font, close) = header_glyph(cfg, false, '\u{ea76}');
        assert_eq!(pin_font.family, egui::FontFamily::Name(crate::render::FONT_ICONS.into()));
        assert_eq!(close_font.family, pin_font.family);
        assert!((pin.height() - close.height()).abs() <= 1.0, "pin {pin:?}, close {close:?}");
        assert!((pin.center() - close.center()).length() <= 1.0, "pin {pin:?}, close {close:?}");
        pins.push(pin);
    }
    assert!(pins[1].height() > pins[0].height(), "mobile mode's pin is bigger: {pins:?}");
}

#[test]
fn the_shortcut_pins_and_unpins_the_focused_pane() {
    let mut s = Sim::new();
    s.act(Action::Split(Axis::Horizontal));
    let right = s.focused();
    s.act(Action::ToggleSticky(0));
    assert_eq!(s.app.sticky, Some(right));
    s.act(Action::NewTab);
    assert!(s.everywhere(right));
    // From another pane it moves the pin there; from the pinned pane it unpins.
    s.act(Action::ToggleSticky(0));
    let other = s.focused();
    assert_ne!(other, right);
    assert_eq!(s.app.sticky, Some(other));
    s.act(Action::ToggleSticky(0));
    assert_eq!(s.app.sticky, None);
}

/// The Agents panes in the Agents tab.
fn agents_in_home(s: &Sim) -> Vec<PaneId> {
    let home = s.app.home_tab.as_ref().expect("the Agents tab is open");
    home.root.panes().into_iter().filter(|id| s.app.agents.contains_key(id)).collect()
}

#[test]
fn the_agents_tab_opens_on_a_pinned_agents_pane_without_adding_another() {
    let mut s = Sim::new();
    s.act(Action::ToggleAgents);
    let pinned = s.focused();
    assert!(s.app.agents.contains_key(&pinned));
    s.act(Action::ToggleSticky(pinned));
    assert_eq!(s.app.sticky, Some(pinned));

    s.act(Action::ToggleAgentsTab);
    assert!(s.app.home_active);
    assert_eq!(agents_in_home(&s), vec![pinned]);
    assert_eq!(s.app.agents.len(), 1, "no second Agents pane was made");
}

#[test]
fn the_agents_tab_opens_on_a_new_agents_pane_when_none_is_pinned() {
    let (mut s, _, sticky) = Sim::pinned();
    assert!(!s.app.agents.contains_key(&sticky), "a terminal is pinned");
    s.act(Action::ToggleAgentsTab);
    assert!(s.app.home_active);
    let made = agents_in_home(&s);
    assert_eq!(made.len(), 1);
    assert!(s.everywhere(sticky), "the pinned terminal joins it");

    // And with nothing pinned at all.
    let mut s = Sim::new();
    s.act(Action::ToggleAgentsTab);
    assert_eq!(agents_in_home(&s).len(), 1);
}
