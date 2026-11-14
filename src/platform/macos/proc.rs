//! A shell's foreground process, read with libproc (macOS has no `/proc`).

use std::ffi::CStr;
use std::mem::{MaybeUninit, size_of};
use std::path::PathBuf;

use crate::platform::ProcInfo;

/// The foreground process group of the shell's terminal (so running `node` shows "node"
/// rather than "zsh"), its name and its working directory.
pub fn proc_info(pid: u32) -> ProcInfo {
    let fg = bsd_info(pid).map(|i| i.e_tpgid).filter(|&p| p > 0).unwrap_or(pid);
    ProcInfo {
        name: name(fg).or_else(|| name(pid)),
        cwd: cwd(fg).or_else(|| cwd(pid)),
    }
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

fn name(pid: u32) -> Option<String> {
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
        assert!(info.name.is_some_and(|n| !n.is_empty()));
    }
}
