//! The pane store: every pane's state in one map ([`Panes`], `App::panes`), as the kind it
//! is ([`Pane`]), with what panes asked for while painting ([`PaneRequest`]), the one way a
//! pane comes in ([`App::insert_pane`]) and goes ([`App::forget_pane`]), and typed lookups
//! for code that wants one kind. Code that treats kinds differently matches on [`Pane`] (or
//! [`App::pane_kind`]), so a new kind is a compile error everywhere it needs handling.

use std::collections::HashMap;

use crate::agents::{self, AgentsPane};
use crate::file_explorer::{self, FileExplorer};
use crate::git_diff::{self, GitDiff};
use crate::layout::{PaneId, PaneKind};
use crate::terminal::Terminal;

use crate::app::App;

/// Every pane, by id. The lookups borrow only the store, not all of `App`.
#[derive(Default)]
pub(in crate::app) struct Panes(HashMap<PaneId, Pane>);

/// A pane, as the kind it is. Terminals, file explorers, Changes and Agents panes share the
/// pane id space and the split tree.
// A window has a handful of panes, each kept in place in the map: boxing the big kinds would
// only add an indirection.
#[allow(clippy::large_enum_variant)]
pub(in crate::app) enum Pane {
    Terminal(Terminal),
    Explorer(FileExplorer),
    Diff(GitDiff),
    Agents(AgentsPane),
}

/// What a pane asked for while painting (a terminal in some folder, a file shown in Files, a
/// jump to an agent), applied with the pending actions (`App::drain_pending`).
pub(in crate::app) enum PaneRequest {
    Explorer(file_explorer::Request),
    Diff(git_diff::Request),
    Agents(agents::Request),
}

impl Pane {
    pub(in crate::app) fn kind(&self) -> PaneKind {
        match self {
            Pane::Terminal(_) => PaneKind::Terminal,
            Pane::Explorer(_) => PaneKind::Explorer,
            Pane::Diff(_) => PaneKind::Diff,
            Pane::Agents(_) => PaneKind::Agents,
        }
    }

    pub(in crate::app) fn terminal(&self) -> Option<&Terminal> {
        if let Pane::Terminal(t) = self { Some(t) } else { None }
    }

    pub(in crate::app) fn terminal_mut(&mut self) -> Option<&mut Terminal> {
        if let Pane::Terminal(t) = self { Some(t) } else { None }
    }

    pub(in crate::app) fn explorer(&self) -> Option<&FileExplorer> {
        if let Pane::Explorer(b) = self { Some(b) } else { None }
    }

    pub(in crate::app) fn explorer_mut(&mut self) -> Option<&mut FileExplorer> {
        if let Pane::Explorer(b) = self { Some(b) } else { None }
    }

    pub(in crate::app) fn diff(&self) -> Option<&GitDiff> {
        if let Pane::Diff(d) = self { Some(d) } else { None }
    }

    pub(in crate::app) fn diff_mut(&mut self) -> Option<&mut GitDiff> {
        if let Pane::Diff(d) = self { Some(d) } else { None }
    }

    pub(in crate::app) fn agents(&self) -> Option<&AgentsPane> {
        if let Pane::Agents(a) = self { Some(a) } else { None }
    }

    pub(in crate::app) fn agents_mut(&mut self) -> Option<&mut AgentsPane> {
        if let Pane::Agents(a) = self { Some(a) } else { None }
    }
}

impl Panes {
    /// Pane `id`, or `None` if it's gone.
    pub(in crate::app) fn get(&self, id: PaneId) -> Option<&Pane> {
        self.0.get(&id)
    }

    /// [`get`](Self::get), to change it.
    pub(in crate::app) fn get_mut(&mut self, id: PaneId) -> Option<&mut Pane> {
        self.0.get_mut(&id)
    }

    /// What pane `id` shows, or `None` if it's gone.
    pub(in crate::app) fn kind(&self, id: PaneId) -> Option<PaneKind> {
        self.get(id).map(Pane::kind)
    }

