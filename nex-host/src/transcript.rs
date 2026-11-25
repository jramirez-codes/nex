//! A sub-agent's transcript as terminal text, followed live: what a sub-agent's tab in nex
//! shows. A sub-agent (see [`crate::subagents`]) runs inside its agent's process and has no
//! terminal, only the file it writes as it goes, so its tab prints that file and keeps
//! printing what's added: the task it was given, what it says, each tool call on one line
//! and the start of each result. It's read-only, and says so.
//!
//! - Claude Code: `agent-<id>.jsonl`, finished once `agent-<id>.meta.json` beside it says
//!   `finished: true`.
//! - Codex (best effort): its rollout file, finished at its `task_complete` event.
//!
//! [`Renderer`] turns one line of the file into what to print (skipping anything it doesn't
//! know or can't parse); [`follow`] reads the file as it grows. `nex-host subagent` runs it
//! for an agent on an ssh host, a hidden `nex --follow-subagent` for one in a local pane.

use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// How often [`follow`] looks for more.
const POLL: Duration = Duration::from_millis(300);
/// A tool result's lines shown, and how long a printed line gets.
const RESULT_LINES: usize = 4;
const LINE_CHARS: usize = 200;
/// A tool call's argument, cut to this.
const ARG_CHARS: usize = 100;

const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// The tab's one dim line saying what it is.
pub const READ_ONLY: &str = "Sub-agent transcript \u{00b7} read-only: it follows the agent's file, typing here does nothing";

/// Whose file it is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Claude,
    Codex,
}

impl Format {
    /// A Codex rollout starts with its `session_meta`; anything else is taken for Claude
    /// Code's.
    pub fn sniff(first_line: &str) -> Format {
        match serde_json::from_str::<serde_json::Value>(first_line) {
            Ok(v) if v.get("type").and_then(|t| t.as_str()) == Some("session_meta") => Format::Codex,
            _ => Format::Claude,
        }
    }
}

/// Turns the file's lines into what the tab prints.
pub struct Renderer {
    format: Format,
    /// The task has been printed (the first user message is the task; later ones are
    /// harness reminders, or messages sent to it).
    tasked: bool,
    /// Codex said its task is complete.
    pub finished: bool,
}

impl Renderer {
    pub fn new(format: Format) -> Renderer {
        Renderer { format, tasked: false, finished: false }
    }

