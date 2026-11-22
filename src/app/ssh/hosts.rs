//! Agents on ssh hosts (see `remote::host`): which hosts are watched, remembering the ones
//! with agents between runs, putting `nex-host`'s shim on them (see `nex-host/src/shim.rs`),
//! offering connection sharing for hosts that want a password, and stopping an agent.
//!
//! A host is watched while a pane is connected to it, or while it has agents. Hosts with
//! agents, and hosts where the shim was declined, are saved next to the session file
//! (`agent-hosts.json`), so detached agents are back in the Agents pane after restarting nex,
//! before any pane connects to their host again.

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use egui::Id;
use serde_json::{Value, json};

use crate::app::ssh::sharing::SharingPrompt;
use crate::config::AgentShim;
use crate::remote::{self, SshTarget};
use crate::remote::host::{self, HostError, Snapshot, State, Watch};
use crate::ui::alert::{Alert, AlertChoice, AlertIcon};
use crate::ui::theme::Theme;
use crate::ui::toast::Toast;

use crate::app::App;

/// An alert about agents on a host.
#[derive(Clone, Debug)]
pub(in crate::app) enum AgentPrompt {
    /// Offer the shim on this host.
    Shim(SshTarget),
    /// Stop this agent? `what` names it ("Claude Code · Fix the tests").
    Stop { target: SshTarget, session: String, what: String },
    /// Something failed: a title and what went wrong.
    Failed(String, String),
}

/// A background job's result.
enum Done {
    Shim(SshTarget, Result<(), HostError>),
    Stop(String, Result<(), HostError>),
}

#[derive(Default)]
pub(in crate::app) struct Hosts {
    watches: HashMap<SshTarget, Watch>,
    /// Hosts with agents, watched with no pane on them.
    remembered: Vec<SshTarget>,
    /// Hosts the shim was declined on (Not Now): not offered again.
    declined: Vec<SshTarget>,
    /// Hosts the shim was offered on (or is going on) this run.
    offered: HashSet<SshTarget>,
    /// Connection sharing was offered for a host that wanted a password, this run.
    sharing_offered: bool,
    loaded: bool,
    /// Finished background jobs, taken each frame.
    done: Arc<Mutex<Vec<Done>>>,
    /// Repaints the window when a watch hears something.
    ctx: Option<egui::Context>,
}

impl Hosts {
    pub(in crate::app) fn set_ctx(&mut self, ctx: &egui::Context) {
        if self.ctx.is_none() {
            self.ctx = Some(ctx.clone());
        }
    }

    fn load_once(&mut self) {
        if !self.loaded {
            self.loaded = true;
            (self.remembered, self.declined) = load();
        }
    }

    /// Watch the hosts `panes` are connected to, and the remembered ones; stop watching hosts
    /// with neither a pane nor an agent. With `enabled` off, watch nothing.
    pub(in crate::app) fn sync(&mut self, panes: &HashSet<SshTarget>, enabled: bool) {
        if !enabled {
            self.watches.clear();
            return;
        }
        self.load_once();
        // Forget hosts that turned out to have no agents left.
        let mut changed = false;
        for (target, w) in &self.watches {
            let s = w.snapshot();
            let has_agents = !s.sessions.is_empty();
            let known = self.remembered.contains(target);
            if has_agents && !known {
                self.remembered.push(target.clone());
                changed = true;
            } else if !has_agents && known && s.state == State::Live {
                self.remembered.retain(|t| t != target);
                changed = true;
            }
        }
        if changed {
            self.save();
        }

        let wanted: HashSet<&SshTarget> = panes.iter().chain(self.remembered.iter()).collect();
        self.watches.retain(|t, _| wanted.contains(t));
        for target in wanted {
            // Tests never connect anywhere.
            if !self.watches.contains_key(target) && !cfg!(test) {
                let ctx = self.ctx.clone();
                let watch = Watch::start(target.clone(), move || {
                    if let Some(c) = &ctx {
                        c.request_repaint();
                    }
                });
                self.watches.insert(target.clone(), watch);
            }
        }
    }

