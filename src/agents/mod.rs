//! The Agents pane: every coding agent running in nex at a glance. One card per Claude Code
//! or Codex pane, grouped by what it's doing (needs you, done, working, idle), with how long it
//! has been at it. Clicking a card (or Enter) jumps to the agent's pane. An agent on an ssh
//! host that started others, or a pipeline, is one card for its whole team: its helpers show
//! as capsules (a pipeline's as steps), and open into a list inside the card, the ones that
//! need you first (a helper changing state glides to its new place). There each helper is a
//! row: its tag (`Build-01`: what it's for and which one) and the task its
//! terminal's title says it's on, live, over its folder. A pipeline's card says how its run
//! stands (`Feature · 2 building · 1 in review`) over the last thing that happened, from the
//! title `nex-host pipe` gives its terminal.
//!
//! `model` (which panes are agents and what they're doing), `group` (teams), `motion` (how
//! cards, rows and capsules move), `view` (drawing and the mouse), and here: the pane's state
//! and keys. The app keeps one [`model::Tracker`] for every Agents pane, updated with the
//! process poll. Its agents are shared (`Arc`) down to the cards, and the teams made from
//! them are kept until the list changes: a frame copies none of them.

mod group;
pub mod model;
mod motion;
mod view;
#[cfg(test)]
mod tests;

use std::collections::HashMap;

use egui::{Key, Modifiers, Rect};

use crate::layout::PaneId;


/// What the pane asks the app for (it can't move focus itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Show this agent's pane: switch to its tab and focus it (for an agent on an ssh host
    /// that no pane shows, open one on it).
    Focus(PaneId),
    /// `Delete` on this agent: for one on an ssh host, stop it (after asking).
    Dismiss(PaneId),
}

pub struct AgentsPane {
    /// The highlighted card's pane.
    pub selected: Option<PaneId>,
    /// What the pane asked for while painting, drained by the app.
    pub requests: Vec<Request>,
    /// Where the pane was opened from, for focus to return to when it closes.
    pub origin: Option<PaneId>,
    scroll: f32,
    /// What the arrow keys move through, as drawn last frame in reading order: each card,
    /// and an open team's helpers after it (content coordinates).
    targets: Vec<(PaneId, Rect)>,
    /// The open teams, by lead, and whether each shows all its helpers.
    expanded: HashMap<PaneId, bool>,
    /// Each team member's (and lead's) lead, as of last frame.
    team_of: HashMap<PaneId, PaneId>,
    view: view::ViewState,
}

impl Default for AgentsPane {
    fn default() -> Self {
        Self::new()
    }
}

impl AgentsPane {
    pub fn new() -> AgentsPane {
        AgentsPane {
            selected: None,
            requests: Vec::new(),
            origin: None,
            scroll: 0.0,
            targets: Vec::new(),
            expanded: HashMap::new(),
            team_of: HashMap::new(),
            view: view::ViewState::new(),
        }
    }

    pub fn title(&self) -> String {
        "Agents".into()
    }

    /// Arrow keys (and j / k / h / l) move the highlight, Enter jumps to the agent, Space
    /// opens or closes a team (anything else: jumps to it), Delete stops one on an ssh host
    /// (after asking).
    /// Returns whether the key was used.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if modifiers.command || modifiers.alt || modifiers.ctrl {
            return false;
        }
        let selected = self.selected.filter(|p| self.targets.iter().any(|(t, _)| t == p));
        match key {
            Key::ArrowDown | Key::J => self.move_vertically(true),
            Key::ArrowUp | Key::K => self.move_vertically(false),
            Key::ArrowRight | Key::L => self.move_by(1),
            Key::ArrowLeft | Key::H => self.move_by(-1),
            Key::Home => self.move_by(isize::MIN),
            Key::End => self.move_by(isize::MAX),
            Key::Space if selected.is_some_and(|p| self.team_of.contains_key(&p)) => {
                if let Some(lead) = selected.and_then(|p| self.team_of.get(&p).copied()) {
                    self.toggle_team(lead);
                }
            }
            Key::Enter | Key::Space => {
                if let Some(pane) = selected {
                    self.requests.push(Request::Focus(pane));
                }
            }
            Key::Delete | Key::Backspace => {
                if let Some(pane) = selected {
                    self.requests.push(Request::Dismiss(pane));
                }
            }
            _ => return false,
        }
        true
    }

    /// Open or close the team `lead` leads. Closing it moves the highlight from a helper up
    /// to the team's card.
    fn toggle_team(&mut self, lead: PaneId) {
        if self.expanded.remove(&lead).is_none() {
            self.expanded.insert(lead, false);
        } else if self.selected.is_some_and(|s| self.team_of.get(&s) == Some(&lead)) {
            self.selected = Some(lead);
        }
        self.view.reveal_selection = true;
    }

    /// Next or previous in reading order (`isize::MIN` / `MAX`: first or last).
    fn move_by(&mut self, step: isize) {
        if self.targets.is_empty() {
            return;
        }
        let last = self.targets.len() as isize - 1;
        let at = self.selected.and_then(|p| self.targets.iter().position(|(o, _)| *o == p));
        let next = match (at, step) {
            (_, isize::MIN) => 0,
            (_, isize::MAX) => last,
            (None, s) if s < 0 => last,
            (None, _) => 0,
            (Some(i), s) => (i as isize + s).clamp(0, last),
        };
        self.selected = Some(self.targets[next as usize].0);
        self.view.reveal_selection = true;
    }

    /// The nearest card or helper row below (or above) the highlighted one, the closest to
    /// it sideways among those: cards on a grid, and full-width teams, alike.
    fn move_vertically(&mut self, down: bool) {
        let Some(&(_, cur)) = self.selected.and_then(|p| self.targets.iter().find(|(o, _)| *o == p)) else {
            return self.move_by(if down { 1 } else { -1 });
        };
        let beyond = |r: &Rect| if down { r.top() >= cur.bottom() - 0.5 } else { r.bottom() <= cur.top() + 0.5 };
        let row_edge = |r: &Rect| if down { r.top() } else { -r.bottom() };
        let candidates: Vec<&(PaneId, Rect)> = self.targets.iter().filter(|(_, r)| beyond(r)).collect();
        let Some(nearest) = candidates.iter().map(|(_, r)| row_edge(r)).min_by(f32::total_cmp) else { return };
        let best = candidates
            .iter()
            .filter(|(_, r)| (row_edge(r) - nearest).abs() < 1.0)
            .min_by(|(_, a), (_, b)| (a.center().x - cur.center().x).abs().total_cmp(&(b.center().x - cur.center().x).abs()));
        if let Some((p, _)) = best {
            self.selected = Some(*p);
            self.view.reveal_selection = true;
        }
    }
}
