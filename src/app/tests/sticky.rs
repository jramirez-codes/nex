//! Sticky panes: pinned into every tab (each keeping its own size for it), surviving the
//! tabs they outlive, and closing out of all of them at once. Several can be pinned.

use egui::{Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::app::actions::Action;
use crate::layout::{Axis, Node, PaneId, PaneKind};

struct Sim {
    ctx: egui::Context,
    app: App,
    t: f64,
}

impl Sim {
    fn new() -> Self {
        Self::with(crate::config::Config::default())
    }

    fn with(cfg: crate::config::Config) -> Self {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let args = crate::cli::Args { no_restore: true, ..Default::default() };
        let app = App::new(&cc, cfg, args, None, None, None);
        let mut s = Sim { ctx, app, t: 0.0 };
        s.frames(3);
        s
    }

    fn frames(&mut self, n: usize) {
        for _ in 0..n {
            self.t += 0.016;
            let input =
                RawInput { screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))), time: Some(self.t), ..Default::default() };
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
        assert_eq!(s.app.sticky, vec![sticky]);
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
    assert_eq!(s.app.panes.ids(PaneKind::Terminal).filter(|&id| id == sticky).count(), 1);
}

/// The split a pane pinned in `cfg` makes in another tab: pinning a tab's only pane (nothing
/// says where it goes), or with `split`, the second pane of a tab split that way.
fn pinned_split(cfg: crate::config::Config, split: Option<Axis>) -> (Axis, f32) {
    let mut s = Sim::with(cfg);
    s.act(Action::NewTab);
    s.act(Action::GotoTab(0));
    if let Some(axis) = split {
        s.act(Action::Split(axis));
    }
    let sticky = s.focused();
    s.act(Action::ToggleSticky(sticky));
    assert_eq!(s.app.sticky, vec![sticky]);
    let Node::Split { axis, ratio, b, .. } = &s.app.tabs[1].root else { panic!("not split") };
    assert!(matches!(**b, Node::Leaf(id) if id == sticky), "pinned on the right or bottom");
    (*axis, *ratio)
}

/// With nothing to go on, a pinned pane joins other tabs as a column on the right on desktop,
/// and as a band along the bottom on mobile. Where it already sits beats either default.
#[test]
fn a_pinned_pane_defaults_beside_on_desktop_and_below_on_mobile() {
    use crate::config::{Config, TabBar};
    let mobile = || Config { tab_bar: TabBar::Sidebar, ..Config::default() };
    let (axis, ratio) = pinned_split(Config::default(), None);
    assert_eq!(axis, Axis::Horizontal, "desktop: side by side");
    assert!((ratio - 0.6).abs() < 0.01, "ratio {ratio}");
    let (axis, ratio) = pinned_split(mobile(), None);
    assert_eq!(axis, Axis::Vertical, "mobile: stacked");
    assert!((ratio - 0.6).abs() < 0.01, "ratio {ratio}");
    // The user split the tab themselves: the pin keeps that layout, on mobile too.
    assert_eq!(pinned_split(mobile(), Some(Axis::Horizontal)).0, Axis::Horizontal);
    assert_eq!(pinned_split(Config::default(), Some(Axis::Vertical)).0, Axis::Vertical);
}

#[test]
fn a_tab_down_to_the_sticky_pane_closes_and_the_pane_lives_on() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    let other = s.app.tabs[1].root.panes().into_iter().find(|&id| id != sticky).unwrap();
    s.act(Action::ClosePane(other));
    assert_eq!(s.app.tabs.len(), 1);
    assert!(s.app.panes.is_terminal(sticky));
    assert_eq!(s.app.sticky, vec![sticky]);
}

#[test]
fn closing_the_sticky_pane_takes_it_out_of_every_tab() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    s.act(Action::ClosePane(sticky));
    assert!(s.app.sticky.is_empty());
    assert_eq!(s.app.tabs.len(), 2);
    assert!(s.app.tabs.iter().all(|t| !t.contains(sticky)));
    assert!(!s.app.panes.is_terminal(sticky));
}

#[test]
fn unpinning_keeps_it_in_the_shown_tab_only() {
    let (mut s, _, sticky) = Sim::pinned();
    s.act(Action::NewTab);
    s.act(Action::ToggleSticky(sticky));
    assert!(s.app.sticky.is_empty());
    assert!(s.app.tabs[1].contains(sticky) && !s.app.tabs[0].contains(sticky));
}

#[test]
fn the_last_tab_down_to_it_keeps_it_as_a_plain_pane() {
    let (mut s, left, sticky) = Sim::pinned();
    s.act(Action::ClosePane(left));
    assert!(s.app.sticky.is_empty());
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
    let [pinned] = r.app.sticky[..] else { panic!("one pinned pane, not {:?}", r.app.sticky) };
    assert_eq!(r.app.tabs.len(), 2);
    assert!(r.everywhere(pinned));
    // Two tabs of two panes each, three panes in all.
    assert_eq!(r.app.panes.ids(PaneKind::Terminal).count(), 3);
    let _ = sticky;
}

