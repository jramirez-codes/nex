//! The sticky pane: one pane that follows the user from tab to tab (Shift + the header's ×
//! turns it into a pin; Ctrl+Shift+P pins the focused pane). It is a leaf in every tab's
//! split tree, the Files tab's too, so each tab keeps its own place and size for it; its
//! terminal is resized to whichever tab shows it. On a tab switch it glides to its new card
//! rather than leaving with the old tab (see `animation/sticky_glide.rs`).

use crate::layout::{self, Direction, Node, PaneId};

use crate::app::workspace::tab::Tab;
use crate::app::{App, PANE_GAP};

/// Where a newly sticky pane goes in a tab when there's nothing better to go on.
const DEFAULT_PLACE: (Direction, f32) = (Direction::Right, 0.4);

impl App {
    pub(in crate::app) fn is_sticky(&self, id: PaneId) -> bool {
        self.sticky == Some(id)
    }

    /// Every tab: the terminal tabs and the Files tab.
    fn every_tab_mut(&mut self) -> impl Iterator<Item = &mut Tab> {
        self.tabs.iter_mut().chain(self.home_tab.iter_mut())
    }

    /// Pin pane `id` (of the shown tab) so it follows the user to every tab, or unpin it.
    /// Only one pane is sticky at a time: pinning another unpins the old one.
    pub(in crate::app) fn toggle_sticky(&mut self, ctx: &egui::Context, id: PaneId) {
        if self.is_sticky(id) {
            self.unstick();
            return;
        }
        if self.sticky.is_some() {
            self.unstick();
        }
        let Some(tab) = self.cur_tab().filter(|t| t.contains(id)) else { return };
        let (home, (dir, frac)) = (tab.uid, self.place_in(tab, id));
        for t in self.every_tab_mut().filter(|t| t.uid != home) {
            t.root.wrap_edge(id, dir, frac);
        }
        self.sticky = Some(id);
        ctx.request_repaint();
    }

    /// Where pane `id` sits in `tab`: the edge of the pane area it runs along and its share.
    fn place_in(&self, tab: &Tab, id: PaneId) -> (Direction, f32) {
        let mut rects = Vec::new();
        tab.root.layout(self.content_area, PANE_GAP, &mut rects, &mut Vec::new());
        rects
            .iter()
            .find(|(p, _)| *p == id)
            .map_or(DEFAULT_PLACE, |&(_, r)| layout::edge_placement(r, self.content_area))
    }

    /// `tab` (just made) with the sticky pane in it, placed as it is in the shown tab.
    pub(in crate::app) fn with_sticky(&self, mut tab: Tab) -> Tab {
        let Some(id) = self.sticky.filter(|&id| !tab.contains(id)) else { return tab };
        let place = self.cur_tab().filter(|t| t.contains(id)).map_or(DEFAULT_PLACE, |t| self.place_in(t, id));
        tab.root.wrap_edge(id, place.0, place.1);
        tab
    }

    /// The sticky pane stops being sticky: it stays in the shown tab only (or, if that one
    /// doesn't have it, the first that does).
    pub(in crate::app) fn unstick(&mut self) {
        let Some(id) = self.sticky.take() else { return };
        let keep = self
            .cur_tab()
            .filter(|t| t.contains(id))
            .or_else(|| self.tabs.iter().chain(self.home_tab.iter()).find(|t| t.contains(id)))
            .map(|t| t.uid);
        let mut lone = Vec::new();
        for t in self.every_tab_mut().filter(|t| Some(t.uid) != keep) {
            if !t.drop_leaf(id) {
                lone.push(t.uid);
            }
        }
        // Tabs that had only it (there shouldn't be any: see `prune_sticky_tabs`).
        self.drop_tabs(&lone);
    }

    /// While a pane is sticky no tab holds only it: such a tab (its last other pane closed
    /// or moved away) closes, and the sticky pane lives on in the others. If every tab is
    /// down to it, it stops being sticky and is the one pane of the tab being shown.
    pub(in crate::app) fn prune_sticky_tabs(&mut self) {
        let Some(id) = self.sticky else { return };
        let lone: Vec<u64> = self
            .tabs
            .iter()
            .chain(self.home_tab.iter())
            .filter(|t| matches!(t.root, Node::Leaf(p) if p == id))
            .map(|t| t.uid)
            .collect();
        if lone.is_empty() {
            return;
        }
        let total = self.tabs.len() + usize::from(self.home_tab.is_some());
        if lone.len() < total {
            self.drop_tabs(&lone);
            return;
        }
        let keep = self.cur_tab().map(|t| t.uid);
        self.sticky = None;
        self.drop_tabs(&lone.into_iter().filter(|&u| Some(u) != keep).collect::<Vec<_>>());
    }

    /// Take the tabs with these uids out of the window, keeping their panes alive.
    fn drop_tabs(&mut self, uids: &[u64]) {
        if self.home_tab.as_ref().is_some_and(|t| uids.contains(&t.uid)) {
            self.home_tab = None;
            self.home_active = false;
        }
        while let Some(i) = self.tabs.iter().position(|t| uids.contains(&t.uid)) {
            self.take_tab(i);
        }
    }

    /// Close the sticky pane `id`: it leaves every tab (collapsing out of the shown one).
    pub(in crate::app) fn close_sticky(&mut self, ctx: &egui::Context, id: PaneId) {
        let (title, process) = self.pane_label_owned(id);
        let shapes = self.panes.terminal_mut(id).map(|t| t.take_shapes()).unwrap_or_default();
        self.forget_pane(id);
        self.sticky = None;
        let shown = self.cur_tab().map(|t| t.uid);
        let mut lone = Vec::new();
        let mut shapes = Some(shapes);
        for t in self.every_tab_mut() {
            if Some(t.uid) == shown {
                if t.remove_pane(id) {
                    let shapes = shapes.take().unwrap_or_default();
                    t.motion.kick_close(ctx, id, title.clone(), process.clone(), shapes);
                } else {
                    lone.push(t.uid);
                }
            } else if !t.drop_leaf(id) {
                lone.push(t.uid);
            }
        }
        self.drop_tabs(&lone);
    }

    /// After closing tab panes: the sticky pane outlives a tab, unless no tab is left.
    pub(in crate::app) fn forget_sticky_if_alone(&mut self) {
        if let Some(id) = self.sticky
            && self.tabs.is_empty()
            && self.home_tab.is_none()
        {
            self.sticky = None;
            self.forget_pane(id);
        }
    }
}

impl Tab {
    /// Take `pane` out of this (not shown) tab without disturbing the rest: focus moves off
    /// it, and the tab stays zoomed unless it was the zoomed pane. `false` if it was the
    /// only pane.
    pub(in crate::app) fn drop_leaf(&mut self, pane: PaneId) -> bool {
        if !self.root.remove(pane) {
            return false;
        }
        if self.focused == pane {
            self.focused = self.root.iter().next().unwrap_or(self.focused);
            self.zoomed = false;
        }
        self.zoomed &= self.root.len() > 1;
        true
    }
}
