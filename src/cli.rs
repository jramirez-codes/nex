//! Command-line flags, for launching nex from scripts and window-manager configs:
//!
//! ```text
//! nex [--config=PATH] [--app-id=ID] [--working-directory=DIR] [--explorer] [--select=PATH] [FOLDER] [-e COMMAND [ARGS...]]
//! nex --remote=USER@HOST[:PORT]
//! nex --file-manager-service
//! ```
//!
//! Flags take their value as `--flag=value` or `--flag value`. Everything after `-e` is the
//! command to run in the first pane instead of the shell. A bare `FOLDER` (a path or a
//! `file://` URI, as a file manager's desktop entry passes it) opens the Files tab there.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: nex [OPTIONS] [FOLDER] [-e COMMAND [ARGS...]]

  FOLDER                    open the Files tab in FOLDER (a path or file:// URI)

  --config=PATH             read this config file instead of ~/.config/nex/config
  --app-id=ID               Wayland app_id / X11 class of the window (default: nex)
  --working-directory=DIR   start the first shell in DIR
  --explorer                open the Files tab at startup
  --select=PATH             open the Files tab in PATH's folder, with PATH selected
  --remote=USER@HOST[:PORT] open every pane on HOST with nex's own ssh client, as
                            the Android app does (password once, then a device key)
  --file-manager-service    show files for other apps (\"Show in Folder\"), over D-Bus
  -e COMMAND [ARGS...]      run COMMAND in the first pane instead of the shell
  -h, --help                show this help
  -V, --version             show the version
";

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub config: Option<PathBuf>,
    pub app_id: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// Open the Files tab at startup (`--explorer`, or a bare folder argument).
    pub explorer: bool,
    /// Select this item in the Files tab at startup (`--select`).
    pub select: Option<PathBuf>,
    /// Serve `org.freedesktop.FileManager1` instead of opening a window.
    pub file_manager_service: bool,
    /// Program and arguments for the first pane.
    pub command: Option<Vec<String>>,
    /// `--remote`: every pane on this host (`user@host[:port]`), with nex's own ssh client.
    pub remote: Option<String>,
    /// Don't bring back the last session (`--no-restore`, hidden: new windows pass it).
    pub no_restore: bool,
    pub help: bool,
    pub version: bool,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut out = Args::default();
        let mut folder = None;
        let mut it = args.into_iter().map(|a| a.into_string().map_err(|a| format!("invalid argument {a:?}")));
        while let Some(arg) = it.next() {
            let arg = arg?;
            let (flag, inline) = match arg.split_once('=') {
                Some((f, v)) if f.starts_with("--") => (f.to_string(), Some(v.to_string())),
                _ => (arg.clone(), None),
            };
            let mut value = || match inline.clone() {
                Some(v) => Ok(v),
                None => it.next().ok_or_else(|| format!("{flag} needs a value"))?,
            };
            match flag.as_str() {
                "--config" => out.config = Some(value()?.into()),
                "--app-id" | "--class" => out.app_id = Some(value()?),
                "--working-directory" => out.working_directory = Some(value()?.into()),
                "-e" | "--command" => {
                    let cmd = it.by_ref().collect::<Result<Vec<_>, _>>()?;
                    if cmd.is_empty() {
                        return Err(format!("{flag} needs a command"));
                    }
                    out.command = Some(cmd);
                }
                "--remote" => out.remote = Some(value()?),
                "--explorer" => out.explorer = true,
                "--select" => out.select = Some(local_path(&value()?)),
                "--file-manager-service" => out.file_manager_service = true,
                "--no-restore" => out.no_restore = true,
                "-h" | "--help" => out.help = true,
                "-V" | "--version" => out.version = true,
                _ if !arg.starts_with('-') && folder.is_none() => folder = Some(folder_arg(&arg)),
                _ if !arg.starts_with('-') => return Err(format!("only one folder can be opened, got `{arg}` too")),
                _ => return Err(format!("unknown argument `{arg}`")),
            }
        }
        if let Some(item) = &out.select {
            if folder.is_some() {
                return Err("--select already names the folder to open".into());
            }
            folder = Some(item.parent().map(PathBuf::from).unwrap_or_else(|| item.clone()));
        }
        if let Some(dir) = folder {
            out.working_directory = Some(dir);
            out.explorer = true;
        }
        Ok(out)
    }

    /// Whether this launch brings back the last session: a plain `nex`, with no command,
    /// folder or Files tab asked for, and not a second window.
    pub fn restores_session(&self) -> bool {
        !self.no_restore
            && self.command.is_none()
            && self.remote.is_none()
            && self.working_directory.is_none()
            && !self.explorer
            && self.select.is_none()
            && !self.file_manager_service
    }

    /// Flags a new window (Ctrl+Shift+N) inherits: same config and app id, not the command,
    /// and never the last session (that is the first window's).
    pub fn for_new_window(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(c) = &self.config {
            v.push(format!("--config={}", c.display()));
        }
        if let Some(id) = &self.app_id {
            v.push(format!("--app-id={id}"));
        }
        v.push("--no-restore".into());
        v
    }
}