impl Sim {
    /// `snap` opened in a fresh window.
    fn restored(snap: crate::session::Session) -> Self {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let args = crate::cli::Args { no_restore: true, ..Default::default() };
        let app = App::new(&cc, crate::config::Config::default(), args, None, Some(snap), None);
        let mut r = Sim { ctx, app, t: 0.0 };
        r.frames(3);
        r
    }

    /// A tab of three panes side by side, its middle and right ones pinned; returns
    /// `(left, [middle, right])`.
    fn two_pinned() -> (Self, PaneId, [PaneId; 2]) {
        let mut s = Sim::new();
        let left = s.focused();
        s.act(Action::Split(Axis::Horizontal));
        let middle = s.focused();
        s.act(Action::Split(Axis::Horizontal));
        let right = s.focused();
        s.act(Action::ToggleSticky(middle));
        s.act(Action::ToggleSticky(right));
        assert_eq!(s.app.sticky, vec![middle, right]);
        (s, left, [middle, right])
    }

    /// Where pane `id` is drawn in the shown tab.
    fn rect_of(&self, id: PaneId) -> Rect {
        let tab = self.app.cur_tab().unwrap();
        let mut rects = Vec::new();
        tab.root.layout(self.app.content_area, crate::app::PANE_GAP, &mut rects, &mut Vec::new());
        rects.into_iter().find(|(p, _)| *p == id).map(|(_, r)| r).expect("in the shown tab")
    }
}

#[test]
fn several_panes_pin_at_once_and_each_joins_every_tab_in_its_place() {
    let (mut s, left, [middle, right]) = Sim::two_pinned();
    let (m0, r0) = (s.rect_of(middle), s.rect_of(right));
    s.act(Action::NewTab);
    s.act(Action::ToggleExplorer);
    assert!(s.everywhere(middle) && s.everywhere(right));
    assert!(!s.app.tabs[1].contains(left));
    // In the new tab both stay on the right, the right one rightmost and as wide as it was,
    // the middle one roughly (it is placed by its share of the window, around the new pane).
    s.act(Action::GotoTab(1));
    let (m1, r1) = (s.rect_of(middle), s.rect_of(right));
    assert!(m1.max.x <= r1.min.x + 1.0 && m1.min.x > s.app.content_area.center().x - 100.0, "middle {m1:?} right {r1:?}");
    assert!((r1.width() - r0.width()).abs() < 10.0 && (m1.width() - m0.width()).abs() < 100.0, "{m0:?} {r0:?} / {m1:?} {r1:?}");
    // One terminal each, however many tabs list them.
    let terms: Vec<PaneId> = s.app.panes.ids(PaneKind::Terminal).collect();
    assert_eq!(terms.iter().filter(|&&id| id == middle || id == right).count(), 2);

    // Sizing one in one tab leaves the other tab alone.
    let ratio = |s: &Sim, i: usize| match s.app.tabs[i].root {
        Node::Split { ratio, .. } => ratio,
        _ => panic!("not split"),
    };
    let before = ratio(&s, 0);
    *s.app.tabs[1].root.ratio_mut(&[]).unwrap() = 0.3;
    s.frames(2);
    assert_eq!(ratio(&s, 0), before);
}

#[test]
fn unpinning_one_leaves_the_others_pinned() {
    let (mut s, _, [middle, right]) = Sim::two_pinned();
    s.act(Action::NewTab);
    s.act(Action::ToggleSticky(middle));
    assert_eq!(s.app.sticky, vec![right]);
    assert!(s.app.tabs[1].contains(middle) && !s.app.tabs[0].contains(middle), "kept in the shown tab only");
    assert!(s.everywhere(right));
    // A tab opened now gets only the one still pinned.
    s.act(Action::NewTab);
    assert!(s.app.tabs[2].contains(right) && !s.app.tabs[2].contains(middle));
}

#[test]
fn closing_one_pinned_pane_leaves_the_others() {
    let (mut s, _, [middle, right]) = Sim::two_pinned();
    s.act(Action::NewTab);
    s.act(Action::ClosePane(right));
    assert_eq!(s.app.sticky, vec![middle]);
    assert_eq!(s.app.tabs.len(), 2);
    assert!(s.app.tabs.iter().all(|t| !t.contains(right)));
    assert!(s.everywhere(middle));
}

#[test]
fn a_tab_down_to_pinned_panes_closes_unless_it_is_the_last() {
    let (mut s, left, [middle, right]) = Sim::two_pinned();
    s.act(Action::NewTab);
    let other = s.app.tabs[1].root.panes().into_iter().find(|&id| !s.app.is_sticky(id)).unwrap();
    s.act(Action::ClosePane(other));
    assert_eq!(s.app.tabs.len(), 1);
    assert_eq!(s.app.sticky, vec![middle, right]);
    // The last tab down to them keeps them as plain panes.
    s.act(Action::ClosePane(left));
    assert!(s.app.sticky.is_empty());
    assert_eq!(s.app.tabs.len(), 1);
    assert_eq!(s.app.tabs[0].root.panes().len(), 2);
}

