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
