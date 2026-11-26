//! Bringing the last session back (`restore-session`): the window's tabs, split trees and
//! folders are snapshotted to `crate::session` while nex runs and on exit, and rebuilt on
//! the first frame of the next plain launch. Restored terminals start a fresh shell and type
//! in what was running in it (Claude Code resumes its conversation).

use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::layout::{Node, PaneId, PaneKind};
use crate::session::{self, PaneNode, PaneState, Session, TabState};

use crate::app::workspace::Location;
use crate::app::workspace::panes::Pane;
use crate::app::workspace::tab::Tab;
use crate::app::{App, PANE_GAP};

/// How soon after the user typed into a pane its shell exiting counts as the user's.
const USER_EXIT_WINDOW: Duration = Duration::from_secs(3);

/// The folder a restored pane opens in: its saved one if it still exists, else home.
fn restore_dir(cwd: Option<&PathBuf>) -> PathBuf {
    cwd.filter(|d| d.is_dir()).cloned().or_else(crate::platform::home_dir).unwrap_or_else(|| PathBuf::from("/"))
}

/// The saved tree with each leaf's index (in `leaves` order) for its id, to lay it out.
fn shape(node: &PaneNode, next: &mut PaneId) -> Node {
    match node {
        PaneNode::Leaf(_) => {
            *next += 1;
            Node::Leaf(*next - 1)
        }
        PaneNode::Split { axis, ratio, a, b } => Node::Split { axis: *axis, ratio: *ratio, a: Box::new(shape(a, next)), b: Box::new(shape(b, next)) },
    }
}

/// The saved tree with leaf `i` as pane `ids[i]`; a leaf that didn't open is left out and its
/// sibling takes the split's place.
fn build(node: &PaneNode, ids: &[Option<PaneId>], next: &mut usize) -> Option<Node> {
    match node {
        PaneNode::Leaf(_) => {
            *next += 1;
            ids[*next - 1].map(Node::Leaf)
        }
        PaneNode::Split { axis, ratio, a, b } => match (build(a, ids, next), build(b, ids, next)) {
            (Some(a), Some(b)) => Some(Node::Split { axis: *axis, ratio: *ratio, a: Box::new(a), b: Box::new(b) }),
            (one, other) => one.or(other),
        },
    }
}

/// The pane for saved leaf `want`, or the nearest one that opened (the earlier on a tie).
fn nearest_open(ids: &[Option<PaneId>], want: usize) -> Option<PaneId> {
    ids.iter().enumerate().filter_map(|(i, id)| id.map(|id| (i.abs_diff(want), id))).min_by_key(|&(d, _)| d).map(|(_, id)| id)
}

/// Where the first shell `restore_session` starts goes (for `Terminal::spawn_early`), or
/// `None` if the session has no terminal panes at all.
pub fn first_shell_dir(s: &Session) -> Option<PathBuf> {
    s.tabs.iter().chain(&s.explorer).flat_map(|t| t.root.leaves()).find(|p| p.kind == PaneKind::Terminal).map(|p| restore_dir(p.cwd.as_ref()))
}

/// What the window knows about the session file: the session to restore, whether this window
/// keeps the file, and what decides when it's saved. Kept by `App` as `session`.
pub(in crate::app) struct SessionState {
    /// The last session, to bring back on the first frame (see `crate::session`).
    pub(in crate::app) restore_at_start: Option<crate::session::Session>,
    /// This window keeps the session file: a plain launch holding the session lock (not a
    /// second window, `-e`, or another nex already running).
    pub(in crate::app) save_session: bool,
    /// The session lock, held for the window's lifetime (see `session::try_lock`).
    pub(in crate::app) _session_lock: Option<crate::session::Lock>,
    /// A tab was open at some point (a window that never opened one leaves the file alone).
    pub(in crate::app) had_tabs: bool,
    /// The last close was the user's: a close action, or a shell exiting right after the user
    /// typed into it (`exit`, Ctrl+D).
    pub(in crate::app) closed_by_user: bool,
    /// A shell exited without the user (logout's SIGHUP, a killed shell, a program ending):
    /// the session file is left as it was (no autosave, save on exit or clear) until the
    /// user next types, clicks or closes something.
    pub(in crate::app) session_suspended: bool,
    /// When the user last pressed a key, clicked or typed into the window.
    pub(in crate::app) last_user_input: Option<Instant>,
    /// The session as last written, so unchanged polls don't touch the disk.
    pub(in crate::app) saved_session: Option<String>,
}

