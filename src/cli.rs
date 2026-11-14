//! Command-line flags, for launching nex from scripts and window-manager configs:
//!
//! ```text
//! nex [--config=PATH] [--app-id=ID] [--working-directory=DIR] [-e COMMAND [ARGS...]]
//! ```
//!
//! Flags take their value as `--flag=value` or `--flag value`. Everything after `-e` is the
//! command to run in the first pane instead of the shell.

use std::ffi::OsString;
use std::path::PathBuf;

pub const USAGE: &str = "\
usage: nex [OPTIONS] [-e COMMAND [ARGS...]]

  --config=PATH             read this config file instead of ~/.config/nex/config
  --app-id=ID               Wayland app_id / X11 class of the window (default: nex)
  --working-directory=DIR   start the first shell in DIR
  -e COMMAND [ARGS...]      run COMMAND in the first pane instead of the shell
  -h, --help                show this help
  -V, --version             show the version
";

#[derive(Debug, Default, PartialEq)]
pub struct Args {
    pub config: Option<PathBuf>,
    pub app_id: Option<String>,
    pub working_directory: Option<PathBuf>,
    /// Program and arguments for the first pane.
    pub command: Option<Vec<String>>,
    pub help: bool,
    pub version: bool,
}

impl Args {
    pub fn parse(args: impl IntoIterator<Item = OsString>) -> Result<Self, String> {
        let mut out = Args::default();
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
                "-h" | "--help" => out.help = true,
                "-V" | "--version" => out.version = true,
                _ => return Err(format!("unknown argument `{arg}`")),
            }
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
    fn errors() {
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
