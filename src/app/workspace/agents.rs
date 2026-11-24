//! Agents panes in the workspace: finding every agent (each process poll), in this
//! machine's panes and on the ssh hosts they connect to, opening and closing Agents panes
//! and the Agents tab, and jumping to an agent (or opening a pane on one).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::Instant;

use crate::agents::AgentsPane;
use crate::agents::model::{AgentKind, Probe, Source, Status, Team, Viewing, is_remote_key, remote_key, sub_key};
use crate::layout::{Axis, PaneId};
use crate::remote::SshTarget;
use crate::session::claude;
use crate::ui::icons;

use crate::app::App;
use crate::app::workspace::tab::Tab;

/// What [`App::scan_panes`] found in this machine's panes.
#[derive(Default)]
struct LocalScan {
    /// The agents running in panes here.
    probes: Vec<Probe>,
    /// The hosts ssh panes connect to.
    hosts: HashSet<SshTarget>,
    /// The `nex-host` session each ssh pane shows: the pane, its tab, and whether it rang.
    showing: HashMap<String, (PaneId, Option<usize>, bool)>,
}

impl App {
    /// Re-read what every agent is doing: those running in panes here, and those `nex-host`
    /// keeps on the ssh hosts panes connect to (see `ssh/hosts.rs`). Run with the process
    /// poll, and only while something shows it (an Agents pane, or the title bar's label).
    pub(in crate::app) fn track_agents(&mut self) {
        if self.agents.is_empty() && !self.shows_agents_label() {
            return;
        }
        let now = Instant::now();
        let LocalScan { mut probes, hosts, showing } = self.scan_panes();
        self.ssh.remote_hosts.sync(&hosts, self.connect.host.as_ref(), self.cfg.ssh_agents);
        self.offer_shim(&hosts);
        self.offer_sharing(&hosts);
        self.ssh.agent_notes = self.host_notes();
        probes.extend(self.remote_probes(&showing, now));
        self.agent_tracker.update(probes, now);
    }

    /// Every pane once, in tab order (the sticky pane is in every tab: its first one): the
    /// agents running in them, the hosts ssh panes connect to, and which remote agent each
    /// of those shows.
    fn scan_panes(&self) -> LocalScan {
        let mut seen = HashSet::new();
        let mut scan = LocalScan::default();
        let tabs = self.tabs.iter().enumerate().map(|(i, t)| (Some(i), t)).chain(self.home_tab.iter().map(|t| (None, t)));
        for (index, tab) in tabs {
            for id in tab.root.iter() {
                if !seen.insert(id) {
                    continue;
                }
                let Some(t) = self.panes.get(&id) else { continue };
                // A pane on the host the window is connected to itself: the session it shows.
                if let Some(remote) = &t.remote {
                    if let Some(session) = remote.session.lock().unwrap_or_else(|e| e.into_inner()).clone() {
                        scan.showing.entry(session).or_insert((id, index, t.attention));
                    }
                    continue;
                }
                if let Some(target) = t.last_ssh_target() {
                    scan.hosts.insert(target.normalized());
                    if let Some(session) = t.remote_agent() {
                        scan.showing.entry(session).or_insert((id, index, t.attention));
                    }
                    continue;
                }
                let Some((pid, argv)) = t.foreground_job() else { continue };
                let Some(kind) = AgentKind::detect(&t.process, argv) else { continue };
                let session = (kind == AgentKind::Claude).then(|| claude::session_info(pid)).flatten().unwrap_or_default();
                let subagents = match kind {
                    AgentKind::Claude => claude::subagents(&session),
                    AgentKind::Codex => codex_subagents(pid, t.cwd.as_deref()),
                    AgentKind::Pipeline => Vec::new(),
                };
                let title = t.title();
                let (mark, rest) = icons::title_mark(&t.process, title);
                let lead = Probe {
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
                    team: Team::default(),
                };
                scan.probes.extend(sub_probes(&lead, Some(id), &subagents));
                scan.probes.push(lead);
            }
        }
        scan
    }

