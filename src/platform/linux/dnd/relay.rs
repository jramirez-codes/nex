//! Touch drags between nex windows, relayed through sway and a local socket.
//!
//! On sway (1.11), a drag started by a finger stays with the window it started in: the
//! compositor keeps reporting it to that window's surface, at positions outside it, and never
//! offers it to the window under the finger (a mouse drag moves between windows as it should).
//! So when this window sees a drag outside its own bounds, it asks sway which window is there
//! and, if that's another nex, passes the hover and the drop on to it over that nex's socket,
//! `$XDG_RUNTIME_DIR/nex-dnd-<pid>.sock`.
//!
//! A message is one connection: `hover X Y`, `leave`, or `drop X Y` followed by the dropped
//! files as a `text/uri-list`. Positions are in the receiving window's surface coordinates.

use std::io::{Read, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::Value;

use super::{Shared, log};
use crate::file_explorer::fs_ops::parse_paths;

fn socket_path(pid: u32) -> Option<PathBuf> {
    let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
    Some(Path::new(&dir).join(format!("nex-dnd-{pid}.sock")))
}

/// This window's socket; removed when dropped.
pub(super) struct Listener(PathBuf);

impl Drop for Listener {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Accept drags relayed to this window by other nex windows.
pub(super) fn listen(shared: Arc<Mutex<Shared>>, ctx: egui::Context) -> Option<Listener> {
    let path = socket_path(std::process::id())?;
    let _ = std::fs::remove_file(&path);
    let listener = UnixListener::bind(&path).ok()?;
    std::thread::Builder::new()
        .name("nex-dnd-relay".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                let mut msg = String::new();
                if stream.read_to_string(&mut msg).is_ok() {
                    apply(&shared, &msg);
                    ctx.request_repaint();
                }
            }
        })
        .ok()?;
    Some(Listener(path))
}

/// Apply a relayed message as if the compositor had sent it.
fn apply(shared: &Mutex<Shared>, msg: &str) {
    let (head, body) = msg.split_once('\n').unwrap_or((msg, ""));
    let mut words = head.split_whitespace();
    let cmd = words.next().unwrap_or("");
    let mut num = || words.next().and_then(|w| w.parse::<f64>().ok());
    let at = num().zip(num());
    log(format_args!("relayed to this window: {cmd} {at:?}"));
    let mut s = shared.lock().unwrap();
    match (cmd, at) {
        ("hover", Some(at)) => s.hover = Some(at),
        ("leave", _) => s.hover = None,
        ("drop", Some(at)) => {
            s.hover = None;
            let paths = parse_paths(body);
            if !paths.is_empty() {
                s.dropped.push((paths, at));
            }
        }
        _ => {}
    }
}

/// Send a message to the nex with process id `pid`.
pub(super) fn send(pid: u32, msg: &str) {
    let Some(path) = socket_path(pid) else { return };
    match UnixStream::connect(&path) {
        Ok(mut stream) => {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let _ = stream.write_all(msg.as_bytes());
        }
        Err(e) => log(format_args!("relay to {}: {e}", path.display())),
    }
}

/// A window's surface on screen, in sway's layout coordinates.
#[derive(Clone, Copy, Debug)]
struct Win {
    pid: u32,
    x: f64,
    y: f64,
    w: f64,
    h: f64,
}

/// Where this window is, and the other nex windows showing now.
#[derive(Debug)]
pub(super) struct Layout {
    origin: (f64, f64),
    others: Vec<Win>,
}

impl Layout {
    /// Ask sway. `None` when not on sway, or this window isn't showing.
    pub(super) fn query() -> Option<Self> {
        let mut wins = Vec::new();
        visible_windows(&sway_tree()?, &mut wins);
        let me = std::process::id();
        let mine = *wins.iter().find(|w| w.pid == me)?;
        // Only windows that are a nex with drag and drop (they have a socket).
        let others = wins.into_iter().filter(|w| w.pid != me && socket_path(w.pid).is_some_and(|p| p.exists())).collect();
        let layout = Layout { origin: (mine.x, mine.y), others };
        log(format_args!("layout: {layout:?}"));
        Some(layout)
    }

    /// The nex window under `at` (in this window's surface coordinates), and where that is in
    /// its surface.
    pub(super) fn target(&self, at: (f64, f64)) -> Option<(u32, (f64, f64))> {
        let (gx, gy) = (self.origin.0 + at.0, self.origin.1 + at.1);
        self.others
            .iter()
            .find(|w| gx >= w.x && gx < w.x + w.w && gy >= w.y && gy < w.y + w.h)
            .map(|w| (w.pid, (gx - w.x, gy - w.y)))
    }
}

