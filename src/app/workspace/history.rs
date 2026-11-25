//! Going back to the previous tab, like a browser's Back: the phone's Back button or gesture
//! (`platform::SYSTEM_BACK`). [`TabHistory`] keeps the places left, newest last, and is
//! platform-independent; the `App` side notes where the window is once a frame (so every
//! way of switching counts: a tap, a swipe, a shortcut, a new or closed tab) and turns the
//! Back key into Escape while something transient is open, else into [`App::go_back`].
//!
//! The home tab counts as a place of its own ([`Place::Home`]): the Agents (or Files) tab
//! the title bar or sidebar marks with the nex dog, the one the connect card's dog hands
//! over to. The connect screen before it isn't a place: nothing is open to go back to.

use egui::{Event, Key, RawInput};

use crate::app::App;
use crate::app::actions::Action;

/// The most places kept; older ones are forgotten.
const MAX_PLACES: usize = 64;

/// Where the window is: the home tab (the dog's Agents or Files tab) or a terminal tab, by uid.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::app) enum Place {
    Home,
    Tab(u64),
}

#[derive(Default)]
pub(in crate::app) struct TabHistory {
    /// Places left, the most recent last.
    back: Vec<Place>,
    /// Where the window was when last noted.
    current: Option<Place>,
}

impl TabHistory {
    /// Note where the window is now. Moving somewhere else records the place it left.
    pub(in crate::app) fn note(&mut self, now: Option<Place>) {
        if now == self.current {
            return;
        }
        if let Some(left) = self.current
            && self.back.last() != Some(&left)
        {
            self.back.push(left);
            if self.back.len() > MAX_PLACES {
                self.back.remove(0);
            }
        }
        self.current = now;
    }

    /// The place to go back to: the most recent one left that still `exists` and isn't
    /// where the window is. It becomes the current place without recording the one left,
    /// so going back again walks further back. Places passed over are dropped.
    pub(in crate::app) fn back(&mut self, exists: impl Fn(Place) -> bool) -> Option<Place> {
        while let Some(place) = self.back.pop() {
            if Some(place) != self.current && exists(place) {
                self.current = Some(place);
                return Some(place);
            }
        }
        None
    }
}

impl App {
    /// Where the window is (`None` on the connect screen, with nothing open).
    pub(in crate::app) fn place(&self) -> Option<Place> {
        if self.home_active && self.home_tab.is_some() {
            Some(Place::Home)
        } else {
            self.tabs.get(self.active).map(|t| Place::Tab(t.uid))
        }
    }

    /// Note where the window is, once a frame after everything that can switch tabs.
    pub(in crate::app) fn note_place(&mut self) {
        let now = self.place();
        self.history.note(now);
    }

    /// Back: show the tab shown before this one that's still open. Does nothing without one.
    pub(in crate::app) fn go_back(&mut self) {
        self.note_place();
        let (home, tabs) = (self.home_tab.is_some(), &self.tabs);
        let exists = |place| match place {
            Place::Home => home,
            Place::Tab(uid) => tabs.iter().any(|t| t.uid == uid),
        };
        match self.history.back(exists) {
            Some(Place::Home) => self.home_active = true,
            Some(Place::Tab(uid)) => {
                if let Some(i) = self.tabs.iter().position(|t| t.uid == uid) {
                    self.show_tab(i);
                }
            }
            None => {}
        }
    }

    /// Whether Escape would close something now rather than reach a pane: a pane being
    /// dragged, the search field, a dialog, a menu, or a pane's own editing or selection.
    pub(in crate::app) fn escape_closes(&mut self, ctx: &egui::Context) -> bool {
        if self.pane_drag.is_some() || self.search.is_some() || self.ssh_modal_open() || egui::Popup::is_any_open(ctx) {
            return true;
        }
        if self.explorers.values().any(|b| b.modal_open()) {
            return true;
        }
        let Some(id) = self.focused_pane() else { return false };
        self.explorers.get(&id).is_some_and(|b| b.escape_closes()) || self.diffs.get(&id).is_some_and(|d| d.escape_closes())
    }

    /// The system Back key (`Key::BrowserBack`, where `system_back` is on): while something
    /// transient is open it becomes Escape, which closes it; otherwise it is taken out of the
    /// input and goes back a tab. Run on the raw input, before the frame reads it.
    pub(in crate::app) fn system_back(&mut self, ctx: &egui::Context, raw_input: &mut RawInput) {
        if !self.system_back || !raw_input.events.iter().any(|e| matches!(e, Event::Key { key: Key::BrowserBack, .. })) {
            return;
        }
        let escape = self.escape_closes(ctx);
        let mut events = std::mem::take(&mut raw_input.events);
        events.retain_mut(|e| {
            let Event::Key { key, pressed, .. } = e else { return true };
            if *key != Key::BrowserBack {
                return true;
            }
            if escape {
                *key = Key::Escape;
                return true;
            }
            // On the connect screen there is nowhere to go back to.
            if *pressed && self.place().is_some() {
                self.pending.push(Action::GoBack);
            }
            false
        });
        raw_input.events = events;
    }
}

#[cfg(test)]
mod tests {
    use super::{Place, TabHistory};

    const A: Place = Place::Tab(1);
    const B: Place = Place::Tab(2);
    const C: Place = Place::Tab(3);

    fn visit(h: &mut TabHistory, places: &[Place]) {
        for &p in places {
            h.note(Some(p));
        }
    }

    #[test]
    fn back_walks_back_through_the_tabs_visited() {
        let mut h = TabHistory::default();
        visit(&mut h, &[A, B, C]);
        assert_eq!(h.back(|_| true), Some(B));
        h.note(Some(B));
        assert_eq!(h.back(|_| true), Some(A));
        h.note(Some(A));
        assert_eq!(h.back(|_| true), None);
    }

    #[test]
    fn closed_tabs_are_skipped() {
        let mut h = TabHistory::default();
        visit(&mut h, &[A, B, C]);
        assert_eq!(h.back(|p| p != B), Some(A));
    }

    #[test]
    fn the_home_tab_is_a_place() {
        let mut h = TabHistory::default();
        visit(&mut h, &[A, Place::Home, B]);
        assert_eq!(h.back(|_| true), Some(Place::Home));
        assert_eq!(h.back(|_| true), Some(A));
    }

    #[test]
    fn back_with_no_history_does_nothing() {
        let mut h = TabHistory::default();
        assert_eq!(h.back(|_| true), None);
        h.note(Some(A));
        assert_eq!(h.back(|_| true), None);
    }

    #[test]
    fn staying_put_records_nothing() {
        let mut h = TabHistory::default();
        visit(&mut h, &[A, A, B, B]);
        assert_eq!(h.back(|_| true), Some(A));
        assert_eq!(h.back(|_| true), None);
    }

    #[test]
    fn the_current_place_is_never_the_one_gone_back_to() {
        let mut h = TabHistory::default();
        // A → B → A: going back from A lands on B, not on A again.
        visit(&mut h, &[A, B, A]);
        assert_eq!(h.back(|_| true), Some(B));
        // B's earlier visit was to A, which is where history goes next.
        assert_eq!(h.back(|_| true), Some(A));
    }

    #[test]
    fn switching_after_going_back_records_the_place_reached() {
        let mut h = TabHistory::default();
        visit(&mut h, &[A, B, C]);
        assert_eq!(h.back(|_| true), Some(B));
        h.note(Some(C));
        assert_eq!(h.back(|_| true), Some(B));
        assert_eq!(h.back(|_| true), Some(A));
    }
}
