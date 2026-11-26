//! Working with a pane's ssh host: following it into new panes, uploading files, sharing the
//! mic, and the agents `nex-host` keeps running there.

pub(super) mod hosts;
pub(super) mod follow;
pub(super) mod mic;
pub(super) mod sharing;
pub(super) mod upload;

/// What nex is doing with the ssh hosts panes connect to: uploads, the shared mic, watched
/// agents, and the connection-sharing and agent dialogs. Kept by `App` as `ssh`.
pub(in crate::app) struct SshState {
    /// Files on their way to an `ssh` pane's remote host.
    pub(in crate::app) uploads: upload::Uploads,
    /// The connection-sharing dialog, after an upload couldn't log in.
    pub(in crate::app) sharing_prompt: Option<sharing::SharingPrompt>,
    /// Not Now was picked in that offer: don't ask again this run.
    pub(in crate::app) sharing_declined: bool,
    /// A split or tab into an ssh session, waiting on the sharing alert (see `follow`).
    pub(in crate::app) pending_pane: Option<follow::PendingPane>,
    /// Hosts this machine's microphone is shared with (see `ssh/mic.rs`).
    pub(in crate::app) mics: mic::Mics,
    /// The ssh hosts whose agents the Agents pane shows (see `ssh/hosts.rs`).
    pub(in crate::app) remote_hosts: hosts::Hosts,
    /// An alert about agents on a host: the shim offer, stopping one (see `ssh/hosts.rs`).
    pub(in crate::app) agent_prompt: Option<hosts::AgentPrompt>,
    /// Hosts the Agents pane can't watch, one line each, for the top of the pane.
    pub(in crate::app) agent_notes: Vec<String>,
}
