//! Copied files and images on the clipboard, where nex has no Wayland data device of its own
//! (X11, or Wayland when `Dnd` couldn't start): read with `wl-paste` or `xclip`, if installed.

use std::time::Duration;

use crate::file_explorer::fs::ops::parse_paths;
use crate::platform::ClipboardFiles;
use crate::platform::unix::output_within;

const TIMEOUT: Duration = Duration::from_secs(3);

pub fn clipboard_files() -> Option<ClipboardFiles> {
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some();
    let read = |mime: &str| -> Option<Vec<u8>> {
        if wayland {
            output_within("wl-paste", &["--no-newline", "--type", mime], TIMEOUT)
        } else {
            output_within("xclip", &["-selection", "clipboard", "-t", mime, "-o"], TIMEOUT)
        }
    };
    let types = if wayland {
        output_within("wl-paste", &["--list-types"], TIMEOUT)?
    } else {
        output_within("xclip", &["-selection", "clipboard", "-t", "TARGETS", "-o"], TIMEOUT)?
    };
    let types = String::from_utf8_lossy(&types);
    let has = |t: &str| types.lines().any(|l| l.trim() == t);
    if has("text/uri-list") {
        let paths = parse_paths(&String::from_utf8_lossy(&read("text/uri-list")?));
        if !paths.is_empty() {
            return Some(ClipboardFiles::Paths(paths));
        }
    }
    if has("image/png") {
        return read("image/png").filter(|b| !b.is_empty()).map(ClipboardFiles::Png);
    }
    None
}
