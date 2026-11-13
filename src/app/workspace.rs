//! Opening and closing tabs and panes, and finding the focused one.

use std::path::PathBuf;

use egui::{
    Pos2, Rect,
};

use crate::chrome::{self};
use crate::files::FileBrowser;
use crate::layout::{Axis, PaneId};
use crate::terminal::Terminal;

use super::{App, PANE_GAP};
use super::metrics::cell_px;
use super::tab::Tab;

impl App {
    /// Spawn a shell sized for `card`, so its first output is already laid out correctly.
    /// `font_size` is the new pane's own zoom level (`None` = config default).
    pub(super) fn spawn_pane(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>, card: Rect, font_size: Option<f32>) -> Option<PaneId> {
        let id = self.next_id;
        self.next_id += 1;
        let m = self.metrics_for(ctx, font_size.unwrap_or(self.cfg.font_size));
        let size = self.grid_for(card, ctx.pixels_per_point(), m);
        let cell_px = cell_px(ctx, m);
        match Terminal::spawn(id, &self.cfg, cwd, size, cell_px, ctx) {
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
        if self.files_active { self.files.as_ref() } else { self.tabs.get(self.active) }
    }

    pub(super) fn cur_tab_mut(&mut self) -> Option<&mut Tab> {
        if self.files_active { self.files.as_mut() } else { self.tabs.get_mut(self.active) }
    }

    /// The tab (Files or terminal) containing `pane`.
    pub(super) fn tab_with(&mut self, pane: PaneId) -> Option<&mut Tab> {
        self.files.iter_mut().chain(self.tabs.iter_mut()).find(|t| t.root.panes().contains(&pane))
    }

    pub(super) fn show_tab(&mut self, index: usize) {
        self.active = index;
        self.files_active = false;
    }

    pub(super) fn focused_cwd(&self) -> Option<PathBuf> {
        self.focused_pane().and_then(|id| self.pane_cwd(id))
    }

    /// Working directory of a pane: the shell's cwd, or the folder a browser is showing.
    pub(super) fn pane_cwd(&self, id: PaneId) -> Option<PathBuf> {
        match self.browsers.get(&id) {
            Some(b) => Some(b.dir.clone()),
            None => self.panes.get(&id).and_then(|t| t.cwd.clone()),
        }
    }

    /// `(title, process)` for a pane's tab and header.
    pub(super) fn pane_label(&self, id: PaneId) -> (String, String) {
        if let Some(b) = self.browsers.get(&id) {
            return (b.title(), chrome::FILES_PROCESS.into());
        }
        self.panes.get(&id).map(|t| (t.title(), t.process.clone())).unwrap_or_default()
    }

    /// Show the Files tab (opening it at the focused directory the first time), or hide it
    /// again, back to the terminal tab that was showing.
    pub(super) fn toggle_files(&mut self) {
        if self.files_active {
            if !self.tabs.is_empty() {
                self.files_active = false;
            }
            return;
        }
        if self.files.is_none() {
            let dir = self
                .focused_cwd()
                .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
                .unwrap_or_else(|| PathBuf::from("/"));
            let id = self.next_id;
            self.next_id += 1;
            self.browsers.insert(id, FileBrowser::new(dir));
            self.files = Some(Tab::new(id, id));
        }
        self.files_active = true;
    }

    pub(super) fn close_files(&mut self) {
        if let Some(tab) = self.files.take() {
            for id in tab.root.panes() {
                self.panes.remove(&id);
                self.pane_fonts.remove(&id);
                self.browsers.remove(&id);
            }
        }
        self.files_active = false;
    }

    pub(super) fn new_tab(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>) {
        if let Some(id) = self.spawn_pane(ctx, cwd, self.content_area, None) {
            self.tabs.push(Tab::new(id, id));
            self.show_tab(self.tabs.len() - 1);
        }
    }

    pub(super) fn split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let cwd = self.pane_cwd(target);
        let card = self
            .card_rects
            .iter()
            .find(|(id, _)| *id == target)
            .map(|&(_, r)| match axis {
                Axis::Horizontal => Rect::from_min_max(Pos2::new(r.center().x + PANE_GAP / 2.0, r.top()), r.max),
                Axis::Vertical => Rect::from_min_max(Pos2::new(r.left(), r.center().y + PANE_GAP / 2.0), r.max),
            })
            .unwrap_or(Rect::NOTHING);
        // The new pane starts at the zoom level of the pane it was split from.
        let font_size = self.pane_fonts.get(&target).copied();
        if let Some(new) = self.spawn_pane(ctx, cwd, card, font_size) {
            if let Some(tab) = self.tab_with(target) {
                tab.root.split(target, axis, new);
                tab.focused = new;
                tab.zoomed = false;
            }
        }
    }

    pub(super) fn close_pane(&mut self, id: PaneId) {
        self.panes.remove(&id);
        self.pane_fonts.remove(&id);
        self.browsers.remove(&id);
        if let Some(tab) = self.files.as_mut().filter(|t| t.root.panes().contains(&id)) {
            if tab.root.remove(id) {
                if tab.focused == id {
                    tab.focused = tab.root.panes()[0];
                }
                tab.zoomed = false;
            } else {
                self.close_files();
            }
            return;
        }
        let Some(ti) = self.tabs.iter().position(|t| t.root.panes().contains(&id)) else { return };
        let tab = &mut self.tabs[ti];
        if tab.root.remove(id) {
            if tab.focused == id {
                tab.focused = tab.root.panes()[0];
            }
            tab.zoomed = false;
        } else {
            self.close_tab(ti);
        }
    }

    pub(super) fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        let tab = self.tabs.remove(index);
        for id in tab.root.panes() {
            self.panes.remove(&id);
            self.pane_fonts.remove(&id);
            self.browsers.remove(&id);
        }
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
    }
}