    /// What to print for `line` (one JSON line of the file), one string per terminal line.
    /// Malformed or unknown lines print nothing.
    pub fn line(&mut self, line: &str) -> Vec<String> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return Vec::new() };
        let mut out = Vec::new();
        match self.format {
            Format::Claude => self.claude(&v, &mut out),
            Format::Codex => self.codex(&v, &mut out),
        }
        out
    }

    fn task(&mut self, text: &str, out: &mut Vec<String>) {
        let text = text.trim();
        if text.is_empty() || text.starts_with('<') {
            return;
        }
        if self.tasked {
            out.push(String::new());
            out.push(format!("{CYAN}{BOLD}\u{203a} Message{RESET}"));
        } else {
            out.push(format!("{CYAN}{BOLD}\u{203a} Task{RESET}"));
            self.tasked = true;
        }
        out.extend(text.lines().map(|l| format!("  {}", cut(l, LINE_CHARS * 4))));
    }

    fn say(text: &str, out: &mut Vec<String>) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        out.push(String::new());
        let mut lines = text.lines();
        if let Some(first) = lines.next() {
            out.push(format!("{BOLD}\u{25cf}{RESET} {first}"));
        }
        out.extend(lines.map(|l| format!("  {l}")));
    }

    fn call(name: &str, arg: Option<String>, out: &mut Vec<String>) {
        let arg = arg.map(|a| format!("{DIM}({a}){RESET}")).unwrap_or_default();
        out.push(format!("{GREEN}\u{25cf}{RESET} {BOLD}{name}{RESET}{arg}"));
    }

    fn result(text: &str, error: bool, out: &mut Vec<String>) {
        let lines: Vec<&str> = text.lines().filter(|l| !l.trim().is_empty()).collect();
        let color = if error { RED } else { DIM };
        if lines.is_empty() {
            out.push(format!("  {color}\u{2514} (no output){RESET}"));
            return;
        }
        for (i, l) in lines.iter().take(RESULT_LINES).enumerate() {
            let lead = if i == 0 { "\u{2514} " } else { "  " };
            out.push(format!("  {color}{lead}{}{RESET}", cut(l, LINE_CHARS)));
        }
        if lines.len() > RESULT_LINES {
            out.push(format!("  {DIM}  \u{2026} +{} lines{RESET}", lines.len() - RESULT_LINES));
        }
    }

    fn claude(&mut self, v: &serde_json::Value, out: &mut Vec<String>) {
        let Some(content) = v.get("message").and_then(|m| m.get("content")) else { return };
        match v.get("type").and_then(|t| t.as_str()) {
            Some("user") => match content {
                serde_json::Value::String(s) => self.task(s, out),
                serde_json::Value::Array(blocks) => {
                    for b in blocks {
                        match b.get("type").and_then(|t| t.as_str()) {
                            Some("tool_result") => {
                                let error = b.get("is_error").and_then(|e| e.as_bool()).unwrap_or(false);
                                Self::result(&text_of(b.get("content")), error, out);
                            }
                            Some("text") if !self.tasked => self.task(b.get("text").and_then(|t| t.as_str()).unwrap_or(""), out),
                            _ => {}
                        }
                    }
                }
                _ => {}
            },
            Some("assistant") => {
                for b in content.as_array().into_iter().flatten() {
                    match b.get("type").and_then(|t| t.as_str()) {
                        Some("text") => Self::say(b.get("text").and_then(|t| t.as_str()).unwrap_or(""), out),
                        Some("tool_use") => {
                            let name = b.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                            Self::call(name, b.get("input").and_then(key_arg), out);
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn codex(&mut self, v: &serde_json::Value, out: &mut Vec<String>) {
        let Some(p) = v.get("payload") else { return };
        let kind = p.get("type").and_then(|t| t.as_str());
        match v.get("type").and_then(|t| t.as_str()) {
            Some("event_msg") if kind == Some("task_complete") => self.finished = true,
            Some("response_item") => match kind {
                Some("message") => {
                    let text = text_of(p.get("content"));
                    match p.get("role").and_then(|r| r.as_str()) {
                        Some("user") => self.task(&text, out),
                        Some("assistant") => Self::say(&text, out),
                        _ => {}
                    }
                }
                Some("function_call") | Some("custom_tool_call") => {
                    let name = p.get("name").and_then(|n| n.as_str()).unwrap_or("tool");
                    // A function call's arguments are a JSON string; a custom tool's input is text.
                    let args = p.get("arguments").and_then(|a| a.as_str()).and_then(|a| serde_json::from_str::<serde_json::Value>(a).ok());
                    let arg = args.as_ref().and_then(key_arg).or_else(|| p.get("input").and_then(|i| i.as_str()).map(|i| cut(i.lines().next().unwrap_or(""), ARG_CHARS)));
                    Self::call(name, arg, out);
                }
                Some("local_shell_call") => {
                    let cmd = p.get("action").and_then(|a| a.get("command")).map(|c| match c {
                        serde_json::Value::Array(words) => words.iter().filter_map(|w| w.as_str()).collect::<Vec<_>>().join(" "),
                        other => other.as_str().unwrap_or("").to_string(),
                    });
                    Self::call("shell", cmd.map(|c| cut(&c, ARG_CHARS)), out);
                }
                Some("function_call_output") | Some("custom_tool_call_output") => {
                    let output = p.get("output");
                    // Sometimes the output is itself JSON with an `output` field.
                    let text = match output.and_then(|o| o.as_str()).and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok()) {
                        Some(inner) if inner.get("output").is_some() => text_of(inner.get("output")),
                        _ => text_of(output),
                    };
                    Self::result(&text, false, out);
                }
                _ => {}
            },
            _ => {}
        }
    }
}

/// The text in a message's content: a string, or the `text` of each block in a list.
fn text_of(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|b| b.as_str().or_else(|| b.get("text").and_then(|t| t.as_str())))
            .collect::<Vec<_>>()
            .join("\n"),
        Some(serde_json::Value::Object(o)) => o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// The argument that says what a tool call does: its command, file, pattern, address, …,
/// else its first short text. One line, cut short.
fn key_arg(input: &serde_json::Value) -> Option<String> {
    const KEYS: [&str; 12] = ["command", "cmd", "file_path", "path", "pattern", "query", "url", "description", "skill", "subagent_type", "prompt", "plan"];
    let obj = input.as_object()?;
    let text = |v: &serde_json::Value| match v {
        serde_json::Value::String(s) => Some(s.clone()),
        serde_json::Value::Array(a) => Some(a.iter().filter_map(|w| w.as_str()).collect::<Vec<_>>().join(" ")).filter(|s| !s.is_empty()),
        _ => None,
    };
    let found = KEYS.iter().find_map(|k| obj.get(*k).and_then(text)).or_else(|| obj.values().find_map(text))?;
    let first = found.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    (!first.is_empty()).then(|| cut(first, ARG_CHARS))
}

/// `s` cut to `n` characters, with an ellipsis when it was longer. Control characters (a
/// result's own escape sequences) are dropped: they'd restyle the tab.
fn cut(s: &str, n: usize) -> String {
    let clean: String = s.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect();
    if clean.chars().count() <= n { clean } else { clean.chars().take(n.saturating_sub(1)).chain(['\u{2026}']).collect() }
}

/// What the meta file beside a Claude Code transcript says: its description, and whether it
/// finished.
fn claude_meta(transcript: &Path) -> (Option<String>, bool) {
    let meta = claude_meta_path(transcript);
    let Some(v) = std::fs::read_to_string(meta).ok().and_then(|t| serde_json::from_str::<serde_json::Value>(&t).ok()) else { return (None, false) };
    let s = |k: &str| v.get(k).and_then(|d| d.as_str()).map(str::trim).filter(|d| !d.is_empty()).map(str::to_string);
    (s("description").or_else(|| s("agentType")), v.get("finished").and_then(|f| f.as_bool()).unwrap_or(false))
}

fn claude_meta_path(transcript: &Path) -> PathBuf {
    let name = transcript.file_name().and_then(|n| n.to_str()).unwrap_or("");
    transcript.with_file_name(format!("{}.meta.json", name.strip_suffix(".jsonl").unwrap_or(name)))
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

/// A transcript file, read as it grows.
pub struct Follower {
    path: PathBuf,
    offset: u64,
    /// The end of a line not yet written in full.
    partial: Vec<u8>,
    renderer: Option<Renderer>,
    meta_seen: Option<SystemTime>,
    meta_finished: bool,
}

impl Follower {
    pub fn new(path: PathBuf) -> Follower {
        Follower { path, offset: 0, partial: Vec::new(), renderer: None, meta_seen: None, meta_finished: false }
    }

    /// What its tab is called: Claude Code's description of the task, else Codex's name
    /// for the thread (`None` until the file says).
    pub fn title(&self) -> Option<String> {
        match self.format()? {
            Format::Claude => claude_meta(&self.path).0,
            Format::Codex => {
                let head = std::fs::File::open(&self.path).ok().and_then(|f| io::BufReader::new(f).lines().next()?.ok())?;
                let h = crate::subagents::parse_codex_head(&head)?;
                h.name.or(h.role)
            }
        }
    }

    fn format(&self) -> Option<Format> {
        if let Some(r) = &self.renderer {
            return Some(r.format);
        }
        let f = std::fs::File::open(&self.path).ok()?;
        let mut first = String::new();
        io::BufReader::new(f.take(256 * 1024)).read_line(&mut first).ok()?;
        (!first.trim().is_empty()).then(|| Format::sniff(&first))
    }

    /// The lines added since last time, rendered (complete lines only).
    pub fn poll(&mut self) -> Vec<String> {
        let Ok(mut f) = std::fs::File::open(&self.path) else { return Vec::new() };
        let len = f.metadata().map(|m| m.len()).unwrap_or(0);
        if len < self.offset {
            // Rewritten from the start: read it again.
            self.offset = 0;
            self.partial.clear();
        }
        if f.seek(SeekFrom::Start(self.offset)).is_err() {
            return Vec::new();
        }
        let mut buf = Vec::new();
        let Ok(n) = f.take(len - self.offset).read_to_end(&mut buf) else { return Vec::new() };
        self.offset += n as u64;
        self.partial.extend_from_slice(&buf);
        let Some(end) = self.partial.iter().rposition(|&b| b == b'\n') else { return Vec::new() };
        let complete: Vec<u8> = self.partial.drain(..=end).collect();
        let text = String::from_utf8_lossy(&complete);
        let mut out = Vec::new();
        for line in text.lines() {
            let r = self.renderer.get_or_insert_with(|| Renderer::new(Format::sniff(line)));
            out.extend(r.line(line));
        }
        out
    }

    /// Whether the sub-agent has finished: its meta file says so (Claude Code), or its
    /// rollout did (Codex).
    pub fn finished(&mut self) -> bool {
        match self.renderer.as_ref().map(|r| r.format) {
            Some(Format::Codex) => self.renderer.as_ref().is_some_and(|r| r.finished),
            _ => {
                let meta = claude_meta_path(&self.path);
                let m = mtime(&meta);
                if m.is_some() && m != self.meta_seen {
                    self.meta_seen = m;
                    self.meta_finished = claude_meta(&self.path).1;
                }
                self.meta_finished
            }
        }
    }
}

/// The line printed once it has finished.
pub fn finished_line() -> String {
    format!("{GREEN}{BOLD}\u{2714} Finished{RESET}{DIM} \u{00b7} the transcript stays here; close the tab when you're done{RESET}")
}

/// Print `path`'s transcript to `out` and follow it until the sub-agent finishes (or
/// `stop()` says to), titling the terminal `title` (else what the file says). Then the
/// finished line; the caller keeps the output on screen.
pub fn follow(path: &Path, title: Option<&str>, out: &mut impl Write, stop: impl Fn() -> bool) -> io::Result<()> {
    let mut f = Follower::new(path.to_path_buf());
    let name = title.map(str::to_string).or_else(|| f.title()).unwrap_or_else(|| "Sub-agent".into());
    // The title, the cursor hidden (nothing is typed here), and the read-only line.
    write!(out, "\x1b]2;{}\x07\x1b[?25l{DIM}{READ_ONLY}{RESET}\n\n", cut(&name, 120))?;
    out.flush()?;
    loop {
        // Checked before reading, so lines written just before it finished still print.
        let done = f.finished();
        let lines = f.poll();
        for l in &lines {
            writeln!(out, "{l}")?;
        }
        if !lines.is_empty() {
            out.flush()?;
        }
        // Codex says so in the file itself: known only once it's read.
        if done || f.renderer.as_ref().is_some_and(|r| r.format == Format::Codex && r.finished) {
            writeln!(out, "\n{}", finished_line())?;
            return out.flush();
        }
        if stop() {
            return Ok(());
        }
        std::thread::sleep(POLL);
    }
}

/// Print a one-line explanation for a sub-agent whose transcript can't be found.
pub fn missing(out: &mut impl Write, why: &str) -> io::Result<()> {
    write!(out, "\x1b[?25l{DIM}{READ_ONLY}{RESET}\n\n{why}\n")?;
    out.flush()
}

/// Keep the tab's output on screen until the terminal goes away: what's typed is read and
/// dropped, without echo (unix).
pub fn hold() {
    #[cfg(unix)]
    quiet_stdin();
    let mut buf = [0u8; 1024];
    let mut stdin = io::stdin().lock();
    while matches!(stdin.read(&mut buf), Ok(n) if n > 0) {}
}

/// Turn off echo and line editing on the terminal, so typing doesn't look like input.
#[cfg(unix)]
pub fn quiet_stdin() {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) == 0 {
            t.c_lflag &= !(libc::ECHO | libc::ICANON);
            let _ = libc::tcsetattr(0, libc::TCSANOW, &t);
        }
    }
}

/// The whole tab: the transcript at `path` followed to its end, then held on screen.
/// `None`: it couldn't be found; `why` says so instead.
pub fn run(path: Option<&Path>, why: &str) -> io::Result<i32> {
    #[cfg(unix)]
    quiet_stdin();
    let mut out = io::stdout().lock();
    match path {
        Some(p) => follow(p, None, &mut out, || false)?,
        None => missing(&mut out, why)?,
    }
    drop(out);
    hold();
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strip(lines: &[String]) -> Vec<String> {
        lines.iter().map(|l| {
            let mut out = String::new();
            let mut chars = l.chars().peekable();
            while let Some(c) = chars.next() {
                if c == '\x1b' {
                    for d in chars.by_ref() {
                        if d.is_ascii_alphabetic() {
                            break;
                        }
                    }
                } else {
                    out.push(c);
                }
            }
            out
        }).collect()
    }

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
        let mut r = Renderer::new(Format::Claude);
        assert_eq!(strip(&r.line(TASK)), ["\u{203a} Task", "  Map the view code", "  and report back"]);
        assert!(r.line(REMINDER).is_empty(), "harness reminders aren't the task");
        assert_eq!(strip(&r.line(TEXT)), ["", "\u{25cf} Looking at view.rs first."], "thinking isn't shown");
        assert_eq!(strip(&r.line(CALL)), ["\u{25cf} Bash(grep -n fn src/agents/view.rs)"], "one line: the command's first");
        let read = r#"{"type":"assistant","message":{"content":[{"type":"tool_use","name":"Read","input":{"file_path":"/w/src/lib.rs","limit":20}}]}}"#;
        assert_eq!(strip(&r.line(read)), ["\u{25cf} Read(/w/src/lib.rs)"]);
    }

    #[test]
    fn a_long_tool_result_is_cut_to_a_few_lines() {
        let mut r = Renderer::new(Format::Claude);
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
        let mut r = Renderer::new(Format::Claude);
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
    fn codex_rollouts_read_the_same_way() {
        let mut r = Renderer::new(Format::Codex);
        let head = r#"{"type":"session_meta","payload":{"id":"k1","cwd":"/w","source":{"subagent":{"thread_spawn":{"parent_thread_id":"p1"}}},"agent_role":"explorer"}}"#;
        assert_eq!(Format::sniff(head), Format::Codex);
        assert_eq!(Format::sniff(TASK), Format::Claude);
        assert!(r.line(head).is_empty());
        let env = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"<environment_context>x</environment_context>"}]}}"#;
        assert!(r.line(env).is_empty());
        let task = r#"{"type":"response_item","payload":{"type":"message","role":"user","content":[{"type":"input_text","text":"Find the bug"}]}}"#;
        assert_eq!(strip(&r.line(task)), ["\u{203a} Task", "  Find the bug"]);
        let call = r#"{"type":"response_item","payload":{"type":"function_call","name":"shell","arguments":"{\"command\":[\"bash\",\"-lc\",\"ls\"]}","call_id":"c1"}}"#;
        assert_eq!(strip(&r.line(call)), ["\u{25cf} shell(bash -lc ls)"]);
        let output = r#"{"type":"response_item","payload":{"type":"function_call_output","call_id":"c1","output":"{\"output\":\"a\\nb\",\"metadata\":{}}"}}"#;
        assert_eq!(strip(&r.line(output)), ["  \u{2514} a", "    b"]);
        assert!(!r.finished);
        assert!(r.line(r#"{"type":"event_msg","payload":{"type":"task_complete"}}"#).is_empty());
        assert!(r.finished);
    }

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nex-transcript-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn following_prints_what_is_added_and_stops_once_finished() {
        let dir = tmp("follow");
        let path = dir.join("agent-a1.jsonl");
        let meta = dir.join("agent-a1.meta.json");
        std::fs::write(&meta, r#"{"agentType":"Explore","description":"Map the view code"}"#).unwrap();
        // Half a line: not printed until it's whole.
        std::fs::write(&path, format!("{TASK}\n{}", &TEXT[..20])).unwrap();
        let mut f = Follower::new(path.clone());
        assert_eq!(f.title().as_deref(), Some("Map the view code"));
        assert_eq!(strip(&f.poll()).len(), 3);
        assert!(!f.finished());
        let mut whole = std::fs::read_to_string(&path).unwrap();
        whole.push_str(&TEXT[20..]);
        whole.push_str("\ngarbage\n");
        std::fs::write(&path, &whole).unwrap();
        assert_eq!(strip(&f.poll()), ["", "\u{25cf} Looking at view.rs first."]);

        // It finishes: the rest is printed, then the finished line, and `follow` returns.
        whole.push_str(CALL);
        whole.push('\n');
        std::fs::write(&path, &whole).unwrap();
        std::fs::write(&meta, r#"{"agentType":"Explore","description":"Map the view code","finished":true}"#).unwrap();
        let mut out = Vec::new();
        follow(&path, None, &mut out, || panic!("it finished: no waiting")).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b]2;Map the view code\x07"), "titled with its description");
        assert!(text.contains(READ_ONLY));
        assert!(text.contains("Bash"), "the last call made it");
        assert!(strip(&[text.clone()])[0].trim_end().ends_with("close the tab when you're done"), "{text}");
        assert!(text.contains("Finished"));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn following_waits_while_it_works() {
        let dir = tmp("wait");
        let path = dir.join("agent-a2.jsonl");
        std::fs::write(&path, format!("{TASK}\n")).unwrap();
        std::fs::write(dir.join("agent-a2.meta.json"), r#"{"description":"Plan"}"#).unwrap();
        let polls = std::cell::Cell::new(0);
        let mut out = Vec::new();
        follow(&path, Some("Given"), &mut out, || {
            polls.set(polls.get() + 1);
            polls.get() > 2
        })
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(text.contains("\x1b]2;Given\x07") && text.contains("Map the view code"));
        assert!(!text.contains("Finished"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