/// A folder argument as a path: `file://` URIs (with or without a `localhost` host) are
/// decoded, and a file opens its parent folder.
fn folder_arg(arg: &str) -> PathBuf {
    let path = local_path(arg);
    match path.parent() {
        Some(parent) if path.is_file() => parent.to_path_buf(),
        _ => path,
    }
}

/// A path or `file://` URI (with or without a `localhost` host) as a path.
fn local_path(arg: &str) -> PathBuf {
    match arg.strip_prefix("file://") {
        Some(rest) => PathBuf::from(crate::file_explorer::fs::ops::percent_decode(rest.strip_prefix("localhost").unwrap_or(rest))),
        None => PathBuf::from(arg),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Args, String> {
        Args::parse(args.iter().map(OsString::from))
    }

    #[test]
    fn no_args() {
        assert_eq!(parse(&[]), Ok(Args::default()));
    }

    #[test]
    fn flags_with_and_without_equals() {
        let a = parse(&["--config=/a/b", "--app-id", "x.y", "--working-directory=/root"]).unwrap();
        assert_eq!(a.config, Some("/a/b".into()));
        assert_eq!(a.app_id.as_deref(), Some("x.y"));
        assert_eq!(a.working_directory, Some("/root".into()));
        assert_eq!(a.command, None);
    }

    #[test]
    fn command_takes_the_rest() {
        let a = parse(&["--app-id=w", "-e", "nmtui", "connect", "--app-id=z"]).unwrap();
        assert_eq!(a.app_id.as_deref(), Some("w"));
        assert_eq!(a.command, Some(vec!["nmtui".into(), "connect".into(), "--app-id=z".into()]));
    }

    #[test]
    fn explorer_flag() {
        let a = parse(&["--explorer"]).unwrap();
        assert!(a.explorer);
        assert_eq!(a.working_directory, None);
    }

    #[test]
    fn folder_opens_the_explorer() {
        let a = parse(&["/tmp"]).unwrap();
        assert!(a.explorer);
        assert_eq!(a.working_directory, Some("/tmp".into()));
        for uri in ["file:///tmp/a%20b", "file://localhost/tmp/a%20b"] {
            assert_eq!(parse(&[uri]).unwrap().working_directory, Some("/tmp/a b".into()));
        }
    }

    #[test]
    fn folder_given_a_file_opens_its_parent() {
        let dir = std::env::temp_dir().join(format!("nex-cli-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("note.txt");
        std::fs::write(&file, "").unwrap();
        assert_eq!(parse(&[file.to_str().unwrap()]).unwrap().working_directory, Some(dir.clone()));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn select_opens_the_items_folder() {
        let a = parse(&["--select=file:///tmp/a%20b.txt"]).unwrap();
        assert!(a.explorer);
        assert_eq!(a.working_directory, Some("/tmp".into()));
        assert_eq!(a.select, Some("/tmp/a b.txt".into()));
        assert!(parse(&["--select", "/tmp/x", "/etc"]).is_err());
    }

    #[test]
    fn file_manager_service_flag() {
        let a = parse(&["--config=/c", "--app-id=t", "--file-manager-service"]).unwrap();
        assert!(a.file_manager_service);
        assert_eq!(a.for_new_window(), vec!["--config=/c".to_string(), "--app-id=t".to_string(), "--no-restore".to_string()]);
    }

    #[test]
    fn folder_and_command() {
        let a = parse(&["/tmp", "-e", "top", "/x"]).unwrap();
        assert_eq!(a.working_directory, Some("/tmp".into()));
        assert_eq!(a.command, Some(vec!["top".into(), "/x".into()]));
    }

    #[test]
    fn errors() {
        assert!(parse(&["/a", "/b"]).is_err());
        assert!(parse(&["--config"]).is_err());
        assert!(parse(&["-e"]).is_err());
        assert!(parse(&["--bogus"]).is_err());
    }

    #[test]
    fn new_window_keeps_config_and_app_id() {
        let a = parse(&["--config=/c", "--app-id=t", "-e", "top"]).unwrap();
        assert_eq!(a.for_new_window(), vec!["--config=/c".to_string(), "--app-id=t".to_string(), "--no-restore".to_string()]);
    }

    #[test]
    fn only_a_plain_launch_restores_the_session() {
        assert!(parse(&[]).unwrap().restores_session());
        assert!(parse(&["--config=/c", "--app-id=t"]).unwrap().restores_session());
        let a = parse(&["--no-restore"]).unwrap();
        assert!(a.no_restore);
        assert!(!a.restores_session());
        for args in [&["-e", "top"][..], &["--working-directory=/tmp"], &["/tmp"], &["--explorer"], &["--select=/tmp/x"], &["--file-manager-service"]] {
            assert!(!parse(args).unwrap().restores_session(), "{args:?}");
        }
        // A new window's flags parse back to a window that doesn't restore.
        let flags = parse(&["--config=/c"]).unwrap().for_new_window();
        assert!(!Args::parse(flags.into_iter().map(OsString::from)).unwrap().restores_session());
    }
}
