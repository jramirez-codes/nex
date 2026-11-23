//! Talking to the host a pane's `ssh` is connected to, over a second connection of nex's own.
//!
//! ```text
//! remote/
//!   mod.rs     this file: connection sharing in ~/.ssh/config, and the shell quoting and
//!              `ssh` command every file here starts from
//!   target.rs  SshTarget: a pane's ssh command line read, and the command lines nex builds
//!              from it (a new session, an agent to attach to, nex's own batch connections)
//!   error.rs   RemoteError: how an `ssh` of nex's own failed, read one way for every caller
//!   mux.rs     one shared connection per host for all of nex's own, so only one logs in
//!   upload.rs  files dropped or pasted into an ssh pane, copied to the remote host
//!   exec.rs    shell scripts run on the host, for Files and Changes panes opened from an
//!              ssh pane (with `ssh`, or over nex's own connection)
//!   host.rs    nex-host on the host: installing it, watching the agents it keeps running there
//!   mic.rs     the local microphone shared with the remote host, as its default mic
//!   audio.rs   recording from this machine's default microphone (cpal), for mic.rs
//!   ssh/       nex's own ssh client, for hosts it connects to itself (the Android app,
//!              `nex --remote`): no `ssh` program involved
//! ```
//!
//! nex's connections reuse the pane's own `ssh` arguments (port, user, identity, jump hosts,
//! `-o` options), minus forwardings and the remote command. They run with `BatchMode=yes`:
//! they can't ask for a password, so they work with keys, an agent, or a `ControlMaster`
//! connection to share. For hosts that want a password, nex offers to add a block to
//! `~/.ssh/config` (see [`enable_sharing`]) that makes every `ssh` started in a nex pane share
//! its connection, so nex's connections ride on the login typed into the pane.

pub mod audio;
mod error;
pub mod exec;
pub mod host;
pub mod mic;
mod mux;
pub mod ssh;
mod target;
mod upload;

pub use error::RemoteError;
pub use target::SshTarget;
pub use upload::{remove_saved, save_png, upload};
#[cfg(target_os = "linux")]
pub use upload::save_image;

use std::path::PathBuf;
use std::process::Command;

/// The first line of the block nex adds to `~/.ssh/config`, to tell whether it's there.
const SHARING_MARK: &str = "# Added by nex:";

/// Connection sharing for `ssh` run inside nex (panes set `TERM_PROGRAM=nex`, and so does the
/// upload). It goes at the end of the file, so the user's own settings come first and win.
/// `ControlPersist` puts the shared connection in the background, so closing the pane that
/// opened it doesn't wait on other panes using it, and it ends a minute after the last one.
/// The `x` prefix keeps `test` happy when the variable is unset, in any shell.
const SHARING_BLOCK: &str = "# Added by nex: ssh started in a nex pane shares its connection, so files dropped
# into the pane are uploaded over the login you already typed. Safe to remove.
Match exec \"test x$TERM_PROGRAM = xnex\"
  ControlMaster auto
  ControlPath ~/.ssh/nex-%C
  ControlPersist 60
";

/// The `ssh` program, as every connection of nex's own runs it: with `TERM_PROGRAM=nex`, so
/// the sharing block in `~/.ssh/config` applies whatever terminal nex itself was started from.
fn ssh_command() -> Command {
    let mut ssh = Command::new("ssh");
    ssh.env("TERM_PROGRAM", "nex");
    ssh
}

/// The user's ssh config file.
fn ssh_config() -> Option<PathBuf> {
    crate::platform::home_dir().map(|h| h.join(".ssh").join("config"))
}

/// Whether `~/.ssh/config` has nex's sharing block (see [`enable_sharing`]).
pub fn sharing_enabled() -> bool {
    ssh_config().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|c| has_sharing(&c))
}

fn has_sharing(config: &str) -> bool {
    config.lines().any(|l| l.trim_start().starts_with(SHARING_MARK))
}

/// Whether a shared connection to `target`'s host is up at the control path the user's
/// config names, so a new session on it needs no login. Asks the control socket only
/// (`ssh -O check`): quick, and never prompts.
pub fn shared_connection_live(target: &SshTarget) -> bool {
    SHARING_SUPPORTED && mux::check(target, None)
}