    /// The ids of every pane of `kind`.
    pub(in crate::app) fn ids(&self, kind: PaneKind) -> impl Iterator<Item = PaneId> + '_ {
        self.0.iter().filter(move |(_, p)| p.kind() == kind).map(|(&id, _)| id)
    }

    /// Whether any pane is of `kind`.
    pub(in crate::app) fn has(&self, kind: PaneKind) -> bool {
        self.ids(kind).next().is_some()
    }

    /// Terminal pane `id`, if it is one.
    pub(in crate::app) fn terminal(&self, id: PaneId) -> Option<&Terminal> {
        self.get(id)?.terminal()
    }

    pub(in crate::app) fn terminal_mut(&mut self, id: PaneId) -> Option<&mut Terminal> {
        self.get_mut(id)?.terminal_mut()
    }

    /// Whether pane `id` is a terminal.
    pub(in crate::app) fn is_terminal(&self, id: PaneId) -> bool {
        self.terminal(id).is_some()
    }

    /// Every terminal pane, with its id.
    pub(in crate::app) fn terminals(&self) -> impl Iterator<Item = (PaneId, &Terminal)> {
        self.0.iter().filter_map(|(&id, p)| Some((id, p.terminal()?)))
    }

    pub(in crate::app) fn terminals_mut(&mut self) -> impl Iterator<Item = (PaneId, &mut Terminal)> {
        self.0.iter_mut().filter_map(|(&id, p)| Some((id, p.terminal_mut()?)))
    }

    /// File-explorer pane `id`, if it is one.
    pub(in crate::app) fn explorer(&self, id: PaneId) -> Option<&FileExplorer> {
        self.get(id)?.explorer()
    }

    pub(in crate::app) fn explorer_mut(&mut self, id: PaneId) -> Option<&mut FileExplorer> {
        self.get_mut(id)?.explorer_mut()
    }

    /// Every file-explorer pane.
    pub(in crate::app) fn explorers(&self) -> impl Iterator<Item = &FileExplorer> {
        self.0.values().filter_map(Pane::explorer)
    }

    /// Changes pane `id`, if it is one.
    pub(in crate::app) fn diff(&self, id: PaneId) -> Option<&GitDiff> {
        self.get(id)?.diff()
    }

    pub(in crate::app) fn diff_mut(&mut self, id: PaneId) -> Option<&mut GitDiff> {
        self.get_mut(id)?.diff_mut()
    }

    /// Agents pane `id`, if it is one.
    pub(in crate::app) fn agents(&self, id: PaneId) -> Option<&AgentsPane> {
        self.get(id)?.agents()
    }

    pub(in crate::app) fn agents_mut(&mut self, id: PaneId) -> Option<&mut AgentsPane> {
        self.get_mut(id)?.agents_mut()
    }
}

impl App {
    /// A fresh id for a new pane.
    pub(in crate::app) fn alloc_pane_id(&mut self) -> PaneId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Keep `pane` as pane `id` (from [`alloc_pane_id`](Self::alloc_pane_id)), at its own
    /// zoom level `font_size` (`None`: the config's). Every pane comes in here.
    pub(in crate::app) fn insert_pane(&mut self, id: PaneId, pane: Pane, font_size: Option<f32>) {
        self.panes.0.insert(id, pane);
        if let Some(fs) = font_size {
            self.pane_fonts.insert(id, fs);
        }
    }

    /// [`insert_pane`](Self::insert_pane) under a fresh id, which it returns.
    pub(in crate::app) fn add_pane(&mut self, pane: Pane, font_size: Option<f32>) -> PaneId {
        let id = self.alloc_pane_id();
        self.insert_pane(id, pane, font_size);
        id
    }

    /// Drop everything kept for pane `id` (its state and its zoom level).
    pub(in crate::app) fn forget_pane(&mut self, id: PaneId) {
        self.panes.0.remove(&id);
        self.pane_fonts.remove(&id);
    }

    /// What pane `id` shows, or `None` if it's gone: for callers that need only the kind.
    pub(in crate::app) fn pane_kind(&self, id: PaneId) -> Option<PaneKind> {
        self.panes.kind(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn typed_lookups_find_only_their_kind() {
        let mut panes = Panes::default();
        panes.0.insert(1, Pane::Agents(AgentsPane::new()));
        panes.0.insert(2, Pane::Explorer(FileExplorer::new(std::env::temp_dir())));
        assert_eq!(panes.kind(1), Some(PaneKind::Agents));
        assert!(panes.agents(1).is_some() && panes.explorer(1).is_none() && !panes.is_terminal(1));
        assert!(panes.explorer_mut(2).is_some() && panes.diff(2).is_none() && panes.agents_mut(2).is_none());
        assert_eq!(panes.ids(PaneKind::Explorer).collect::<Vec<_>>(), [2]);
        assert!(panes.has(PaneKind::Agents) && !panes.has(PaneKind::Diff));
        assert_eq!(panes.explorers().count(), 1);
        assert_eq!(panes.terminals().count(), 0);
        assert_eq!(panes.kind(3), None);
    }
}
