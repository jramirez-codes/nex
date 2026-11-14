//! POSIX pieces shared by the Linux and macOS backends.

use std::path::{Path, PathBuf};

/// `$XDG_CONFIG_HOME/nex`, else `~/.config/nex` (also on macOS, like Ghostty).
pub fn config_dir() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home_dir().map(|h| h.join(".config")))?;
    Some(base.join("nex"))
}

pub fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").filter(|v| !v.is_empty()).map(PathBuf::from)
}

pub fn hostname() -> String {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; gethostname NUL-terminates within it on success.
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let len = buf.iter().position(|&b| b == 0).unwrap_or(buf.len());
    let name = String::from_utf8_lossy(&buf[..len]);
    // macOS often reports `name.local`; the short name reads better in a title.
    let short = name.split('.').next().unwrap_or_default();
    if ok && !short.is_empty() { short.to_string() } else { "localhost".into() }
}

pub fn symlink(target: &Path, link: &Path) -> std::io::Result<()> {
    std::os::unix::fs::symlink(target, link)
}

/// Quote a path for a POSIX shell (only when needed).
pub fn shell_quote(path: &Path) -> String {
    let s = path.to_string_lossy();
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./-+:,@%=~".contains(c);
    if !s.is_empty() && s.chars().all(safe) { s.into_owned() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

/// Run `program uri` in the background, without inheriting stdio.
pub fn spawn_opener(program: &str, uri: &std::ffi::OsStr) {
    let _ = std::process::Command::new(program)
        .arg(uri)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// PTY options for alacritty_terminal (the same struct on every unix).
pub fn pty_options(
    shell: Option<alacritty_terminal::tty::Shell>,
    working_directory: Option<PathBuf>,
    env: std::collections::HashMap<String, String>,
) -> alacritty_terminal::tty::Options {
    alacritty_terminal::tty::Options { shell, working_directory, drain_on_exit: true, env }
}

pub fn pty_pid(pty: &alacritty_terminal::tty::Pty) -> u32 {
    pty.child().id()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote(Path::new("/tmp/plain-file.txt")), "/tmp/plain-file.txt");
        assert_eq!(shell_quote(Path::new("/tmp/it's here")), r"'/tmp/it'\''s here'");
    }

    #[test]
    fn hostname_is_short_and_nonempty() {
        let h = hostname();
        assert!(!h.is_empty() && !h.contains('.'));
    }
}