impl App {
    /// The window as it is now, or `None` once every tab is closed.
    pub(in crate::app) fn snapshot_session(&self) -> Option<Session> {
        let tabs: Vec<TabState> = self.tabs.iter().map(|t| self.tab_state(t)).collect();
        let explorer = self.home_tab.as_ref().map(|t| self.tab_state(t));
        if tabs.is_empty() && explorer.is_none() {
            return None;
        }
        Some(Session { tabs, active: self.active, explorer, explorer_active: self.home_active })
    }

    fn tab_state(&self, tab: &Tab) -> TabState {
        let focused = tab.root.iter().position(|p| p == tab.focused).unwrap_or(0);
        TabState { root: self.pane_node(&tab.root), focused, zoomed: tab.zoomed }
    }

    fn pane_node(&self, node: &Node) -> PaneNode {
        match node {
            Node::Leaf(id) => PaneNode::Leaf(PaneState {
                kind: self.pane_kind(*id).unwrap_or(PaneKind::Terminal),
                cwd: self.pane_cwd(*id),
                font_size: self.pane_fonts.get(id).copied(),
                command: self.panes.terminal(*id).and_then(|t| t.running_command()),
                sticky: self.sticky == Some(*id),
            }),
            Node::Split { axis, ratio, a, b } => {
                PaneNode::Split { axis: *axis, ratio: *ratio, a: Box::new(self.pane_node(a)), b: Box::new(self.pane_node(b)) }
            }
        }
    }

    /// Rebuild the tabs of `s`. Panes that fail to spawn are left out (their sibling takes
    /// their place). Returns `false` if nothing at all could be opened.
    pub(in crate::app) fn restore_session(&mut self, ctx: &egui::Context, s: Session) -> bool {
        let mut active = None;
        for (i, t) in s.tabs.iter().enumerate() {
            if let Some(tab) = self.restore_tab(ctx, t) {
                if i <= s.active {
                    active = Some(self.tabs.len());
                }
                self.tabs.push(tab);
            }
        }
        if let Some(t) = &s.explorer {
            self.home_tab = self.restore_tab(ctx, t);
        }
        // Only an Agents tab came back: there's nothing to watch without a terminal, so start
        // a fresh one beside it.
        if self.tabs.is_empty() && !self.home_keeps_window() {
            return false;
        }
        self.active = active.unwrap_or(0);
        self.home_active = self.home_tab.is_some() && (s.explorer_active || self.tabs.is_empty());
        self.prune_sticky_tabs();
        true
    }

    /// Open tab `t`'s panes, each sized for its card in the saved layout (a zoomed tab's
    /// focused pane fills the area), so shells draw their first prompt at the right width.
    fn restore_tab(&mut self, ctx: &egui::Context, t: &TabState) -> Option<Tab> {
        let area = self.content_area;
        let mut cards = Vec::new();
        shape(&t.root, &mut 0).layout(area, PANE_GAP, &mut cards, &mut Vec::new());
        let leaves = t.root.leaves();
        let zoomed = t.zoomed && leaves.len() > 1;
        let ids: Vec<Option<PaneId>> = leaves
            .iter()
            .enumerate()
            .map(|(i, p)| {
                let card = match cards.iter().find(|(id, _)| *id == i as PaneId) {
                    Some(&(_, r)) if !(zoomed && i == t.focused) => r,
                    _ => area,
                };
                // Every tab lists the sticky pane: the first one opens it, the rest share it.
                if p.sticky
                    && let Some(id) = self.sticky
                {
                    return Some(id);
                }
                let id = self.restore_pane(ctx, p, card);
                if p.sticky {
                    self.sticky = id;
                }
                id
            })
            .collect();
        let root = build(&t.root, &ids, &mut 0)?;
        let focused = nearest_open(&ids, t.focused)?;
        let mut tab = Tab::new(self.alloc_pane_id(), focused);
        tab.zoomed = zoomed && root.len() > 1;
        tab.root = root;
        Some(tab)
    }

