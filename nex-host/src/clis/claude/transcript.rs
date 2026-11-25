//! A Claude Code sub-agent's transcript, `agent-<id>.jsonl`: one JSON line per message
//! (`user`: the task, then tool results; `assistant`: text and tool calls). It has finished
//! once `agent-<id>.meta.json` beside it says `finished: true`; that file also has its
//! description, the tab's title.

use std::path::{Path, PathBuf};

use crate::transcript::{Printer, key_arg, text_of};

pub(super) fn render(v: &serde_json::Value, out: &mut Printer) {
    let Some(content) = v.get("message").and_then(|m| m.get("content")) else { return };
    match v.get("type").and_then(|t| t.as_str()) {
        Some("user") => match content {
            serde_json::Value::String(s) => out.task(s),
            serde_json::Value::Array(blocks) => {
                for b in blocks {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("tool_result") => {
                            let error = b.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                            out.result(&text_of(b.get("content")), error);
                        }
                        Some("text") if !out.tasked() => out.task(b.get("text").and_then(|t| t.as_str()).unwrap_or("")),
                        _ => {}
                    }
                }
            }
            _ => {}
        },
        Some("assistant") => {
            for b in content.as_array().into_iter().flatten() {
                match b.get("type").and_then(|t| t.as_str()) {
                    Some("text") => out.say(b.get("text").and_then(|t| t.as_str()).unwrap_or("")),
                    Some("tool_use") => {
                        let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                        out.call(name, b.get("input").and_then(key_arg));
                    }
                    _ => {}
                }
            }
        }
        _ => {}
    }
}

/// What the meta file beside a transcript says: its description, and whether it finished.
pub(super) fn meta(transcript: &Path) -> (Option<String>, bool) {
    read_meta(&meta_path(transcript))
}

/// What the meta file `file` says (see [`meta`]).
pub(super) fn read_meta(file: &Path) -> (Option<String>, bool) {
    let Some(v) = std::fs::read_to_string(file).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return (None, false) };
    let s = |k: &str| v.get(k).and_then(|d| d.as_str()).map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
    (s("description").or_else(|| s("agentType")), v.get("finished").and_then(|f| f.as_bool()).unwrap_or(false))
}

/// `agent-<id>.meta.json`, beside `agent-<id>.jsonl`.
pub(super) fn meta_path(transcript: &Path) -> PathBuf {
    let name = transcript.file_name().and_then(|n| n.to_str()).unwrap_or("");
    transcript.with_file_name(format!("{}.meta.json", name.strip_suffix(".jsonl").unwrap_or(name)))
}

#[cfg(test)]
mod tests {
    use crate::clis::CLAUDE;
    use crate::transcript::Renderer;
    use crate::transcript::testing::strip;

    const TASK: &str = r#"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"Map the view code\nand report back"},"uuid":"u1"}"#;
    const REMINDER: &str = r#"{"type":"user","message":{"role":"user","content":"<system-reminder>\nignore me\n</system-reminder>"}}"#;
    const TEXT: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Looking at view.rs first."}]}}"#;
    const CALL: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"grep -n fn src/agents/view.rs\nsecond line","description":"Find functions"}}]}}"#;

    fn long_result() -> String {
        let text: Vec<String> = (1..=10).map(|i| format!("line {i}")).collect();
        serde_json::json!({"type":"user","message":{"role":"user","content":[{"type":"tool_result","tool_use_id":"t1","content":text.join("\n")}]}}).to_string()
    }

    #[test]
    fn claude_codes_task_text_and_tool_calls() {
        let mut r = Renderer::new(CLAUDE);
        assert_eq!(strip(&r.line(TASK)), ["\u{203a} Task", "  Map the view code", "  and report back"]);
        assert!(r.line(REMINDER).is_empty(), "harness reminders aren't the task");
        assert_eq!(strip(&r.line(TEXT)), ["", "\u{25cf} Looking at view.rs first."], "thinking isn't shown");
        assert_eq!(strip(&r.line(CALL)), ["\u{25cf} Bash(grep -n fn src/agents/view.rs)"], "one line: the command's first");
        let read = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/w/src/lib.rs","limit":20}}]}}"#;
        assert_eq!(strip(&r.line(read)), ["\u{25cf} Read(/w/src/lib.rs)"]);
    }


    #[test]
    fn a_long_tool_result_is_cut_to_a_few_lines() {
        let mut r = Renderer::new(CLAUDE);
        assert_eq!(strip(&r.line(&long_result())), ["  \u{2514} line 1", "    line 2", "    line 3", "    line 4", "    \u{2026} +6 lines"]);
        let blocks = r#"{"type":"user","message":{"content":[{"type":"tool_result","is_error":true,"content":[{"type":"text","text":"boom"}]}]}}"#;
        assert_eq!(strip(&r.line(blocks)), ["  \u{2514} boom"]);
        let wide = serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":"x".repeat(500)}]}}).to_string();
        let got = strip(&r.line(&wide));
        assert_eq!(got.len(), 1);
        assert!(got[0].chars().count() < 220 && got[0].ends_with('\u{2026}'));
    }


    #[test]
    fn malformed_and_unknown_lines_print_nothing() {
        let mut r = Renderer::new(CLAUDE);
        for junk in ["not json", "{\"type\":", "[]", "{}", r#"{"type":"attachment","attachment":{}}"#, r#"{"type":"assistant","message":{"content":"odd"}}"#, r#"{"type":"assistant","message":{"content":[{"type":"tool_use"}]}}"#] {
            let out = r.line(junk);
            assert!(out.iter().all(|l| !l.contains("panic")), "{junk}");
        }
        assert!(r.line("not json").is_empty());
        assert!(r.line(r#"{"type":"attachment","attachment":{}}"#).is_empty());
        // A tool call with no input still reads.
        assert_eq!(strip(&r.line(r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Glob"}]}}"#)), ["\u{25cf} Glob"]);
    }


    #[test]
    fn the_meta_file_sits_beside_the_transcript() {
        let p = std::path::Path::new("/c/subagents/agent-a1.jsonl");
        assert_eq!(super::meta_path(p), std::path::Path::new("/c/subagents/agent-a1.meta.json"));
        assert_eq!(super::meta(std::path::Path::new("/nowhere/agent-x.jsonl")), (None, false));
    }
}
