//! [`SshTarget`]: the host a pane's `ssh` is connected to, read from its command line, and the
//! command lines nex builds to reach that host again: interactive ones for new panes (another
//! session, an agent to attach to) and batch ones for nex's own connections.

use std::process::{Command, ExitStatus, Output};

use crate::remote::error::RemoteError;
use crate::remote::host::REMOTE_BIN;
use crate::remote::{cd_word, mux, posix_quote, ssh_command};

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
pub(super) const BATCH_OPTS: &[&str] = &[
    "-T",
    "-o", "BatchMode=yes",
    "-o", "ConnectTimeout=8",
    "-o", "ClearAllForwardings=yes",
    "-o", "RemoteCommand=none",
    "-o", "RequestTTY=no",
    "-o", "PermitLocalCommand=no",
    "-o", "ForwardX11=no",
];

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
    /// does in the pane it came from. With connection sharing on (see [`crate::remote::enable_sharing`]) the
    /// session rides on the pane's existing login.
    pub fn session_command(&self, dir: Option<&str>) -> Vec<String> {
        // Run by the remote login shell, whatever it is: `cd` and `exec` work in sh, bash, zsh,
        // fish and csh alike.
        let mut remote = String::new();
        // The login shell starts in the home folder: no `cd` needed to be there.
        if let Some(dir) = dir.map(cd_word).filter(|d| d != "~") {
            remote.push_str(&format!("cd {dir}; "));
        }
        remote.push_str("exec \"$SHELL\" -l");

        let ssh = self.interactive_argv(remote).iter().map(|a| posix_quote(a)).collect::<Vec<_>>().join(" ");
        // The local shell to come back to: a login shell on macOS, where terminals start one.
        let login = if cfg!(target_os = "macos") { " -l" } else { "" };
        // `set -m` runs ssh as the terminal's foreground job, so the pane sees `ssh` (for its
        // title, uploads, and splitting it again) rather than `sh`.
        vec!["/bin/sh".into(), "-c".into(), format!("set -m; {ssh}; exec \"${{SHELL:-/bin/sh}}\"{login}")]
    }

    /// The command line for a pane showing agent `id` on this host: `ssh -t` running
    /// `nex-host attach`. When the agent ends (or another pane takes it), ssh ends, and so
    /// does the pane.
    pub fn attach_command(&self, id: &str) -> Vec<String> {
        self.interactive_argv(format!("{REMOTE_BIN} attach {}", posix_quote(id)))
    }

    /// The agent a pane running `argv` (an `ssh` command line) attaches to, if it's one of
    /// [`attach_command`](Self::attach_command)'s.
    pub fn attach_id(argv: &[String]) -> Option<String> {
        let words: Vec<&str> = argv.last()?.split_whitespace().collect();
        match words.as_slice() {
            [bin, "attach", id] if bin.ends_with("nex-host") => Some(id.trim_matches('\'').to_string()),
            _ => None,
        }
    }

    /// The `ssh` command line (program name first) for a pane that runs `remote_cmd` on this
    /// host with a terminal. `RemoteCommand=none` keeps a config file's own command out of
    /// the way; [`normalized`](Self::normalized) takes it back off when the pane is read.
    fn interactive_argv(&self, remote_cmd: String) -> Vec<String> {
        let mut ssh: Vec<String> = ["ssh", "-t", "-o", "RemoteCommand=none"].map(String::from).to_vec();
        ssh.extend(self.opts.iter().cloned());
        ssh.push("--".into());
        ssh.push(self.destination.clone());
        ssh.push(remote_cmd);
        ssh
    }

    /// The full `ssh` argument list (without the program name) for one of nex's own
    /// connections to this host, running `script` under `sh` there.
    #[cfg(test)]
    pub(super) fn batch_args(&self, script: &str) -> Vec<String> {
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
    pub(super) fn batch_command(&self, script: &str) -> Command {
        let control = mux::ensure_master(self);
        let mut ssh = ssh_command();
        ssh.args(self.batch_args_via(control.as_deref(), script));
        ssh
    }

    /// How the [`batch_command`](Self::batch_command) that ended with `out` failed, or `None`
    /// if it succeeded.
    pub(super) fn batch_failure(&self, out: &Output) -> Option<RemoteError> {
        mux::note_exit(self, out.status);
        RemoteError::of_output(out)
    }

    /// [`batch_failure`](Self::batch_failure), for a command whose stderr was read apart
    /// from its exit status.
    pub(super) fn batch_exit(&self, status: ExitStatus, stderr: &str) -> Option<RemoteError> {
        mux::note_exit(self, status);
        RemoteError::of(status, stderr)
    }

    /// The same host without the options nex adds to its own panes' command lines, so a
    /// pane nex opened and the one it came from count as one host.
    pub fn normalized(&self) -> SshTarget {
        let mut opts = Vec::with_capacity(self.opts.len());
        let mut i = 0;
        while i < self.opts.len() {
            if self.opts[i] == "-o" && self.opts.get(i + 1).is_some_and(|v| v == "RemoteCommand=none") {
                i += 2;
                continue;
            }
            opts.push(self.opts[i].clone());
            i += 1;
        }
        SshTarget { opts, destination: self.destination.clone() }
    }
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
    fn attach_commands_round_trip() {
        let t = target(&["-p", "2222"], "me@devbox").unwrap();
        let argv = t.attach_command("3fa91c0e");
        assert_eq!(argv, ["ssh", "-t", "-o", "RemoteCommand=none", "-p", "2222", "--", "me@devbox", "~/.nex/bin/nex-host attach 3fa91c0e"]);
        assert_eq!(SshTarget::attach_id(&argv).as_deref(), Some("3fa91c0e"));
        // The pane's ssh reads as a session on the same host, so uploads and the mic work.
        assert_eq!(SshTarget::from_argv(&argv).map(|t| t.normalized()), Some(t));
        assert_eq!(SshTarget::attach_id(&["ssh".into(), "devbox".into()]), None);
    }
}