    fn restore_pane(&mut self, ctx: &egui::Context, p: &PaneState, card: egui::Rect) -> Option<PaneId> {
        let dir = restore_dir(p.cwd.as_ref());
        match p.kind {
            PaneKind::Terminal => {
                let id = self.spawn_pane(ctx, Some(dir), None, card, p.font_size)?;
                if let (Some(cmd), Some(t)) = (&p.command, self.panes.terminal_mut(id)) {
                    t.queue_command(cmd);
                }
                Some(id)
            }
            PaneKind::Explorer => Some(self.add_pane(Pane::Explorer(self.explorer_at(ctx, Location::Local(dir))), p.font_size)),
            PaneKind::Diff => Some(self.add_pane(Pane::Diff(crate::git_diff::GitDiff::new(ctx, dir)), p.font_size)),
            PaneKind::Agents => Some(self.add_pane(Pane::Agents(crate::agents::AgentsPane::new()), None)),
        }
    }

    /// Whether this window keeps the session file: it holds the lock, `restore-session` is
    /// on (off, the file is neither written nor removed), and no burst of shell exits has
    /// suspended it.
    pub(in crate::app) fn keeps_session(&self) -> bool {
        self.session.save_session && self.cfg.restore_session && !self.session.session_suspended
    }

    /// Whether the window emptying now means the next launch should start fresh: the user
    /// closed the last tab (not a logout killing the shells, nor a first frame that never
    /// opened a tab).
    pub(in crate::app) fn forgets_session_when_empty(&self) -> bool {
        self.keeps_session() && self.session.had_tabs && self.session.closed_by_user
    }

    /// Whether pane `id`'s shell exiting was the user's doing: they typed into it (and into
    /// the window) in the last few seconds, as with `exit` or Ctrl+D.
    pub(in crate::app) fn exit_by_user(&self, id: PaneId) -> bool {
        let recent = |at: Instant| at.elapsed() < USER_EXIT_WINDOW;
        self.session.last_user_input.is_some_and(recent) && self.panes.terminal(id).is_some_and(|t| recent(t.last_input))
    }

    /// The user closed a pane or tab (a close action or button): the session follows again.
    pub(in crate::app) fn user_closing(&mut self) {
        self.session.closed_by_user = true;
        self.session.session_suspended = false;
    }

    /// Write the session if it changed since the last write (called every process poll).
    pub(in crate::app) fn autosave_session(&mut self) {
        if !self.keeps_session() {
            return;
        }
        let Some(s) = self.snapshot_session() else { return };
        let json = s.to_json();
        if self.session.saved_session.as_deref() == Some(json.as_str()) {
            return;
        }
        if let Err(e) = session::save(&s) {
            eprintln!("nex: session: {e}");
        }
        // Remembered even on failure, so a broken disk isn't retried (and reported) every poll.
        self.session.saved_session = Some(json);
    }

    /// Quitting with tabs open: save them. (A window that emptied already decided in
    /// `close_if_empty` whether to forget the session.)
    pub(in crate::app) fn save_session_on_exit(&mut self) {
        if !self.keeps_session() {
            return;
        }
        // Catch a program started since the last poll.
        for (_, t) in self.panes.terminals_mut() {
            t.refresh_process_info();
        }
        if let Some(s) = self.snapshot_session()
            && let Err(e) = session::save(&s)
        {
            eprintln!("nex: session: {e}");
        }
    }

    /// The user closed the last tab: the next launch starts fresh.
    pub(in crate::app) fn forget_session(&mut self) {
        session::clear();
        self.session.saved_session = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn focus_falls_back_to_the_nearest_open_pane() {
        assert_eq!(nearest_open(&[Some(1), Some(2), Some(3)], 1), Some(2));
        assert_eq!(nearest_open(&[None, Some(2), None, Some(4)], 2), Some(2));
        assert_eq!(nearest_open(&[Some(1), None, None, Some(4)], 2), Some(4));
        assert_eq!(nearest_open(&[Some(1), None, Some(3)], 1), Some(1));
        assert_eq!(nearest_open(&[None, None], 0), None);
    }

    #[test]
    fn a_failed_leaf_hands_its_split_to_its_sibling() {
        use crate::layout::Axis;
        let leaf = || PaneNode::Leaf(PaneState { kind: PaneKind::Explorer, cwd: None, font_size: None, command: None, sticky: false });
        let split = |a, b| PaneNode::Split { axis: Axis::Horizontal, ratio: 0.3, a: Box::new(a), b: Box::new(b) };
        let tree = split(leaf(), split(leaf(), leaf()));
        assert_eq!(build(&tree, &[Some(7), None, Some(9)], &mut 0).unwrap().panes(), vec![7, 9]);
        assert!(build(&tree, &[None, None, None], &mut 0).is_none());
        assert_eq!(shape(&tree, &mut 0).panes(), vec![0, 1, 2]);
    }
}
