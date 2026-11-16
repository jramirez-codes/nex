//! Opening and closing tabs and panes, and finding the focused one.

use std::path::PathBuf;

use egui::{Pos2, Rect};

use crate::file_explorer::FileExplorer;
use crate::app::layout::{Axis, PaneId};
use crate::terminal::Terminal;
use crate::ui::icons;

use super::metrics::cell_px;
use super::tab::Tab;
use super::tab_motion::{TabDrawn, TabGhost};
use super::{App, PANE_GAP};

impl App {
    /// A fresh id for a new pane (terminal or file explorer).
    fn alloc_pane_id(&mut self) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Drop everything kept for pane `id` (its terminal or explorer, and its zoom level).
    fn forget_pane(&mut self, id: PaneId) {
        self.panes.remove(&id);
        self.pane_fonts.remove(&id);
        self.explorers.remove(&id);
    }

    /// Spawn a shell sized for `card`, so its first output is already laid out correctly.
    /// `font_size` is the new pane's own zoom level (`None` = config default).
    pub(super) fn spawn_pane(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>, card: Rect, font_size: Option<f32>) -> Option<PaneId> {
        let id = self.alloc_pane_id();
        let m = self.metrics_for(ctx, font_size.unwrap_or(self.cfg.font_size));
        let size = self.grid_for(card, ctx.pixels_per_point(), m);
        let cell_px = cell_px(ctx, m);
        let spawned = match self.early_terminal.take() {
            Some(mut t) => {
                t.resize(size, cell_px);
                t.attach(ctx);
                Ok(t)
            }
            None => Terminal::spawn(id, &self.cfg, self.first_command.take(), cwd, size, cell_px, ctx),
        };
        match spawned {
            Ok(t) => {
                self.panes.insert(id, t);
                if let Some(fs) = font_size {
                    self.pane_fonts.insert(id, fs);
                }
                Some(id)
            }
            Err(e) => {
                eprintln!("nex: failed to spawn shell: {e}");
                None
            }
        }
    }

    pub(super) fn focused_pane(&self) -> Option<PaneId> {
        self.cur_tab().map(|t| t.focused)
    }

    /// The tab being shown: the Files tab or the active terminal tab.
    pub(super) fn cur_tab(&self) -> Option<&Tab> {
        if self.explorer_active { self.explorer_tab.as_ref() } else { self.tabs.get(self.active) }
    }

    pub(super) fn cur_tab_mut(&mut self) -> Option<&mut Tab> {
        if self.explorer_active { self.explorer_tab.as_mut() } else { self.tabs.get_mut(self.active) }
    }

    /// The tab (Files or terminal) containing `pane`.
    pub(super) fn tab_with(&mut self, pane: PaneId) -> Option<&mut Tab> {
        self.explorer_tab.iter_mut().chain(self.tabs.iter_mut()).find(|t| t.contains(pane))
    }

    pub(super) fn show_tab(&mut self, index: usize) {
        self.active = index;
        self.explorer_active = false;
    }

    pub(super) fn focused_cwd(&self) -> Option<PathBuf> {
        self.focused_pane().and_then(|id| self.pane_cwd(id))
    }

    /// Working directory of a pane: the shell's cwd, or the folder a explorer is showing.
    pub(super) fn pane_cwd(&self, id: PaneId) -> Option<PathBuf> {
        match self.explorers.get(&id) {
            Some(b) => Some(b.dir.clone()),
            None => self.panes.get(&id).and_then(|t| t.cwd.clone()),
        }
    }

    /// `(title, process)` for a pane's tab and header.
    pub(super) fn pane_label(&self, id: PaneId) -> (String, String) {
        if let Some(b) = self.explorers.get(&id) {
            return (b.title(), icons::EXPLORER_PROCESS.into());
        }
        self.panes.get(&id).map(|t| (t.title(), t.process.clone())).unwrap_or_default()
    }

    /// Show the Files tab (opening it at the focused directory the first time), or hide it
    /// again, back to the terminal tab that was showing.
    pub(super) fn toggle_explorer(&mut self) {
        if self.explorer_active {
            if !self.tabs.is_empty() {
                self.explorer_active = false;
            }
            return;
        }
        if self.explorer_tab.is_none() {
            let dir = self
                .focused_cwd()
                .or_else(crate::platform::home_dir)
                .unwrap_or_else(|| PathBuf::from("/"));
            let id = self.alloc_pane_id();
            self.explorers.insert(id, FileExplorer::new(dir));
            self.explorer_tab = Some(Tab::new(id, id));
        }
        self.explorer_active = true;
    }

    pub(super) fn close_explorer(&mut self) {
        if let Some(tab) = self.explorer_tab.take() {
            for id in tab.root.panes() {
                self.forget_pane(id);
            }
        }
        self.explorer_active = false;
    }

    pub(super) fn new_tab(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>) {
        if let Some(id) = self.spawn_pane(ctx, cwd, self.content_area, None) {
            self.tabs.push(Tab::new(id, id));
            self.show_tab(self.tabs.len() - 1);
        }
    }

