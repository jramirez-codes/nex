//! Claude Code (`claude`, the npm package `@anthropic-ai/claude-code`), with its files in
//! `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
//!
//! - session.rs     its session file, `sessions/<pid>.json`: conversation, name, status
//! - subagents.rs   its Task tool's sub-agents, under `projects/<folder>/<session id>/`
//! - transcript.rs  a sub-agent's transcript, for its tab
//! - resume.rs      its command line rewritten to resume the conversation (restoring a pane)
//!
//! Its terminal title leads with [`IDLE_MARK`] at its prompt and a spinner frame while it
//! works, and before it has a session file, asking the user something (trusting the folder,
//! a new MCP server, a tool's permission) shows only on its screen ([`DIALOG`]).

use std::path::{Path, PathBuf};

use crate::clis::{AgentCli, Manual, Running};
use crate::proto::{Claude, Status, SubAgent};
use crate::transcript::Printer;

mod resume;
mod session;
mod subagents;
mod transcript;

pub use resume::resume_argv;
pub use session::{parse, read_in, valid_id};

/// Claude Code's title glyph while it waits at its prompt; spinner frames replace it while
/// it works.
pub const IDLE_MARK: &str = "✳";

/// What Claude Code's dialogs show: asking to trust the folder, a choice confirmed with
/// Enter (a new MCP server, settings), a tool's permission prompt, and the Esc hint they
/// all end with.
pub const DIALOG: &[&str] = &["trust this folder", "Enter to confirm", "Do you want to proceed?", "Esc to cancel"];

pub struct ClaudeCode;

impl AgentCli for ClaudeCode {
    fn kind(&self) -> &'static str {
        "claude"
    }

    fn label(&self) -> &'static str {
        "Claude Code"
    }

    fn script(&self) -> &'static str {
        "@anthropic-ai/claude-code/cli.js"
    }

    fn home_var(&self) -> &'static str {
        "CLAUDE_CONFIG_DIR"
    }

    fn home_folder(&self) -> &'static str {
        ".claude"
    }

    fn session(&self, home: &Path, pid: u32, start: Option<&str>) -> Option<Claude> {
        read_in(&home.join("sessions"), pid, start)
    }

    fn subagents(&self, home: &Path, agent: &Running, now: u64) -> Vec<SubAgent> {
        agent.session.map(|s| subagents::list(home, s, now)).unwrap_or_default()
    }

    fn subagent_transcript(&self, home: &Path, agent: &Running, id: &str) -> Option<PathBuf> {
        subagents::transcript(home, agent.session?, id)
    }

    fn render(&self, line: &serde_json::Value, out: &mut Printer) {
        transcript::render(line, out)
    }

    fn transcript_title(&self, path: &Path) -> Option<String> {
        transcript::meta(path).0
    }

    fn finish_file(&self, path: &Path) -> Option<PathBuf> {
        Some(transcript::meta_path(path))
    }

    fn finished_in(&self, file: &Path) -> bool {
        transcript::read_meta(file).1
    }

    fn title_status(&self, mark: &str) -> Option<Status> {
        Some(if mark == IDLE_MARK { Status::Idle } else { Status::Working })
    }

    fn dialog_marks(&self) -> &'static [&'static str] {
        DIALOG
    }

    fn report_args(&self) -> &'static [&'static str] {
        &["--allowedTools", "Bash(nex-host:*)"]
    }

    fn resume(&self, argv: &[String], session_id: Option<&str>) -> Option<Vec<String>> {
        Some(resume_argv(argv, session_id))
    }

    fn manual(&self, manual: &str) -> Option<Manual> {
        Some(Manual::File {
            path: "skills/nex/SKILL.md",
            text: format!(
                "---\nname: nex\ndescription: Delegate to and talk with other coding agents through nex-host (spawn helpers, send messages, get their reports, run pipelines). Use when $NEX_AGENT_ID is set and the work would go better split between agents, or when a <nex-message> block arrives in your input.\n---\n\n{manual}"
            ),
        })
    }
}
