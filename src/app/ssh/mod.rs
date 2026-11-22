//! Working with a pane's ssh host: following it into new panes, uploading files, sharing the
//! mic, and the agents `nex-host` keeps running there.

pub(super) mod hosts;
pub(super) mod follow;
pub(super) mod mic;
pub(super) mod sharing;
pub(super) mod upload;