    pub(super) fn split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let cwd = self.pane_cwd(target);
        self.split_at(ctx, target, axis, cwd);
    }

    /// Split a terminal starting in `cwd` off `target`.
    pub(super) fn split_at(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis, cwd: Option<PathBuf>) {
        let card = self.split_card(target, axis);
        // The new pane starts at the zoom level of the pane it was split from.
        let font_size = self.pane_fonts.get(&target).copied();
        if let Some(new) = self.spawn_pane(ctx, cwd, card, font_size) {
            self.attach_split(ctx, target, axis, new);
        }
    }

    /// Split a file explorer off `target`, showing `target`'s directory, in the same tab.
    pub(super) fn split_explorer(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let dir = self
            .pane_cwd(target)
            .or_else(crate::platform::home_dir)
            .unwrap_or_else(|| PathBuf::from("/"));
        let new = self.alloc_pane_id();
        self.explorers.insert(new, FileExplorer::new(dir));
        self.attach_split(ctx, target, axis, new);
    }

    /// Where a pane split off `target` will land: the half of its card on the new side.
    fn split_card(&self, target: PaneId, axis: Axis) -> Rect {
        self.card_rects
            .iter()
            .find(|(id, _)| *id == target)
            .map(|&(_, r)| match axis {
                Axis::Horizontal => Rect::from_min_max(Pos2::new(r.center().x + PANE_GAP / 2.0, r.top()), r.max),
                Axis::Vertical => Rect::from_min_max(Pos2::new(r.left(), r.center().y + PANE_GAP / 2.0), r.max),
            })
            .unwrap_or(Rect::NOTHING)
    }

    /// Put `new` beside `target` in whichever tab holds it, and focus it. The new pane slides
    /// out of `target`'s far edge.
    fn attach_split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis, new: PaneId) {
        match self.tab_with(target) {
            Some(tab) => {
                tab.root.split(target, axis, new);
                tab.focused = new;
                tab.zoomed = false;
                tab.motion.kick_split(ctx, target, axis, new);
            }
            None => self.forget_pane(new),
        }
    }

    /// Swap panes `a` and `b` (in the shown tab). Focus stays on whichever was focused, now
    /// in its new slot; both cards glide there.
    pub(super) fn swap_panes(&mut self, ctx: &egui::Context, a: PaneId, b: PaneId) {
        let Some(tab) = self.cur_tab_mut() else { return };
        if tab.zoomed || !tab.root.swap(a, b) {
            return;
        }
        tab.motion.kick_reflow(ctx);
    }

    /// Put a pane being dragged by its header back where it was.
    pub(super) fn cancel_pane_drag(&mut self, ctx: &egui::Context) {
        if let Some(tab) = self.cur_tab_mut().filter(|t| t.drag.is_some()) {
            tab.drag = None;
            tab.motion.kick_reflow(ctx);
        }
    }

    /// Close one pane; closing a tab's last pane closes the tab. A pane closed out of a
    /// split collapses and fades out (its last frame is kept for that).
    pub(super) fn close_pane(&mut self, ctx: &egui::Context, id: PaneId) {
        let (title, process) = self.pane_label(id);
        let shapes = self.panes.get_mut(&id).map(|t| t.take_shapes()).unwrap_or_default();
        self.forget_pane(id);
        if let Some(tab) = self.explorer_tab.as_mut().filter(|t| t.contains(id)) {
            if tab.remove_pane(id) {
                tab.motion.kick_close(ctx, id, title, process, shapes);
            } else {
                self.close_explorer();
            }
            return;
        }
        let Some(ti) = self.tabs.iter().position(|t| t.contains(id)) else { return };
        let tab = &mut self.tabs[ti];
        if tab.remove_pane(id) {
            tab.motion.kick_close(ctx, id, title, process, shapes);
        } else {
            // The pane is already forgotten: hand its label to the tab's close animation.
            self.close_tab_labeled(ti, Some((title, process)));
        }
    }

    pub(super) fn close_tab(&mut self, index: usize) {
        self.close_tab_labeled(index, None);
    }

    /// Close tab `index`, leaving a ghost that folds into its × (if it was on screen).
    /// `label` is its `(title, process)` when its pane is already gone.
    fn close_tab_labeled(&mut self, index: usize, label: Option<(String, String)>) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        if let Some(&drawn) = self.tab_rects.get(&tab.uid) {
            for g in self.tab_ghosts.iter_mut().filter(|g| g.index > index) {
                g.index -= 1;
            }
            let (title, process) = label.unwrap_or_else(|| self.pane_label(tab.focused));
            let (mark, title) = icons::title_mark(&process, &title);
            let (mark, title) = (mark.map(str::to_string), title.to_string());
            let drawn = TabDrawn { active: drawn.active && !self.explorer_active, ..drawn };
            self.tab_ghosts.push(TabGhost { drawn, index, start: None, title, process, mark });
        }
        for id in tab.root.panes() {
            self.forget_pane(id);
        }
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }
}
