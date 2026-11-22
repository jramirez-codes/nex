//! The window layout saved on quit and brought back on the next plain launch: every tab's
//! split tree, each pane's working directory, zoom and running command, focus, and the Files
//! tab. Stored as JSON in `$XDG_STATE_HOME/nex/session.json` (`~/.local/state/nex/session.json`),
//! or `~/Library/Application Support/nex/session.json` on macOS.

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use crate::layout::Axis;

pub mod claude;

/// Format version written to and required in the file.
pub const VERSION: u32 = 1;

const RATIO_MIN: f32 = 0.05;
const RATIO_MAX: f32 = 0.95;
/// Most arguments, and bytes in all, a saved command may have; a real one is far smaller.
const COMMAND_MAX_ARGS: usize = 256;
const COMMAND_MAX_BYTES: usize = 16 * 1024;

/// Everything restored on launch.
#[derive(Clone, Debug, PartialEq)]
pub struct Session {
    /// Terminal tabs, in title-bar order.
    pub tabs: Vec<TabState>,
    /// Index into `tabs`.
    pub active: usize,
    /// The Files tab, if it was open.
    pub explorer: Option<TabState>,
    /// Whether the Files tab was the one showing.
    pub explorer_active: bool,
}

/// One tab's split tree and focus.
#[derive(Clone, Debug, PartialEq)]
pub struct TabState {
    pub root: PaneNode,
    /// Index of the focused leaf in left-to-right (`Node::panes`) order.
    pub focused: usize,
    pub zoomed: bool,
}

/// A split tree mirroring `layout::Node`, with pane state at the leaves.
#[derive(Clone, Debug, PartialEq)]
pub enum PaneNode {
    Leaf(PaneState),
    Split { axis: Axis, ratio: f32, a: Box<PaneNode>, b: Box<PaneNode> },
}

/// What a pane needs to come back.
#[derive(Clone, Debug, PartialEq)]
pub struct PaneState {
    pub kind: PaneKind,
    /// Working directory (terminal) or folder shown (explorer).
    pub cwd: Option<PathBuf>,
    /// Per-pane zoom override; `None` = the config's size.
    pub font_size: Option<f32>,
    /// What a terminal was running in its shell (`npm run dev`), run again on restore.
    /// `None` when it was at its prompt, and always for explorers.
    pub command: Option<Vec<String>>,
    /// The window's sticky pane, which every tab's tree lists (with its own place in each).
    /// All sticky leaves are one pane: it is opened once.
    pub sticky: bool,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PaneKind {
    Terminal,
    Explorer,
    /// A Changes (git diff) pane; `cwd` is the folder whose repository it shows.
    Diff,
    /// An Agents pane (`cwd` is unused).
    Agents,
}

impl Session {
    /// The session as JSON, including `"version"`.
    pub fn to_json(&self) -> String {
        let v = json!({
            "version": VERSION,
            "tabs": self.tabs.iter().map(tab_to_value).collect::<Vec<_>>(),
            "active": self.active,
            "explorer": self.explorer.as_ref().map(tab_to_value),
            "explorer_active": self.explorer_active,
        });
        serde_json::to_string_pretty(&v).unwrap_or_default()
    }

    /// Parses and normalizes a session: tabs with an invalid tree are dropped, ratios, font
    /// sizes and indices are clamped. Errors on bad JSON, a wrong version, or nothing to restore.
    pub fn from_json(s: &str) -> Result<Session, String> {
        let v: Value = serde_json::from_str(s).map_err(|e| format!("invalid JSON: {e}"))?;
        let obj = v.as_object().ok_or("not a JSON object")?;
        match obj.get("version").and_then(Value::as_u64) {
            Some(n) if n == u64::from(VERSION) => {}
            Some(n) => return Err(format!("unsupported version {n}")),
            None => return Err("missing version".into()),
        }
        let tabs: Vec<TabState> = obj
            .get("tabs")
            .and_then(Value::as_array)
            .map(|a| a.iter().filter_map(tab_from_value).collect())
            .unwrap_or_default();
        let explorer = obj.get("explorer").and_then(tab_from_value);
        if tabs.is_empty() && explorer.is_none() {
            return Err("no tabs".into());
        }
        let active = obj.get("active").and_then(Value::as_u64).unwrap_or(0) as usize;
        let active = active.min(tabs.len().saturating_sub(1));
        let explorer_active = explorer.is_some()
            && (tabs.is_empty() || obj.get("explorer_active").and_then(Value::as_bool).unwrap_or(false));
        Ok(Session { tabs, active, explorer, explorer_active })
    }
}

impl PaneNode {
    /// The leaves in left-to-right order, matching `layout::Node::panes`.
    pub fn leaves(&self) -> Vec<&PaneState> {
        let mut out = Vec::new();
        self.collect(&mut out);
        out
    }

