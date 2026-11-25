//! What every registered CLI must do, and what a new one gets from [`AgentCli`]'s defaults.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

use super::*;

fn v(s: &[&str]) -> Vec<String> {
    s.iter().map(|s| s.to_string()).collect()
}

fn tmp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("nex-clis-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

#[test]
fn every_cli_is_found_by_its_kind_and_is_named() {
    let mut kinds = HashSet::new();
    for cli in ALL {
        assert!(kinds.insert(cli.kind()), "{} is registered twice", cli.kind());
        assert_eq!(by_kind(cli.kind()), Some(*cli));
        assert!(!cli.label().is_empty() && !cli.script().is_empty());
        assert!(!cli.home_var().is_empty() && cli.home_folder().starts_with('.'));
    }
    assert_eq!(by_kind("vim"), None);
    assert_eq!(by_kind(crate::proto::PIPELINE_KIND), None, "a pipeline isn't a CLI");
    assert_eq!(by_kind(crate::proto::SHELL_KIND), None);
    assert!(ALL.contains(&DEFAULT));
}

#[test]
fn every_cli_is_told_by_its_command_line() {
    for cli in ALL {
        let k = cli.kind();
        let script = format!("/usr/lib/node_modules/{}", cli.script());
        assert_eq!(cli.program_len(&v(&[k])), Some(1));
        assert_eq!(cli.program_len(&v(&[&format!("/home/u/.local/bin/{k}"), "--flag"])), Some(1));
        assert_eq!(cli.program_len(&v(&["node", &script, "hi"])), Some(2));
        assert_eq!(cli.program_len(&v(&["/usr/bin/bun", &format!("/x/{k}")])), Some(2));
        assert_eq!(cli.program_len(&v(&[&format!("{k}-monitor")])), None);
        assert_eq!(cli.program_len(&v(&["node", "server.js"])), None);
        assert_eq!(cli.program_len(&[]), None);
        assert_eq!(detect(&v(&[k])), Some(*cli));
        assert_eq!(detect(&v(&["node", &script])), Some(*cli));
        // No other CLI takes its command line.
        for other in ALL.iter().filter(|o| *o != cli) {
            assert_eq!(other.program_len(&v(&[k])), None, "{} takes {k}", other.kind());
            assert_eq!(other.program_len(&v(&["node", &script])), None, "{} takes {k}'s script", other.kind());
        }
    }
    assert_eq!(detect(&v(&["vim", "claude"])), None);
}

#[test]
fn every_cli_has_its_home_in_the_users() {
    for cli in ALL {
        // Unless the environment running the tests names one itself.
        if std::env::var_os(cli.home_var()).is_none() {
            assert_eq!(cli.home_in(Some("/home/u".into())), Some(Path::new("/home/u").join(cli.home_folder())));
            assert_eq!(cli.home_in(None), None);
        }
    }
}

#[test]
fn nothing_in_an_empty_home() {
    let home = tmp("empty");
    let session = Claude { session_id: Some("4c4075db-1701-4574-82f5-7b6f9e0e0882".into()), cwd: Some("/w".into()), ..Claude::default() };
    for cli in ALL {
        for s in [None, Some(&session)] {
            let agent = Running { pid: 0, cwd: "/w", session: s };
            assert!(cli.subagents(&home, &agent, crate::subagents::now_ms()).is_empty(), "{}", cli.kind());
            assert_eq!(cli.subagent_transcript(&home, &agent, "a1"), None);
            assert_eq!(cli.subagent_transcript(&home, &agent, "../../etc/passwd"), None, "never a path");
        }
        assert_eq!(cli.session(&home, 1, None), None);
    }
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn transcripts_go_to_the_cli_that_wrote_them() {
    let codex_head = r#"{"type":"session_meta","payload":{"id":"k1","cwd":"/w"}}"#;
    let claude_line = r#"{"type":"user","message":{"role":"user","content":"Find the bug"}}"#;
    assert_eq!(of_transcript(codex_head), CODEX);
    assert_eq!(of_transcript(claude_line), CLAUDE);
    assert_eq!(of_transcript("not json"), CLAUDE, "anything unclaimed is Claude Code's");
    // At most one CLI claims a first line.
    for line in [codex_head, claude_line, "{}", "garbage"] {
        assert!(ALL.iter().filter(|c| c.claims_transcript(line)).count() <= 1, "{line}");
    }
}

#[test]
fn every_cli_prints_nothing_for_junk() {
    for cli in ALL {
        let mut r = crate::transcript::Renderer::new(*cli);
        for junk in ["not json", "{}", "[]", "null", r#"{"type":"user"}"#, r#"{"payload":7}"#, r#"{"type":"response_item","payload":{}}"#] {
            assert!(r.line(junk).is_empty(), "{}: {junk}", cli.kind());
        }
        assert!(!r.finished());
        assert_eq!(cli.transcript_title(Path::new("/nowhere/agent-x.jsonl")), None);
    }
}

#[test]
fn what_the_screen_says() {
    assert_eq!(CLAUDE.title_status(claude::IDLE_MARK), Some(Status::Idle));
    assert_eq!(CLAUDE.title_status("⠋"), Some(Status::Working));
    assert_eq!(CODEX.title_status("👩\u{200d}💻"), None, "Codex's title doesn't say");
    assert!(!CLAUDE.dialog_marks().is_empty());
    assert!(CODEX.dialog_marks().is_empty());
}

#[test]
fn spawning_and_restoring() {
    for cli in ALL {
        assert!(cli.task_on_command_line(), "{}", cli.kind());
    }
    assert_eq!(CLAUDE.report_args(), ["--allowedTools", "Bash(nex-host:*)"]);
    assert!(CODEX.report_args().is_empty());
    assert_eq!(CLAUDE.resume(&v(&["claude"]), None), Some(v(&["claude", "--continue"])));
    assert_eq!(CODEX.resume(&v(&["codex"]), None), None, "run again as it was");
}

#[test]
fn every_manual_has_a_place_of_its_own() {
    let mut paths = HashSet::new();
    for cli in ALL {
        if let Some(m) = cli.manual("MANUAL") {
            let (Manual::File { path, text } | Manual::Block { path, text }) = m;
            assert!(paths.insert((cli.home_folder(), path)));
            assert!(text.contains("MANUAL"));
            assert!(!path.starts_with('/') && !path.contains(".."));
        }
    }
}

#[test]
fn handles_compare_and_hash_by_kind() {
    assert_eq!(CLAUDE, Cli(&claude::ClaudeCode));
    assert_ne!(CLAUDE, CODEX);
    let set: HashSet<Cli> = [CLAUDE, CODEX, CLAUDE].into_iter().collect();
    assert_eq!(set.len(), 2);
    assert_eq!(format!("{CODEX:?}"), "Cli(codex)");
}

/// A CLI added with only what it must say: everything else is the default, and does
/// nothing it wasn't told to.
struct Gemini;

impl AgentCli for Gemini {
    fn kind(&self) -> &'static str {
        "gemini"
    }
    fn label(&self) -> &'static str {
        "Gemini CLI"
    }
    fn script(&self) -> &'static str {
        "@google/gemini-cli/dist/index.js"
    }
    fn home_var(&self) -> &'static str {
        "NEX_TEST_GEMINI_HOME"
    }
    fn home_folder(&self) -> &'static str {
        ".gemini"
    }
    fn subagents(&self, _home: &Path, _agent: &Running, _now: u64) -> Vec<SubAgent> {
        Vec::new()
    }
    fn subagent_transcript(&self, _home: &Path, _agent: &Running, _id: &str) -> Option<PathBuf> {
        None
    }
    fn render(&self, line: &serde_json::Value, out: &mut Printer) {
        if let Some(text) = line.get("text").and_then(|t| t.as_str()) {
            out.say(text);
        }
    }
    fn transcript_title(&self, _path: &Path) -> Option<String> {
        None
    }
}

