//! Motion: panes sliding in and out, tabs closing, the tab-switch transitions, and the
//! connect screen handing over to the workspace (`handoff.rs`).

pub(super) mod handoff;
pub(super) mod pane_motion;
pub(super) mod sticky_glide;
pub(super) mod tab_close;
pub(super) mod tab_switch;
pub(super) mod transition;
