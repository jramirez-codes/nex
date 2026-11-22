//! Agents panes in the workspace: finding every agent (each process poll), in this
//! machine's panes and on the ssh hosts they connect to, opening and closing Agents panes
//! and the Agents tab, and jumping to an agent (or opening a pane on one).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::agents::AgentsPane;
use crate::agents::model::{AgentKind, Probe, Source, Status, is_remote_key, remote_key};
use crate::layout::{Axis, PaneId};
use crate::session::claude;
use crate::ui::icons;

use crate::app::App;
use crate::app::workspace::tab::Tab;

impl App {
    /// Re-read what every agent is doing: those running in panes here, and those `nex-host`
    /// keeps on the ssh hosts panes connect to (see `ssh/hosts.rs`). Run with the process
    /// poll, and only while something shows it (an Agents pane, or the title bar's label).
    pub(in crate::app) fn track_agents(&mut self) {
        if self.agents.is_empty() && !self.shows_agents_label() {
            return;
        }
        let now = Instant::now();
        // Each pane once, in tab order (the sticky pane is in every tab: its first one).
        let mut seen = HashSet::new();
        let mut probes = Vec::new();
        // The hosts ssh panes connect to, and which remote agent each pane shows.
        let mut hosts = HashSet::new();
        let mut showing: HashMap<String, (PaneId, Option<usize>, bool)> = HashMap::new();
        let tabs = self.tabs.iter().enumerate().map(|(i, t)| (Some(i), t)).chain(self.explorer_tab.iter().map(|t| (None, t)));
        for (index, tab) in tabs {
            for id in tab.root.panes() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(t) = self.panes.get(&id) else { continue };
                if let Some(target) = t.last_ssh_target() {
                    hosts.insert(target.normalized());
                    if let Some(session) = t.remote_agent() {
                        showing.entry(session).or_insert((id, index, t.attention));
                    }
                    continue;
                }
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
                    source: Source::Local,
                    reported: None,
                });
            }
        }

        self.remote_hosts.sync(&hosts, self.cfg.ssh_agents);
        self.offer_shim(&hosts);
        self.offer_sharing(&hosts);
        self.agent_notes = self.host_notes();
        // Two panes may reach one host under different names: list each agent once.
        let mut listed = HashSet::new();
        for (target, snap) in self.remote_hosts.snapshots() {
            let host = target.host().to_string();
            for s in &snap.sessions {
                let Some(kind) = AgentKind::from_name(&s.kind) else { continue };
                // One whose program ended has nothing left to show or attach to.
                let Some(status) = Status::from_host(s.status) else { continue };
                if !listed.insert((host.clone(), s.id.clone())) {
                    continue;
                }
                let shown = showing.get(&s.id).copied();
                let title = s.title.clone().unwrap_or_default();
                let (mark, rest) = icons::title_mark(kind.process(), &title);
                probes.push(Probe {
                    pane: remote_key(&host, &s.id),
                    tab: shown.and_then(|(_, tab, _)| tab),
                    kind,
                    title: rest.to_string(),
                    mark: mark.map(str::to_string),
                    cwd: Some(PathBuf::from(home_relative(&s.cwd, snap.home.as_deref()))),
                    session_name: Some(s.name.clone()),
                    session_status: None,
                    attention: shown.is_some_and(|(_, _, attention)| attention),
                    last_output: now,
                    last_input: now,
                    source: Source::Remote { host: host.clone(), target: target.clone(), session: s.id.clone(), pane: shown.map(|(p, _, _)| p) },
                    reported: Some(status),
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

    /// Show the agent behind card `key`: jump to its pane, or, for an agent on an ssh host
    /// that no pane shows, open a tab attached to it.
    pub(in crate::app) fn focus_agent(&mut self, ctx: &egui::Context, key: PaneId) {
        if !is_remote_key(key) {
            self.focus_pane(key);
            return;
        }
        let Some(agent) = self.agent_tracker.agents().iter().find(|a| a.pane == key) else { return };
        let Source::Remote { target, session, pane, .. } = agent.source.clone() else { return };
        match pane {
            Some(p) => self.focus_pane(p),
            None => {
                self.new_tab(ctx, None, Some(target.attach_command(&session)));
                self.track_agents();
            }
        }
    }

    /// `Delete` on card `key`, for an agent on an ssh host: ask before stopping it.
    pub(in crate::app) fn dismiss_agent(&mut self, key: PaneId) {
        let Some(agent) = self.agent_tracker.agents().iter().find(|a| a.pane == key) else { return };
        let Source::Remote { target, session, .. } = agent.source.clone() else { return };
        let what = format!("{} \u{00b7} {}", agent.kind.label(), agent.name);
        self.confirm_stop(target, session, what);
    }

    /// What the toast says when pane `id` closes while it shows an agent on an ssh host: it
    /// keeps running there.
    pub(in crate::app) fn detach_notice(&self, id: PaneId) -> Option<String> {
        let t = self.panes.get(&id)?;
        let target = t.last_ssh_target()?;
        let session = t.remote_agent()?;
        let host = target.host().to_string();
        let agent = self.agent_tracker.agents().iter().find(|a| a.pane == remote_key(&host, &session));
        let what = agent.map_or("The agent", |a| a.kind.label());
        Some(format!("{what} keeps running on {host} \u{00b7} reopen it from Agents"))
    }

    /// Show pane `pane`: its tab, focused.
    fn focus_pane(&mut self, pane: PaneId) {
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

/// `dir` on a host whose home is `home`, as `~/…` when it's in there.
fn home_relative(dir: &str, home: Option<&str>) -> String {
    match home.filter(|h| h.len() > 1).and_then(|h| dir.strip_prefix(h)) {
        Some("") => "~".into(),
        Some(rest) if rest.starts_with('/') => format!("~{rest}"),
        _ => dir.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::home_relative;

    #[test]
    fn remote_folders_read_from_the_remote_home() {
        assert_eq!(home_relative("/home/me/code/nex", Some("/home/me")), "~/code/nex");
        assert_eq!(home_relative("/home/me", Some("/home/me")), "~");
        assert_eq!(home_relative("/home/media", Some("/home/me")), "/home/media");
        assert_eq!(home_relative("/srv/app", None), "/srv/app");
    }
}