    /// The agents `nex-host` reports on the watched hosts, each once (two panes may reach
    /// one host under different names), with the pane showing it, if one is (`showing`).
    fn remote_probes(&self, showing: &HashMap<String, (PaneId, Option<usize>, bool)>, now: Instant) -> Vec<Probe> {
        let mut probes = Vec::new();
        let mut listed = HashSet::new();
        for (target, snap) in self.ssh.remote_hosts.snapshots() {
            let host = target.host().to_string();
            for s in &snap.sessions {
                let Some(kind) = AgentKind::from_name(s.agent_kind()) else { continue };
                // One whose program ended has nothing left to show or attach to.
                let Some(status) = Status::from_host(s.status) else { continue };
                if !listed.insert((host.clone(), s.id.clone())) {
                    continue;
                }
                let shown = showing.get(&s.id).copied();
                let title = s.title.clone().unwrap_or_default();
                let (mark, rest) = icons::title_mark(kind.process(), &title);
                let lead = Probe {
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
                    team: Team { parent: s.parent.as_deref().map(|p| remote_key(&host, p)), role: s.role.clone(), started: s.started, sub: false },
                };
                probes.extend(sub_probes(&lead, shown.map(|(p, _, _)| p), &s.subagents));
                probes.push(lead);
            }
        }
        probes
    }

    /// Whether the title bar (or sidebar) label is the Agents one this frame: it is unless
    /// the special tab is showing a file explorer (opened with `--explorer` or a folder).
    pub(in crate::app) fn shows_agents_label(&self) -> bool {
        !self.home_is_files()
    }

    /// Whether the special tab holds a file explorer rather than an Agents pane.
    pub(in crate::app) fn home_is_files(&self) -> bool {
        self.home_tab.as_ref().is_some_and(|t| t.root.iter().any(|id| self.explorers.contains_key(&id)) && !t.root.iter().any(|id| self.agents.contains_key(&id)))
    }

    /// Whether the special tab is worth keeping the window open for once the terminal tabs
    /// are gone: it has a terminal or a file explorer (an Agents pane alone has nothing
    /// left to watch).
    pub(in crate::app) fn home_keeps_window(&self) -> bool {
        self.home_tab
            .as_ref()
            .is_some_and(|t| t.root.iter().any(|id| self.panes.contains_key(&id) || self.explorers.contains_key(&id)))
    }

    /// What the Agents pane `id` marks "Viewing": the panes of the terminal tab you're on;
    /// only the focused one while it's zoomed. The focused pane is the primary; when that's
    /// the Agents pane itself (split beside them), the pane it was opened from, else the tab's
    /// last focused terminal. While the Agents tab is shown, nothing: no terminal tab is on
    /// screen, so no card is marked viewed.
    pub(in crate::app) fn viewing(&self, id: PaneId) -> Viewing {
        if self.home_active {
            return Viewing::default();
        }
        let Some(tab) = self.tabs.get(self.active) else { return Viewing::default() };
        let panes = if tab.zoomed { vec![tab.focused] } else { tab.root.panes() };
        let primary = if tab.focused == id {
            self.agents.get(&id).and_then(|a| a.origin).filter(|o| tab.contains(*o)).or(tab.last_terminal)
        } else {
            Some(tab.focused)
        };
        Viewing { panes, primary }
    }

