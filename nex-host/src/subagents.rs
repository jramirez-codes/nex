//! Sub-agents an agent runs inside its own process, read from the files the agent keeps
//! (they're its own state, not an interface: anything unexpected means none).
//!
//! - Claude Code (the Task tool): `<config>/projects/<folder>/<session id>/subagents/`
//!   holds `agent-<id>.jsonl` (its transcript) and `agent-<id>.meta.json`
//!   (`{agentType, description, finished, …}`), written when it starts and again, with
//!   `finished: true`, when it ends.
//! - Codex (spawned threads, best effort): each thread is a rollout file under
//!   `<codex home>/sessions/YYYY/MM/DD/`, whose first line (`session_meta`) names the thread
//!   that spawned it.
//!
//! A finished sub-agent is reported for [`DONE_FOR_MS`] after it ends; one whose transcript
//! hasn't changed in [`STALE_MS`] is taken to be gone. Files are re-read only when they
//! change, so a poll is a directory listing and a few stats.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::proto::{Claude, SubAgent};

/// How long a finished sub-agent stays listed (unix ms).
pub const DONE_FOR_MS: u64 = 120_000;
/// A sub-agent whose transcript hasn't changed in this long is no longer listed.
pub const STALE_MS: u64 = 600_000;
/// Cached files beyond this many are forgotten (all at once: it's only a cache).
const CACHE_MAX: usize = 4096;

/// What a file said, as of its mtime.
struct Cached<T> {
    mtime: SystemTime,
    value: Option<T>,
}

#[derive(Default)]
struct Cache {
    claude_meta: HashMap<PathBuf, Cached<ClaudeMeta>>,
    codex_head: HashMap<PathBuf, Cached<CodexHead>>,
    /// Where each Claude Code conversation's folder is, once found.
    claude_dirs: HashMap<(PathBuf, String), PathBuf>,
}

impl Cache {
    fn trim(&mut self) {
        if self.claude_meta.len() + self.codex_head.len() > CACHE_MAX {
            *self = Cache::default();
        }
    }
}

fn cache() -> std::sync::MutexGuard<'static, Cache> {
    static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();
    CACHE.get_or_init(Default::default).lock().unwrap_or_else(|e| e.into_inner())
}

fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn now_ms() -> u64 {
    ms(SystemTime::now())
}

/// Read `path` through `map` unless it hasn't changed since last time.
fn cached<T: Clone>(map: &mut HashMap<PathBuf, Cached<T>>, path: &Path, mtime: SystemTime, read: impl FnOnce(&Path) -> Option<T>) -> Option<T> {
    if let Some(c) = map.get(path)
        && c.mtime == mtime
    {
        return c.value.clone();
    }
    let value = read(path);
    map.insert(path.to_path_buf(), Cached { mtime, value: value.clone() });
    value
}

fn clean(s: &str) -> Option<String> {
    let s = s.lines().next().unwrap_or("").trim();
    (!s.is_empty()).then(|| s.chars().take(120).collect())
}

// ---- Claude Code ----------------------------------------------------------------------

#[derive(Clone, Debug, PartialEq, Eq)]
struct ClaudeMeta {
    kind: Option<String>,
    description: Option<String>,
    finished: bool,
}

fn parse_claude_meta(text: &str) -> Option<ClaudeMeta> {
    let v: serde_json::Value = serde_json::from_str(text).ok()?;
    let text = |k: &str| v.get(k).and_then(|s| s.as_str()).and_then(clean);
    Some(ClaudeMeta { kind: text("agentType"), description: text("description"), finished: v.get("finished").and_then(|f| f.as_bool()).unwrap_or(false) })
}

/// The folder Claude Code names a project by: its path with everything but letters and
/// digits turned into `-`.
fn claude_project(cwd: &str) -> String {
    cwd.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).collect()
}

