//! `nex-host`: the helper nex installs on an ssh host (in `~/.nex/bin`) so coding agents
//! started from nex keep running after the pane they were started in closes.
//!
//! ```text
//! nex-host/src/
//!   lib.rs      this file
//!   proto.rs    what nex and nex-host say to each other (shared with nex)
//!   claude.rs   Claude Code's session file (~/.claude/sessions/<pid>.json)
//!   paths.rs    where nex-host keeps its socket and lock (~/.nex/run)
//!   main.rs     the `nex-host` program: its subcommands
//!   daemon.rs   the background process that owns the agents
//!   agent.rs    one agent: its pty, its process, and the screen it has drawn
//!   redraw.rs   that screen as escape sequences, for a pane attaching to it
//!   attach.rs   `nex-host attach`: a pane's view of one agent
//!   client.rs   talking to the daemon (starting it when it isn't running)
//! ```
//!
//! Without the default `daemon` feature only `proto`, `claude` and `paths` are built: that's
//! what nex itself uses.

pub mod claude;
pub mod paths;
pub mod proto;
