//! The phone's Back (`Key::BrowserBack`, see `workspace/history.rs`): it walks back through
//! the tabs shown, the home (dog) tab among them, skipping closed ones, and closes what's
//! open first, as Escape does.

use egui::{Event, Key, Modifiers, Pos2, RawInput, Rect, Vec2};

use crate::app::App;
use crate::app::actions::Action;
use crate::app::workspace::history::Place;

struct Sim {
    ctx: egui::Context,
    app: App,
    t: f64,
}

impl Sim {
    /// A window with `tabs` terminal tabs, opened one after another, the last one showing.
    fn new(tabs: usize) -> Self {
        let ctx = egui::Context::default();
        let cc = eframe::CreationContext::_new_kittest(ctx.clone());
        let args = crate::cli::Args { no_restore: true, ..Default::default() };
        let mut app = App::new(&cc, crate::config::Config::default(), args, None, None, None);
        app.system_back = true;
        let mut s = Sim { ctx, app, t: 0.0 };
        s.frames(3, Vec::new());
        for _ in 1..tabs {
            s.act(Action::NewTab);
        }
        assert_eq!(s.app.tabs.len(), tabs);
        s
    }

    /// `n` frames, the first with `events`, each through `raw_input_hook` as eframe does.
    /// Returns the events the first frame's hook left for egui.
    fn frames(&mut self, n: usize, mut events: Vec<Event>) -> Vec<Event> {
        let mut left = Vec::new();
        for i in 0..n {
            self.t += 0.016;
            let mut input = RawInput {
                screen_rect: Some(Rect::from_min_size(Pos2::ZERO, Vec2::new(1200.0, 700.0))),
                time: Some(self.t),
                events: std::mem::take(&mut events),
                ..Default::default()
            };
            eframe::App::raw_input_hook(&mut self.app, &self.ctx, &mut input);
            if i == 0 {
                left = input.events.clone();
            }
            let app = &mut self.app;
            let _ = self.ctx.run(input, |ctx| eframe::App::update(app, ctx, &mut eframe::Frame::_new_kittest()));
        }
        left
    }

    fn act(&mut self, a: Action) {
        self.app.pending.push(a);
        self.frames(2, Vec::new());
    }

    /// Press and release Back; returns the key events egui was left with.
    fn back(&mut self) -> Vec<Key> {
        let key = |pressed| Event::Key { key: Key::BrowserBack, physical_key: None, pressed, repeat: false, modifiers: Modifiers::NONE };
        let left = self.frames(2, vec![key(true), key(false)]);
        left.into_iter().filter_map(|e| if let Event::Key { key, .. } = e { Some(key) } else { None }).collect()
    }

    fn uid(&self, i: usize) -> Place {
        Place::Tab(self.app.tabs[i].uid)
    }

    fn place(&self) -> Option<Place> {
        self.app.place()
    }
}

#[test]
fn back_walks_back_through_the_tabs() {
    let mut s = Sim::new(3);
    let (a, b, c) = (s.uid(0), s.uid(1), s.uid(2));
    assert_eq!(s.place(), Some(c));
    assert!(s.back().is_empty(), "Back is taken out of the input");
    assert_eq!(s.place(), Some(b));
    s.back();
    assert_eq!(s.place(), Some(a));
    s.back();
    assert_eq!(s.place(), Some(a), "nothing further back");
}

#[test]
fn back_follows_the_order_tabs_were_shown_in() {
    let mut s = Sim::new(3);
    let (a, c) = (s.uid(0), s.uid(2));
    s.act(Action::GotoTab(0));
    s.act(Action::PrevTab);
    assert_eq!(s.place(), Some(c));
    s.back();
    assert_eq!(s.place(), Some(a));
    s.back();
    assert_eq!(s.place(), Some(c));
}

#[test]
fn closed_tabs_are_skipped() {
    let mut s = Sim::new(3);
    let (a, c) = (s.uid(0), s.uid(2));
    s.act(Action::CloseTab(1));
    assert_eq!(s.place(), Some(c));
    s.back();
    assert_eq!(s.place(), Some(a));
}

#[test]
fn the_dog_tab_is_one_of_the_places() {
    let mut s = Sim::new(2);
    let (a, b) = (s.uid(0), s.uid(1));
    s.act(Action::GotoTab(0));
    // The home tab, which the title bar marks with the dog.
    s.act(Action::ToggleAgentsTab);
    assert_eq!(s.place(), Some(Place::Home));
    s.act(Action::GotoTab(1));
    s.back();
    assert_eq!(s.place(), Some(Place::Home));
    s.back();
    assert_eq!(s.place(), Some(a));
    s.back();
    assert_eq!(s.place(), Some(b), "leaving the dog tab was recorded too");
}

#[test]
fn a_closed_dog_tab_is_skipped() {
    let mut s = Sim::new(2);
    let a = s.uid(0);
    s.act(Action::GotoTab(0));
    s.act(Action::ToggleExplorer);
    assert_eq!(s.place(), Some(Place::Home));
    s.act(Action::GotoTab(1));
    s.act(Action::CloseExplorer);
    s.back();
    assert_eq!(s.place(), Some(a));
}

#[test]
fn back_with_no_history_does_nothing() {
    let mut s = Sim::new(1);
    let a = s.uid(0);
    assert!(s.back().is_empty());
    assert_eq!(s.place(), Some(a));
    assert_eq!(s.app.tabs.len(), 1);
}

#[test]
fn back_closes_the_search_first() {
    let mut s = Sim::new(2);
    let (a, b) = (s.uid(0), s.uid(1));
    s.act(Action::ToggleSearch);
    assert!(s.app.input.search.is_some());
    assert_eq!(s.back(), vec![Key::Escape, Key::Escape], "Back arrives as Escape");
    assert!(s.app.input.search.is_none());
    assert_eq!(s.place(), Some(b));
    s.back();
    assert_eq!(s.place(), Some(a));
}

#[test]
fn back_closes_an_open_menu_first() {
    let mut s = Sim::new(2);
    let b = s.uid(1);
    let id = egui::Id::new("some menu");
    egui::Popup::open_id(&s.ctx, id);
    assert_eq!(s.back(), vec![Key::Escape, Key::Escape]);
    assert_eq!(s.place(), Some(b));
}

#[test]
fn without_a_system_back_the_key_is_left_alone() {
    let mut s = Sim::new(2);
    let b = s.uid(1);
    s.app.system_back = false;
    assert_eq!(s.back(), vec![Key::BrowserBack, Key::BrowserBack]);
    assert_eq!(s.place(), Some(b));
}
