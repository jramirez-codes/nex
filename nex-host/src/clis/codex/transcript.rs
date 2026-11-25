//! A Codex thread's rollout, as a sub-agent's transcript: `session_meta` first, then
//! `response_item`s (messages, tool calls and their output) and `event_msg`s, one of which,
//! `task_complete`, says it has finished.

use crate::transcript::{ARG_CHARS, Printer, cut, key_arg, text_of};

/// Whether `first_line` is a rollout's: its `session_meta`.
pub(super) fn is_head(first_line: &str) -> bool {
    match serde_json::from_str::<serde_json::Value>(first_line) {
        Ok(v) => v.get("type").and_then(|t| t.as_str()) == Some("session_meta"),
        Err(_) => false,
    }
}

pub(super) fn render(v: &serde_json::Value, out: &mut Printer) {
    let Some(p) = v.get("payload") else { return };
    let kind = p.get("type").and_then(|t| t.as_str());
    match v.get("type").and_then(|t| t.as_str()) {
        Some("event_msg") if kind == Some("task_complete") => out.finish(),
        Some("response_item") => match kind {
            Some("message") => {
                let text = text_of(p.get("content"));
                match p.get("role").and_then(|r| r.as_str()) {
                    Some("user") => out.task(&text),
                    Some("assistant") => out.say(&text),
                    _ => {}
                }
            }
            Some("function_call") | Some("custom_tool_call") => {
                let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                // A function call's arguments are a JSON string; a custom tool's input is text.
                let args = p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
                let arg = args.as_ref().and_then(key_arg).or_else(|| p.get("input").and_then(|i| i.as_str()).map(|i| cut(i.lines().next().unwrap_or(""), ARG_CHARS)));
                out.call(name, arg);
            }
            Some("local_shell_call") => {
                let cmd = p.get("action").and_then(|a| a.get("command")).map(|c| match c {
                    serde_json::Value::Array(words) => words.iter().filter_map(|w| w.as_str()).collect::<Vec<_>>().join(" "),
                    other => other.as_str().unwrap_or("").to_string(),
                });
                out.call("shell", cmd.map(|c| cut(&c, ARG_CHARS)));
            }
            Some("function_call_output") | Some("custom_tool_call_output") => {
                let output = p.get("output");
                // Sometimes the output is itself JSON with an `output` field.
                let text = match output.and_then(|o| o.as_str()).and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) {
                    Some(inner) if inner.get("output").is_some() => text_of(inner.get("output")),
                    _ => text_of(output),
                };
                out.result(&text, false);
            }
            _ => {}
        },
        _ => {}
    }
}

#[cfg(test)]
mod tests {
    use crate::clis::CODEX;
    use crate::transcript::Renderer;
    use crate::transcript::testing::strip;

    const CLAUDE_TASK: &str = r#"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"Map the view code"},"uuid":"u1"}"#;

    #[test]
    fn codex_rollouts_read_the_same_way() {
        let mut r = Renderer::new(CODEX);
        let head = r#"{"type":"session_meta","payload":{"id":"k1","cwd":"/w","source":{"subagent":{"thread_spawn":{"parent_thread_id":"p1"}}},"agent_role":"explorer"}}"#;
        assert_eq!(crate::clis::of_transcript(head), CODEX);
        assert_eq!(crate::clis::of_transcript(CLAUDE_TASK), crate::clis::CLAUDE);
        assert!(r.line(head).is_empty());
        let env = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#;
        assert!(r.line(env).is_empty());
        let task = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Find the bug"}]}}"#;
        assert_eq!(strip(&r.line(task)), ["\u{203a} Task", "  Find the bug"]);
        let call = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"]}","call_id":"c1"}}"#;
        assert_eq!(strip(&r.line(call)), ["\u{25cf} shell(bash -lc ls)"]);
        let output = r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"output\":\"a\\nb\",\"metadata\":{}}"}}"#;
        assert_eq!(strip(&r.line(output)), ["  \u{2514} a", "    b"]);
        assert!(!r.finished());
        assert!(r.line(r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#).is_empty());
        assert!(r.finished());
    }

}
