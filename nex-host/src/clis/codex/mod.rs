//! Codex (`codex`, the npm package `@openai/codex`), with its files in `$CODEX_HOME`, else
//! `~/.codex`. It keeps no session file nex reads.
//!
//! - rollouts.rs    its threads' rollout files: the sub-agents a thread spawned
//! - transcript.rs  a rollout, for a sub-agent's tab

use std::io::{self, BufRead};
use std::path::{Path, PathBuf};

use crate::clis::{AgentCli, Manual, Running};
use crate::proto::SubAgent;
use crate::transcript::Printer;

mod rollouts;
mod transcript;

pub use rollouts::{list, rollout};

pub struct Codex;

impl AgentCli for Codex {
    fn kind(&self) -> &'static str {
        "codex"
    }

    fn label(&self) -> &'static str {
        "Codex"
    }

    fn script(&self) -> &'static str {
        "@openai/codex/bin/codex.js"
    }

    fn home_var(&self) -> &'static str {
        "CODEX_HOME"
    }

    fn home_folder(&self) -> &'static str {
        ".codex"
    }

    fn subagents(&self, home: &Path, agent: &Running, now: u64) -> Vec<SubAgent> {
        rollouts::list(home, agent.pid, agent.cwd, now)
    }

    fn subagent_transcript(&self, home: &Path, _agent: &Running, id: &str) -> Option<PathBuf> {
        rollouts::rollout(home, id)
    }

    fn claims_transcript(&self, first_line: &str) -> bool {
        transcript::is_head(first_line)
    }

    fn render(&self, line: &serde_json::Value, out: &mut Printer) {
        transcript::render(line, out)
    }

    /// The thread's name, else its role, from the rollout's first line.
    fn transcript_title(&self, path: &Path) -> Option<String> {
        let head = std::fs::File::open(path).ok().and_then(|f| io::BufReader::new(f).lines().next()?.ok())?;
        let h = rollouts::parse_head(&head)?;
        h.name.or(h.role)
    }

    fn manual(&self, manual: &str) -> Option<Manual> {
        Some(Manual::Block { path: "AGENTS.md", text: manual.to_string() })
    }
}
