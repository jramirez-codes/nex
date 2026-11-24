//! Why one of nex's own connections to a host failed, read from `ssh`'s exit status and
//! stderr the same way everywhere. The errors of uploads, the mic, nex-host and remote
//! scripts each wrap a [`RemoteError`] and say it in their own words.

use std::process::{ExitStatus, Output};

/// Why a connection of nex's own didn't do what it was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteError {
    /// The host wants a password (or another prompt) nex's connections can't answer.
    NeedsLogin,
    /// ssh's last line of complaint, its exit status, or the caller's own reason.
    Other(String),
}

impl RemoteError {
    /// How the `ssh` that ended with `out` failed, or `None` if it succeeded.
    pub(super) fn of_output(out: &Output) -> Option<RemoteError> {
        Self::of(out.status, &String::from_utf8_lossy(&out.stderr))
    }

    /// [`of_output`](Self::of_output), for an `ssh` whose stderr was read apart from it.
    ///
    /// The one rule for a login nex can't do: ssh failed, and said so. An `ssh` that exited
    /// 0 logged in, so a `Permission denied (…)` in its stderr came from the script it ran
    /// (a `git fetch` on the host, say), not from the host turning nex away.
    pub(super) fn of(status: ExitStatus, stderr: &str) -> Option<RemoteError> {
        if status.success() {
            return None;
        }
        Some(if needs_login(stderr) {
            RemoteError::NeedsLogin
        } else {
            RemoteError::Other(last_line(stderr).unwrap_or_else(|| format!("ssh exited with {status}")))
        })
    }
}

impl From<String> for RemoteError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl From<&str> for RemoteError {
    fn from(e: &str) -> Self {
        Self::Other(e.to_string())
    }
}

/// ssh's error when every way it could log in without asking was refused:
/// `user@host: Permission denied (publickey,password).`
pub(super) fn needs_login(stderr: &str) -> bool {
    stderr.lines().any(|l| l.contains("Permission denied ("))
}

/// The last non-empty line of an error output.
pub(super) fn last_line(s: &str) -> Option<String> {
    s.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_lines() {
        assert_eq!(last_line("warning\nPermission denied (publickey).\n\n").as_deref(), Some("Permission denied (publickey)."));
        assert_eq!(last_line(" \n\n"), None);
    }

    #[test]
    fn login_errors() {
        assert!(needs_login("jr@10.0.0.2: Permission denied (publickey,password).\n"));
        assert!(!needs_login("ssh: connect to host x port 22: Connection refused\n"));
        assert!(!needs_login("tar: a.txt: Cannot open: Permission denied\n"));
    }

    #[cfg(unix)]
    fn output(code: i32, stderr: &str) -> Output {
        let status = std::process::Command::new("sh").args(["-c", &format!("exit {code}")]).status().unwrap();
        Output { status, stdout: Vec::new(), stderr: stderr.as_bytes().to_vec() }
    }

    #[cfg(unix)]
    #[test]
    fn reads_how_ssh_failed() {
        assert_eq!(RemoteError::of_output(&output(255, "me@x: Permission denied (publickey,password).\n")), Some(RemoteError::NeedsLogin));
        assert_eq!(
            RemoteError::of_output(&output(255, "ssh: connect to host x port 22: Connection refused\n\n")),
            Some(RemoteError::Other("ssh: connect to host x port 22: Connection refused".into()))
        );
        assert!(matches!(RemoteError::of_output(&output(1, "")), Some(RemoteError::Other(e)) if e.starts_with("ssh exited with")));
    }

    /// An ssh that exited 0 logged in: what the script it ran printed isn't the host asking
    /// for a password, for any caller.
    #[cfg(unix)]
    #[test]
    fn a_login_error_needs_a_failed_ssh() {
        let out = output(0, "git@github.com: Permission denied (publickey).\n");
        assert_eq!(RemoteError::of_output(&out), None);
        assert_eq!(RemoteError::of(out.status, "git@github.com: Permission denied (publickey).\n"), None);
    }
}