/// The folder of the Claude Code conversation `claude` describes, under `config`.
fn claude_dir(cache: &mut Cache, config: &Path, claude: &Claude) -> Option<PathBuf> {
    let id = claude.session_id.as_deref().filter(|id| crate::claude::valid_id(id))?;
    let projects = config.join("projects");
    let key = (projects.clone(), id.to_string());
    match cache.claude_dirs.get(&key) {
        Some(d) if d.is_dir() => Some(d.clone()),
        _ => {
            let guess = claude.cwd.as_deref().map(|c| projects.join(claude_project(c)).join(id)).filter(|d| d.is_dir());
            // Started in another folder than the one it's in now: look for it.
            let found = guess.or_else(|| std::fs::read_dir(&projects).ok()?.flatten().map(|e| e.path().join(id)).find(|d| d.is_dir()))?;
            cache.claude_dirs.insert(key, found.clone());
            Some(found)
        }
    }
}

/// Whether `id` can name a sub-agent's file: no path in it.
fn valid_sub_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The transcript of sub-agent `id` of the Claude Code conversation `claude` describes, with
/// Claude Code's config folder at `config` (its `.meta.json` is beside it).
pub fn claude_transcript(config: &Path, claude: &Claude, id: &str) -> Option<PathBuf> {
    if !valid_sub_id(id) {
        return None;
    }
    let mut cache = cache();
    let dir = claude_dir(&mut cache, config, claude)?.join("subagents");
    let meta = dir.join(format!("agent-{id}.meta.json"));
    let transcript = dir.join(format!("agent-{id}.jsonl"));
    (meta.is_file() || transcript.is_file()).then_some(transcript)
}

/// The rollout of the Codex thread `id`, with Codex's home at `home`: in the newest day
/// folders, named after it or saying so on its first line.
pub fn codex_rollout(home: &Path, id: &str) -> Option<PathBuf> {
    if !valid_sub_id(id) {
        return None;
    }
    let mut rollouts: Vec<PathBuf> = codex_days(home).into_iter().flat_map(|d| std::fs::read_dir(d).into_iter().flatten().flatten().map(|e| e.path())).filter(|p| is_rollout(p)).collect();
    rollouts.sort();
    rollouts.reverse();
    let named = rollouts.iter().find(|p| p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.ends_with(&format!("-{id}.jsonl"))));
    named.cloned().or_else(|| rollouts.into_iter().find(|p| first_line(p).and_then(|l| parse_codex_head(&l)).is_some_and(|h| h.id == id)))
}

/// The sub-agents of the Claude Code conversation the session file `claude` describes, with
/// Claude Code's config folder at `config` (see [`crate::claude::config_dir`]).
pub fn claude_in(config: &Path, claude: &Claude, now: u64) -> Vec<SubAgent> {
    let mut cache = cache();
    cache.trim();
    let Some(dir) = claude_dir(&mut cache, config, claude) else { return Vec::new() };
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
        let Some(meta) = cached(&mut cache.claude_meta, &meta_path, meta_mtime, |p| parse_claude_meta(&std::fs::read_to_string(p).ok()?)) else { continue };
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

/// [`claude_in`] with Claude Code's config folder on this machine.
pub fn claude(claude: &Claude) -> Vec<SubAgent> {
    crate::claude::config_dir().map(|c| claude_in(&c, claude, now_ms())).unwrap_or_default()
}

// ---- Codex ----------------------------------------------------------------------------

/// What a rollout's first line (`session_meta`) says.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CodexHead {
    pub(crate) id: String,
    cwd: Option<String>,
    /// The thread that spawned it, for a sub-agent.
    parent: Option<String>,
    pub(crate) name: Option<String>,
    pub(crate) role: Option<String>,
}

fn find_key<'a>(v: &'a serde_json::Value, key: &str) -> Option<&'a serde_json::Value> {
    match v {
        serde_json::Value::Object(m) => m.get(key).or_else(|| m.values().find_map(|v| find_key(v, key))),
        _ => None,
    }
}

