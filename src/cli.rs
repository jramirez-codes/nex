//! Command-line flags, for launching nex from scripts and window-manager configs:
//!
//! ```text
//! nex [--config=PATH] [--app-id=ID] [--working-directory=DIR] [--explorer] [FOLDER] [-e COMMAND [ARGS...]]
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
    /// Program and arguments for the first pane.
    pub command: Option<Vec<String>>,
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
                "--explorer" => out.explorer = true,
                "-h" | "--help" => out.help = true,
                "-V" | "--version" => out.version = true,
                _ if !arg.starts_with('-') && folder.is_none() => folder = Some(folder_arg(&arg)),
                _ if !arg.starts_with('-') => return Err(format!("only one folder can be opened, got `{arg}` too")),
                _ => return Err(format!("unknown argument `{arg}`")),
            }
        }
        if let Some(dir) = folder {
            out.working_directory = Some(dir);
            out.explorer = true;
        }
        Ok(out)
    }

    /// Flags a new window (Ctrl+Shift+N) inherits: same config and app id, not the command.
    pub fn for_new_window(&self) -> Vec<String> {
        let mut v = Vec::new();
        if let Some(c) = &self.config {
            v.push(format!("--config={}", c.display()));
        }
        if let Some(id) = &self.app_id {
            v.push(format!("--app-id={id}"));
        }
        v
    }
}

/// A folder argument as a path: `file://` URIs (with or without a `localhost` host) are
/// decoded, and a file opens its parent folder.
fn folder_arg(arg: &str) -> PathBuf {
    let path = match arg.strip_prefix("file://") {
        Some(rest) => PathBuf::from(crate::file_explorer::fs_ops::percent_decode(rest.strip_prefix("localhost").unwrap_or(rest))),
        None => PathBuf::from(arg),
    };
    match path.parent() {
        Some(parent) if path.is_file() => parent.to_path_buf(),
        _ => path,
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
        assert_eq!(a.for_new_window(), vec!["--config=/c".to_string(), "--app-id=t".to_string()]);
    }
}
