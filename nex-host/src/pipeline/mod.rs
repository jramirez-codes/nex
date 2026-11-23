//! Pipelines: a list of stages, each an agent given a task, run with the same calls an
//! agent delegating by hand makes (`spawn`, `send`, `recv`). There's no engine of its own:
//! a stage is "spawn (or send a new task to) this agent, then take its `done`".
//!
//! The file format, with examples, is in nex-host's README (*Pipelines*). A last stage
//! with `each = true` is a queue: every report from the stages before it gets in line and
//! one agent takes them one at a time; a `changes` from it goes back to the stage that
//! sent the report, up to [`SEND_BACKS`] times. When `retry` sends a run back, a stage
//! whose agent is still running gets its task again as a new message, so it keeps what it
//! learned the first time.
//!
//! `pipe` runs the runner as a nex-host session of its own (kind `pipeline`), so it keeps
//! going when the terminal closes and its stages show up in nex's Agents pane as its
//! children.
//!
//! ```text
//! pipeline/
//!   mod.rs     this file
//!   spec.rs    the file's stages, their order, filling in tasks, where a run goes next
//!   files.rs   finding pipeline files, and the run a new `pipe` joins
//!   run.rs     running one: its stages' agents and their reports
//!   each.rs    an `each` stage's queue (bookkeeping only, no I/O)
//!   tests.rs
//! ```

use nex_host::proto::Payload;

mod each;
mod files;
mod run;
mod spec;
#[cfg(test)]
mod tests;

pub use files::{find, joins, list};
pub use run::{Ran, run};
pub use spec::{Pipeline, parse_duration};

/// What a stage reported: its `done`'s whole text and fields.
type Outcome = Payload;

/// How many times an `each` stage can send one report back.
const SEND_BACKS: u32 = 3;