#[test]
fn a_new_cli_needs_only_the_required_parts() {
    let g = Cli(&Gemini);
    assert_eq!(g.program_len(&v(&["gemini", "-p", "hi"])), Some(1));
    assert_eq!(g.program_len(&v(&["node", "/usr/lib/node_modules/@google/gemini-cli/dist/index.js"])), Some(2));
    assert_eq!(g.home_in(Some("/home/u".into())), Some(PathBuf::from("/home/u/.gemini")));
    assert_eq!(g.session(Path::new("/"), 1, None), None);
    assert!(!g.claims_transcript(r#"{"type":"session_meta"}"#));
    assert_eq!(g.finish_file(Path::new("/x.jsonl")), None);
    assert_eq!(g.title_status("✳"), None);
    assert!(g.dialog_marks().is_empty() && g.report_args().is_empty());
    assert!(g.task_on_command_line());
    assert_eq!(g.resume(&v(&["gemini"]), Some("id")), None);
    assert_eq!(g.manual("MANUAL"), None);
    let mut r = crate::transcript::Renderer::new(g);
    assert_eq!(crate::transcript::testing::strip(&r.line(r#"{"text":"hello"}"#)), ["", "\u{25cf} hello"]);
    assert!(!r.finished());
    // Not registered: nothing else knows it.
    assert_eq!(by_kind("gemini"), None);
    assert_eq!(detect(&v(&["gemini"])), None);
}
