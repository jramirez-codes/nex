//! A tab: a split tree of panes plus its header's pane-icon deck.

use crate::layout::{Node, PaneId};
use crate::ui::glide::Glide;

use crate::app::motion::PaneMotion;

pub(in crate::app) struct Tab {
    /// Stable identity (survives reordering) for egui widget ids.
    pub(in crate::app) uid: u64,
    pub(in crate::app) root: Node,
    pub(in crate::app) focused: PaneId,
    pub(in crate::app) zoomed: bool,
    /// Split / close transitions and focus fades of this tab's panes.
    pub(in crate::app) motion: PaneMotion,
    /// Pane-icon deck in the tab header: rotation (in cards) of the stack, plus the focus and
    /// pane count it was last drawn for, to tell focus moves (animate) from splits (snap).
    deck: Glide,
    deck_focus: PaneId,
    deck_len: usize,
}

impl Tab {
    pub(in crate::app) fn new(uid: u64, pane: PaneId) -> Self {
        Self { uid, root: Node::Leaf(pane), focused: pane, zoomed: false, motion: PaneMotion::default(), deck: Glide::new(0.28), deck_focus: pane, deck_len: 1 }
    }

    pub(in crate::app) fn contains(&self, pane: PaneId) -> bool {
        self.root.panes().contains(&pane)
    }

    /// Take `pane` out of the split tree, moving focus off it if needed. Returns `false` if it
    /// was the last pane (the tab itself should close).
    pub(in crate::app) fn remove_pane(&mut self, pane: PaneId) -> bool {
        if !self.root.remove(pane) {
            return false;
        }
        if self.focused == pane {
            self.focused = self.root.panes()[0];
        }
        self.zoomed = false;
        true
    }

    /// `(process, depth)` for every pane's card in the header deck, 0 being the top.
    /// Focus moves rotate the deck the short way round, flipping through the cards between.
    pub(in crate::app) fn deck_cards<T>(&mut self, ctx: &egui::Context, process_of: impl Fn(PaneId) -> T) -> Vec<(T, f32)> {
        let ids = self.root.panes();
        let n = ids.len();
        let Some(idx) = ids.iter().position(|&p| p == self.focused) else { return Vec::new() };
        let prev = ids.iter().position(|&p| p == self.deck_focus);
        match prev {
            Some(prev) if n == self.deck_len => {
                if prev != idx {
                    // Shortest signed step in (-n/2, n/2].
                    let mut k = (idx as i64 - prev as i64).rem_euclid(n as i64);
                    if k > n as i64 / 2 {
                        k -= n as i64;
                    }
                    let target = self.deck.to + k as f32;
                    self.deck.value(ctx, target);
                }
            }
            _ => self.deck.snap(ctx, idx as f32),
        }
        self.deck_focus = self.focused;
        self.deck_len = n;
        let target = self.deck.to;
        let d = self.deck.value(ctx, target);
        ids.iter()
            .enumerate()
            .map(|(j, id)| {
                (process_of(*id), (j as f32 - d).rem_euclid(n as f32))
            })
            .collect()
    }
}
