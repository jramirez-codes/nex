//! Claude Code's sub-agents (its Task tool): `<home>/projects/<folder>/<session id>/subagents/`
//! holds `agent-<id>.jsonl` (its transcript) and `agent-<id>.meta.json`
//! (`{agentType, description, finished, …}`), written when it starts and again, with
//! `finished: true`, when it ends.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard, OnceLock};

use crate::proto::{Claude, SubAgent};
use crate::subagents::{CACHE_MAX, Cached, DONE_FOR_MS, STALE_MS, cached, clean, ms, valid_sub_id};

#[derive(Default)]
struct Cache {
    meta: HashMap<PathBuf, Cached<Meta>>,
    /// Where each conversation's folder is, once found.
    dirs: HashMap<(PathBuf, String), PathBuf>,
}

fn cache() -> MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    let mut c = CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner());
    if c.meta.len() > CACHE_MAX {
        *c = Cache::default();
    }
    c
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct Meta {
    kind: Option<String>,
    description: Option<String>,
    finished: bool,
}

pub(super) fn parse_meta(text: &str) -> Option<Meta> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let text = |k: &str| v.get(k).and_then(|s| s.as_str()).and_then(clean);
    Some(Meta { kind: text("agentType"), description: text("description"), finished: v.get("finished").and_then(|f| f.as_bool()).unwrap_or(false) })
}

