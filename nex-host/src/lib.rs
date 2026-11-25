//! `nex-host`: the helper nex installs on an ssh host (in `~/.nex/bin`) so coding agents
//! started from nex keep running after the pane they were started in closes.
//!
//! ```text
//! nex-host/src/
//!   lib.rs      this file
//!   proto.rs    what nex and nex-host say to each other (shared with nex)
//!   claude.rs   Claude Code's session file (~/.claude/sessions/<pid>.json)
//!   subagents.rs  the sub-agents Claude Code and Codex run inside their own process
//!   transcript.rs a sub-agent's transcript as terminal text, followed live (its tab)
//!   paths.rs    where nex-host keeps its socket and lock (~/.nex/run)
//!   main.rs     the `nex-host` program: its subcommands
//!   daemon/     the background process that owns the agents
//!   agent.rs    one agent: its process, and the screen it has drawn
//!   pty.rs      the pty and process it runs in: login shell, environment, size
//!   redraw.rs   that screen as escape sequences, for a pane attaching to it
//!   attach.rs   `nex-host attach`: a pane's view of one agent
//!   client.rs   talking to the daemon (starting it when it isn't running)
//!   locked.rs   taking a lock a panicked thread left poisoned
//!   broker.rs   the daemon's post office: addresses, mailboxes, `recv` waiting on one
//!   deliver.rs  how a message reaches an agent: its text, typed into its terminal
//!   agentcli/   the commands agents use to work with each other (`spawn`, `send`, `done`, …)
//!   pipeline/   pipelines: stages of agents, run with those same commands
//!   shim.rs     `nex-host shim`: `claude` and `codex` typed in a nex pane start through nex-host
//!   skill.rs    the manual for agents, and installing it for Claude Code and Codex
//! ```
//!
//! Without the default `daemon` feature only `proto`, `claude`, `subagents`, `transcript` and `paths` are built: that's
//! what nex itself uses.

pub mod claude;
pub mod paths;
pub mod proto;
pub mod subagents;
pub mod transcript;
