//! Motion: panes sliding in and out, tabs closing, the tab-switch transitions, the connect
//! screen handing over to the workspace (`handoff.rs`), and agent panes' header titles
//! rolling over when they change (`title_roll.rs`), and a header's long title scrolling
//! sideways while it's held (`title_scroll.rs`).

pub(super) mod handoff;
pub(super) mod pane_motion;
pub(super) mod sticky_glide;
pub(super) mod tab_close;
pub(super) mod tab_switch;
pub(super) mod title_roll;
pub(super) mod title_scroll;
pub(super) mod transition;
