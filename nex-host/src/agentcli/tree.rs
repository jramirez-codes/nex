//! `ls`: the sessions as a tree.

use nex_host::proto::Session;

use crate::agentcli::me;

/// The sessions as a tree: each agent under the one that spawned it.
pub fn tree(sessions: &[Session]) -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let me = me();
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    let mut out = Vec::new();
    fn walk(s: &Session, all: &[Session], depth: usize, home: &str, me: Option<&str>, out: &mut Vec<String>) {
        let cwd = match s.cwd.strip_prefix(home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => s.cwd.clone(),
        };
        let indent = if depth == 0 { String::new() } else { format!("{}└ ", "  ".repeat(depth - 1)) };
        let role = s.role.as_ref().filter(|r| **r != s.name).map(|r| format!(" ({r})")).unwrap_or_default();
        let mut marks = String::new();
        if s.attached {
            marks.push_str("  (attached)");
        }
        if me == Some(s.id.as_str()) {
            marks.push_str("  ← you");
        }
        out.push(format!("{}  {:<9}  {:<8}  {indent}{}{role}  {cwd}{marks}", s.id, s.status.label(), s.kind, s.name));
        for c in all.iter().filter(|c| c.parent.as_deref() == Some(&s.id)) {
            walk(c, all, depth + 1, home, me, out);
        }
    }
    for s in sessions.iter().filter(|s| s.parent.as_deref().is_none_or(|p| !ids.contains(&p))) {
        walk(s, sessions, 0, &home, me.as_deref(), &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use nex_host::proto::Status;

    use super::*;

    fn s(id: &str, name: &str, parent: Option<&str>) -> Session {
        Session {
            id: id.into(),
            kind: "claude".into(),
            name: name.into(),
            cwd: "/x".into(),
            argv: vec![],
            pid: 1,
            started: 0,
            title: None,
            bell: false,
            attached: false,
            last_output: 0,
            last_input: 0,
            exit: None,
            claude: None,
            running: None,
            status: Status::Idle,
            parent: parent.map(Into::into),
            role: None,
            last_message: 0,
            subagents: Vec::new(),
        }
    }

    #[test]
    fn children_sit_under_their_parent() {
        let t = tree(&[s("a", "lead", None), s("b", "other", None), s("c", "build", Some("a")), s("d", "tests", Some("c")), s("e", "orphan", Some("gone"))]);
        let order: Vec<bool> = ["  lead  ", "  └ build  ", "    └ tests  ", "  other  ", "  orphan  "].iter().zip(&t).map(|(want, line)| line.contains(want)).collect();
        assert_eq!(order, [true; 5], "{t:#?}");
    }
}
