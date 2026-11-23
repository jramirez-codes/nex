//! Tabs and panes: opening and closing them and finding the focused one (here), drawing them,
//! their cell metrics, and dragging panes around.

pub(super) mod agents;
pub(super) mod draw;
pub(super) mod metrics;
pub(super) mod pane_drag;
pub(super) mod sticky;
pub(super) mod tab;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui::{Pos2, Rect};

use crate::agents::AgentsPane;
use crate::file_explorer::FileExplorer;
use crate::git_diff::GitDiff;
use crate::layout::{Axis, Direction, Node, PaneId, PaneKind};
use crate::remote::exec::Transport;
use crate::terminal::Terminal;
use crate::ui::icons;

use crate::app::ssh::follow::Place;
use crate::app::workspace::metrics::cell_px;
use crate::app::workspace::tab::Tab;
use crate::app::animation::tab_close::{TabDrawn, TabGhost};
use crate::app::{App, PANE_GAP};

/// Where a pane works: a folder here, or one on an ssh host.
pub(in crate::app) enum Location {
    Local(PathBuf),
    /// A folder on the host `Transport` reaches: absolute, or starting with `~`.
    Remote(Arc<Transport>, PathBuf),
}

/// A pane, as the kind it is. [`App::pane`] finds it with one lookup per kind; match on it
/// rather than probing the per-kind maps, so a new kind of pane is a compile error everywhere
/// it needs handling.
pub(in crate::app) enum PaneRef<'a> {
    Terminal(&'a Terminal),
    Explorer(&'a FileExplorer),
    Diff(&'a GitDiff),
    Agents(&'a AgentsPane),
}

/// [`PaneRef`], to change the pane (see [`App::pane_mut`]).
pub(in crate::app) enum PaneMut<'a> {
    Terminal(&'a mut Terminal),
    Explorer(&'a mut FileExplorer),
    Diff(&'a mut GitDiff),
    Agents(&'a mut AgentsPane),
}

impl PaneRef<'_> {
    pub(in crate::app) fn kind(&self) -> PaneKind {
        match self {
            PaneRef::Terminal(_) => PaneKind::Terminal,
            PaneRef::Explorer(_) => PaneKind::Explorer,
            PaneRef::Diff(_) => PaneKind::Diff,
            PaneRef::Agents(_) => PaneKind::Agents,
        }
    }
}

impl App {
    /// A fresh id for a new pane (terminal or file explorer).
    pub(in crate::app) fn alloc_pane_id(&mut self) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Pane `id`, or `None` if it's gone.
    pub(in crate::app) fn pane(&self, id: PaneId) -> Option<PaneRef<'_>> {
        if let Some(t) = self.panes.get(&id) {
            Some(PaneRef::Terminal(t))
        } else if let Some(b) = self.explorers.get(&id) {
            Some(PaneRef::Explorer(b))
        } else if let Some(d) = self.diffs.get(&id) {
            Some(PaneRef::Diff(d))
        } else {
            self.agents.get(&id).map(PaneRef::Agents)
        }
    }

    /// [`pane`](Self::pane), to change it.
    pub(in crate::app) fn pane_mut(&mut self, id: PaneId) -> Option<PaneMut<'_>> {
        if let Some(t) = self.panes.get_mut(&id) {
            Some(PaneMut::Terminal(t))
        } else if let Some(b) = self.explorers.get_mut(&id) {
            Some(PaneMut::Explorer(b))
        } else if let Some(d) = self.diffs.get_mut(&id) {
            Some(PaneMut::Diff(d))
        } else {
            self.agents.get_mut(&id).map(PaneMut::Agents)
        }
    }

    /// What pane `id` shows, or `None` if it's gone: for callers that need only the kind.
    pub(in crate::app) fn pane_kind(&self, id: PaneId) -> Option<PaneKind> {
        self.pane(id).map(|p| p.kind())
    }

    /// Drop everything kept for pane `id` (its terminal or explorer, and its zoom level).
    fn forget_pane(&mut self, id: PaneId) {
        self.panes.remove(&id);
        self.pane_fonts.remove(&id);
        self.explorers.remove(&id);
        self.diffs.remove(&id);
        self.agents.remove(&id);
    }

    /// Forget every pane of `tab` (already out of the strip) except the sticky one, which
    /// lives on in the other tabs.
    fn forget_tab_panes(&mut self, tab: &Tab) {
        let sticky = self.sticky;
        for id in tab.root.panes().into_iter().filter(|&id| Some(id) != sticky) {
            self.forget_pane(id);
        }
    }

    /// Spawn a shell sized for `card`, so its first output is already laid out correctly
    /// (on the connected host, where the window is a thin client).
    /// `font_size` is the new pane's own zoom level (`None` = config default). `command` runs
    /// instead of the shell.
    pub(in crate::app) fn spawn_pane(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>, command: Option<Vec<String>>, card: Rect, font_size: Option<f32>) -> Option<PaneId> {
        if self.connect.active {
            return self.spawn_remote_pane(ctx, card, font_size);
        }
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
            None => {
                let command = command.or_else(|| self.first_command.take());
                Terminal::spawn(id, &self.cfg, command, cwd, size, cell_px, ctx)
            }
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

    /// A pane on the host the window is connected to: a shell it left there (see
    /// `App::adopt`), else a new one in the focused pane's folder there.
    #[cfg(unix)]
    fn spawn_remote_pane(&mut self, ctx: &egui::Context, card: Rect, font_size: Option<f32>) -> Option<PaneId> {
        let host = self.connect.host.clone()?;
        let cwd = self.next_remote_cwd.take().or_else(|| self.focused_pane().and_then(|id| self.panes.get(&id)).and_then(|t| t.remote_cwd()));
        // An agent picked in the Agents pane stays running when its pane closes; a shell
        // doesn't.
        let (session, keep) = match self.attach_next.clone() {
            Some(agent) => (Some(agent), true),
            None => (self.adopt.pop(), false),
        };
        let session = std::sync::Arc::new(std::sync::Mutex::new(session));
        let id = self.alloc_pane_id();
        let m = self.metrics_for(ctx, font_size.unwrap_or(self.cfg.font_size));
        let size = self.grid_for(card, ctx.pixels_per_point(), m);
        let remote = crate::terminal::RemotePane { host, session, keep };
        match Terminal::spawn_remote(&self.cfg, remote, cwd, size, cell_px(ctx, m), ctx) {
            Ok(t) => {
                self.panes.insert(id, t);
                if let Some(fs) = font_size {
                    self.pane_fonts.insert(id, fs);
                }
                Some(id)
            }
            Err(e) => {
                eprintln!("nex: failed to open a pane on the host: {e}");
                None
            }
        }
    }

    /// Remote panes need a unix socket pair (see `terminal/remote.rs`).
    #[cfg(not(unix))]
    fn spawn_remote_pane(&mut self, _ctx: &egui::Context, _card: Rect, _font_size: Option<f32>) -> Option<PaneId> {
        None
    }

    pub(in crate::app) fn focused_pane(&self) -> Option<PaneId> {
        self.cur_tab().map(|t| t.focused)
    }

    /// The tab being shown: the Files tab or the active terminal tab.
    pub(in crate::app) fn cur_tab(&self) -> Option<&Tab> {
        if self.home_active { self.home_tab.as_ref() } else { self.tabs.get(self.active) }
    }

    pub(in crate::app) fn cur_tab_mut(&mut self) -> Option<&mut Tab> {
        if self.home_active { self.home_tab.as_mut() } else { self.tabs.get_mut(self.active) }
    }

    /// The tab (Files or terminal) containing `pane`: the shown one if it does (the sticky
    /// pane is in every tab).
    pub(in crate::app) fn tab_with(&mut self, pane: PaneId) -> Option<&mut Tab> {
        let shown = self.cur_tab().filter(|t| t.contains(pane)).map(|t| t.uid);
        self.home_tab
            .iter_mut()
            .chain(self.tabs.iter_mut())
            .find(|t| shown.map_or_else(|| t.contains(pane), |uid| t.uid == uid))
    }

    pub(in crate::app) fn show_tab(&mut self, index: usize) {
        self.active = index;
        self.home_active = false;
    }

    pub(in crate::app) fn focused_cwd(&self) -> Option<PathBuf> {
        self.focused_pane().and_then(|id| self.pane_cwd(id))
    }

    /// Working directory of a pane on this machine: the shell's cwd, or the folder a explorer
    /// is showing (`None` for one showing a folder on an ssh host).
    pub(in crate::app) fn pane_cwd(&self, id: PaneId) -> Option<PathBuf> {
        match self.pane(id)? {
            PaneRef::Terminal(t) => t.cwd.clone(),
            PaneRef::Explorer(b) => Some(b).filter(|b| b.transport().is_none()).map(|b| b.dir.clone()),
            PaneRef::Diff(d) => Some(d).filter(|d| d.remote.is_none()).map(|d| d.dir.clone()),
            PaneRef::Agents(_) => None,
        }
    }

    /// Where a pane works, ssh hosts included: a terminal in an `ssh` session (or on the
    /// host the window is connected to) is in its folder there. `fresh` asks the OS what
    /// the terminal runs now, rather than taking the last poll's answer (cheap enough for
    /// every frame).
    pub(in crate::app) fn pane_location(&mut self, id: PaneId, fresh: bool) -> Option<Location> {
        match self.pane_mut(id)? {
            PaneMut::Terminal(t) => {
                // Panes elsewhere need a Unix shell to run their scripts (see `remote::exec`).
                if cfg!(unix) {
                    let transport = match &t.remote {
                        Some(r) => Some(Transport::Nex(r.host.clone())),
                        None => if fresh { t.ssh_target() } else { t.last_ssh_target() }.map(Transport::Ssh),
                    };
                    if let Some(transport) = transport {
                        // Where the remote shell didn't say, its home.
                        let dir = t.remote_cwd().unwrap_or_else(|| "~".into());
                        return Some(Location::Remote(Arc::new(transport), PathBuf::from(dir)));
                    }
                }
                t.cwd.clone().map(Location::Local)
            }
            PaneMut::Explorer(b) => {
                Some(match b.transport() {
                    Some(t) => Location::Remote(t.clone(), b.dir.clone()),
                    None => Location::Local(b.dir.clone()),
                })
            }
            PaneMut::Diff(d) => {
                Some(match &d.remote {
                    Some(t) => Location::Remote(t.clone(), d.dir.clone()),
                    None => Location::Local(d.dir.clone()),
                })
            }
            PaneMut::Agents(_) => None,
        }
    }

    /// [`pane_location`](Self::pane_location), else this machine's home folder.
    fn pane_location_or_home(&mut self, id: PaneId) -> Location {
        self.pane_location(id, true)
            .unwrap_or_else(|| Location::Local(crate::platform::home_dir().unwrap_or_else(|| PathBuf::from("/"))))
    }

    /// `(title, process)` for a pane's tab and header.
    pub(in crate::app) fn pane_label(&self, id: PaneId) -> (String, String) {
        match self.pane(id) {
            Some(PaneRef::Terminal(t)) => (t.title(), t.process.clone()),
            Some(PaneRef::Explorer(b)) => (b.title(), icons::EXPLORER_PROCESS.into()),
            Some(PaneRef::Diff(d)) => (d.title(), icons::DIFF_PROCESS.into()),
            Some(PaneRef::Agents(a)) => (a.title(), icons::AGENTS_PROCESS.into()),
            None => Default::default(),
        }
    }

    /// The pane whose label names `tab` in the tab strip (see [`Tab::header_pane`]).
    pub(in crate::app) fn tab_header_pane(&self, tab: &Tab) -> PaneId {
        tab.header_pane(|id| self.panes.contains_key(&id))
    }

    /// Note each tab's focused terminal, so its header keeps that name while another kind
    /// of pane has focus. Run once a frame, it catches focus however it moved.
    pub(in crate::app) fn remember_terminal_focus(&mut self) {
        for tab in &mut self.tabs {
            if self.panes.contains_key(&tab.focused) {
                tab.last_terminal = Some(tab.focused);
            }
        }
    }

    /// Show the Files tab (opening it at the focused directory the first time), or hide it
    /// again, back to the terminal tab that was showing.
    pub(in crate::app) fn toggle_explorer(&mut self) {
        if self.home_active {
            if !self.tabs.is_empty() {
                self.home_active = false;
            }
            return;
        }
        if self.home_tab.is_none() {
            let dir = self
                .focused_cwd()
                .or_else(crate::platform::home_dir)
                .unwrap_or_else(|| PathBuf::from("/"));
            let id = self.alloc_pane_id();
            self.explorers.insert(id, self.new_explorer(dir));
            self.home_tab = Some(self.with_sticky(Tab::new(id, id)));
        }
        self.home_active = true;
    }

    /// Select `item` in the Files tab (opened at `item`'s folder by `--select`).
    pub(in crate::app) fn select_in_explorer(&mut self, item: &Path) {
        let Some(id) = self.home_tab.as_ref().map(|t| t.focused) else { return };
        if let Some(b) = self.explorers.get_mut(&id) {
            b.reveal_item(item);
        }
    }

    /// A file explorer at `dir`, in the configured layout (`explorer-view`).
    pub(in crate::app) fn new_explorer(&self, dir: PathBuf) -> FileExplorer {
        let mut b = FileExplorer::new(dir);
        b.view = self.cfg.explorer_view;
        b
    }

    /// [`new_explorer`](Self::new_explorer) at `at`, which may be on an ssh host.
    fn explorer_at(&self, ctx: &egui::Context, at: Location) -> FileExplorer {
        match at {
            Location::Local(dir) => self.new_explorer(dir),
            Location::Remote(t, dir) => {
                let mut b = FileExplorer::new_remote(ctx, t, dir);
                b.view = self.cfg.explorer_view;
                b
            }
        }
    }

    pub(in crate::app) fn close_explorer(&mut self) {
        if let Some(tab) = self.home_tab.take() {
            self.forget_tab_panes(&tab);
        }
        self.home_active = false;
        self.forget_sticky_if_alone();
    }

    pub(in crate::app) fn new_tab(&mut self, ctx: &egui::Context, cwd: Option<PathBuf>, command: Option<Vec<String>>) {
        if let Some(id) = self.spawn_pane(ctx, cwd, command, self.content_area, None) {
            let tab = self.with_sticky(Tab::new(id, id));
            self.tabs.push(tab);
            self.show_tab(self.tabs.len() - 1);
        }
    }

    /// Split a terminal off `target`, in its directory (and its ssh session, see `follow`).
    pub(in crate::app) fn split(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        self.open_from(ctx, Some(target), Place::Split(target, axis));
    }

    /// Split a terminal starting in `cwd` (running `command`, else the shell) off `target`.
    pub(in crate::app) fn split_at(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis, cwd: Option<PathBuf>, command: Option<Vec<String>>) {
        let card = self.split_card(target, axis);
        // The new pane starts at the zoom level of the pane it was split from.
        let font_size = self.pane_fonts.get(&target).copied();
        if let Some(new) = self.spawn_pane(ctx, cwd, command, card, font_size) {
            self.attach_split(ctx, target, axis, new);
        }
    }

    /// Split a file explorer off `target`, showing `target`'s directory (on its ssh host, for
    /// a terminal in an `ssh` session), in the same tab.
    pub(in crate::app) fn split_explorer(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let at = self.pane_location_or_home(target);
        let new = self.alloc_pane_id();
        let b = self.explorer_at(ctx, at);
        self.explorers.insert(new, b);
        self.attach_split(ctx, target, axis, new);
    }

    /// Ctrl+G: close the shown tab's Changes pane if it has one (focus goes back to the pane
    /// it was opened from), else split one off the focused pane, showing its repository.
    pub(in crate::app) fn toggle_git_diff(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.cur_tab() else { return };
        if let Some(id) = tab.root.panes().into_iter().find(|id| self.diffs.contains_key(id)) {
            let origin = self.diffs.get(&id).and_then(|d| d.origin);
            self.close_pane(ctx, id);
            if let Some(origin) = origin
                && let Some(tab) = self.tab_with(origin)
            {
                tab.focused = origin;
            }
            return;
        }
        let Some(from) = self.focused_pane() else { return };
        self.split_diff(ctx, from, Axis::Horizontal);
    }

    /// Split a Changes pane off `target`, showing the repository of `target`'s directory (on
    /// its ssh host, for a terminal in an `ssh` session).
    pub(in crate::app) fn split_diff(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let (remote, dir) = match self.pane_location_or_home(target) {
            Location::Local(dir) => (None, dir),
            Location::Remote(t, dir) => (Some(t), dir),
        };
        let new = self.alloc_pane_id();
        let mut d = GitDiff::new_at(ctx, remote, dir);
        d.origin = Some(target);
        self.diffs.insert(new, d);
        // The diff starts at the zoom level of the pane it was opened from.
        if let Some(&fs) = self.pane_fonts.get(&target) {
            self.pane_fonts.insert(new, fs);
        }
        self.attach_split(ctx, target, axis, new);
    }

    /// Changes panes follow their tab's focus: focusing a terminal (or a Files pane) shows
    /// its directory's repository. Run every frame; it only reloads when the folder changes.
    pub(in crate::app) fn follow_focus(&mut self) {
        if self.diffs.is_empty() {
            return;
        }
        let mut tabs: Vec<(Vec<PaneId>, PaneId)> = self.home_tab.iter().chain(&self.tabs).map(|t| (t.root.panes(), t.focused)).collect();
        // The shown tab goes last, so a pinned (sticky) Changes pane, which is in every tab,
        // follows the focus of the tab on screen.
        if let Some(shown) = self.cur_tab().map(|t| t.uid)
            && let Some(i) = self.home_tab.iter().chain(&self.tabs).position(|t| t.uid == shown)
        {
            let t = tabs.remove(i);
            tabs.push(t);
        }
        for (panes, focused) in tabs {
            if self.diffs.contains_key(&focused) {
                continue;
            }
            let (remote, dir) = match self.pane_location(focused, false) {
                Some(Location::Local(dir)) => (None, dir),
                Some(Location::Remote(t, dir)) => (Some(t), dir),
                None => continue,
            };
            for id in panes {
                if let Some(d) = self.diffs.get_mut(&id) {
                    d.follow(remote.clone(), dir.clone());
                    d.origin = Some(focused);
                }
            }
        }
    }

    /// Show `path` (picked in Changes pane `pane`) in a Files pane split off it.
    pub(in crate::app) fn reveal_from_diff(&mut self, ctx: &egui::Context, pane: PaneId, path: &Path) {
        let Some(dir) = path.parent().map(Path::to_path_buf) else { return };
        let remote = self.diffs.get(&pane).and_then(|d| d.remote.clone());
        let new = self.alloc_pane_id();
        let at = match remote {
            Some(t) => Location::Remote(t, dir),
            None => Location::Local(dir),
        };
        let mut b = self.explorer_at(ctx, at);
        b.reveal_item(path);
        self.explorers.insert(new, b);
        self.attach_split(ctx, pane, Axis::Vertical, new);
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
    pub(in crate::app) fn swap_panes(&mut self, ctx: &egui::Context, a: PaneId, b: PaneId) {
        let Some(tab) = self.cur_tab_mut() else { return };
        if tab.zoomed || !tab.root.swap(a, b) {
            return;
        }
        tab.motion.kick_reflow(ctx);
    }

    /// Move `pane` to the `dir` side of `target`, focusing it. From another tab, that tab
    /// loses it (closing if it was its last pane) and `target`'s tab is shown.
    pub(in crate::app) fn move_pane(&mut self, ctx: &egui::Context, pane: PaneId, target: PaneId, dir: Direction) {
        if let Some(tab) = self.tab_with(pane).filter(|t| t.contains(target)) {
            if !tab.zoomed && tab.root.move_pane(pane, target, dir) {
                tab.focused = pane;
                tab.motion.kick_reflow(ctx);
            }
            return;
        }
        // The sticky pane is in every tab already.
        if self.is_sticky(pane) {
            return;
        }
        let (Some(src), Some(dst)) = (self.tab_index_with(pane), self.tab_index_with(target)) else { return };
        self.transfer(ctx, pane, src, dst, |root| root.insert(target, dir, pane));
    }

    /// Move `pane` into the tab with `uid`, beside its focused pane: to the right of a wide
    /// pane, below a tall one.
    pub(in crate::app) fn pane_to_tab(&mut self, ctx: &egui::Context, pane: PaneId, uid: u64) {
        let (Some(src), Some(dst)) = (self.tab_index_with(pane), self.tabs.iter().position(|t| t.uid == uid)) else { return };
        if src == dst || self.is_sticky(pane) {
            return;
        }
        let tab = &self.tabs[dst];
        let target = tab.focused;
        let mut rects = Vec::new();
        tab.root.layout(self.content_area, PANE_GAP, &mut rects, &mut Vec::new());
        let tall = rects.iter().find(|(id, _)| *id == target).is_some_and(|(_, r)| r.height() > r.width());
        let dir = if tall { Direction::Down } else { Direction::Right };
        self.transfer(ctx, pane, src, dst, |root| root.insert(target, dir, pane));
    }

    /// Give `pane` a tab of its own, just after the one it leaves (which keeps its others).
    pub(in crate::app) fn pane_to_new_tab(&mut self, ctx: &egui::Context, pane: PaneId) {
        if self.is_sticky(pane) {
            return;
        }
        let Some(src) = self.tab_index_with(pane) else { return };
        let tab = &mut self.tabs[src];
        if !tab.remove_pane(pane) {
            return;
        }
        tab.motion.kick_reflow(ctx);
        // A fresh uid: the old tab's uid may be this pane's id.
        let uid = self.alloc_pane_id();
        let tab = self.with_sticky(Tab::new(uid, pane));
        self.tabs.insert(src + 1, tab);
        self.show_tab(src + 1);
        self.prune_sticky_tabs();
    }

    /// Move `pane` from tab `src` into tab `dst`, where `place` puts it in the split tree.
    /// It is focused there and that tab is shown; `src` closes if it was its last pane (the
    /// pane lives on).
    fn transfer(&mut self, ctx: &egui::Context, pane: PaneId, src: usize, dst: usize, place: impl FnOnce(&mut Node) -> bool) {
        if src == dst || !place(&mut self.tabs[dst].root) {
            return;
        }
        let uid = self.tabs[dst].uid;
        let to = &mut self.tabs[dst];
        to.focused = pane;
        to.zoomed = false;
        to.motion.kick_reflow(ctx);
        let from = &mut self.tabs[src];
        if from.remove_pane(pane) {
            from.motion.kick_reflow(ctx);
        } else {
            self.take_tab(src);
        }
        self.show_tab_uid(uid);
        self.prune_sticky_tabs();
    }

    /// Index of the terminal tab holding `pane` (not the Files tab).
    fn tab_index_with(&self, pane: PaneId) -> Option<usize> {
        self.tabs.iter().position(|t| t.contains(pane))
    }

    /// Show the tab (terminal or Files) with `uid`, if it is still open.
    pub(in crate::app) fn show_tab_uid(&mut self, uid: u64) {
        if self.home_tab.as_ref().is_some_and(|t| t.uid == uid) {
            self.home_active = true;
        } else if let Some(i) = self.tabs.iter().position(|t| t.uid == uid) {
            self.show_tab(i);
        }
    }

    /// Put a pane being dragged by its header back where it was, showing its tab again if
    /// the drag had opened another.
    pub(in crate::app) fn cancel_pane_drag(&mut self, ctx: &egui::Context) {
        let Some(d) = self.pane_drag.take() else { return };
        if let Some(tab) = self.cur_tab_mut() {
            tab.motion.kick_reflow(ctx);
        }
        self.show_tab_uid(d.home);
    }

    /// Close one pane; closing a tab's last pane closes the tab. A pane closed out of a
    /// split collapses and fades out (its last frame is kept for that).
    pub(in crate::app) fn close_pane(&mut self, ctx: &egui::Context, id: PaneId) {
        if self.is_sticky(id) {
            self.close_sticky(ctx, id);
            return;
        }
        self.close_unpinned(ctx, id);
        self.prune_sticky_tabs();
    }

    fn close_unpinned(&mut self, ctx: &egui::Context, id: PaneId) {
        if let Some(n) = self.detach_notice(id) {
            self.notice = Some(n);
        }
        let (title, process) = self.pane_label(id);
        let shapes = self.panes.get_mut(&id).map(|t| t.take_shapes()).unwrap_or_default();
        self.forget_pane(id);
        if let Some(tab) = self.home_tab.as_mut().filter(|t| t.contains(id)) {
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

    pub(in crate::app) fn close_tab(&mut self, index: usize) {
        self.close_tab_labeled(index, None);
    }

    /// Close tab `index`, leaving a ghost that folds into its × (if it was on screen).
    /// `label` is its `(title, process)` when its pane is already gone.
    fn close_tab_labeled(&mut self, index: usize, label: Option<(String, String)>) {
        if index >= self.tabs.len() {
            return;
        }
        if let Some(n) = self.tabs[index].root.panes().into_iter().find_map(|id| self.detach_notice(id)) {
            self.notice = Some(n);
        }
        let tab = self.take_tab(index);
        if let Some(&drawn) = self.strip.tab_rects.get(&tab.uid) {
            for g in self.strip.tab_ghosts.iter_mut().filter(|g| g.index > index) {
                g.index -= 1;
            }
            let (title, process) = label.unwrap_or_else(|| self.pane_label(self.tab_header_pane(&tab)));
            let (mark, title) = icons::title_mark(&process, &title);
            let (mark, title) = (mark.map(str::to_string), title.to_string());
            let drawn = TabDrawn { active: drawn.active && !self.home_active, ..drawn };
            self.strip.tab_ghosts.push(TabGhost { drawn, index, start: None, title, process, mark });
        }
        self.forget_tab_panes(&tab);
        self.forget_sticky_if_alone();
    }

    /// Take tab `index` out of the strip, keeping its panes alive (the caller closes or
    /// rehomes them), and keep the active tab in range.
    fn take_tab(&mut self, index: usize) -> Tab {
        let tab = self.tabs.remove(index);
        if self.active >= index && self.active > 0 {
            self.active -= 1;
        }
        self.active = self.active.min(self.tabs.len().saturating_sub(1));
        tab
    }
}