    /// Every watched host and what's known about it.
    pub(in crate::app) fn snapshots(&self) -> Vec<(SshTarget, Snapshot)> {
        let mut v: Vec<_> = self.watches.iter().map(|(t, w)| (t.clone(), w.snapshot())).collect();
        v.sort_by(|a, b| a.0.destination.cmp(&b.0.destination));
        v
    }

    /// The host a pane is on that could have the shim and doesn't, not yet offered or declined.
    fn shim_candidate(&self, panes: &HashSet<SshTarget>) -> Option<SshTarget> {
        self.watches
            .iter()
            .filter(|(t, _)| panes.contains(*t) && !self.offered.contains(*t) && !self.declined.contains(*t))
            .find(|(_, w)| {
                let s = w.snapshot();
                s.state == State::Live && s.shim == Some(false)
            })
            .map(|(t, _)| t.clone())
    }

    /// A host a pane is on that nex can't log into without a password.
    fn needs_login(&self, panes: &HashSet<SshTarget>) -> Option<&SshTarget> {
        self.watches.iter().find(|(t, w)| panes.contains(*t) && w.snapshot().state == State::Failed(HostError::NeedsLogin)).map(|(t, _)| t)
    }

    fn decline(&mut self, target: SshTarget) {
        if !self.declined.contains(&target) {
            self.declined.push(target);
            self.save();
        }
    }

    fn save(&self) {
        save(&self.remembered, &self.declined);
    }

    /// Run `job` on a worker thread; its result comes back through [`App::poll_agent_jobs`].
    fn spawn(&self, job: impl FnOnce() -> Done + Send + 'static) {
        let done = self.done.clone();
        let ctx = self.ctx.clone();
        std::thread::spawn(move || {
            let result = job();
            if let Ok(mut d) = done.lock() {
                d.push(result);
            }
            if let Some(c) = ctx {
                c.request_repaint();
            }
        });
    }

    fn start_shim_install(&mut self, target: SshTarget) {
        self.offered.insert(target.clone());
        self.spawn(move || {
            let r = host::install_shim(&target);
            Done::Shim(target, r)
        });
    }
}

impl App {
    /// After the hosts are synced: offer the shim on a host a pane is on (or, with
    /// `ssh-agent-shim = on`, just put it there).
    pub(in crate::app) fn offer_shim(&mut self, panes: &HashSet<SshTarget>) {
        if self.cfg.ssh_agent_shim == AgentShim::Off || self.ssh.agent_prompt.is_some() || self.ssh.sharing_prompt.is_some() {
            return;
        }
        let Some(target) = self.ssh.remote_hosts.shim_candidate(panes) else { return };
        match self.cfg.ssh_agent_shim {
            AgentShim::On => self.ssh.remote_hosts.start_shim_install(target),
            _ => {
                self.ssh.remote_hosts.offered.insert(target.clone());
                self.ssh.agent_prompt = Some(AgentPrompt::Shim(target));
            }
        }
    }

    /// A host a pane is on wants a password, so its agents can't be seen: offer connection
    /// sharing (once a run), which lets nex ride the login typed into that pane. The prompt
    /// then says to log in once more, since the pane's session started before sharing did.
    pub(in crate::app) fn offer_sharing(&mut self, panes: &HashSet<SshTarget>) {
        if !remote::SHARING_SUPPORTED || self.ssh.remote_hosts.sharing_offered || self.ssh.sharing_declined || self.ssh.sharing_prompt.is_some() || self.ssh.agent_prompt.is_some() {
            return;
        }
        let Some(host) = self.ssh.remote_hosts.needs_login(panes).map(|t| t.host().to_string()) else { return };
        self.ssh.remote_hosts.sharing_offered = true;
        if !remote::sharing_enabled() {
            self.ssh.sharing_prompt = Some(SharingPrompt::Offer(host, "Its agents show up in Agents a moment later"));
        }
    }

    /// Ask before stopping the agent `session` on `target`.
    pub(in crate::app) fn confirm_stop(&mut self, target: SshTarget, session: String, what: String) {
        self.ssh.agent_prompt = Some(AgentPrompt::Stop { target, session, what });
    }

