//! Claude Code panes come back on the same conversation: the saved command is rewritten to
//! `claude --resume <id>`, with the id read from the session file Claude Code keeps for
//! each running process (`~/.claude/sessions/<pid>.json`). That file is Claude Code's own
//! state, not an interface, so anything unexpected in it falls back to the id in the
//! command line, then to `claude --continue` (the folder's latest conversation).

use std::path::{Path, PathBuf};

/// Flags carried over to the resumed command, each followed by a value.
const VALUE_FLAGS: &[&str] = &["--model", "--permission-mode", "--add-dir", "--settings", "--mcp-config", "--agent"];
/// Flags carried over that take no value.
const BOOL_FLAGS: &[&str] = &["--dangerously-skip-permissions", "--verbose", "--ide", "--chrome"];
/// Flags naming a conversation, each followed by its id.
const ID_FLAGS: &[&str] = &["--resume", "-r", "--session-id"];

/// How many leading words of `argv` are the program: `claude` (1) or `node …/claude` (2).
/// `None` if it isn't Claude Code.
fn program_len(argv: &[String]) -> Option<usize> {
    let base = |s: &str| s.rsplit('/').next().unwrap_or(s).to_string();
    match argv {
        [first, ..] if base(first) == "claude" => Some(1),
        [runtime, script, ..]
            if matches!(base(runtime).as_str(), "node" | "bun")
                && (base(script) == "claude" || script.ends_with("@anthropic-ai/claude-code/cli.js")) =>
        {
            Some(2)
        }
        _ => None,
    }
}

pub fn is_claude(argv: &[String]) -> bool {
    program_len(argv).is_some()
}

/// A conversation id is a UUID; anything else in the file or argv is ignored.
fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

/// `argv` rewritten to resume conversation `id` (from the running process), else the id the
/// command line already named, else the folder's latest one. The prompt and `-p` are
/// dropped so restoring doesn't send the first message again; a few settings carry over.
/// `argv` must be Claude Code's (see [`is_claude`]).
pub fn resume_argv(argv: &[String], id: Option<&str>) -> Vec<String> {
    let n = program_len(argv).unwrap_or(1).min(argv.len());
    let mut out: Vec<String> = argv[..n].to_vec();
    let mut kept = Vec::new();
    let mut named = None;
    let mut args = argv[n..].iter();
    while let Some(arg) = args.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v.to_string())),
            _ => (arg.as_str(), None),
        };
        if ID_FLAGS.contains(&flag) {
            let value = inline.or_else(|| args.next().cloned());
            if let Some(v) = value.filter(|v| valid_id(v)) {
                named = Some(v);
            }
        } else if VALUE_FLAGS.contains(&flag) {
            if let Some(v) = inline.or_else(|| args.next().cloned()) {
                kept.push(flag.to_string());
                kept.push(v);
            }
        } else if BOOL_FLAGS.contains(&flag) {
            kept.push(flag.to_string());
        }
    }
    match id.filter(|i| valid_id(i)).map(str::to_string).or(named) {
        Some(id) => out.extend(["--resume".to_string(), id]),
        None => out.push("--continue".to_string()),
    }
    out.extend(kept);
    out
}

/// Claude Code's config folder: `$CLAUDE_CONFIG_DIR`, else `~/.claude`.
fn config_dir() -> Option<PathBuf> {
    match std::env::var_os("CLAUDE_CONFIG_DIR").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => Some(p),
        None => Some(crate::platform::home_dir()?.join(".claude")),
    }
}

/// What Claude Code says about one of its running processes, from its session file.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct SessionInfo {
    /// The conversation id (a UUID), if the file names a valid one.
    pub id: Option<String>,
    /// The session's display name (Claude Code derives one from the folder, or the user
    /// renames it).
    pub name: Option<String>,
    /// What it is doing, in Claude Code's own words (`idle`, `busy`, …).
    pub status: Option<String>,
}

/// The conversation the Claude Code process `pid` is in, if it says.
pub fn session_id(pid: u32) -> Option<String> {
    session_info(pid)?.id
}

/// Everything the session file of the Claude Code process `pid` says that nex uses.
pub fn session_info(pid: u32) -> Option<SessionInfo> {
    session_info_in(&config_dir()?.join("sessions"), pid, proc_start(pid).as_deref())
}

#[cfg(test)]
fn session_id_in(dir: &Path, pid: u32, start: Option<&str>) -> Option<String> {
    session_info_in(dir, pid, start)?.id
}

/// Read `<dir>/<pid>.json`. When the process's start time is known (`start`), the file must
/// name the same one, so a file left by an earlier process with a reused pid is ignored.
fn session_info_in(dir: &Path, pid: u32, start: Option<&str>) -> Option<SessionInfo> {
    let text = std::fs::read_to_string(dir.join(format!("{pid}.json"))).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    if v.get("pid")?.as_u64()? != u64::from(pid) {
        return None;
    }
    if let Some(start) = start {
        let saved = v.get("procStart")?;
        let saved = saved.as_str().map(str::to_string).or_else(|| saved.as_u64().map(|n| n.to_string()))?;
        if saved != start {
            return None;
        }
    }
    let text = |key: &str| {
        v.get(key)
            .and_then(|s| s.as_str())
            .map(str::trim)
            .filter(|s| !s.is_empty() && s.len() <= 200)
            .map(str::to_string)
    };
    Some(SessionInfo {
        id: text("sessionId").filter(|id| valid_id(id)),
        name: text("name"),
        status: text("status"),
    })
}

