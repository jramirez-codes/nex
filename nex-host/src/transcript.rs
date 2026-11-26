//! A sub-agent's transcript as terminal text, followed live: what a sub-agent's tab in nex
//! shows. A sub-agent (see [`crate::subagents`]) runs inside its agent's process and has no
//! terminal, only the file it writes as it goes, so its tab prints that file and keeps
//! printing what's added: the task it was given, what it says, each tool call on one line
//! and the start of each result. It's read-only, and says so.
//!
//! Each CLI knows its own file ([`AgentCli::render`](crate::clis::AgentCli::render), in its
//! module under [`crate::clis`]): which lines are the task, what it says, its tool calls and
//! their results, and how to tell it has finished. [`Printer`] is how it prints them, the
//! same for every CLI.
//!
//! [`Renderer`] turns one line of the file into what to print (skipping anything it doesn't
//! know or can't parse); [`follow`] reads the file as it grows. `nex-host subagent` runs it
//! for an agent on an ssh host, a hidden `nex --follow-subagent` for one in a local pane.

use std::io::{self, BufRead, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use crate::clis::Cli;

/// How often [`follow`] looks for more.
const POLL: Duration = Duration::from_millis(300);
/// A tool result's lines shown, and how long a printed line gets.
const RESULT_LINES: usize = 4;
const LINE_CHARS: usize = 200;
/// A tool call's argument, cut to this.
pub const ARG_CHARS: usize = 100;

const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RED: &str = "\x1b[31m";
const GREEN: &str = "\x1b[32m";
const CYAN: &str = "\x1b[36m";
const RESET: &str = "\x1b[0m";

/// The tab's one dim line saying what it is.
pub const READ_ONLY: &str = "Sub-agent transcript \u{00b7} read-only: it follows the agent's file, typing here does nothing";

/// Turns the file's lines into what the tab prints, for the CLI that wrote it.
pub struct Renderer {
    cli: Cli,
    out: Printer,
}

impl Renderer {
    pub fn new(cli: Cli) -> Renderer {
        Renderer { cli, out: Printer::default() }
    }

    /// Whose file it is.
    pub fn cli(&self) -> Cli {
        self.cli
    }

    /// The file itself said the sub-agent has finished.
    pub fn finished(&self) -> bool {
        self.out.finished
    }

    /// What to print for `line` (one JSON line of the file), one string per terminal line.
    /// Malformed or unknown lines print nothing.
    pub fn line(&mut self, line: &str) -> Vec<String> {
        let line = line.trim();
        if line.is_empty() {
            return Vec::new();
        }
        let Ok(v) = serde_json::from_str::<serde_json::Value>(line) else { return Vec::new() };
        self.cli.render(&v, &mut self.out);
        std::mem::take(&mut self.out.lines)
    }
}

/// What a CLI's [`render`](crate::clis::AgentCli::render) prints a transcript's lines with:
/// the same look for every CLI, and nothing from the file acted on (see [`clean`]).
#[derive(Default)]
pub struct Printer {
    lines: Vec<String>,
    /// The task has been printed (the first user message is the task; later ones are
    /// harness reminders, or messages sent to it).
    tasked: bool,
    finished: bool,
}

impl Printer {
    /// Whether the task has been printed yet.
    pub fn tasked(&self) -> bool {
        self.tasked
    }

    /// A message to the sub-agent: the first is its task. Harness text (`<…>`) is skipped.
    pub fn task(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() || text.starts_with('<') {
            return;
        }
        let out = &mut self.lines;
        if self.tasked {
            out.push(String::new());
            out.push(format!("{CYAN}{BOLD}\u{203a} Message{RESET}"));
        } else {
            out.push(format!("{CYAN}{BOLD}\u{203a} Task{RESET}"));
            self.tasked = true;
        }
        out.extend(text.lines().map(|l| format!("  {}", cut(l, LINE_CHARS * 4))));
    }

    /// What the sub-agent says.
    pub fn say(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() {
            return;
        }
        let out = &mut self.lines;
        out.push(String::new());
        // The text often quotes what it read: its escape sequences must not reach the tab.
        let mut lines = text.lines().map(clean);
        if let Some(first) = lines.next() {
            out.push(format!("{BOLD}\u{25cf}{RESET} {first}"));
        }
        out.extend(lines.map(|l| format!("  {l}")));
    }

    /// A tool call, with the argument that says what it does (see [`key_arg`]).
    pub fn call(&mut self, name: &str, arg: Option<String>) {
        let arg = arg.map(|a| format!("{DIM}({a}){RESET}")).unwrap_or_default();
        self.lines.push(format!("{GREEN}\u{25cf}{RESET} {BOLD}{}{RESET}{arg}", cut(name, ARG_CHARS)));
    }

    /// A tool's result: its first few lines.
    pub fn result(&mut self, text: &str, error: bool) {
        let out = &mut self.lines;
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

    /// The file says the sub-agent has finished.
    pub fn finish(&mut self) {
        self.finished = true;
    }
}

/// The text in a message's content: a string, or the `text` of each block in a list.
pub fn text_of(v: Option<&serde_json::Value>) -> String {
    match v {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Array(blocks)) => {
            blocks.iter().filter_map(|b| b.as_str().or_else(|| b.get("text").and_then(|t| t.as_str()))).collect::<Vec<_>>().join("\n")
        }
        Some(serde_json::Value::Object(o)) => o.get("text").and_then(|t| t.as_str()).unwrap_or("").to_string(),
        _ => String::new(),
    }
}

/// The argument that says what a tool call does: its command, file, pattern, address, …,
/// else its first short text. One line, cut short.
pub fn key_arg(input: &serde_json::Value) -> Option<String> {
    const KEYS: [&str; 12] =
        ["command", "cmd", "file_path", "path", "pattern", "query", "url", "description", "skill", "subagent_type", "prompt", "plan"];
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
pub fn cut(s: &str, n: usize) -> String {
    let clean = clean(s);
    if clean.chars().count() <= n { clean } else { clean.chars().take(n.saturating_sub(1)).chain(['\u{2026}']).collect() }
}

/// `s` with tabs as spaces and every other control character dropped (ESC, BEL, the 8-bit
/// C1 ones such as CSI): text from the file is printed, never acted on. A sequence a sub-agent
/// read (`ESC ] 52`, which sets the clipboard; `ESC [ 2J`) loses its ESC and is left as
/// plain characters.
fn clean(s: &str) -> String {
    s.chars().map(|c| if c == '\t' { ' ' } else { c }).filter(|c| !c.is_control()).collect()
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

    /// What its tab is called, as its CLI reads it from the file (`None` until it says).
    pub fn title(&self) -> Option<String> {
        self.cli()?.transcript_title(&self.path)
    }

    /// Whose file it is: known once its first line is (see [`crate::clis::of_transcript`]).
    fn cli(&self) -> Option<Cli> {
        if let Some(r) = &self.renderer {
            return Some(r.cli());
        }
        let f = std::fs::File::open(&self.path).ok()?;
        let mut first = String::new();
        io::BufReader::new(f.take(256 * 1024)).read_line(&mut first).ok()?;
        (!first.trim().is_empty()).then(|| crate::clis::of_transcript(&first))
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
            let r = self.renderer.get_or_insert_with(|| Renderer::new(crate::clis::of_transcript(line)));
            out.extend(r.line(line));
        }
        out
    }

    /// Whether the sub-agent has finished: the file said so, or the file its CLI keeps
    /// beside it does ([`AgentCli::finish_file`](crate::clis::AgentCli::finish_file)). Until
    /// the file has a line, it's taken for Claude Code's.
    pub fn finished(&mut self) -> bool {
        if self.renderer.as_ref().is_some_and(Renderer::finished) {
            return true;
        }
        let cli = self.cli().unwrap_or(crate::clis::CLAUDE);
        let Some(file) = cli.finish_file(&self.path) else { return false };
        let m = mtime(&file);
        if m.is_some() && m != self.meta_seen {
            self.meta_seen = m;
            self.meta_finished = cli.finished_in(&file);
        }
        self.meta_finished
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
        // A CLI may say so in the file itself: known only once it's read.
        if done || f.renderer.as_ref().is_some_and(Renderer::finished) {
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
    // It names the ids it was given.
    write!(out, "\x1b[?25l{DIM}{READ_ONLY}{RESET}\n\n{}\n", clean(why))?;
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

/// What the CLIs' transcript tests share.
#[cfg(test)]
pub(crate) mod testing {
    /// `lines` without their escape sequences (the styling).
    pub(crate) fn strip(lines: &[String]) -> Vec<String> {
        lines
            .iter()
            .map(|l| {
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
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::transcript::testing::strip;

    const TASK: &str = r#"{"parentUuid":null,"isSidechain":true,"agentId":"a1","type":"user","message":{"role":"user","content":"Map the view code\nand report back"},"uuid":"u1"}"#;
    const TEXT: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"thinking","thinking":"hmm"},{"type":"text","text":"Looking at view.rs first."}]}}"#;
    const CALL: &str = r#"{"type":"assistant","message":{"role":"assistant","content":[{"type":"tool_use","id":"t1","name":"Bash","input":{"command":"grep -n fn src/agents/view.rs\nsecond line","description":"Find functions"}}]}}"#;

    #[test]
    fn escape_sequences_in_the_file_never_reach_the_terminal() {
        const EVIL: &str = "x\u{1b}]52;c;aGk=\u{7}y\u{1b}[2J\u{9b}31mz\u{1b}]2;pwned\u{7}";
        // Our own styling is ESC-led too: compare without it.
        let ours = |lines: &[String]| {
            lines.iter().map(|l| [DIM, BOLD, RED, GREEN, CYAN, RESET].iter().fold(l.clone(), |l, s| l.replace(s, ""))).collect::<Vec<_>>()
        };
        let mut r = Renderer::new(crate::clis::CLAUDE);
        let text =
            serde_json::json!({"type":"assistant","message":{"content":[{"type":"text","text":format!("{EVIL}\nsecond {EVIL}")}]}}).to_string();
        let call =
            serde_json::json!({"type":"assistant","message":{"content":[{"type":"tool_use","name":EVIL,"input":{"command":EVIL}}]}}).to_string();
        let task = serde_json::json!({"type":"user","message":{"content":format!("do {EVIL}")}}).to_string();
        let result = serde_json::json!({"type":"user","message":{"content":[{"type":"tool_result","content":EVIL}]}}).to_string();
        let mut codex = Renderer::new(crate::clis::CODEX);
        let codex_text = serde_json::json!({"type":"response_item","payload":{"type":"message","role":"assistant","content":[{"type":"output_text","text":EVIL}]}}).to_string();
        let codex_call = serde_json::json!({"type":"response_item","payload":{"type":"custom_tool_call","name":EVIL,"input":EVIL}}).to_string();
        let mut all = Vec::new();
        for line in [&task, &text, &call, &result] {
            all.extend(r.line(line));
        }
        all.extend(codex.line(&codex_text));
        all.extend(codex.line(&codex_call));
        assert!(all.len() >= 7);
        for l in ours(&all) {
            assert!(!l.chars().any(|c| c.is_control()), "a control character got through: {l:?}");
        }
        assert!(ours(&all).iter().any(|l| l.contains("x]52;c;aGk=y[2J31mz]2;pwned")), "the rest is kept, as plain text");

        let mut out = Vec::new();
        missing(&mut out, EVIL).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert!(!text.contains("\u{7}") && !text.contains("]52;c;aGk=\u{7}") && !text.contains("\u{9b}"));
        assert_eq!(text.matches('\u{1b}').count(), 3, "only our own: hide the cursor, dim, reset");
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
