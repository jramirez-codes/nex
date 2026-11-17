//! A shell's foreground process, read with libproc (macOS has no `/proc`).

use std::ffi::CStr;
use std::mem::{MaybeUninit, size_of};
use std::path::PathBuf;

use crate::platform::ProcInfo;

/// The foreground process group of the shell's terminal (so running `node` shows "node"
/// rather than "zsh"), its name and its working directory.
///
/// The pty starts shells through `/usr/bin/login`, which runs as root, so the kernel won't
/// describe it to us; we then ask its child, the user's shell, instead.
pub fn proc_info(pid: u32) -> ProcInfo {
    let shell = if bsd_info(pid).is_some() { pid } else { first_child(pid).unwrap_or(pid) };
    let fg = bsd_info(shell).map(|i| i.e_tpgid).filter(|&p| p > 0).unwrap_or(shell);
    ProcInfo {
        name: name(fg).or_else(|| name(shell)),
        cwd: cwd(fg).or_else(|| cwd(shell)),
        argv: args(fg).unwrap_or_default(),
    }
}

fn first_child(pid: u32) -> Option<u32> {
    let mut pids = [0 as libc::c_int; 16];
    let size = size_of_val(&pids) as libc::c_int;
    // SAFETY: the buffer is valid for `size` bytes; the call returns how many pids it wrote.
    let n = unsafe { libc::proc_listchildpids(pid as libc::c_int, pids.as_mut_ptr().cast(), size) };
    pids.get(..n.max(0) as usize)?.iter().find(|&&p| p > 0).map(|&p| p as u32)
}

/// `proc_pidinfo(pid, flavor)` into a `T`, if the kernel filled all of it.
fn pid_info<T>(pid: u32, flavor: libc::c_int) -> Option<T> {
    let mut info = MaybeUninit::<T>::zeroed();
    let size = size_of::<T>() as libc::c_int;
    // SAFETY: the buffer is exactly `size` bytes; the kernel writes at most that many.
    let n = unsafe { libc::proc_pidinfo(pid as libc::c_int, flavor, 0, info.as_mut_ptr().cast(), size) };
    // SAFETY: a full-size reply initialised the whole struct (and it started zeroed anyway).
    (n == size).then(|| unsafe { info.assume_init() })
}

fn bsd_info(pid: u32) -> Option<libc::proc_bsdinfo> {
    pid_info(pid, libc::PROC_PIDTBSDINFO)
}

/// The name a process was started as (`argv[0]`), as `ps` shows it. The kernel's own name
/// is the executable's file name, which for Claude Code's native install is its version
/// (`~/.local/share/claude/versions/2.1.3`), so that is only the fallback.
fn name(pid: u32) -> Option<String> {
    argv0(pid).as_deref().and_then(command_name).or_else(|| kernel_name(pid))
}

/// `-zsh` (a login shell) → `zsh`, `/opt/homebrew/bin/yazi` → `yazi`.
fn command_name(argv0: &str) -> Option<String> {
    let first = argv0.split_whitespace().next()?;
    let base = first.rsplit('/').next()?.trim_start_matches('-');
    (!base.is_empty()).then(|| base.to_string())
}

/// `argv[0]` (see [`args`]).
fn argv0(pid: u32) -> Option<String> {
    args(pid)?.into_iter().next()
}

/// The command line from `KERN_PROCARGS2`: an `int` argc, the executable path, NUL padding,
/// then the arguments, each ended by a NUL (then the environment, which is skipped).
fn args(pid: u32) -> Option<Vec<String>> {
    let mut buf = vec![0u8; 64 * 1024];
    let mut len = buf.len();
    let mut mib = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid as libc::c_int];
    // SAFETY: `buf` is valid for `len` bytes and the kernel writes at most that many (a
    // longer reply fails with ENOMEM, and we fall back to the kernel's name).
    let rc = unsafe { libc::sysctl(mib.as_mut_ptr(), 3, buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
    if rc != 0 {
        return None;
    }
    parse_procargs(buf.get(..len)?)
}

fn parse_procargs(buf: &[u8]) -> Option<Vec<String>> {
    let argc = libc::c_int::from_ne_bytes(buf.get(..size_of::<libc::c_int>())?.try_into().ok()?);
    let rest = &buf[size_of::<libc::c_int>()..];
    let after_path = &rest[rest.iter().position(|&b| b == 0)?..];
    let mut at = &after_path[after_path.iter().position(|&b| b != 0)?..];
    let mut out = Vec::new();
    for _ in 0..argc.max(0) {
        let Ok(arg) = CStr::from_bytes_until_nul(at) else { break };
        out.push(arg.to_string_lossy().into_owned());
        at = &at[arg.to_bytes().len() + 1..];
    }
    (!out.is_empty()).then_some(out)
}

fn kernel_name(pid: u32) -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer is valid for its length; proc_name NUL-terminates within it.
    let n = unsafe { libc::proc_name(pid as libc::c_int, buf.as_mut_ptr().cast(), buf.len() as u32) };
    (n > 0).then(|| String::from_utf8_lossy(&buf[..n as usize]).into_owned())
}

fn cwd(pid: u32) -> Option<PathBuf> {
    let info: libc::proc_vnodepathinfo = pid_info(pid, libc::PROC_PIDVNODEPATHINFO)?;
    // `vip_path` is a MAXPATHLEN-byte C string, declared as [[c_char; 32]; 32].
    let raw = &info.pvi_cdir.vip_path;
    // SAFETY: the nested array is one contiguous block of bytes.
    let bytes: &[u8] = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast(), size_of_val(raw)) };
    let path = CStr::from_bytes_until_nul(bytes).ok()?.to_str().ok()?;
    (!path.is_empty()).then(|| PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_own_process() {
        let info = proc_info(std::process::id());
        assert_eq!(info.cwd, std::env::current_dir().ok());
        assert_eq!(info.name.as_deref(), argv0(std::process::id()).as_deref().and_then(command_name).as_deref());
        assert!(info.name.is_some_and(|n| !n.is_empty()));
    }

    #[test]
    fn procargs() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend_from_slice(b"/usr/bin/ssh\0\0\0ssh\0-p2222\0host\0HOME=/x\0");
        assert_eq!(parse_procargs(&buf).unwrap(), ["ssh", "-p2222", "host"]);
    }

    #[test]
    fn command_names() {
        assert_eq!(command_name("-zsh").as_deref(), Some("zsh"));
        assert_eq!(command_name("/opt/homebrew/bin/yazi").as_deref(), Some("yazi"));
        assert_eq!(command_name("claude").as_deref(), Some("claude"));
        assert_eq!(command_name("").as_deref(), None);
    }
}
