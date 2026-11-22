//! The Agents pane: every coding agent running in nex at a glance. One card per Claude Code
//! or Codex pane, grouped by what it's doing (needs you, working, idle), with how long it
//! has been at it. Clicking a card (or Enter) jumps to the agent's pane.
//!
//! `model` (which panes are agents and what they're doing), `view` (drawing and the mouse),
//! and here: the pane's state and keys. The app keeps one [`model::Tracker`] for every
//! Agents pane, updated with the process poll.

pub mod model;
mod motion;
mod view;
#[cfg(test)]
mod tests;

use egui::{Key, Modifiers};

use crate::layout::PaneId;


/// What the pane asks the app for (it can't move focus itself).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Request {
    /// Show this agent's pane: switch to its tab and focus it.
    Focus(PaneId),
}

pub struct AgentsPane {
    /// The highlighted card's pane.
    pub selected: Option<PaneId>,
    /// What the pane asked for while painting, drained by the app.
    pub requests: Vec<Request>,
    /// Where the pane was opened from, for focus to return to when it closes.
    pub origin: Option<PaneId>,
    scroll: f32,
    /// The cards drawn last frame, in reading order, and how many to a row: what the arrow
    /// keys move through.
    order: Vec<PaneId>,
    columns: usize,
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
            order: Vec::new(),
            columns: 1,
            view: view::ViewState::new(),
        }
    }

    pub fn title(&self) -> String {
        "Agents".into()
    }

    /// Arrow keys (and j / k / h / l) move the highlight, Enter or Space jumps to the agent.
    /// Returns whether the key was used.
    pub fn key(&mut self, key: Key, modifiers: Modifiers) -> bool {
        if modifiers.command || modifiers.alt || modifiers.ctrl {
            return false;
        }
        let step: isize = match key {
            Key::ArrowDown | Key::J => self.columns.max(1) as isize,
            Key::ArrowUp | Key::K => -(self.columns.max(1) as isize),
            Key::ArrowRight | Key::L => 1,
            Key::ArrowLeft | Key::H => -1,
            Key::Home => isize::MIN,
            Key::End => isize::MAX,
            Key::Enter | Key::Space => {
                if let Some(pane) = self.selected.filter(|p| self.order.contains(p)) {
                    self.requests.push(Request::Focus(pane));
                }
                return true;
            }
            _ => return false,
        };
        self.move_selection(step);
        true
    }

    fn move_selection(&mut self, step: isize) {
        if self.order.is_empty() {
            return;
        }
        let last = self.order.len() as isize - 1;
        let at = self.selected.and_then(|p| self.order.iter().position(|&o| o == p));
        let next = match (at, step) {
            (_, isize::MIN) => 0,
            (_, isize::MAX) => last,
            (None, s) if s < 0 => last,
            (None, _) => 0,
            (Some(i), s) => (i as isize + s).clamp(0, last),
        };
        self.selected = Some(self.order[next as usize]);
        self.view.reveal_selection = true;
    }
}