    fn collect<'a>(&'a self, out: &mut Vec<&'a PaneState>) {
        match self {
            PaneNode::Leaf(p) => out.push(p),
            PaneNode::Split { a, b, .. } => {
                a.collect(out);
                b.collect(out);
            }
        }
    }
}

fn tab_to_value(t: &TabState) -> Value {
    json!({ "root": node_to_value(&t.root), "focused": t.focused, "zoomed": t.zoomed })
}

fn node_to_value(n: &PaneNode) -> Value {
    match n {
        PaneNode::Leaf(p) => {
            let mut m = Map::new();
            let kind = match p.kind {
                PaneKind::Terminal => "terminal",
                PaneKind::Explorer => "explorer",
                PaneKind::Diff => "diff",
                PaneKind::Agents => "agents",
            };
            m.insert("kind".into(), kind.into());
            if let Some(cwd) = &p.cwd {
                m.insert("cwd".into(), cwd.to_string_lossy().into_owned().into());
            }
            if let Some(size) = p.font_size {
                m.insert("font_size".into(), short_f32(size));
            }
            if let Some(cmd) = p.command.as_ref().filter(|_| p.kind == PaneKind::Terminal) {
                m.insert("command".into(), json!(cmd));
            }
            if p.sticky {
                m.insert("sticky".into(), true.into());
            }
            Value::Object(m)
        }
        PaneNode::Split { axis, ratio, a, b } => {
            let axis = match axis {
                Axis::Horizontal => "horizontal",
                Axis::Vertical => "vertical",
            };
            json!({ "axis": axis, "ratio": short_f32(*ratio), "a": node_to_value(a), "b": node_to_value(b) })
        }
    }
}

/// An f32 as the shortest decimal that reads back as it (0.6, not 0.6000000238418579).
fn short_f32(x: f32) -> Value {
    x.to_string().parse::<f64>().map_or(Value::Null, |f| json!(f))
}

fn tab_from_value(v: &Value) -> Option<TabState> {
    let obj = v.as_object()?;
    let root = node_from_value(obj.get("root")?, 0)?;
    let leaves = root.leaves().len();
    let focused = (obj.get("focused").and_then(Value::as_u64).unwrap_or(0) as usize).min(leaves - 1);
    let zoomed = obj.get("zoomed").and_then(Value::as_bool).unwrap_or(false);
    Some(TabState { root, focused, zoomed })
}

/// Deepest split tree accepted; a real window never gets near it.
const MAX_DEPTH: usize = 64;

fn node_from_value(v: &Value, depth: usize) -> Option<PaneNode> {
    if depth > MAX_DEPTH {
        return None;
    }
    let obj = v.as_object()?;
    if let Some(kind) = obj.get("kind") {
        let kind = match kind.as_str()? {
            "terminal" => PaneKind::Terminal,
            "explorer" => PaneKind::Explorer,
            "diff" => PaneKind::Diff,
            "agents" => PaneKind::Agents,
            _ => return None,
        };
        let cwd = match obj.get("cwd") {
            None | Some(Value::Null) => None,
            Some(c) => Some(PathBuf::from(c.as_str()?)).filter(|p| !p.as_os_str().is_empty()),
        };
        let font_size = match obj.get("font_size") {
            None | Some(Value::Null) => None,
            Some(f) => {
                let f = f.as_f64()? as f32;
                f.is_finite().then(|| f.clamp(crate::config::FONT_MIN, crate::config::FONT_MAX))
            }
        };
        let command = obj.get("command").filter(|_| kind == PaneKind::Terminal).and_then(command_from_value);
        let sticky = obj.get("sticky").and_then(Value::as_bool).unwrap_or(false);
        return Some(PaneNode::Leaf(PaneState { kind, cwd, font_size, command, sticky }));
    }
    let axis = match obj.get("axis")?.as_str()? {
        "horizontal" => Axis::Horizontal,
        "vertical" => Axis::Vertical,
        _ => return None,
    };
    let ratio = obj.get("ratio")?.as_f64()? as f32;
    if !ratio.is_finite() {
        return None;
    }
    let a = node_from_value(obj.get("a")?, depth + 1)?;
    let b = node_from_value(obj.get("b")?, depth + 1)?;
    Some(PaneNode::Split { axis, ratio: ratio.clamp(RATIO_MIN, RATIO_MAX), a: Box::new(a), b: Box::new(b) })
}

