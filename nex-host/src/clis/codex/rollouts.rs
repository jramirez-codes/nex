//! Codex's spawned threads (best effort): each thread is a rollout file under
//! `<home>/sessions/YYYY/MM/DD/`, whose first line (`session_meta`) names the thread that
//! spawned it. A thread has finished once its rollout's last line is a `task_complete`.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::proto::SubAgent;
use crate::subagents::{CACHE_MAX, Cached, DONE_FOR_MS, STALE_MS, cached, clean, first_line, ms, valid_sub_id};

#[derive(Default)]
struct Cache {
    heads: HashMap<PathBuf, Cached<Head>>,
}

fn cache() -> MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if c.heads.len() > CACHE_MAX {
        *c = Cache::default();
    }
    c
}

/// The rollout of the Codex thread `id`, with Codex's home at `home`: in the newest day
/// folders, named after it or saying so on its first line.
pub fn rollout(home: &Path, id: &str) -> Option<PathBuf> {
    if !valid_sub_id(id) {
        return None;
    }
    let mut rollouts: Vec<PathBuf> = days(home)
        .into_iter()
        .flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path()))
        .filter(|p| is_rollout(p))
        .collect();
    rollouts.sort();
    rollouts.reverse();
    let named = rollouts.iter().find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&format!("-{id}.jsonl"))));
    named.cloned().or_else(|| rollouts.into_iter().find(|p| first_line(p).and_then(|l| parse_head(&l)).is_some_and(|h| h.id == id)))
}

/// What a rollout's first line (`session_meta`) says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Head {
    pub(super) id: String,
    cwd: Option<String>,
    /// The thread that spawned it, for a sub-agent.
    parent: Option<String>,
    pub(super) name: Option<String>,
    pub(super) role: Option<String>,
}

fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m.get(key).or_else(|| m.values().find_map(|v| find_key(v, key))),
        _ => None,
    }
}

pub(super) fn parse_head(line: &str) -> Option<Head> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = v.get("payload")?;
    let s = |v: Option<&serde_json::Value>| v.and_then(|s| s.as_str()).and_then(clean);
    let source = p.get("source");
    let parent = source.and_then(|s| find_key(s, "parent_thread_id")).and_then(|s| s.as_str()).map(str::to_string);
    let pick = |k: &str| s(p.get(k)).or_else(|| s(source.and_then(|src| find_key(src, k))));
    Some(Head { id: s(p.get("id"))?, cwd: s(p.get("cwd")), parent, name: pick("agent_nickname"), role: pick("agent_role") })
}

/// Whether the rollout's last line says the thread finished its task.
fn finished(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    let Ok(mut f) = std::fs::File::open(path) else { return false };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let _ = f.seek(SeekFrom::Start(len.saturating_sub(8192)));
    let mut tail = String::new();
    let _ = f.read_to_string(&mut tail);
    tail.lines().rev().find(|l| !l.trim().is_empty()).is_some_and(|l| l.contains("\"task_complete\""))
}

fn is_rollout(p: &Path) -> bool {
    p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("rollout-") && n.ends_with(".jsonl"))
}

/// The newest day folders under `<home>/sessions` (yesterday's too, across midnight).
fn days(home: &Path) -> Vec<PathBuf> {
    let newest = |dir: &Path, n: usize| -> Vec<PathBuf> {
        let mut v: Vec<PathBuf> = std::fs::read_dir(dir).into_iter().flatten().flatten().map(|e| e.path()).filter(|p| p.is_dir()).collect();
        v.sort();
        v.into_iter().rev().take(n).collect()
    };
    let mut days = Vec::new();
    for y in newest(&home.join("sessions"), 2) {
        for m in newest(&y, 2) {
            days.extend(newest(&m, 2));
        }
    }
    days.sort();
    days.into_iter().rev().take(2).collect()
}

/// The rollout the Codex process `pid` is writing, if it has one open (Linux).
fn open_rollout(pid: u32) -> Option<PathBuf> {
    std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten().filter_map(|e| std::fs::read_link(e.path()).ok()).find(|p| is_rollout(p))
}

