//! Claude Code's session file: `<home>/sessions/<pid>.json`, one per running process,
//! with its conversation id, display name, status and folder. It's Claude Code's own state,
//! not an interface, so anything unexpected in it is ignored.

use std::path::Path;

use crate::proto::Claude;

/// Read `<dir>/<pid>.json` (see [`parse`]).
pub fn read_in(dir: &Path, pid: u32, start: Option<&str>) -> Option<Claude> {
    parse(&std::fs::read_to_string(dir.join(format!("{pid}.json"))).ok()?, pid, start)
}

/// A session file's contents, if it's for process `pid`. When the process's start time is
/// known (`start`), the file must name the same one, so a file left by an earlier process
/// with a reused pid is ignored.
pub fn parse(text: &str, pid: u32, start: Option<&str>) -> Option<Claude> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
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
    Some(Claude {
        session_id: text("sessionId").filter(|id| valid_id(id)),
        name: text("name"),
        status: text("status"),
        cwd: text("cwd"),
    })
}

/// A conversation id is a UUID; anything else is ignored.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-')
}

#[cfg(test)]
mod tests {
    use super::*;

    const ID: &str = "f216e068-1eb3-47fc-a19f-b8739e6274e7";

    #[test]
    fn reads_what_nex_uses() {
        let text = format!(r#"{{"pid":7,"sessionId":"{ID}","name":"nex","status":"busy","cwd":"/home/u/nex","procStart":"123"}}"#);
        let c = parse(&text, 7, Some("123")).unwrap();
        assert_eq!(c.session_id.as_deref(), Some(ID));
        assert_eq!(c.name.as_deref(), Some("nex"));
        assert_eq!(c.status.as_deref(), Some("busy"));
        assert_eq!(c.cwd.as_deref(), Some("/home/u/nex"));
        assert_eq!(parse(&text, 7, None), Some(c));
    }

    #[test]
    fn ignores_other_processes_and_junk() {
        let text = r#"{"pid":7,"procStart":"123","sessionId":"not a uuid!"}"#;
        assert_eq!(parse(text, 8, None), None, "another pid");
        assert_eq!(parse(text, 7, Some("999")), None, "a reused pid");
        assert_eq!(parse(text, 7, None).unwrap().session_id, None);
        assert_eq!(parse("not json", 7, None), None);
    }

    #[test]
    fn reads_the_session_file() {
        let dir = std::env::temp_dir().join(format!("nex-claude-sessions-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = format!(r#"{{"pid":4242,"sessionId":"{ID}","procStart":"3887066"}}"#);
        std::fs::write(dir.join("4242.json"), file).unwrap();
        let id = |pid, start| read_in(&dir, pid, start).and_then(|c| c.session_id);
        assert_eq!(id(4242, Some("3887066")).as_deref(), Some(ID));
        assert_eq!(id(4242, None).as_deref(), Some(ID));
        // Another process that reused the pid.
        assert_eq!(id(4242, Some("99")), None);
        assert_eq!(id(4243, None), None);
        std::fs::write(dir.join("4244.json"), r#"{"pid":1,"sessionId":"abc"}"#).unwrap();
        assert_eq!(id(4244, None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_bad_id_keeps_the_rest() {
        let text = r#"{"pid":78,"sessionId":"nope!","name":"nex-2e","status":"busy"}"#;
        let c = parse(text, 78, None).unwrap();
        assert_eq!((c.session_id, c.name.as_deref(), c.status.as_deref()), (None, Some("nex-2e"), Some("busy")));
    }
}