/// A saved command: a non-empty array of strings within the size caps; anything else is
/// dropped (the pane still comes back, at its prompt).
fn command_from_value(v: &Value) -> Option<Vec<String>> {
    let arr = v.as_array().filter(|a| !a.is_empty() && a.len() <= COMMAND_MAX_ARGS)?;
    let argv: Vec<String> = arr.iter().map(|a| a.as_str().map(str::to_string)).collect::<Option<_>>()?;
    let bytes: usize = argv.iter().map(String::len).sum();
    (bytes <= COMMAND_MAX_BYTES && !argv[0].is_empty()).then_some(argv)
}

/// Where the session file lives, or `None` with no home directory.
pub fn path() -> Option<PathBuf> {
    path_from(std::env::var_os("XDG_STATE_HOME").map(PathBuf::from), crate::platform::home_dir())
}

fn path_from(xdg_state: Option<PathBuf>, home: Option<PathBuf>) -> Option<PathBuf> {
    let dir = if cfg!(target_os = "macos") {
        home?.join("Library").join("Application Support")
    } else {
        match xdg_state.filter(|p| p.is_absolute()) {
            Some(p) => p,
            None => home?.join(".local").join("state"),
        }
    };
    Some(dir.join("nex").join("session.json"))
}

/// The saved session; `None` if there is none or it can't be used (logged to stderr).
pub fn load() -> Option<Session> {
    load_from(&path()?)
}

fn load_from(path: &Path) -> Option<Session> {
    let text = match std::fs::read_to_string(path) {
        Ok(t) => t,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return None,
        Err(e) => {
            eprintln!("nex: session: can't read {}: {e}", path.display());
            return None;
        }
    };
    match Session::from_json(&text) {
        Ok(s) => Some(s),
        Err(e) => {
            eprintln!("nex: session: ignoring {}: {e}", path.display());
            None
        }
    }
}

/// Writes the session atomically (temp file, then rename), creating the directory.
pub fn save(s: &Session) -> std::io::Result<()> {
    let path = path().ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "no home directory"))?;
    save_to(&path, s)
}

fn save_to(path: &Path, s: &Session) -> std::io::Result<()> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    // Per-process name, so two writers never share (and tear) one temp file.
    let tmp = path.with_extension(format!("json.{}.tmp", std::process::id()));
    let written = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        std::io::Write::write_all(&mut f, s.to_json().as_bytes())?;
        f.sync_all()?;
        std::fs::rename(&tmp, path)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// Held by the one nex window that owns the session file (restores, autosaves, clears it):
/// a lock on `session.json.lock` (see [`crate::platform::lock_file`]), released on drop.
pub struct Lock {
    _file: Option<crate::platform::FileLock>,
}

/// Takes the session lock; `None` if another nex holds it. If the lock file can't be opened
/// (no home, unwritable state dir) the lock is a no-op, as where the OS has no file locks.
pub fn try_lock() -> Option<Lock> {
    match path() {
        Some(p) => try_lock_at(&p),
        None => Some(Lock { _file: None }),
    }
}

fn try_lock_at(path: &Path) -> Option<Lock> {
    use crate::platform::LockFailed;
    let lock = path.with_extension("json.lock");
    match crate::platform::lock_file(&lock) {
        Ok(file) => Some(Lock { _file: Some(file) }),
        Err(LockFailed::Held) => None,
        Err(LockFailed::Open(e)) => {
            eprintln!("nex: session: can't open {}: {e}", lock.display());
            Some(Lock { _file: None })
        }
        Err(LockFailed::Lock(e)) => {
            eprintln!("nex: session: can't lock {}: {e}", lock.display());
            Some(Lock { _file: None })
        }
    }
}

/// Deletes the session file, so the next launch starts fresh.
pub fn clear() {
    if let Some(path) = path() {
        clear_at(&path);
    }
}

