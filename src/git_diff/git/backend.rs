//! Which machine the Changes pane's folder is on, and the one place that picks between
//! running `git` here (`status.rs`, `diff.rs`) or on an ssh host (`remote.rs`).

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use crate::git_diff::git::diff::{self, FileDiff};
use crate::git_diff::git::remote;
use crate::git_diff::git::status::{self, FileChange, Status};
use crate::remote::exec::Transport;

/// How often the status is re-read while the pane is on screen.
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// The same for a folder on an ssh host (each read is a trip there).
const REMOTE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Where the repository is: this machine, or the ssh host a transport reaches. Cheap to
/// clone, so background loads take their own.
#[derive(Clone, Debug, Default, PartialEq)]
pub enum GitBackend {
    #[default]
    Local,
    Remote(Arc<Transport>),
}

impl GitBackend {
    /// [`Local`](Self::Local) for `None`.
    pub fn new(transport: Option<Arc<Transport>>) -> Self {
        transport.map_or(Self::Local, Self::Remote)
    }

    /// The host, if the repository isn't on this machine.
    pub fn transport(&self) -> Option<&Arc<Transport>> {
        match self {
            Self::Local => None,
            Self::Remote(t) => Some(t),
        }
    }

    pub fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    /// How often the status is read again.
    pub fn refresh_interval(&self) -> Duration {
        if self.is_remote() { REMOTE_REFRESH_INTERVAL } else { REFRESH_INTERVAL }
    }

    /// The host's name for messages ("the host" here, where none is asked for).
    pub fn host_label(&self) -> String {
        self.transport().map_or_else(|| "the host".into(), |t| t.label())
    }

    /// The working tree's status for the repository holding `dir`.
    pub fn load_status(&self, dir: &Path) -> Status {
        match self {
            Self::Local => status::load_status(dir),
            Self::Remote(t) => remote::load_status_remote(t, dir),
        }
    }

    /// `file`'s diff in the repository at `root`.
    pub fn load_diff(&self, root: &Path, file: &FileChange) -> Result<FileDiff, String> {
        match self {
            Self::Local => diff::load_diff(root, file),
            Self::Remote(t) => remote::load_diff_remote(t, root, file),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_when_no_host() {
        let b = GitBackend::new(None);
        assert_eq!(b, GitBackend::Local);
        assert!(!b.is_remote() && b.transport().is_none());
        assert_eq!(b.refresh_interval(), REFRESH_INTERVAL);
        assert_eq!(b.host_label(), "the host");
    }
}
