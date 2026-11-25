//! Sub-agents an agent runs inside its own process (Claude Code's Task tool, a thread Codex
//! spawned), read from the files the agent keeps: each CLI's reader is in its module under
//! [`crate::clis`] ([`AgentCli::subagents`](crate::clis::AgentCli::subagents)); what they
//! share is here.
//!
//! A finished sub-agent is reported for [`DONE_FOR_MS`] after it ends; one whose transcript
//! hasn't changed in [`STALE_MS`] is taken to be gone. Files are re-read only when they
//! change ([`cached`]), so a poll is a directory listing and a few stats.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

/// How long a finished sub-agent stays listed (unix ms).
pub const DONE_FOR_MS: u64 = 120_000;
/// A sub-agent whose transcript hasn't changed in this long is no longer listed.
pub const STALE_MS: u64 = 600_000;
/// Cached files beyond this many are forgotten (all at once: it's only a cache).
pub(crate) const CACHE_MAX: usize = 4096;

/// What a file said, as of its mtime.
pub(crate) struct Cached<T> {
    pub(crate) mtime: SystemTime,
    pub(crate) value: Option<T>,
}

pub(crate) fn ms(t: SystemTime) -> u64 {
    t.duration_since(UNIX_EPOCH).map(|d| d.as_millis() as u64).unwrap_or(0)
}

pub fn now_ms() -> u64 {
    ms(SystemTime::now())
}

/// Read `path` through `map` unless it hasn't changed since last time.
pub(crate) fn cached<T: Clone>(map: &mut HashMap<PathBuf, Cached<T>>, path: &Path, mtime: SystemTime, read: impl FnOnce(&Path) -> Option<T>) -> Option<T> {
    if let Some(c) = map.get(path)
        && c.mtime == mtime
    {
        return c.value.clone();
    }
    let value = read(path);
    map.insert(path.to_path_buf(), Cached { mtime, value: value.clone() });
    value
}

/// A name or description from a file: its first line, trimmed and cut short.
pub(crate) fn clean(s: &str) -> Option<String> {
    let s = s.lines().next().unwrap_or("").trim();
    (!s.is_empty()).then(|| s.chars().take(120).collect())
}

/// Whether `id` can name a sub-agent's file: no path in it.
pub(crate) fn valid_sub_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 128 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
}

/// The first line of the file at `path` (reading at most 256 KiB).
pub(crate) fn first_line(path: &Path) -> Option<String> {
    use std::io::{BufRead, Read};
    let f = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    std::io::BufReader::new(f.take(256 * 1024)).read_line(&mut line).ok()?;
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sub_ids_name_no_path() {
        assert!(valid_sub_id("a1b2-c_3"));
        for bad in ["", "../x", "a/b", "a.b"] {
            assert!(!valid_sub_id(bad), "{bad}");
        }
    }

    #[test]
    fn names_are_one_short_line() {
        assert_eq!(clean("  two\nlines  ").as_deref(), Some("two"));
        assert_eq!(clean("   "), None);
        assert_eq!(clean(&"x".repeat(500)).map(|s| s.len()), Some(120));
    }
}