/// The process's start time in clock ticks since boot (`/proc/<pid>/stat` field 22), as
/// Claude Code records it. `None` where there's no `/proc`.
fn proc_start(pid: u32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // Fields after `(comm)` start at field 3 (state), so field 22 is the 20th of them.
    stat[stat.rfind(')')? + 1..].split_whitespace().nth(19).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    const ID: &str = "f216e068-1eb3-47fc-a19f-b8739e6274e7";

    #[test]
    fn recognizes_claude_code() {
        assert!(is_claude(&v(&["claude"])));
        assert!(is_claude(&v(&["/home/u/.local/bin/claude", "--model", "opus"])));
        assert!(is_claude(&v(&["node", "/usr/lib/node_modules/@anthropic-ai/claude-code/cli.js"])));
        assert!(is_claude(&v(&["/usr/bin/node", "/home/u/.npm/bin/claude"])));
        assert!(!is_claude(&v(&["node", "server.js"])));
        assert!(!is_claude(&v(&["claude-monitor"])));
        assert!(!is_claude(&[]));
    }

    #[test]
    fn rewrites_to_resume() {
        assert_eq!(resume_argv(&v(&["claude"]), Some(ID)), v(&["claude", "--resume", ID]));
        assert_eq!(resume_argv(&v(&["claude"]), None), v(&["claude", "--continue"]));
        // The prompt and print mode are dropped, settings carry over.
        assert_eq!(
            resume_argv(&v(&["claude", "--model", "opus", "-p", "fix the bug", "--dangerously-skip-permissions"]), Some(ID)),
            v(&["claude", "--resume", ID, "--model", "opus", "--dangerously-skip-permissions"])
        );
        assert_eq!(resume_argv(&v(&["claude", "--model=opus"]), Some(ID)), v(&["claude", "--resume", ID, "--model", "opus"]));
        // The running process's id wins over one in the command line; that one is the fallback.
        let old = "0e0b5e34-4ef7-4649-8c9d-009ff74902eb";
        assert_eq!(resume_argv(&v(&["claude", "--resume", old]), Some(ID)), v(&["claude", "--resume", ID]));
        assert_eq!(resume_argv(&v(&["claude", "-r", old]), None), v(&["claude", "--resume", old]));
        assert_eq!(resume_argv(&v(&["claude", "--continue"]), None), v(&["claude", "--continue"]));
        // A node shim keeps its interpreter.
        assert_eq!(resume_argv(&v(&["node", "/x/claude", "hi"]), Some(ID)), v(&["node", "/x/claude", "--resume", ID]));
        // Garbage ids are ignored.
        assert_eq!(resume_argv(&v(&["claude"]), Some("$(rm -rf ~)")), v(&["claude", "--continue"]));
    }

    #[test]
    fn reads_the_session_file() {
        let dir = std::env::temp_dir().join(format!("nex-claude-sessions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = format!(r#"{{"pid":4242,"sessionId":"{ID}","procStart":"3887066"}}"#);
        std::fs::write(dir.join("4242.json"), file).unwrap();
        assert_eq!(session_id_in(&dir, 4242, Some("3887066")).as_deref(), Some(ID));
        assert_eq!(session_id_in(&dir, 4242, None).as_deref(), Some(ID));
        // Another process that reused the pid.
        assert_eq!(session_id_in(&dir, 4242, Some("99")), None);
        assert_eq!(session_id_in(&dir, 4243, None), None);
        std::fs::write(dir.join("4244.json"), r#"{"pid":1,"sessionId":"abc"}"#).unwrap();
        assert_eq!(session_id_in(&dir, 4244, None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn reads_status_and_name() {
        let dir = std::env::temp_dir().join(format!("nex-claude-info-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = format!(r#"{{"pid":77,"sessionId":"{ID}","name":"nex-2e","status":"idle","procStart":"5"}}"#);
        std::fs::write(dir.join("77.json"), file).unwrap();
        let info = session_info_in(&dir, 77, Some("5")).unwrap();
        assert_eq!(info, SessionInfo { id: Some(ID.into()), name: Some("nex-2e".into()), status: Some("idle".into()) });
        // A bad id doesn't hide the rest.
        std::fs::write(dir.join("78.json"), r#"{"pid":78,"sessionId":"nope!","status":"busy"}"#).unwrap();
        let info = session_info_in(&dir, 78, None).unwrap();
        assert_eq!((info.id, info.status.as_deref()), (None, Some("busy")));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_own_start_time() {
        assert!(proc_start(std::process::id()).is_some_and(|s| s.parse::<u64>().is_ok()));
    }
}