#[test]
fn every_pinned_card_glides_on_a_tab_switch() {
    let (mut s, _, [middle, right]) = Sim::two_pinned();
    s.act(Action::NewTab);
    s.act(Action::GotoTab(0));
    let mut glides: Vec<PaneId> = s.app.sticky_glide.keys().copied().collect();
    glides.sort();
    let mut want = vec![middle, right];
    want.sort();
    assert_eq!(glides, want);
    // Unpinned, its glide goes.
    s.act(Action::ToggleSticky(middle));
    assert_eq!(s.app.sticky_glide.keys().copied().collect::<Vec<_>>(), vec![right]);
}

#[test]
fn several_pinned_panes_survive_a_restart_each_as_one_pane() {
    let (mut s, _, _) = Sim::two_pinned();
    s.act(Action::NewTab);
    let snap = s.app.snapshot_session().unwrap();
    let json = snap.to_json();
    assert!(json.contains("\"pin\": 1"), "the second pin is numbered: {json}");
    let mut r = Sim::restored(crate::session::Session::from_json(&json).unwrap());
    assert_eq!(r.app.sticky.len(), 2);
    assert_eq!(r.app.tabs.len(), 2);
    assert!(r.app.sticky.iter().all(|&id| r.everywhere(id)));
    // Three panes in the first tab, one more in the second: four in all.
    assert_eq!(r.app.panes.ids(PaneKind::Terminal).count(), 4);
    // Pinned in the same order: unpinning the first leaves the second.
    let first = r.app.sticky[0];
    r.act(Action::ToggleSticky(first));
    assert_eq!(r.app.sticky.len(), 1);
}

#[test]
fn an_old_session_with_one_sticky_pane_loads_as_before() {
    // As written before several pins: `"sticky": true` and no `"pin"`.
    let leaf = |cwd: &str, sticky: bool| {
        let mut v = serde_json::json!({ "kind": "terminal", "cwd": cwd });
        if sticky {
            v["sticky"] = true.into();
        }
        v
    };
    let tab = |cwd: &str| serde_json::json!({ "root": { "axis": "horizontal", "ratio": 0.6, "a": leaf(cwd, false), "b": leaf("/tmp", true) } });
    let json = serde_json::json!({ "version": 1, "tabs": [tab("/"), tab("/")], "active": 0 }).to_string();
    let r = Sim::restored(crate::session::Session::from_json(&json).unwrap());
    let [pinned] = r.app.sticky[..] else { panic!("one pinned pane, not {:?}", r.app.sticky) };
    assert!(r.everywhere(pinned));
    assert_eq!(r.app.panes.ids(PaneKind::Terminal).count(), 3);
    // And it saves the same way again.
    let again = r.app.snapshot_session().unwrap().to_json();
    assert_eq!(again.matches("\"sticky\"").count(), 2);
    assert!(!again.contains("\"pin\""));
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
    assert_eq!(s.app.sticky, vec![right]);
    s.act(Action::NewTab);
    assert!(s.everywhere(right));
    // From another pane it pins that one too; from a pinned pane it unpins just that one.
    s.act(Action::ToggleSticky(0));
    let other = s.focused();
    assert_ne!(other, right);
    assert_eq!(s.app.sticky, vec![right, other]);
    s.act(Action::ToggleSticky(0));
    assert_eq!(s.app.sticky, vec![right]);
}

/// The Agents panes in the Agents tab.
fn agents_in_home(s: &Sim) -> Vec<PaneId> {
    let home = s.app.home_tab.as_ref().expect("the Agents tab is open");
    home.root.panes().into_iter().filter(|id| s.app.panes.agents(*id).is_some()).collect()
}

#[test]
fn the_agents_tab_opens_on_a_pinned_agents_pane_without_adding_another() {
    let mut s = Sim::new();
    s.act(Action::ToggleAgents);
    let pinned = s.focused();
    assert!(s.app.panes.agents(pinned).is_some());
    s.act(Action::ToggleSticky(pinned));
    assert_eq!(s.app.sticky, vec![pinned]);

    s.act(Action::ToggleAgentsTab);
    assert!(s.app.home_active);
    assert_eq!(agents_in_home(&s), vec![pinned]);
    assert_eq!(s.app.panes.ids(PaneKind::Agents).count(), 1, "no second Agents pane was made");
}

#[test]
fn the_agents_tab_opens_on_a_new_agents_pane_when_none_is_pinned() {
    let (mut s, _, sticky) = Sim::pinned();
    assert!(s.app.panes.agents(sticky).is_none(), "a terminal is pinned");
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
