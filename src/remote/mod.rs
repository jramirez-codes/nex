//! Talking to the host a pane's `ssh` is connected to, over a second connection of nex's own.
//!
//! ```text
//! remote/
//!   mod.rs     this file: reading a pane's ssh command line (SshTarget), the options nex's
//!              own connections use, and connection sharing in ~/.ssh/config
//!   mux.rs     one shared connection per host for all of nex's own, so only one logs in
//!   upload.rs  files dropped or pasted into an ssh pane, copied to the remote host
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
pub mod host;
pub mod mic;
mod mux;
pub mod ssh;
mod upload;

pub use upload::{UploadError, remove_saved, save_png, upload};
#[cfg(target_os = "linux")]
pub use upload::save_image;

use std::path::PathBuf;
use std::process::{Command, Stdio};

/// ssh options that take a value (from ssh's `getopt` string).
const VALUE_OPTS: &str = "BbcDEeFIiJLlmOoPpQRSWw";
/// Options that make ssh do something other than open a session (print its config, its
/// version, a query or a control command, a stdio forward, a subsystem): no upload.
const NOT_A_SESSION: &str = "GVQOWs";
/// Options left off the upload's command line: forwardings (`L R D`), session and tty
/// flags, backgrounding, becoming a control master, verbosity, the escape character.
const DROPPED: &str = "LRDNTtfnMve";

/// Options for nex's own connections (uploads, the mic), put before the user's own: ssh keeps
/// the first value it sees for an option, so these win over the pane's `-o` options and the
/// config file.
const BATCH_OPTS: &[&str] = &[
    "-T",
    "-o", "BatchMode=yes",
    "-o", "ConnectTimeout=8",
    "-o", "ClearAllForwardings=yes",
    "-o", "RemoteCommand=none",
    "-o", "RequestTTY=no",
    "-o", "PermitLocalCommand=no",
    "-o", "ForwardX11=no",
];

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

/// The host a pane's `ssh` is connected to, as the options needed to connect again.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct SshTarget {
    /// The pane's connection options, in order (`-p 2222`, `-J jump`, `-o …`).
    pub opts: Vec<String>,
    /// `host` or `user@host` (or an `ssh://` URI).
    pub destination: String,
}

