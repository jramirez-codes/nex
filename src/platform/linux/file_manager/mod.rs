//! `nex --file-manager-service`: answers `org.freedesktop.FileManager1` on the session bus,
//! the interface apps call to show a file or folder in the file manager (Firefox's and
//! Chromium's "Show in Folder", VS Code's "Reveal in File Explorer", portals, …).
//!
//! The bus starts this on the first call when nex's service file is installed
//! (`assets/org.freedesktop.FileManager1.service`, see the README). It then stays on the bus
//! and opens a nex window for each request. `--config` and `--app-id` given to the service are
//! passed on to those windows (`Exec=nex --config=… --file-manager-service`):
//!
//! | Method | nex opens |
//! |---|---|
//! | `ShowFolders(uris, startup_id)` | each folder in the Files tab |
//! | `ShowItems(uris, startup_id)` | each item's folder, with the item selected |
//! | `ShowItemProperties(uris, startup_id)` | the same as `ShowItems` |

mod dbus;

use std::path::{Path, PathBuf};

const NAME: &str = "org.freedesktop.FileManager1";
const OBJECT: &str = "/org/freedesktop/FileManager1";

const INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node>
  <interface name="org.freedesktop.FileManager1">
    <method name="ShowFolders"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
    <method name="ShowItems"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
    <method name="ShowItemProperties"><arg type="as" name="URIs" direction="in"/><arg type="s" name="StartupId" direction="in"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Introspectable">
    <method name="Introspect"><arg type="s" name="xml_data" direction="out"/></method>
  </interface>
  <interface name="org.freedesktop.DBus.Peer">
    <method name="Ping"/>
  </interface>
</node>
"#;

/// Own `org.freedesktop.FileManager1` and serve it until the bus goes away. Every window
/// gets `flags` too (the service's own `--config` / `--app-id`), so it looks like the
/// rest of the user's nex windows.
pub fn file_manager_service(flags: &[String]) -> Result<(), String> {
    let mut bus = dbus::Connection::session().map_err(|e| format!("can't connect to the session bus: {e}"))?;
    match bus.request_name(NAME).map_err(|e| format!("can't request {NAME}: {e}"))? {
        dbus::PRIMARY_OWNER | dbus::ALREADY_OWNER => {}
        _ => return Err(format!("another file manager already owns {NAME}")),
    }
    let nex = std::env::current_exe().map_err(|e| format!("can't find nex's own path: {e}"))?;
    let display = display_env();
    loop {
        let Some(msg) = bus.read().map_err(|e| format!("reading from the bus: {e}"))? else { return Ok(()) };
        if msg.kind != dbus::METHOD_CALL {
            continue;
        }
        let result = match (msg.interface.as_str(), msg.member.as_str()) {
            (NAME | "", member @ ("ShowFolders" | "ShowItems" | "ShowItemProperties")) => match msg.string_array() {
                Ok(uris) => {
                    for args in window_args(member == "ShowFolders", &uris) {
                        open_window(&nex, flags, &args, &display);
                    }
                    Ok(None)
                }
                Err(e) => Err(("org.freedesktop.DBus.Error.InvalidArgs", e.to_string())),
            },
            ("org.freedesktop.DBus.Introspectable" | "", "Introspect") if msg.path == OBJECT || msg.path == "/" => {
                let mut body = dbus::Writer::default();
                body.string(if msg.path == OBJECT { INTROSPECTION } else { ROOT_INTROSPECTION });
                Ok(Some(body.buf))
            }
            ("org.freedesktop.DBus.Peer" | "", "Ping") => Ok(None),
            (interface, member) => Err(("org.freedesktop.DBus.Error.UnknownMethod", format!("nex has no method {interface}.{member}"))),
        };
        if !msg.wants_reply() {
            continue;
        }
        let sent = match result {
            Ok(None) => bus.reply(&msg, "", &[]),
            Ok(Some(body)) => bus.reply(&msg, "s", &body),
            Err((name, text)) => bus.reply_error(&msg, name, &text),
        };
        sent.map_err(|e| format!("replying on the bus: {e}"))?;
    }
}

const ROOT_INTROSPECTION: &str = r#"<!DOCTYPE node PUBLIC "-//freedesktop//DTD D-BUS Object Introspection 1.0//EN"
 "http://www.freedesktop.org/standards/dbus/1.0/introspect.dtd">
<node><node name="org"/></node>
"#;

