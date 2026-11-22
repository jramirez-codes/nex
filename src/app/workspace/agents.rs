//! Agents panes in the workspace: finding every agent running in a pane (each process
//! poll), opening and closing Agents panes and the Agents tab, and jumping to an agent.

use std::collections::HashSet;
use std::time::Instant;

use crate::agents::AgentsPane;
use crate::agents::model::{AgentKind, Probe};
use crate::layout::{Axis, PaneId};
use crate::session::claude;
use crate::ui::icons;

use crate::app::App;
use crate::app::workspace::tab::Tab;

impl App {
    /// Re-read what every agent pane is doing. Run with the process poll, and only while
    /// something shows it (an Agents pane, or the title bar's Agents label).
    pub(in crate::app) fn track_agents(&mut self) {
        if self.agents.is_empty() && !self.shows_agents_label() {
            return;
        }
        let now = Instant::now();
        // Each pane once, in tab order (the sticky pane is in every tab: its first one).
        let mut seen = HashSet::new();
        let mut probes = Vec::new();
        let tabs = self.tabs.iter().enumerate().map(|(i, t)| (Some(i), t)).chain(self.explorer_tab.iter().map(|t| (None, t)));
        for (index, tab) in tabs {
            for id in tab.root.panes() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(t) = self.panes.get(&id) else { continue };
                let Some((pid, argv)) = t.foreground_job() else { continue };
                let Some(kind) = AgentKind::detect(&t.process, argv) else { continue };
                let session = (kind == AgentKind::Claude).then(|| claude::session_info(pid)).flatten().unwrap_or_default();
                let title = t.title();
                let (mark, rest) = icons::title_mark(&t.process, &title);
                probes.push(Probe {
                    pane: id,
                    tab: index,
                    kind,
                    title: rest.to_string(),
                    mark: mark.map(str::to_string),
                    cwd: t.cwd.clone(),
                    session_name: session.name,
                    session_status: session.status,
                    attention: t.attention,
                    last_output: t.last_output,
                    last_input: t.last_input,
                });
            }
        }
        self.agent_tracker.update(probes, now);
    }

    /// Whether the title bar (or sidebar) label is the Agents one this frame: it is unless
    /// the special tab is showing a file explorer (opened with `--explorer` or a folder).
    pub(in crate::app) fn shows_agents_label(&self) -> bool {
        !self.home_is_files()
    }

    /// Whether the special tab holds a file explorer rather than an Agents pane.
    pub(in crate::app) fn home_is_files(&self) -> bool {
        self.explorer_tab.as_ref().is_some_and(|t| t.root.panes().iter().any(|id| self.explorers.contains_key(id)) && !t.root.panes().iter().any(|id| self.agents.contains_key(id)))
    }

    /// Whether the special tab is worth keeping the window open for once the terminal tabs
    /// are gone: it has a terminal or a file explorer (an Agents pane alone has nothing
    /// left to watch).
    pub(in crate::app) fn home_keeps_window(&self) -> bool {
        self.explorer_tab
            .as_ref()
            .is_some_and(|t| t.root.panes().iter().any(|id| self.panes.contains_key(id) || self.explorers.contains_key(id)))
    }

    /// Ctrl+Shift+A: close the shown tab's Agents pane if it has one (focus goes back to the
    /// pane it was opened from), else split one off the focused pane.
    pub(in crate::app) fn toggle_agents(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.cur_tab() else { return };
        if let Some(id) = tab.root.panes().into_iter().find(|id| self.agents.contains_key(id)) {
            let origin = self.agents.get(&id).and_then(|a| a.origin);
            self.close_pane(ctx, id);
            if let Some(origin) = origin
                && let Some(tab) = self.tab_with(origin)
            {
                tab.focused = origin;
            }
            return;
        }
        let Some(from) = self.focused_pane() else { return };
        self.split_agents(ctx, from, Axis::Horizontal);
    }

    /// Split an Agents pane off `target`.
    pub(in crate::app) fn split_agents(&mut self, ctx: &egui::Context, target: PaneId, axis: Axis) {
        let new = self.alloc_pane_id();
        let mut a = AgentsPane::new();
        a.origin = Some(target);
        self.agents.insert(new, a);
        self.track_agents();
        self.attach_split(ctx, target, axis, new);
    }

    /// The title bar's Agents label: show the special tab (opening it on an Agents pane the
    /// first time), or from it, go back to the terminal tab that was showing.
    pub(in crate::app) fn toggle_agents_tab(&mut self) {
        if self.explorer_active {
            if !self.tabs.is_empty() {
                self.explorer_active = false;
            }
            return;
        }
        if self.explorer_tab.is_none() {
            let id = self.alloc_pane_id();
            self.agents.insert(id, AgentsPane::new());
            self.explorer_tab = Some(self.with_sticky(Tab::new(id, id)));
            self.track_agents();
        }
        self.explorer_active = true;
    }

    /// Jump to the agent in `pane`: show its tab and focus it.
    pub(in crate::app) fn focus_agent(&mut self, pane: PaneId) {
        if let Some(i) = self.tabs.iter().position(|t| t.contains(pane)) {
            // The sticky pane is in every tab: stay on the shown one if it can.
            if !self.cur_tab().is_some_and(|t| t.contains(pane)) {
                self.show_tab(i);
            }
        } else if self.explorer_tab.as_ref().is_some_and(|t| t.contains(pane)) {
            self.explorer_active = true;
        } else {
            return;
        }
        if let Some(tab) = self.cur_tab_mut() {
            tab.focused = pane;
            tab.zoomed = false;
        }
    }
}