impl SshTarget {
    /// Read `ssh`'s command line, if it is an interactive session to some host.
    pub fn from_argv(argv: &[String]) -> Option<Self> {
        let (prog, args) = argv.split_first()?;
        if prog.rsplit('/').next()? != "ssh" {
            return None;
        }
        let mut opts = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            i += 1;
            if arg == "--" {
                let destination = args.get(i)?.clone();
                return Some(Self { opts, destination });
            }
            let Some(flags) = arg.strip_prefix('-').filter(|f| !f.is_empty()) else {
                // The destination; anything after it is the remote command.
                return Some(Self { opts, destination: arg.clone() });
            };
            // A cluster of flags (`-tt`, `-4A`), possibly ending in one that takes a value,
            // either the rest of this argument (`-p2222`) or the next one (`-p 2222`).
            for (at, c) in flags.char_indices() {
                if NOT_A_SESSION.contains(c) {
                    return None;
                }
                if VALUE_OPTS.contains(c) {
                    let rest = &flags[at + c.len_utf8()..];
                    let value = if rest.is_empty() {
                        i += 1;
                        args.get(i - 1)?.clone()
                    } else {
                        rest.to_string()
                    };
                    if !DROPPED.contains(c) {
                        opts.push(format!("-{c}"));
                        opts.push(value);
                    }
                    break;
                }
                if !DROPPED.contains(c) {
                    opts.push(format!("-{c}"));
                }
            }
        }
        None
    }

    /// The host part of the destination, for messages ("Uploaded to devbox").
    pub fn host(&self) -> &str {
        let d = self.destination.strip_prefix("ssh://").unwrap_or(&self.destination);
        let d = d.rsplit('@').next().unwrap_or(d);
        // `[::1]:22`, `host:22` (URI form only), `host`.
        let d = d.split('/').next().unwrap_or(d);
        match d.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(v6),
            None if self.destination.starts_with("ssh://") => d.split(':').next().unwrap_or(d),
            None => d,
        }
    }

    /// The command line for a new pane that opens another session on this host, in remote
    /// directory `dir` (else the remote home), for a split or new tab of an `ssh` pane.
    ///
    /// It runs under `sh` so that leaving the session (`exit`) drops to a local shell, as it
    /// does in the pane it came from. With connection sharing on (see [`enable_sharing`]) the
    /// session rides on the pane's existing login.
    pub fn session_command(&self, dir: Option<&str>) -> Vec<String> {
        // Run by the remote login shell, whatever it is: `cd` and `exec` work in sh, bash, zsh,
        // fish and csh alike.
        let mut remote = String::new();
        if let Some(dir) = dir.filter(|d| !d.is_empty() && *d != "~") {
            remote.push_str(&format!("cd {}; ", remote_dir(dir)));
        }
        remote.push_str("exec \"$SHELL\" -l");

        let mut ssh: Vec<String> = ["ssh", "-t", "-o", "RemoteCommand=none"].map(String::from).to_vec();
        ssh.extend(self.opts.iter().cloned());
        ssh.push("--".into());
        ssh.push(self.destination.clone());
        ssh.push(remote);
        let ssh = ssh.iter().map(|a| posix_quote(a)).collect::<Vec<_>>().join(" ");
        // The local shell to come back to: a login shell on macOS, where terminals start one.
        let login = if cfg!(target_os = "macos") { " -l" } else { "" };
        // `set -m` runs ssh as the terminal's foreground job, so the pane sees `ssh` (for its
        // title, uploads, and splitting it again) rather than `sh`.
        vec!["/bin/sh".into(), "-c".into(), format!("set -m; {ssh}; exec \"${{SHELL:-/bin/sh}}\"{login}")]
    }

    /// The full `ssh` argument list (without the program name) for one of nex's own
    /// connections to this host, running `script` under `sh` there.
    #[cfg(test)]
    fn batch_args(&self, script: &str) -> Vec<String> {
        self.batch_args_via(None, script)
    }

    /// [`batch_args`](Self::batch_args), riding the master connection at `control` if given.
    fn batch_args_via(&self, control: Option<&str>, script: &str) -> Vec<String> {
        let mut args: Vec<String> = BATCH_OPTS.iter().map(|s| s.to_string()).collect();
        // Share a master connection, but never become one here: see `mux`.
        args.extend(["-o".into(), "ControlMaster=no".into()]);
        if let Some(path) = control {
            args.extend(["-o".into(), mux::control_path_opt(path)]);
        }
        args.extend(self.opts.iter().cloned());
        args.push("--".into());
        args.push(self.destination.clone());
        // ssh hands this to the remote login shell (which may be fish or csh): one
        // single-quoted word that every shell passes to `sh` unchanged.
        args.push(format!("sh -c {}", posix_quote(script)));
        args
    }

    /// A `Command` for [`batch_args`](Self::batch_args), with no stdin or stdout set. It
    /// rides nex's shared connection to the host (see `mux`), starting it if need be, so only
    /// the first one pays for a login.
    fn batch_command(&self, script: &str) -> Command {
        let control = mux::ensure_master(self);
        let mut ssh = Command::new("ssh");
        // So the sharing block in `~/.ssh/config` finds the pane's connection, whatever
        // terminal nex itself was started from.
        ssh.env("TERM_PROGRAM", "nex").args(self.batch_args_via(control.as_deref(), script));
        ssh
    }
}