/// The sub-agents the Codex process `pid`, working in `cwd`, spawned, with Codex's home at
/// `home`.
pub fn list(home: &Path, pid: u32, cwd: &str, now: u64) -> Vec<SubAgent> {
    let mut cache = cache();
    let mut recent = Vec::new();
    for day in days(home) {
        for e in std::fs::read_dir(day).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(mtime) = e.metadata().ok().and_then(|m| m.modified().ok()) else { continue };
            if !is_rollout(&p) || now.saturating_sub(ms(mtime)) > STALE_MS {
                continue;
            }
            // The first line never changes once written: read it once.
            let head = match cache.heads.get(&p) {
                Some(c) if c.value.is_some() => c.value.clone(),
                _ => cached(&mut cache.heads, &p, mtime, |p| parse_head(&first_line(p)?)),
            };
            if let Some(h) = head {
                recent.push((p, mtime, h));
            }
        }
    }
    let open = open_rollout(pid);
    let me = match open.as_ref().and_then(|o| recent.iter().find(|(p, _, _)| p == o)) {
        Some((_, _, h)) => h.id.clone(),
        None if open.is_some() => match open.as_deref().and_then(first_line).and_then(|l| parse_head(&l)) {
            Some(h) => h.id,
            None => return Vec::new(),
        },
        // Not one we can see open: the newest top-level thread in its folder.
        None => match recent.iter().filter(|(_, _, h)| h.parent.is_none() && h.cwd.as_deref() == Some(cwd)).max_by_key(|(_, t, _)| *t) {
            Some((_, _, h)) => h.id.clone(),
            None => return Vec::new(),
        },
    };
    let mut out = Vec::new();
    for (p, mtime, h) in &recent {
        if h.parent.as_deref() != Some(me.as_str()) {
            continue;
        }
        let done = finished(p);
        if done && now.saturating_sub(ms(*mtime)) > DONE_FOR_MS {
            continue;
        }
        let started = std::fs::metadata(p).ok().and_then(|m| m.created().ok()).map_or(ms(*mtime), ms);
        out.push(SubAgent {
            id: h.id.clone(),
            name: h.name.clone().or_else(|| h.role.clone()).unwrap_or_else(|| "Sub-agent".into()),
            role: h.role.clone(),
            started,
            done,
        });
    }
    out.sort_by(|a, b| (a.started, &a.id).cmp(&(b.started, &b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagents::now_ms;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nex-subagents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn head_line(id: &str, cwd: &str, parent: Option<&str>, role: Option<&str>) -> String {
        let source = match parent {
            Some(p) => format!(r#"{{"subagent":{{"thread_spawn":{{"parent_thread_id":"{p}","depth":1}}}}}}"#),
            None => r#""cli""#.into(),
        };
        let role = role.map(|r| format!(r#","agent_role":"{r}""#)).unwrap_or_default();
        format!(r#"{{"timestamp":"2026-11-23T10:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","source":{source}{role}}}}}"#)
            + "\n"
    }

    #[test]
    fn reads_codexs_spawned_threads() {
        let home = tmp("codex");
        let day = home.join("sessions/2026/11/23");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-1-parent.jsonl"), head_line("p1", "/w", None, None)).unwrap();
        std::fs::write(day.join("rollout-2-kid.jsonl"), head_line("k1", "/w", Some("p1"), Some("explorer"))).unwrap();
        let done = head_line("k2", "/w", Some("p1"), Some("worker")) + r#"{"type":"event_msg","payload":{"type":"task_complete"}}"# + "\n";
        std::fs::write(day.join("rollout-3-kid.jsonl"), done).unwrap();
        std::fs::write(day.join("rollout-4-other.jsonl"), head_line("k3", "/w", Some("someone-else"), None)).unwrap();
        // pid 0 has no open files: found by its folder.
        let subs = list(&home, 0, "/w", now_ms());
        let got: Vec<(&str, &str, bool)> = subs.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.done)).collect();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.contains(&("k1", "explorer", false)));
        assert!(got.contains(&("k2", "worker", true)));
        assert!(list(&home, 0, "/nowhere", now_ms()).is_empty());
        // Each thread's rollout, by its id: in its name, else on its first line.
        assert_eq!(rollout(&home, "kid"), Some(day.join("rollout-3-kid.jsonl")), "the newest named for it");
        assert_eq!(rollout(&home, "k3"), Some(day.join("rollout-4-other.jsonl")));
        assert_eq!(rollout(&home, "nope"), None);
        assert_eq!(rollout(&home, "../k1"), None);
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn codex_heads_need_session_meta() {
        assert_eq!(parse_head(r#"{"type":"response_item","payload":{}}"#), None);
        assert_eq!(parse_head("garbage"), None);
        assert_eq!(parse_head(&head_line("a", "/w", None, None)).unwrap().parent, None);
    }
}