/// The folder Claude Code names a project by: its path with everything but letters and
/// digits turned into `-`.
fn project(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The folder of the conversation `claude` describes, under `config`.
fn conversation_dir(cache: &mut Cache, config: &Path, claude: &Claude) -> Option<PathBuf> {
    let id = claude.session_id.as_deref().filter(|id| super::session::valid_id(id))?;
    let projects = config.join("projects");
    let key = (projects.clone(), id.to_string());
    match cache.dirs.get(&key) {
        Some(d) if d.is_dir() => Some(d.clone()),
        _ => {
            let guess = claude.cwd.as_deref().map(|c| projects.join(project(c)).join(id)).filter(|d| d.is_dir());
            // Started in another folder than the one it's in now: look for it.
            let found = guess.or_else(|| std::fs::read_dir(&projects).ok()?.flatten().map(|e| e.path().join(id)).find(|d| d.is_dir()))?;
            cache.dirs.insert(key, found.clone());
            Some(found)
        }
    }
}

/// The transcript of sub-agent `id` of the conversation `claude` describes, with Claude
/// Code's config folder at `config` (its `.meta.json` is beside it).
pub fn transcript(config: &Path, claude: &Claude, id: &str) -> Option<PathBuf> {
    if !valid_sub_id(id) {
        return None;
    }
    let mut cache = cache();
    let dir = conversation_dir(&mut cache, config, claude)?.join("subagents");
    let meta = dir.join(format!("agent-{id}.meta.json"));
    let transcript = dir.join(format!("agent-{id}.jsonl"));
    (meta.is_file() || transcript.is_file()).then_some(transcript)
}

/// The sub-agents of the conversation the session file `claude` describes, with Claude
/// Code's config folder at `config`.
pub fn list(config: &Path, claude: &Claude, now: u64) -> Vec<SubAgent> {
    let mut cache = cache();
    let Some(dir) = conversation_dir(&mut cache, config, claude) else { return Vec::new() };
    let Ok(entries) = std::fs::read_dir(dir.join("subagents")) else { return Vec::new() };
    let mut out = Vec::new();
    for e in entries.flatten() {
        let name = e.file_name();
        let Some(agent) = name.to_str().and_then(|n| n.strip_prefix("agent-")).and_then(|n| n.strip_suffix(".meta.json")) else { continue };
        let meta_path = e.path();
        let Ok(meta_md) = e.metadata() else { continue };
        let Ok(meta_mtime) = meta_md.modified() else { continue };
        let transcript = std::fs::metadata(meta_path.with_file_name(format!("agent-{agent}.jsonl"))).ok();
        let last = transcript.as_ref().and_then(|m| m.modified().ok()).map_or(meta_mtime, |t| t.max(meta_mtime));
        // Long finished or long gone: not worth opening.
        if now.saturating_sub(ms(last)) > STALE_MS {
            continue;
        }
        let Some(meta) = cached(&mut cache.meta, &meta_path, meta_mtime, |p| parse_meta(&std::fs::read_to_string(p).ok()?)) else { continue };
        if meta.finished && now.saturating_sub(ms(meta_mtime)) > DONE_FOR_MS {
            continue;
        }
        let started = transcript.as_ref().and_then(|m| m.created().or_else(|_| m.modified()).ok()).or_else(|| meta_md.created().ok()).map_or(0, ms);
        out.push(SubAgent {
            id: agent.to_string(),
            name: meta.description.clone().or_else(|| meta.kind.clone()).unwrap_or_else(|| "Sub-agent".into()),
            role: meta.kind,
            started,
            done: meta.finished,
        });
    }
    out.sort_by(|a, b| (a.started, &a.id).cmp(&(b.started, &b.id)));
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subagents::now_ms;

    const ID: &str = "4c4075db-1701-4574-82f5-7b6f9e0e0882";

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nex-subagents-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    /// As Claude Code v2.1.330 writes them.
    fn meta(desc: &str, finished: bool) -> String {
        let fin = if finished { r#","finished":true"# } else { "" };
        format!(
            r#"{{"agentType":"Explore","isBuiltIn":true,"description":"{desc}","toolUseId":"toolu_01","spawnDepth":1,"requestShape":"background","model":"haiku"{fin}}}"#
        )
    }

    fn session(cwd: &str) -> Claude {
        Claude { session_id: Some(ID.into()), cwd: Some(cwd.into()), ..Claude::default() }
    }

    #[test]
    fn reads_claude_codes_subagents() {
        let config = tmp("claude");
        let dir = config.join("projects/-home-u-code-nex").join(ID).join("subagents");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("agent-a1.meta.json"), meta("Map the view code", false)).unwrap();
        std::fs::write(dir.join("agent-a1.jsonl"), "{}\n").unwrap();
        std::fs::write(dir.join("agent-a2.meta.json"), meta("Feature inventory", true)).unwrap();
        std::fs::write(dir.join("notes.txt"), "x").unwrap();
        let now = now_ms();
        let subs = list(&config, &session("/home/u/code/nex"), now);
        let got: Vec<(&str, &str, Option<&str>, bool)> = subs.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.role.as_deref(), s.done)).collect();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&("a1", "Map the view code", Some("Explore"), false)));
        assert!(got.contains(&("a2", "Feature inventory", Some("Explore"), true)));

        // Finished a while ago: dropped. Gone quiet for long: dropped.
        assert!(list(&config, &session("/home/u/code/nex"), now + DONE_FOR_MS + 5_000).iter().all(|s| !s.done));
        assert!(list(&config, &session("/home/u/code/nex"), now + STALE_MS + 5_000).is_empty());

        // Moved to another folder since it started: still found.
        assert_eq!(list(&config, &session("/elsewhere"), now).len(), 2);
        // No conversation, or a bad id: none.
        assert!(list(&config, &Claude::default(), now).is_empty());
        assert!(list(&config, &Claude { session_id: Some("../x".into()), ..Claude::default() }, now).is_empty());

        // Each one's transcript, by id; never a path.
        assert_eq!(transcript(&config, &session("/home/u/code/nex"), "a1"), Some(dir.join("agent-a1.jsonl")));
        assert_eq!(transcript(&config, &session("/home/u/code/nex"), "nope"), None);
        assert_eq!(transcript(&config, &session("/home/u/code/nex"), "../a1"), None);
        std::fs::remove_dir_all(&config).unwrap();
    }

    #[test]
    fn a_conversation_without_subagents_has_none() {
        let config = tmp("claude-none");
        std::fs::create_dir_all(config.join("projects/-a").join(ID)).unwrap();
        assert!(list(&config, &session("/a"), now_ms()).is_empty());
        std::fs::remove_dir_all(&config).unwrap();
    }

    #[test]
    fn junk_meta_is_skipped() {
        assert_eq!(parse_meta("not json"), None);
        let m = parse_meta(r#"{"description":"  two\nlines  "}"#).unwrap();
        assert_eq!((m.description.as_deref(), m.kind, m.finished), (Some("two"), None, false));
    }
}