fn clear_at(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        eprintln!("nex: session: can't remove {}: {e}", path.display());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cwd: &str) -> PaneNode {
        PaneNode::Leaf(PaneState { kind: PaneKind::Terminal, cwd: Some(cwd.into()), font_size: None, command: None, sticky: false })
    }

    fn split(axis: Axis, ratio: f32, a: PaneNode, b: PaneNode) -> PaneNode {
        PaneNode::Split { axis, ratio, a: Box::new(a), b: Box::new(b) }
    }

    fn tab(root: PaneNode) -> TabState {
        TabState { root, focused: 0, zoomed: false }
    }

    fn one_tab(root: Value) -> String {
        json!({ "version": 1, "tabs": [{ "root": root }] }).to_string()
    }

    #[test]
    fn sticky_round_trips_and_defaults_off() {
        let pinned = PaneNode::Leaf(PaneState { kind: PaneKind::Terminal, cwd: Some("/s".into()), font_size: None, command: None, sticky: true });
        let split = |x| PaneNode::Split { axis: Axis::Horizontal, ratio: 0.6, a: Box::new(x), b: Box::new(pinned.clone()) };
        let s = Session { tabs: vec![tab(split(term("/a"))), tab(split(term("/b")))], active: 0, explorer: None, explorer_active: false };
        let json = s.to_json();
        assert_eq!(json.matches("\"sticky\"").count(), 2, "only sticky leaves say so");
        assert_eq!(Session::from_json(&json).unwrap(), s);
    }

    #[test]
    fn round_trip() {
        let s = Session {
            tabs: vec![
                tab(term("/a")),
                TabState {
                    root: PaneNode::Leaf(PaneState { kind: PaneKind::Terminal, cwd: None, font_size: Some(18.5), command: None, sticky: false }),
                    focused: 0,
                    zoomed: true,
                },
            ],
            active: 1,
            explorer: None,
            explorer_active: false,
        };
        assert_eq!(Session::from_json(&s.to_json()).unwrap(), s);
        assert!(s.to_json().contains("\"version\": 1"));
    }

    #[test]
    fn commands_round_trip() {
        let cmd = |c: &[&str]| Some(c.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        let leaf = |command| PaneNode::Leaf(PaneState { kind: PaneKind::Terminal, cwd: Some("/a".into()), font_size: None, command, sticky: false });
        let s = Session {
            tabs: vec![tab(split(Axis::Horizontal, 0.5, leaf(cmd(&["npm", "run", "dev"])), leaf(cmd(&["echo", "a b", "it's"]))))],
            active: 0,
            explorer: None,
            explorer_active: false,
        };
        assert_eq!(Session::from_json(&s.to_json()).unwrap(), s);
        // A pane at its prompt saves no command at all.
        assert!(!Session { tabs: vec![tab(term("/a"))], ..s.clone() }.to_json().contains("command"));
    }

    #[test]
    fn bad_commands_are_dropped() {
        let command = |c: Value| {
            let s = Session::from_json(&one_tab(json!({ "kind": "terminal", "command": c }))).unwrap();
            s.tabs[0].root.leaves()[0].command.clone()
        };
        assert_eq!(command(json!(["ls", "-l"])), Some(vec!["ls".to_string(), "-l".to_string()]));
        assert_eq!(command(json!([])), None);
        assert_eq!(command(json!([""])), None);
        assert_eq!(command(json!("npm run dev")), None);
        assert_eq!(command(json!(["ls", 3])), None);
        assert_eq!(command(json!(vec!["x"; COMMAND_MAX_ARGS + 1])), None);
        assert_eq!(command(json!(["x".repeat(COMMAND_MAX_BYTES + 1)])), None);
        assert_eq!(command(Value::Null), None);
        // Explorers never run anything.
        let j = one_tab(json!({ "kind": "explorer", "cwd": "/x", "command": ["rm", "-rf", "/"] }));
        assert_eq!(Session::from_json(&j).unwrap().tabs[0].root.leaves()[0].command, None);
    }

    #[test]
    fn nested_splits_round_trip() {
        let root = split(
            Axis::Horizontal,
            0.3,
            term("/a"),
            split(Axis::Vertical, 0.6, term("/b"), split(Axis::Horizontal, 0.5, term("/c"), term("/d"))),
        );
        let s = Session { tabs: vec![TabState { root, focused: 2, zoomed: false }], active: 0, explorer: None, explorer_active: false };
        let back = Session::from_json(&s.to_json()).unwrap();
        assert_eq!(back, s);
        let cwds: Vec<_> = back.tabs[0].root.leaves().iter().map(|p| p.cwd.clone().unwrap()).collect();
        assert_eq!(cwds, ["/a", "/b", "/c", "/d"].map(PathBuf::from));
    }

    #[test]
    fn explorer_tab_round_trip() {
        let ex = |d: &str| PaneNode::Leaf(PaneState { kind: PaneKind::Explorer, cwd: Some(d.into()), font_size: None, command: None, sticky: false });
        let s = Session {
            tabs: vec![tab(term("/a"))],
            active: 0,
            explorer: Some(TabState { root: split(Axis::Horizontal, 0.5, ex("/x"), ex("/y")), focused: 1, zoomed: false }),
            explorer_active: true,
        };
        assert_eq!(Session::from_json(&s.to_json()).unwrap(), s);
    }

    #[test]
    fn explorer_only_is_valid_and_active() {
        let j = json!({ "version": 1, "tabs": [], "explorer": { "root": { "kind": "explorer", "cwd": "/x" } } });
        let s = Session::from_json(&j.to_string()).unwrap();
        assert!(s.tabs.is_empty());
        assert!(s.explorer_active);
    }

    #[test]
    fn invalid_tabs_are_dropped() {
        let j = json!({ "version": 1, "active": 1, "tabs": [
            { "root": { "kind": "nope" } },
            { "root": { "axis": "diagonal", "ratio": 0.5, "a": { "kind": "terminal" }, "b": { "kind": "terminal" } } },
            { "root": { "axis": "vertical", "ratio": 0.5, "a": { "kind": "terminal" } } },
            { "root": { "axis": "vertical", "ratio": "half", "a": { "kind": "terminal" }, "b": { "kind": "terminal" } } },
            { "root": { "kind": "terminal", "cwd": 5 } },
            { "nothing": true },
            7,
            { "root": { "kind": "terminal", "cwd": "/ok" } },
        ]});
        let s = Session::from_json(&j.to_string()).unwrap();
        assert_eq!(s.tabs, vec![tab(term("/ok"))]);
        assert_eq!(s.active, 0);
    }

    #[test]
    fn ratio_is_clamped() {
        for (given, want) in [(0.0, 0.05), (-3.0, 0.05), (1.0, 0.95), (7.0, 0.95), (0.4, 0.4)] {
            let j = one_tab(json!({ "axis": "horizontal", "ratio": given, "a": { "kind": "terminal" }, "b": { "kind": "terminal" } }));
            let s = Session::from_json(&j).unwrap();
            let PaneNode::Split { ratio, .. } = s.tabs[0].root else { panic!("not a split") };
            assert_eq!(ratio, want, "ratio {given}");
        }
    }

    #[test]
    fn font_size_is_clamped() {
        for (given, want) in [(1.0, 6.0), (500.0, 72.0), (13.0, 13.0)] {
            let s = Session::from_json(&one_tab(json!({ "kind": "terminal", "font_size": given }))).unwrap();
            assert_eq!(s.tabs[0].root.leaves()[0].font_size, Some(want));
        }
        let s = Session::from_json(&one_tab(json!({ "kind": "terminal", "font_size": null }))).unwrap();
        assert_eq!(s.tabs[0].root.leaves()[0].font_size, None);
    }

    #[test]
    fn focused_and_active_are_clamped() {
        let j = json!({ "version": 1, "active": 9, "tabs": [
            { "root": { "kind": "terminal" } },
            { "focused": 5, "root": { "axis": "vertical", "ratio": 0.5, "a": { "kind": "terminal" }, "b": { "kind": "terminal" } } },
        ]});
        let s = Session::from_json(&j.to_string()).unwrap();
        assert_eq!(s.active, 1);
        assert_eq!(s.tabs[1].focused, 1);
    }

    #[test]
    fn explorer_active_needs_an_explorer() {
        let j = json!({ "version": 1, "explorer_active": true, "tabs": [{ "root": { "kind": "terminal" } }] });
        assert!(!Session::from_json(&j.to_string()).unwrap().explorer_active);
    }

    #[test]
    fn empty_cwd_is_none() {
        let s = Session::from_json(&one_tab(json!({ "kind": "terminal", "cwd": "" }))).unwrap();
        assert_eq!(s.tabs[0].root.leaves()[0].cwd, None);
    }

    #[test]
    fn nothing_to_restore_is_an_error() {
        assert!(Session::from_json(r#"{"version":1,"tabs":[]}"#).is_err());
        assert!(Session::from_json(r#"{"version":1}"#).is_err());
        assert!(Session::from_json(r#"{"version":1,"tabs":[{"root":{"kind":"x"}}]}"#).is_err());
    }

    #[test]
    fn wrong_or_missing_version_is_an_error() {
        let tabs = r#""tabs":[{"root":{"kind":"terminal"}}]"#;
        assert!(Session::from_json(&format!(r#"{{"version":2,{tabs}}}"#)).is_err());
        assert!(Session::from_json(&format!(r#"{{"version":"1",{tabs}}}"#)).is_err());
        assert!(Session::from_json(&format!(r#"{{{tabs}}}"#)).is_err());
        assert!(Session::from_json(&format!(r#"{{"version":1,{tabs}}}"#)).is_ok());
    }

    #[test]
    fn garbage_is_an_error() {
        for s in ["", "   ", "null", "[]", "42", "{", "not json", "\u{0}\u{1}", r#"{"version":1,"tabs":"x"}"#] {
            assert!(Session::from_json(s).is_err(), "{s:?}");
        }
    }

    #[test]
    fn absurdly_deep_tree_is_dropped() {
        let mut node = json!({ "kind": "terminal" });
        for _ in 0..200 {
            node = json!({ "axis": "vertical", "ratio": 0.5, "a": node, "b": { "kind": "terminal" } });
        }
        assert!(Session::from_json(&one_tab(node)).is_err());
    }

    #[test]
    fn floats_are_written_short() {
        let s = Session {
            tabs: vec![TabState {
                root: split(
                    Axis::Vertical,
                    0.6,
                    PaneNode::Leaf(PaneState { kind: PaneKind::Terminal, cwd: None, font_size: Some(13.3), command: None, sticky: false }),
                    term("/b"),
                ),
                focused: 0,
                zoomed: false,
            }],
            active: 0,
            explorer: None,
            explorer_active: false,
        };
        let j = s.to_json();
        let v: Value = serde_json::from_str(&j).unwrap();
        assert_eq!(v["tabs"][0]["root"]["ratio"].to_string(), "0.6");
        assert_eq!(v["tabs"][0]["root"]["a"]["font_size"].to_string(), "13.3");
        assert!(!j.contains("0.6000000"), "{j}");
        assert_eq!(Session::from_json(&j).unwrap(), s);
    }

    #[test]
    fn short_f32_reads_back_exactly() {
        for x in [0.05f32, 0.1, 0.3, 1.0 / 3.0, 0.5, 0.6, 0.95, 6.0, 13.3, 72.0] {
            assert_eq!(short_f32(x).as_f64().unwrap() as f32, x);
        }
        assert_eq!(short_f32(0.6).as_f64(), Some(0.6));
        assert_eq!(short_f32(f32::NAN), Value::Null);
    }

    #[cfg(unix)]
    #[test]
    fn lock_is_exclusive_and_released_on_drop() {
        let dir = std::env::temp_dir().join(format!("nex-session-lock-test-{}", std::process::id()));
        let path = dir.join("nex").join("session.json");
        let first = try_lock_at(&path).unwrap();
        assert!(first._file.is_some());
        assert!(dir.join("nex").join("session.json.lock").exists());
        assert!(try_lock_at(&path).is_none());
        drop(first);
        assert!(try_lock_at(&path).unwrap()._file.is_some());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn path_honors_xdg_state_home() {
        let home = Some(PathBuf::from("/home/u"));
        if cfg!(target_os = "macos") {
            assert_eq!(path_from(None, home), Some(PathBuf::from("/home/u/Library/Application Support/nex/session.json")));
        } else {
            assert_eq!(path_from(Some("/st".into()), home.clone()), Some(PathBuf::from("/st/nex/session.json")));
            assert_eq!(path_from(None, home.clone()), Some(PathBuf::from("/home/u/.local/state/nex/session.json")));
            // A relative XDG_STATE_HOME is invalid per the spec and ignored.
            assert_eq!(path_from(Some("rel".into()), home), Some(PathBuf::from("/home/u/.local/state/nex/session.json")));
            assert_eq!(path_from(None, None), None);
        }
    }

    #[test]
    fn save_load_clear() {
        let dir = std::env::temp_dir().join(format!("nex-session-test-{}", std::process::id()));
        let path = dir.join("nex").join("session.json");
        let s = Session { tabs: vec![tab(term("/a"))], active: 0, explorer: None, explorer_active: false };
        assert_eq!(load_from(&path), None);
        save_to(&path, &s).unwrap();
        assert!(!path.with_extension(format!("json.{}.tmp", std::process::id())).exists());
        assert_eq!(load_from(&path), Some(s));
        std::fs::write(&path, "garbage").unwrap();
        assert_eq!(load_from(&path), None);
        clear_at(&path);
        assert!(!path.exists());
        clear_at(&path);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