/// Every visible application window in sway's tree, by its surface's place on screen.
fn visible_windows(node: &Value, out: &mut Vec<Win>) {
    if let (Some(pid), Some(true)) = (node["pid"].as_u64(), node["visible"].as_bool()) {
        let (r, wr) = (&node["rect"], &node["window_rect"]);
        let f = |v: &Value| v.as_f64().unwrap_or(0.0);
        out.push(Win { pid: pid as u32, x: f(&r["x"]) + f(&wr["x"]), y: f(&r["y"]) + f(&wr["y"]), w: f(&wr["width"]), h: f(&wr["height"]) });
    }
    for key in ["nodes", "floating_nodes"] {
        for child in node[key].as_array().into_iter().flatten() {
            visible_windows(child, out);
        }
    }
}

/// Ask sway to focus this window (after files were dropped on it from another window, so
/// what was typed in can be acted on without tapping the window first).
pub(super) fn focus_this_window() {
    let cmd = format!("[pid={}] focus", std::process::id());
    let reply = sway_request(RUN_COMMAND, cmd.as_bytes());
    log(format_args!("focus this window: {reply:?}"));
}

const RUN_COMMAND: u32 = 0;
const GET_TREE: u32 = 4;

/// sway's layout tree.
fn sway_tree() -> Option<Value> {
    sway_request(GET_TREE, b"")
}

/// One request over sway's IPC socket, and its JSON reply.
fn sway_request(kind: u32, payload: &[u8]) -> Option<Value> {
    let sock = std::env::var_os("SWAYSOCK").map(PathBuf::from).filter(|p| p.exists()).or_else(|| {
        let dir = std::env::var_os("XDG_RUNTIME_DIR")?;
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).find(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("sway-ipc.") && n.ends_with(".sock"))
        })
    })?;
    let mut stream = UnixStream::connect(sock).ok()?;
    stream.set_read_timeout(Some(Duration::from_secs(1))).ok()?;
    let mut request = b"i3-ipc".to_vec();
    request.extend((payload.len() as u32).to_le_bytes());
    request.extend(kind.to_le_bytes());
    request.extend(payload);
    stream.write_all(&request).ok()?;
    let mut header = [0u8; 14];
    stream.read_exact(&mut header).ok()?;
    let len = u32::from_le_bytes(header[6..10].try_into().ok()?) as usize;
    let mut body = vec![0; len];
    stream.read_exact(&mut body).ok()?;
    serde_json::from_slice(&body).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_the_window_under_a_point() {
        let tree: Value = serde_json::from_str(
            r#"{"nodes": [{"nodes": [
                {"pid": 10, "visible": true, "rect": {"x": 0, "y": 0}, "window_rect": {"x": 0, "y": 0, "width": 960, "height": 1080}},
                {"pid": 20, "visible": true, "rect": {"x": 960, "y": 0}, "window_rect": {"x": 0, "y": 0, "width": 960, "height": 1080}},
                {"pid": 30, "visible": false, "rect": {"x": 0, "y": 0}, "window_rect": {"x": 0, "y": 0, "width": 1920, "height": 1080}}
            ]}]}"#,
        )
        .unwrap();
        let mut wins = Vec::new();
        visible_windows(&tree, &mut wins);
        assert_eq!(wins.iter().map(|w| w.pid).collect::<Vec<_>>(), vec![10, 20]);
        // Seen from window 20: a finger 200 px left of it is over window 10.
        let layout = Layout { origin: (960.0, 0.0), others: vec![wins[0]] };
        assert_eq!(layout.target((-200.0, 500.0)), Some((10, (760.0, 500.0))));
        assert_eq!(layout.target((100.0, 500.0)), None);
    }

    #[test]
    fn relayed_messages_update_the_drag() {
        let shared = Mutex::new(Shared::default());
        apply(&shared, "hover 12 34\n");
        assert_eq!(shared.lock().unwrap().hover, Some((12.0, 34.0)));
        apply(&shared, "leave\n");
        assert_eq!(shared.lock().unwrap().hover, None);
        let file = std::env::current_exe().unwrap();
        apply(&shared, &format!("drop 5 6\n{}\r\n", super::super::file_uri(&file)));
        let s = shared.lock().unwrap();
        assert_eq!(s.dropped, vec![(vec![file], (5.0, 6.0))]);
    }
}