/// The nex arguments for each window a request opens: one per folder for `ShowFolders`, and
/// one per folder holding the items for the others (selecting the first item in each).
/// Only local `file://` URIs (or plain paths) can be shown; others are skipped.
fn window_args(folders: bool, uris: &[String]) -> Vec<String> {
    let mut seen: Vec<PathBuf> = Vec::new();
    let mut out = Vec::new();
    for uri in uris {
        let Some(path) = local_path(uri) else {
            eprintln!("nex: can't show `{uri}`: only local files can be shown");
            continue;
        };
        let folder = if folders { path.clone() } else { path.parent().map(Path::to_path_buf).unwrap_or_else(|| path.clone()) };
        if seen.contains(&folder) {
            continue;
        }
        seen.push(folder);
        let flag = if folders || path.parent().is_none() { "" } else { "--select=" };
        out.push(format!("{flag}{}", path.display()));
    }
    out
}

/// A `file://` URI (with an empty or `localhost` host) or an absolute path, as a path.
fn local_path(uri: &str) -> Option<PathBuf> {
    let path = match uri.strip_prefix("file://") {
        Some(rest) => crate::file_explorer::fs::ops::percent_decode(rest.strip_prefix("localhost").unwrap_or(rest)),
        None if uri.starts_with('/') => uri.to_string(),
        None => return None,
    };
    path.starts_with('/').then(|| PathBuf::from(path).components().collect())
}

/// Start a nex window (a separate process, like Ctrl+Shift+N) and reap it in the background.
fn open_window(nex: &Path, flags: &[String], arg: &str, display: &[(&'static str, String)]) {
    let child = std::process::Command::new(nex)
        .args(flags)
        .arg(arg)
        .envs(display.iter().map(|(k, v)| (*k, v)))
        .current_dir(crate::platform::home_dir().unwrap_or_else(|| PathBuf::from("/")))
        .stdin(std::process::Stdio::null())
        .spawn();
    match child {
        Ok(mut child) => {
            std::thread::spawn(move || child.wait());
        }
        Err(e) => eprintln!("nex: can't start {}: {e}", nex.display()),
    }
}

/// Display variables for the windows, when the bus started this service without them (bus
/// activation only passes what was handed to `dbus-update-activation-environment`): the
/// first Wayland socket in `$XDG_RUNTIME_DIR`, else X display `:0` if its socket exists.
fn display_env() -> Vec<(&'static str, String)> {
    let set = |k| std::env::var_os(k).is_some_and(|v| !v.is_empty());
    if set("WAYLAND_DISPLAY") || set("DISPLAY") {
        return Vec::new();
    }
    let runtime = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from);
    if let Some(name) = runtime.as_deref().and_then(first_wayland_socket) {
        return vec![("WAYLAND_DISPLAY", name)];
    }
    if Path::new("/tmp/.X11-unix/X0").exists() {
        return vec![("DISPLAY", ":0".into())];
    }
    Vec::new()
}

/// `wayland-0`, `wayland-1`, …: the lowest-numbered compositor socket in `dir`.
fn first_wayland_socket(dir: &Path) -> Option<String> {
    use std::os::unix::fs::FileTypeExt;
    let mut found: Vec<(u32, String)> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_socket()))
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let n = name.strip_prefix("wayland-")?.parse().ok()?;
            Some((n, name))
        })
        .collect();
    found.sort();
    found.into_iter().next().map(|(_, name)| name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn strings(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn local_paths() {
        assert_eq!(local_path("file:///tmp/a%20b.txt"), Some("/tmp/a b.txt".into()));
        assert_eq!(local_path("file://localhost/etc/"), Some("/etc".into()));
        assert_eq!(local_path("/home/x"), Some("/home/x".into()));
        assert_eq!(local_path("sftp://host/x"), None);
        assert_eq!(local_path("file://otherhost/x"), None);
    }

    #[test]
    fn items_open_their_folder_once_with_the_first_selected() {
        let args = window_args(false, &strings(&["file:///tmp/a.txt", "file:///tmp/b.txt", "file:///etc/hosts", "trash:///x"]));
        assert_eq!(args, strings(&["--select=/tmp/a.txt", "--select=/etc/hosts"]));
    }

    #[test]
    fn folders_open_themselves() {
        assert_eq!(window_args(true, &strings(&["file:///tmp/", "file:///tmp"])), strings(&["/tmp"]));
        assert_eq!(window_args(false, &strings(&["file:///"])), strings(&["/"]));
    }

    #[test]
    fn finds_the_lowest_wayland_socket() {
        let dir = std::env::temp_dir().join(format!("nex-fm-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let _a = std::os::unix::net::UnixListener::bind(dir.join("wayland-3")).unwrap();
        let _b = std::os::unix::net::UnixListener::bind(dir.join("wayland-1")).unwrap();
        std::fs::write(dir.join("wayland-0.lock"), "").unwrap();
        std::fs::write(dir.join("wayland-0"), "").unwrap(); // a plain file, not a socket
        assert_eq!(first_wayland_socket(&dir).as_deref(), Some("wayland-1"));
        std::fs::remove_dir_all(dir).unwrap();
    }
}