    /// Results of the shim and stop jobs: toasts, and alerts when they failed.
    pub(in crate::app) fn poll_agent_jobs(&mut self, ctx: &egui::Context) {
        let done = self.ssh.remote_hosts.done.lock().map(|mut d| std::mem::take(&mut *d)).unwrap_or_default();
        for d in done {
            match d {
                Done::Shim(target, Ok(())) => {
                    if let Some(w) = self.ssh.remote_hosts.watches.get(&target) {
                        w.set_shim(true);
                    }
                    let msg = format!("claude and codex on {} now keep running after their pane closes", target.host());
                    self.toast = Some(Toast::long(ctx, msg));
                }
                Done::Shim(target, Err(e)) => {
                    let title = format!("Couldn't set it up on {}", target.host());
                    self.ssh.agent_prompt = Some(AgentPrompt::Failed(title, e.to_string()));
                }
                Done::Stop(_, Ok(())) => {}
                Done::Stop(host, Err(e)) => {
                    self.ssh.agent_prompt = Some(AgentPrompt::Failed(format!("Couldn't stop it on {host}"), e.to_string()));
                }
            }
        }
    }

    /// The alert, if one is up.
    pub(in crate::app) fn agent_dialog(&mut self, ctx: &egui::Context, theme: &Theme) {
        let Some(prompt) = self.ssh.agent_prompt.clone() else { return };
        let (icon, title, body, primary, secondary) = match &prompt {
            AgentPrompt::Shim(target) => (
                AlertIcon::Done,
                format!("Keep agents running on {}?", target.host()),
                "claude and codex started in nex's ssh panes there will keep running after \
                 their pane closes, and show up in Agents. nex adds a few marked lines to the \
                 host's shell config, which you can remove at any time."
                    .to_string(),
                "Set Up",
                Some("Not Now"),
            ),
            AgentPrompt::Stop { what, target, .. } => (
                AlertIcon::Warning,
                "Stop this agent?".to_string(),
                format!("{what} on {} will be ended. Whatever it's in the middle of is lost.", target.host()),
                "Stop",
                Some("Cancel"),
            ),
            AgentPrompt::Failed(title, e) => (AlertIcon::Warning, title.clone(), e.clone(), "OK", None),
        };
        let alert = Alert { id: Id::new("nex-agents-alert"), icon, title: &title, body: &body, steps: &[], primary, secondary, touch: self.cfg.sidebar() };
        let Some(choice) = alert.show(ctx, theme) else { return };
        self.ssh.agent_prompt = None;
        match (prompt, choice) {
            (AgentPrompt::Shim(target), AlertChoice::Primary) => self.ssh.remote_hosts.start_shim_install(target),
            (AgentPrompt::Shim(target), AlertChoice::Secondary) => self.ssh.remote_hosts.decline(target),
            (AgentPrompt::Stop { target, session, .. }, AlertChoice::Primary) => {
                let host = target.host().to_string();
                self.ssh.remote_hosts.spawn(move || Done::Stop(host, host::kill(&target, &session)));
            }
            _ => {}
        }
    }

    /// One line per watched host that isn't working, for the top of the Agents pane.
    pub(in crate::app) fn host_notes(&self) -> Vec<String> {
        let snapshots = self.ssh.remote_hosts.snapshots();
        // Read only when it matters: a host wants a password.
        let sharing = snapshots.iter().any(|(_, s)| s.state == State::Failed(HostError::NeedsLogin)) && remote::sharing_enabled();
        snapshots
            .into_iter()
            .filter_map(|(t, s)| match s.state {
                // Sharing is on, but the pane's login started before it was.
                State::Failed(HostError::NeedsLogin) if sharing => {
                    Some(format!("{} needs a password: log out of it (exit) and ssh back in, and nex shares that login", t.host()))
                }
                State::Failed(HostError::NeedsLogin) => Some(format!("{} needs a password, so nex can't see its agents", t.host())),
                State::Failed(HostError::Unsupported(system)) => Some(format!("{}: nex has no helper for {system}", t.host())),
                State::Failed(HostError::Other(e)) => Some(format!("{}: can't connect ({e}), retrying", t.host())),
                State::Connecting | State::Live => None,
            })
            .collect()
    }
}