/// ssh's error when every way it could log in without asking was refused:
/// `user@host: Permission denied (publickey,password).`
pub(super) fn needs_login(stderr: &str) -> bool {
    stderr.lines().any(|l| l.contains("Permission denied ("))
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

/// Whether a shared connection to `target`'s host is up, so a new session on it needs no
/// login. Asks the control socket only (`ssh -O check`): quick, and never prompts.
pub fn shared_connection_live(target: &SshTarget) -> bool {
    if !SHARING_SUPPORTED {
        return false;
    }
    Command::new("ssh")
        .env("TERM_PROGRAM", "nex")
        .args(["-o", "BatchMode=yes", "-O", "check"])
        .args(&target.opts)
        .arg("--")
        .arg(&target.destination)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// A remote directory as a word for `cd`: quoted, but with a leading `~/` left bare so the
/// remote shell still expands it.
fn remote_dir(dir: &str) -> String {
    match dir.strip_prefix("~/") {
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

/// Why one of nex's short ssh commands failed, read from its exit status and stderr the same
/// way everywhere; each caller turns it into its own error type.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum SshFailure {
    /// The host wants a password (or another prompt) nex's connections can't answer.
    NeedsLogin,
    /// ssh's last line of complaint, or its exit status.
    Other(String),
}

impl SshFailure {
    /// How ssh failed, or `None` if it succeeded.
    pub(super) fn of(status: std::process::ExitStatus, stderr: &str) -> Option<SshFailure> {
        if status.success() {
            return None;
        }
        Some(if needs_login(stderr) {
            SshFailure::NeedsLogin
        } else {
            SshFailure::Other(last_line(stderr).unwrap_or_else(|| format!("ssh exited with {status}")))
        })
    }
}

/// The last non-empty line of an error output.
pub(super) fn last_line(s: &str) -> Option<String> {
    s.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string)
}

/// Quote for a POSIX shell (only when needed).
pub(super) fn posix_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./-+:,@%=~".contains(c);
    if !s.is_empty() && s.chars().all(safe) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(cmd: &str) -> Option<SshTarget> {
        SshTarget::from_argv(&cmd.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    fn target(opts: &[&str], dest: &str) -> Option<SshTarget> {
        Some(SshTarget { opts: opts.iter().map(|s| s.to_string()).collect(), destination: dest.into() })
    }

    /// The ssh part of a session command.
    fn session(t: &SshTarget, dir: Option<&str>) -> String {
        let cmd = t.session_command(dir);
        assert_eq!(cmd[..2], ["/bin/sh", "-c"]);
        let script = cmd[2].strip_prefix("set -m; ").unwrap();
        script.split("; exec \"${SHELL:-/bin/sh}\"").next().unwrap().to_string()
    }

    #[test]
    fn session_commands() {
        let t = target(&[], "host").unwrap();
        assert_eq!(session(&t, None), r#"ssh -t -o RemoteCommand=none -- host 'exec "$SHELL" -l'"#);
        assert_eq!(session(&t, Some("~")), r#"ssh -t -o RemoteCommand=none -- host 'exec "$SHELL" -l'"#);
        assert_eq!(session(&t, Some("/var/log")), r#"ssh -t -o RemoteCommand=none -- host 'cd /var/log; exec "$SHELL" -l'"#);
        assert_eq!(
            session(&t, Some("~/my dir")),
            r#"ssh -t -o RemoteCommand=none -- host 'cd ~/'\''my dir'\''; exec "$SHELL" -l'"#
        );
        let t = target(&["-p", "2222", "-J", "jump"], "me@host").unwrap();
        assert_eq!(session(&t, Some("/srv")), r#"ssh -t -o RemoteCommand=none -p 2222 -J jump -- me@host 'cd /srv; exec "$SHELL" -l'"#);
    }

    #[test]
    fn session_command_round_trips() {
        // The new pane's ssh is what a split of *it* reads back.
        let t = target(&["-p", "2222"], "me@host").unwrap();
        let mut argv = vec!["ssh".to_string(), "-t".into(), "-o".into(), "RemoteCommand=none".into()];
        argv.extend(t.opts.iter().cloned());
        argv.extend(["--".into(), t.destination.clone(), "cd /x; exec \"$SHELL\" -l".into()]);
        assert_eq!(SshTarget::from_argv(&argv), target(&["-o", "RemoteCommand=none", "-p", "2222"], "me@host"));
    }

    #[test]
    fn batch_args_ride_a_master() {
        let t = target(&["-p", "2222"], "host").unwrap();
        let plain = t.batch_args("true");
        assert!(plain.windows(2).any(|w| w == ["-o", "ControlMaster=no"]));
        assert!(!plain.iter().any(|a| a.starts_with("ControlPath")));
        let via = t.batch_args_via(Some("/home/me/.ssh/nex-ab"), "true");
        let at = via.iter().position(|a| a == "ControlPath=/home/me/.ssh/nex-ab").expect("control path");
        // Before the pane's options, so it wins over them.
        assert!(at < via.iter().position(|a| a == "2222").unwrap());
        assert!(via.windows(2).any(|w| w == ["-o", "ControlMaster=no"]));
    }

    #[test]
    fn remote_dirs() {
        assert_eq!(remote_dir("~/src"), "~/src");
        assert_eq!(remote_dir("~/"), "~");
        assert_eq!(remote_dir("/a b"), "'/a b'");
        assert_eq!(remote_dir("~/it's"), r"~/'it'\''s'");
    }

    #[test]
    fn plain_hosts() {
        assert_eq!(parse("ssh host"), target(&[], "host"));
        assert_eq!(parse("/usr/bin/ssh user@host"), target(&[], "user@host"));
        assert_eq!(parse("ssh ssh://user@host:2222"), target(&[], "ssh://user@host:2222"));
    }

    #[test]
    fn keeps_connection_options() {
        assert_eq!(parse("ssh -p 2222 user@host"), target(&["-p", "2222"], "user@host"));
        assert_eq!(parse("ssh -p2222 -i ~/.ssh/k host"), target(&["-p", "2222", "-i", "~/.ssh/k"], "host"));
        assert_eq!(parse("ssh -o Port=22 -J jump host"), target(&["-o", "Port=22", "-J", "jump"], "host"));
        assert_eq!(parse("ssh -4A host"), target(&["-4", "-A"], "host"));
        assert_eq!(parse("ssh -Cl me host"), target(&["-C", "-l", "me"], "host"));
    }

    #[test]
    fn drops_forwardings_tty_and_command() {
        assert_eq!(parse("ssh -t host tmux attach"), target(&[], "host"));
        assert_eq!(parse("ssh -tt -v host"), target(&[], "host"));
        assert_eq!(parse("ssh -L 8080:localhost:80 -R9000:x:9000 host"), target(&[], "host"));
        assert_eq!(parse("ssh -J jump host -- ls -la"), target(&["-J", "jump"], "host"));
        assert_eq!(parse("ssh -p 22 -- host"), target(&["-p", "22"], "host"));
    }

    #[test]
    fn not_a_session() {
        assert_eq!(parse("ssh"), None);
        assert_eq!(parse("ssh -V"), None);
        assert_eq!(parse("ssh -G host"), None);
        assert_eq!(parse("ssh -W host:22 jump"), None);
        assert_eq!(parse("ssh -O check host"), None);
        assert_eq!(parse("ssh -p"), None);
        assert_eq!(parse("mosh host"), None);
        assert_eq!(parse("sshd -D"), None);
    }

    #[test]
    fn hosts() {
        assert_eq!(parse("ssh me@devbox").unwrap().host(), "devbox");
        assert_eq!(parse("ssh ssh://me@devbox:2222").unwrap().host(), "devbox");
        assert_eq!(parse("ssh ssh://[::1]:22").unwrap().host(), "::1");
        assert_eq!(parse("ssh fe80::1").unwrap().host(), "fe80::1");
    }

    #[test]
    fn quoting() {
        assert_eq!(posix_quote("/tmp/nex.ab12/a.png"), "/tmp/nex.ab12/a.png");
        assert_eq!(posix_quote("/tmp/x/it's here"), r"'/tmp/x/it'\''s here'");
        assert_eq!(last_line("warning\nPermission denied (publickey).\n\n").as_deref(), Some("Permission denied (publickey)."));
    }

    #[test]
    fn login_errors() {
        assert!(needs_login("jr@10.0.0.2: Permission denied (publickey,password).\n"));
        assert!(!needs_login("ssh: connect to host x port 22: Connection refused\n"));
        assert!(!needs_login("tar: a.txt: Cannot open: Permission denied\n"));
    }

    #[cfg(unix)]
    #[test]
    fn reads_how_ssh_failed() {
        let status = |code: i32| std::process::Command::new("sh").args(["-c", &format!("exit {code}")]).status().unwrap();
        assert_eq!(SshFailure::of(status(0), "Permission denied (publickey).\n"), None);
        assert_eq!(SshFailure::of(status(255), "me@x: Permission denied (publickey,password).\n"), Some(SshFailure::NeedsLogin));
        assert_eq!(
            SshFailure::of(status(255), "ssh: connect to host x port 22: Connection refused\n\n"),
            Some(SshFailure::Other("ssh: connect to host x port 22: Connection refused".into()))
        );
        assert!(matches!(SshFailure::of(status(1), ""), Some(SshFailure::Other(e)) if e.starts_with("ssh exited with")));
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