/// A remote directory as a word for `cd`: quoted, but with a leading `~` left bare so the
/// remote shell still expands it. No directory is the remote home.
pub fn cd_word(dir: &str) -> String {
    match dir.strip_prefix("~/") {
        _ if dir.is_empty() || dir == "~" => "~".into(),
        Some("") => "~".into(),
        Some(rest) => format!("~/{}", posix_quote(rest)),
        None => posix_quote(dir),
    }
}

/// Connection sharing is a Unix ssh feature; Windows' OpenSSH doesn't have it.
pub const SHARING_SUPPORTED: bool = cfg!(unix);

/// Add the sharing block to the end of `~/.ssh/config`, making the file (private, as ssh
/// requires) and `~/.ssh` if needed. Sessions started from then on share their connection.
pub fn enable_sharing() -> std::io::Result<()> {
    use std::io::Write;
    let path = ssh_config().ok_or_else(|| std::io::Error::other("no home folder"))?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if has_sharing(&existing) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)?;
    }
    let mut open = std::fs::OpenOptions::new();
    open.append(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    open.open(&path)?.write_all(sharing_addition(&existing).as_bytes())
}

/// What to append to a config file holding `existing`: the block, set off by a blank line.
fn sharing_addition(existing: &str) -> String {
    let sep = match existing {
        "" => "",
        e if e.ends_with("\n\n") => "",
        e if e.ends_with('\n') => "\n",
        _ => "\n\n",
    };
    format!("{sep}{SHARING_BLOCK}")
}

/// Quote for a POSIX shell (only when needed).
pub(crate) fn posix_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./-+:,@%=~".contains(c);
    if !s.is_empty() && s.chars().all(safe) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cd_words() {
        assert_eq!(cd_word(""), "~");
        assert_eq!(cd_word("~"), "~");
        assert_eq!(cd_word("~/"), "~");
        assert_eq!(cd_word("~/src"), "~/src");
        assert_eq!(cd_word("~/my dir"), "~/'my dir'");
        assert_eq!(cd_word("~/it's"), r"~/'it'\''s'");
        assert_eq!(cd_word("/srv/app"), "/srv/app");
        assert_eq!(cd_word("/a b"), "'/a b'");
    }

    #[test]
    fn quoting() {
        assert_eq!(posix_quote("/tmp/nex.ab12/a.png"), "/tmp/nex.ab12/a.png");
        assert_eq!(posix_quote("/tmp/x/it's here"), r"'/tmp/x/it'\''s here'");
    }

    #[test]
    fn sharing_block() {
        assert!(!has_sharing("Host x\n  User me\n"));
        assert!(has_sharing(&format!("Host x\n  User me\n{}", sharing_addition("Host x\n  User me\n"))));
        assert_eq!(sharing_addition(""), SHARING_BLOCK);
        assert_eq!(sharing_addition("Host x"), format!("\n\n{SHARING_BLOCK}"));
        assert!(sharing_addition("Host x\n").starts_with("\n# Added by nex:"));
        assert!(sharing_addition("Host x\n\n").starts_with("# Added by nex:"));
    }

    /// ssh itself reads the block: the sharing options apply inside nex only.
    #[test]
    fn ssh_reads_sharing_block() {
        let dir = std::env::temp_dir().join(format!("nex-sharing-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        std::fs::write(&cfg, format!("Host *\n  Port 2200\n{}", sharing_addition("Host *\n  Port 2200\n"))).unwrap();
        let resolved = |term: &str| {
            let out = Command::new("ssh").args(["-G", "-F"]).arg(&cfg).arg("example.invalid").env("TERM_PROGRAM", term).output();
            out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        if let (Some(inside), Some(outside)) = (resolved("nex"), resolved("xterm")) {
            assert!(inside.lines().any(|l| l == "controlmaster auto"), "{inside}");
            assert!(inside.lines().any(|l| l == "controlpersist 60"), "{inside}");
            assert!(outside.lines().any(|l| l == "controlmaster false"), "{outside}");
            assert!(inside.lines().any(|l| l == "port 2200"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

}
