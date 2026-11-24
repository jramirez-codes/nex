//! What nex and `nex-host` say to each other.
//!
//! Every connection to the daemon's socket starts with one [`Request`] as a JSON line, and
//! the daemon answers with one [`Response`] line. Then:
//! - `watch` gets a new [`Response`] line with every session whenever something changes
//!   (and every [`HEARTBEAT_SECS`] regardless), until the client hangs up;
//! - `attach` turns into a terminal: the daemon sends the agent's screen and then its output
//!   as raw bytes, and the client sends [`frame`]s (keys, resizes) back;
//! - `recv` waits (up to its timeout) for a message before it answers;
//! - everything else is done after its answer.
//!
//! Agents talk to each other with [`Message`]s: `send` puts one in the recipient's mailbox
//! and types it into the recipient's terminal (`nex-host/src/deliver.rs` has the format it
//! arrives in); `recv` takes one out of the caller's mailbox instead.
//!
//! `nex-host connect` passes one such connection through its stdin and stdout, for clients
//! that reach the host over an ssh channel rather than a terminal (nex's own ssh client).
//!
//! ```text
//! proto/
//!   mod.rs      versions, the agent tag, and re-exports of everything below
//!   session.rs  a session and its status: [`Session`], [`Status`], [`Claude`], [`SubAgent`]
//!   request.rs  a connection's first line and its answer: [`Request`], [`Response`]
//!   message.rs  what agents send each other: [`Message`], [`MessageKind`], [`Payload`], [`Verdict`]
//!   frame.rs    the frames an attached client sends: [`frame`], [`resize_frame`]
//!   tests.rs
//! ```

mod frame;
mod message;
mod request;
mod session;
#[cfg(test)]
mod tests;

pub use frame::{FRAME_INPUT, FRAME_MAX, FRAME_RESIZE, frame, resize_frame};
pub use message::{Message, MessageKind, Payload, USER, Verdict};
pub use request::{RECV_DEFAULT_MS, Request, Response};
pub use session::{BUSY_WINDOW_MS, CLAUDE_IDLE_MARK, Claude, PIPELINE_KIND, SHELL_KIND, Session, Status, SubAgent, TITLE_SEP, pipeline_title, split_pipeline_title};

/// Bumped whenever a change here would confuse the other side.
pub const PROTOCOL: u32 = 3;
/// The `nex-host` version, which nex compares to its own to decide whether to reinstall.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
/// How often `watch` resends the session list when nothing has changed: a client that hears
/// nothing for longer than this knows the connection is gone.
pub const HEARTBEAT_SECS: u64 = 15;

/// What a daemon that's handing over to a newer nex-host answers a `launch` or `spawn`
/// with: the client starts the new daemon and asks it again.
pub const RESTARTING: &str = "nex-host is restarting";

/// What `nex-host --version` prints: `nex-host 0.12.2 (protocol 3)`.
pub fn version_line() -> String {
    format!("nex-host {VERSION} (protocol {PROTOCOL})")
}

/// Whether version line `a` is from an older nex-host than `b` (by version, then
/// protocol). `false` when either isn't a version line: only a nex-host known to be older
/// is ever replaced.
pub fn older(a: &str, b: &str) -> bool {
    fn parse(line: &str) -> Option<(Vec<u64>, u32)> {
        let rest = line.trim().strip_prefix("nex-host ")?;
        let (version, protocol) = rest.split_once(" (protocol ")?;
        let protocol = protocol.strip_suffix(')')?.parse().ok()?;
        // `0.12.2`, or `0.13.0-rc1`: the numbers before any suffix.
        let version = version.split(['-', '+']).next()?.split('.').map(|n| n.parse().ok()).collect::<Option<Vec<u64>>>()?;
        Some((version, protocol))
    }
    matches!((parse(a), parse(b)), (Some(a), Some(b)) if a < b)
}

/// What `attach` prints to tell the terminal it's in (nex) which agent it shows:
/// `ESC ] 7700 ; nex-agent=<id> BEL`, and the same with no id when it stops. Other
/// terminals ignore it.
pub const AGENT_TAG: &str = "7700;nex-agent=";

/// The tag sequence for `id` (empty: no agent any more).
pub fn agent_tag(id: &str) -> String {
    format!("\x1b]{AGENT_TAG}{id}\x07")
}