    /// Ctrl+Shift+A: close the shown tab's Agents pane if it has one (focus goes back to the
    /// pane it was opened from), else split one off the focused pane.
    pub(in crate::app) fn toggle_agents(&mut self, ctx: &egui::Context) {
        let Some(tab) = self.cur_tab() else { return };
        if let Some(id) = tab.root.iter().find(|id| self.agents.contains_key(id)) {
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
    /// first time: the pinned one if that's an Agents pane, else a new one), or from it, go
    /// back to the terminal tab that was showing.
    pub(in crate::app) fn toggle_agents_tab(&mut self) {
        if self.home_active {
            if !self.tabs.is_empty() {
                self.home_active = false;
            }
            return;
        }
        if self.home_tab.is_none() {
            let tab = match self.sticky.filter(|id| self.agents.contains_key(id)) {
                // The pinned Agents pane already shows every agent: a second one would repeat it.
                Some(pinned) => Tab::new(pinned, pinned),
                None => {
                    let id = self.alloc_pane_id();
                    self.agents.insert(id, AgentsPane::new());
                    self.with_sticky(Tab::new(id, id))
                }
            };
            self.home_tab = Some(tab);
            self.track_agents();
        }
        self.home_active = true;
    }

    /// Show the agent behind card `key`: jump to its pane, or, for an agent on an ssh host
    /// that no pane shows, open a tab attached to it.
    pub(in crate::app) fn focus_agent(&mut self, ctx: &egui::Context, key: PaneId) {
        if !is_remote_key(key) && !crate::agents::model::is_sub_key(key) {
            self.focus_pane(key);
            return;
        }
        let Some(agent) = self.agent_tracker.agents().iter().find(|a| a.pane == key) else { return };
        // A sub-agent is shown in its agent's pane.
        if let Source::Sub { lead, .. } = agent.source {
            return self.focus_agent(ctx, lead);
        }
        let Source::Remote { target, session, pane, .. } = agent.source.clone() else { return };
        match pane {
            Some(p) => self.focus_pane(p),
            None if self.connect.active => {
                // On the host the window is connected to: a pane attached to the session,
                // which closing only detaches.
                self.attach_next = Some(session);
                self.new_tab(ctx, None, None);
                self.attach_next = None;
                self.track_agents();
            }
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
        let (host, session) = match &t.remote {
            Some(remote) if remote.keep => {
                let session = remote.session.lock().unwrap_or_else(|e| e.into_inner()).clone()?;
                (crate::app::ssh::hosts::direct_target(&remote.host).host().to_string(), session)
            }
            Some(_) => return None,
            None => (t.last_ssh_target()?.host().to_string(), t.remote_agent()?),
        };
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
        } else if self.home_tab.as_ref().is_some_and(|t| t.contains(pane)) {
            self.home_active = true;
        } else {
            return;
        }
        if let Some(tab) = self.cur_tab_mut() {
            tab.focused = pane;
            tab.zoomed = false;
        }
    }
}

/// The cards of the sub-agents `lead` is running (see `nex_host::subagents`), each shown in
/// `pane`, the pane showing `lead`, if one is.
fn sub_probes(lead: &Probe, pane: Option<PaneId>, subs: &[nex_host::proto::SubAgent]) -> Vec<Probe> {
    subs.iter()
        .map(|sa| Probe {
            pane: sub_key(lead.pane, &sa.id),
            tab: lead.tab,
            kind: lead.kind,
            title: sa.name.clone(),
            mark: None,
            cwd: lead.cwd.clone(),
            session_name: None,
            session_status: None,
            attention: false,
            last_output: lead.last_output,
            last_input: lead.last_input,
            source: Source::Sub { lead: lead.pane, pane },
            reported: Some(if sa.done { Status::Done } else { Status::Working }),
            team: Team { parent: Some(lead.pane), role: sa.role.clone(), started: sa.started, sub: true },
        })
        .collect()
}

/// The sub-agents the Codex process `pid` (in `cwd`) spawned, with Codex's home on this
/// machine: `$CODEX_HOME`, else `~/.codex`.
fn codex_subagents(pid: u32, cwd: Option<&std::path::Path>) -> Vec<nex_host::proto::SubAgent> {
    let home = match std::env::var_os("CODEX_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(h) => h,
        None => match crate::platform::home_dir() {
            Some(h) => h.join(".codex"),
            None => return Vec::new(),
        },
    };
    let cwd = cwd.map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    nex_host::subagents::codex_in(&home, pid, &cwd, nex_host::subagents::now_ms())
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
