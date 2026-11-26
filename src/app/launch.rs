//! What the command line and the connect screen hand to the first panes opened: one-shot
//! state, each piece taken by the pane (or frame) that uses it.

use std::path::PathBuf;

use crate::terminal::Terminal;

/// Kept by `App` as `launch`.
pub(in crate::app) struct LaunchState {
    /// Flags passed on to the nex started by Ctrl+Shift+N (`--config`, `--app-id`).
    pub(in crate::app) new_window_args: Vec<String>,
    /// `--explorer` (or a folder argument): show the Files tab once the first shell opens.
    pub(in crate::app) open_explorer: bool,
    /// `--select`: the item to select in that Files tab.
    pub(in crate::app) select_at_start: Option<PathBuf>,
    /// `-e` command, run by the first pane only; later panes start the shell.
    pub(in crate::app) first_command: Option<Vec<String>>,
    /// The first shell, started before the window existed (see `Terminal::spawn_early`);
    /// the first pane opened takes it.
    pub(in crate::app) early_terminal: Option<Terminal>,
    /// Shells already on that host that the next panes opened attach to, instead of starting
    /// new ones (the ones this device left open there).
    pub(in crate::app) adopt: Vec<String>,
    /// The agent session the next remote pane attaches to (picked in the Agents pane).
    pub(in crate::app) attach_next: Option<String>,
    /// What the next remote pane runs instead of a shell (a sub-agent's tab).
    pub(in crate::app) launch_next: Option<Vec<String>>,
    /// The first frame has opened the first tab (or restored the session).
    pub(in crate::app) started: bool,
}

impl LaunchState {
    pub(in crate::app) fn new(args: crate::cli::Args, early_terminal: Option<Terminal>) -> Self {
        Self {
            new_window_args: args.for_new_window(),
            open_explorer: args.explorer,
            select_at_start: args.select.clone(),
            // An early terminal already runs the `-e` command.
            first_command: if early_terminal.is_some() { None } else { args.command },
            early_terminal,
            adopt: Vec::new(),
            attach_next: None,
            launch_next: None,
            started: false,
        }
    }
}
