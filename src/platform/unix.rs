//! POSIX pieces shared by the Linux, macOS and Android backends. Android runs no local
//! programs, so the helpers for starting them go unused there.

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
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
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

/// Permission bits (`0o755`), shown in the explorer's Properties.
pub fn file_mode(meta: &std::fs::Metadata) -> Option<u32> {
    use std::os::unix::fs::PermissionsExt;
    Some(meta.permissions().mode() & 0o7777)
}

/// A regular file with any execute bit set (scripts, binaries).
pub fn is_executable(meta: &std::fs::Metadata) -> bool {
    meta.is_file() && file_mode(meta).is_some_and(|m| m & 0o111 != 0)
}

/// The owning user's and group's names (numeric ids when they have no name).
pub fn owner_names(meta: &std::fs::Metadata) -> Option<(String, String)> {
    use std::os::unix::fs::MetadataExt;
    let (uid, gid) = (meta.uid(), meta.gid());
    Some((user_name(uid).unwrap_or_else(|| uid.to_string()), group_name(gid).unwrap_or_else(|| gid.to_string())))
}

fn user_name(uid: libc::uid_t) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    // SAFETY: an all-zero passwd is a valid value to be overwritten.
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call; on success `pw_name` points into `buf`.
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || pwd.pw_name.is_null() {
        return None;
    }
    // SAFETY: `pw_name` is a NUL-terminated string inside `buf`, which is still alive.
    Some(unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }.to_string_lossy().into_owned())
}

fn group_name(gid: libc::gid_t) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 4096];
    // SAFETY: an all-zero group is a valid value to be overwritten.
    let mut grp: libc::group = unsafe { std::mem::zeroed() };
    let mut result = std::ptr::null_mut();
    // SAFETY: every pointer is valid for the call; on success `gr_name` points into `buf`.
    let rc = unsafe { libc::getgrgid_r(gid, &mut grp, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() || grp.gr_name.is_null() {
        return None;
    }
    // SAFETY: `gr_name` is a NUL-terminated string inside `buf`, which is still alive.
    Some(unsafe { std::ffi::CStr::from_ptr(grp.gr_name) }.to_string_lossy().into_owned())
}

/// `2026-11-15 14:03` in the local time zone.
pub fn format_local_time(t: std::time::SystemTime) -> Option<String> {
    let secs = t.duration_since(std::time::UNIX_EPOCH).ok()?.as_secs() as libc::time_t;
    // SAFETY: an all-zero tm is a valid value to be overwritten.
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    // SAFETY: both pointers are valid; localtime_r only writes into `tm`.
    if unsafe { libc::localtime_r(&secs, &mut tm) }.is_null() {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02} {:02}:{:02}", tm.tm_year + 1900, tm.tm_mon + 1, tm.tm_mday, tm.tm_hour, tm.tm_min))
}

/// The output of a command that succeeded and printed something, trimmed.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(super) fn command_output(cmd: &mut std::process::Command) -> Option<String> {
    let out = cmd.stdin(std::process::Stdio::null()).stderr(std::process::Stdio::null()).output().ok()?;
    let s = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (out.status.success() && !s.is_empty()).then_some(s)
}

/// A file's MIME type from `file(1)`, following symlinks.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(super) fn file_mime(path: &Path) -> Option<String> {
    command_output(std::process::Command::new("file").args(["--mime-type", "-b", "-L"]).arg(path))
}

/// Start `argv` detached in `cwd`, without inheriting stdio. A thread reaps it when it exits.
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(super) fn spawn_detached(argv: &[String], cwd: Option<&Path>) {
    let Some((program, args)) = argv.split_first() else { return };
    let mut cmd = std::process::Command::new(program);
    cmd.args(args).stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null());
    if let Some(dir) = cwd {
        cmd.current_dir(dir);
    }
    if let Ok(mut child) = cmd.spawn() {
        std::thread::spawn(move || {
            let _ = child.wait();
        });
    }
}

/// Run `program args` and return its output, or `None` if it isn't installed, fails, or
/// takes longer than `timeout` (it is then killed).
#[cfg_attr(any(target_os = "android", target_os = "ios"), allow(dead_code))]
pub(super) fn output_within(program: &str, args: &[&str], timeout: std::time::Duration) -> Option<Vec<u8>> {
    use std::io::Read;
    use std::process::{Command, Stdio};
    let mut child = Command::new(program).args(args).stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    // Read on a thread so a large output can't fill the pipe while we wait.
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        stdout.read_to_end(&mut out).map(|_| out)
    });
    let deadline = std::time::Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if std::time::Instant::now() < deadline => std::thread::sleep(std::time::Duration::from_millis(5)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?.ok()?;
    status.success().then_some(out)
}

/// An exclusive advisory `flock` on a file, released when dropped (closing the file).
pub struct FileLock {
    _file: std::fs::File,
}

/// Lock the file at `path` (created, with its folder, if missing) without waiting.
pub fn lock_file(path: &Path) -> Result<FileLock, super::LockFailed> {
    use std::os::fd::AsRawFd;
    let file = path
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            // std opens with O_CLOEXEC, so shells nex spawns don't inherit the lock.
            std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
        })
        .map_err(super::LockFailed::Open)?;
    // SAFETY: flock on a file descriptor we own for the duration of the call.
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } == 0 {
        return Ok(FileLock { _file: file });
    }
    let e = std::io::Error::last_os_error();
    Err(if e.kind() == std::io::ErrorKind::WouldBlock { super::LockFailed::Held } else { super::LockFailed::Lock(e) })
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
