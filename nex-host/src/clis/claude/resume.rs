//! Restoring a Claude Code pane picks up the same conversation: its command is rewritten to
//! `claude --resume <id>`, with the id from the session file of the process that ran there
//! (see [`super::session`]). Anything unexpected falls back to the id in the command line,
//! then to `claude --continue` (the folder's latest conversation).

use super::session::valid_id;

/// Flags carried over to the resumed command, each followed by a value.
const VALUE_FLAGS: &[&str] = &["--model", "--permission-mode", "--add-dir", "--settings", "--mcp-config", "--agent"];
/// Flags carried over that take no value.
const BOOL_FLAGS: &[&str] = &["--dangerously-skip-permissions", "--verbose", "--ide", "--chrome"];
/// Flags naming a conversation, each followed by its id.
const ID_FLAGS: &[&str] = &["--resume", "-r", "--session-id"];

/// `argv` rewritten to resume conversation `id` (from the running process), else the id the
/// command line already named, else the folder's latest one. The prompt and `-p` are
/// dropped so restoring doesn't send the first message again; a few settings carry over.
/// `argv` must be Claude Code's.
pub fn resume_argv(argv: &[String], id: Option<&str>) -> Vec<String> {
    let n = crate::clis::CLAUDE.program_len(argv).unwrap_or(1).min(argv.len());
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

#[cfg(test)]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    const ID: &str = "f216e068-1eb3-47fc-a19f-b8739e6274e7";

    #[test]
    fn recognizes_claude_code() {
        let is_claude = |argv: &[String]| crate::clis::CLAUDE.program_len(argv).is_some();
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
}