pub(crate) fn parse_codex_head(line: &str) -> Option<CodexHead> {
    let v: serde_json::Value = serde_json::from_str(line).ok()?;
    if v.get("type")?.as_str()? != "session_meta" {
        return None;
    }
    let p = v.get("payload")?;
    let s = |v: Option<&serde_json::Value>| v.and_then(|s| s.as_str()).and_then(clean);
    let source = p.get("source");
    let parent = source.and_then(|s| find_key(s, "parent_thread_id")).and_then(|s| s.as_str()).map(str::to_string);
    let pick = |k: &str| s(p.get(k)).or_else(|| s(source.and_then(|src| find_key(src, k))));
    Some(CodexHead { id: s(p.get("id"))?, cwd: s(p.get("cwd")), parent, name: pick("agent_nickname"), role: pick("agent_role") })
}

fn first_line(path: &Path) -> Option<String> {
    use std::io::{BufRead, Read};
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f.take(256 * 1024)).read_line(&mut line).ok()?;
    Some(line)
}

/// Whether the rollout's last line says the thread finished its task.
fn codex_finished(path: &Path) -> bool {
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
fn codex_days(home: &Path) -> Vec<PathBuf> {
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
fn codex_open_rollout(pid: u32) -> Option<PathBuf> {
    std::fs::read_dir(format!("/proc/{pid}/fd")).ok()?.flatten().filter_map(|e| std::fs::read_link(e.path()).ok()).find(|p| is_rollout(p))
}

/// The sub-agents the Codex process `pid`, working in `cwd`, spawned, with Codex's home at
/// `home` (`$CODEX_HOME`, else `~/.codex`).
pub fn codex_in(home: &Path, pid: u32, cwd: &str, now: u64) -> Vec<SubAgent> {
    let mut cache = cache();
    cache.trim();
    let mut recent = Vec::new();
    for day in codex_days(home) {
        for e in std::fs::read_dir(day).into_iter().flatten().flatten() {
            let p = e.path();
            let Some(mtime) = e.metadata().ok().and_then(|m| m.modified().ok()) else { continue };
            if !is_rollout(&p) || now.saturating_sub(ms(mtime)) > STALE_MS {
                continue;
            }
            // The first line never changes once written: read it once.
            let head = match cache.codex_head.get(&p) {
                Some(c) if c.value.is_some() => c.value.clone(),
                _ => cached(&mut cache.codex_head, &p, mtime, |p| parse_codex_head(&first_line(p)?)),
            };
            if let Some(h) = head {
                recent.push((p, mtime, h));
            }
        }
    }
    let open = codex_open_rollout(pid);
    let me = match open.as_ref().and_then(|o| recent.iter().find(|(p, _, _)| p == o)) {
        Some((_, _, h)) => h.id.clone(),
        None if open.is_some() => match open.as_deref().and_then(first_line).and_then(|l| parse_codex_head(&l)) {
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
        let done = codex_finished(p);
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

/// Codex's home folder: `$CODEX_HOME`, else `~/.codex`.
pub fn codex_home() -> Option<PathBuf> {
    match std::env::var_os("CODEX_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        Some(p) => Some(p),
        None => Some(PathBuf::from(std::env::var_os("HOME")?).join(".codex")),
    }
}

/// [`codex_in`] with Codex's home on this machine.
pub fn codex(pid: u32, cwd: &str) -> Vec<SubAgent> {
    codex_home().map(|h| codex_in(&h, pid, cwd, now_ms())).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

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
        format!(r#"{{"agentType":"Explore","isBuiltIn":true,"description":"{desc}","toolUseId":"toolu_01","spawnDepth":1,"requestShape":"background","model":"haiku"{fin}}}"#)
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
        let subs = claude_in(&config, &session("/home/u/code/nex"), now);
        let got: Vec<(&str, &str, Option<&str>, bool)> = subs.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.role.as_deref(), s.done)).collect();
        assert_eq!(got.len(), 2);
        assert!(got.contains(&("a1", "Map the view code", Some("Explore"), false)));
        assert!(got.contains(&("a2", "Feature inventory", Some("Explore"), true)));

        // Finished a while ago: dropped. Gone quiet for long: dropped.
        assert!(claude_in(&config, &session("/home/u/code/nex"), now + DONE_FOR_MS + 5_000).iter().all(|s| !s.done));
        assert!(claude_in(&config, &session("/home/u/code/nex"), now + STALE_MS + 5_000).is_empty());

        // Moved to another folder since it started: still found.
        assert_eq!(claude_in(&config, &session("/elsewhere"), now).len(), 2);
        // No conversation, or a bad id: none.
        assert!(claude_in(&config, &Claude::default(), now).is_empty());
        assert!(claude_in(&config, &Claude { session_id: Some("../x".into()), ..Claude::default() }, now).is_empty());
        std::fs::remove_dir_all(&config).unwrap();
    }

    #[test]
    fn a_conversation_without_subagents_has_none() {
        let config = tmp("claude-none");
        std::fs::create_dir_all(config.join("projects/-a").join(ID)).unwrap();
        assert!(claude_in(&config, &session("/a"), now_ms()).is_empty());
        std::fs::remove_dir_all(&config).unwrap();
    }

    #[test]
    fn junk_meta_is_skipped() {
        assert_eq!(parse_claude_meta("not json"), None);
        let m = parse_claude_meta(r#"{"description":"  two\nlines  "}"#).unwrap();
        assert_eq!((m.description.as_deref(), m.kind, m.finished), (Some("two"), None, false));
    }

    fn rollout(id: &str, cwd: &str, parent: Option<&str>, role: Option<&str>) -> String {
        let source = match parent {
            Some(p) => format!(r#"{{"subagent":{{"thread_spawn":{{"parent_thread_id":"{p}","depth":1}}}}}}"#),
            None => r#""cli""#.into(),
        };
        let role = role.map(|r| format!(r#","agent_role":"{r}""#)).unwrap_or_default();
        format!(r#"{{"timestamp":"2026-11-23T10:00:00Z","type":"session_meta","payload":{{"id":"{id}","cwd":"{cwd}","source":{source}{role}}}}}"#) + "\n"
    }

    #[test]
    fn reads_codexs_spawned_threads() {
        let home = tmp("codex");
        let day = home.join("sessions/2026/11/23");
        std::fs::create_dir_all(&day).unwrap();
        std::fs::write(day.join("rollout-1-parent.jsonl"), rollout("p1", "/w", None, None)).unwrap();
        std::fs::write(day.join("rollout-2-kid.jsonl"), rollout("k1", "/w", Some("p1"), Some("explorer"))).unwrap();
        let done = rollout("k2", "/w", Some("p1"), Some("worker")) + r#"{"type":"event_msg","payload":{"type":"task_complete"}}"# + "\n";
        std::fs::write(day.join("rollout-3-kid.jsonl"), done).unwrap();
        std::fs::write(day.join("rollout-4-other.jsonl"), rollout("k3", "/w", Some("someone-else"), None)).unwrap();
        // pid 0 has no open files: found by its folder.
        let subs = codex_in(&home, 0, "/w", now_ms());
        let got: Vec<(&str, &str, bool)> = subs.iter().map(|s| (s.id.as_str(), s.name.as_str(), s.done)).collect();
        assert_eq!(got.len(), 2, "{got:?}");
        assert!(got.contains(&("k1", "explorer", false)));
        assert!(got.contains(&("k2", "worker", true)));
        assert!(codex_in(&home, 0, "/nowhere", now_ms()).is_empty());
        std::fs::remove_dir_all(&home).unwrap();
    }

    #[test]
    fn codex_heads_need_session_meta() {
        assert_eq!(parse_codex_head(r#"{"type":"response_item","payload":{}}"#), None);
        assert_eq!(parse_codex_head("garbage"), None);
        assert_eq!(parse_codex_head(&rollout("a", "/w", None, None)).unwrap().parent, None);
    }
}