/// `agent-hosts.json`, next to the session file.
fn path() -> Option<PathBuf> {
    Some(crate::session::path()?.with_file_name("agent-hosts.json"))
}

fn load() -> (Vec<SshTarget>, Vec<SshTarget>) {
    // Tests never reach this machine's real hosts.
    if cfg!(test) {
        return Default::default();
    }
    path().and_then(|p| std::fs::read_to_string(p).ok()).map(|t| from_json(&t)).unwrap_or_default()
}

fn save(hosts: &[SshTarget], declined: &[SshTarget]) {
    if cfg!(test) {
        return;
    }
    let Some(path) = path() else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Err(e) = std::fs::write(&path, to_json(hosts, declined)) {
        eprintln!("nex: can't save {}: {e}", path.display());
    }
}

fn targets_to_json(hosts: &[SshTarget]) -> Vec<Value> {
    hosts.iter().map(|t| json!({ "destination": t.destination, "opts": t.opts })).collect()
}

fn to_json(hosts: &[SshTarget], declined: &[SshTarget]) -> String {
    serde_json::to_string_pretty(&json!({ "hosts": targets_to_json(hosts), "shim_declined": targets_to_json(declined) })).unwrap_or_default()
}

fn targets_from_json(v: Option<&Value>) -> Vec<SshTarget> {
    let Some(hosts) = v.and_then(Value::as_array) else { return Vec::new() };
    hosts
        .iter()
        .filter_map(|h| {
            let destination = h.get("destination")?.as_str()?.to_string();
            let opts = h.get("opts")?.as_array()?.iter().map(|o| o.as_str().map(str::to_string)).collect::<Option<Vec<_>>>()?;
            // Only options ssh takes before a destination: nothing that could run a command.
            (!destination.starts_with('-') && opts.iter().all(|o| !o.contains('\n'))).then_some(SshTarget { opts, destination })
        })
        .collect()
}

fn from_json(text: &str) -> (Vec<SshTarget>, Vec<SshTarget>) {
    let Ok(v) = serde_json::from_str::<Value>(text) else { return Default::default() };
    (targets_from_json(v.get("hosts")), targets_from_json(v.get("shim_declined")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_a_pane_host_that_wants_a_password() {
        let (devbox, prod) = (SshTarget { opts: vec![], destination: "devbox".into() }, SshTarget { opts: vec![], destination: "prod".into() });
        let mut hosts = Hosts::default();
        let failed = Snapshot { state: State::Failed(HostError::NeedsLogin), ..Default::default() };
        hosts.watches.insert(devbox.clone(), Watch::fake(failed.clone()));
        hosts.watches.insert(prod.clone(), Watch::fake(Snapshot { state: State::Live, ..Default::default() }));
        assert_eq!(hosts.needs_login(&HashSet::from([devbox.clone(), prod.clone()])), Some(&devbox));
        // Only hosts a pane is on: that pane's login is the one to share.
        assert_eq!(hosts.needs_login(&HashSet::from([prod])), None);
    }

    #[test]
    fn remembered_hosts_round_trip() {
        let hosts = vec![
            SshTarget { opts: vec![], destination: "devbox".into() },
            SshTarget { opts: vec!["-p".into(), "2222".into(), "-J".into(), "jump".into()], destination: "me@gpu".into() },
        ];
        let declined = vec![SshTarget { opts: vec![], destination: "prod".into() }];
        assert_eq!(from_json(&to_json(&hosts, &declined)), (hosts.clone(), declined));
        assert_eq!(from_json(r#"{"hosts":[{"destination":"devbox","opts":[]}]}"#), (hosts[..1].to_vec(), vec![]), "older files");
        assert_eq!(from_json("not json"), (vec![], vec![]));
        assert_eq!(from_json(r#"{"hosts":[{"destination":"-oProxyCommand=x","opts":[]}]}"#).0, vec![]);
    }
}
