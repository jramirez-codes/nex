//! The agent CLIs ([`nex_host::clis`]) as they run in this machine's own panes: the same
//! modules nex-host uses on ssh hosts, with their files under this machine's home folder
//! ([`crate::platform::home_dir`]) rather than `$HOME`.

use std::path::PathBuf;

use nex_host::clis::{Cli, Running};
use nex_host::proto::{Claude, SubAgent};

fn home(cli: Cli) -> Option<PathBuf> {
    cli.home_in(crate::platform::home_dir())
}

/// What the session file of `cli`'s process `pid` says, for a CLI that keeps one.
pub fn session(cli: Cli, pid: u32) -> Option<Claude> {
    let start = crate::platform::process_start(pid);
    cli.session(&home(cli)?, pid, start.as_deref())
}

/// The sub-agents `cli`'s process `pid` (in `cwd`, its session file saying `session`) is
/// running, or just finished.
pub fn subagents(cli: Cli, pid: u32, cwd: Option<&std::path::Path>, session: Option<&Claude>) -> Vec<SubAgent> {
    let Some(home) = home(cli) else { return Vec::new() };
    let cwd = cwd.map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    cli.subagents(&home, &Running { pid, cwd: &cwd, session }, nex_host::subagents::now_ms())
}

/// The transcript of sub-agent `id` of `cli`'s process `pid`.
pub fn subagent_transcript(cli: Cli, pid: u32, cwd: Option<&std::path::Path>, id: &str) -> Option<PathBuf> {
    let home = home(cli)?;
    let session = session(cli, pid);
    let cwd = cwd.map(|c| c.to_string_lossy().into_owned()).unwrap_or_default();
    cli.subagent_transcript(&home, &Running { pid, cwd: &cwd, session: session.as_ref() }, id)
}

/// The command to restore a pane whose foreground job `pid` ran `argv` with: a CLI's own
/// rewrite to resume its conversation (`AgentCli::resume`), if it has one.
pub fn resume_command(argv: &[String], pid: u32) -> Option<Vec<String>> {
    let cli = nex_host::clis::detect(argv)?;
    let id = session(cli, pid).and_then(|s| s.session_id);
    cli.resume(argv, id.as_deref())
}
