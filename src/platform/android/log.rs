//! Android drops a native app's stdout and stderr. nex reports problems with `eprintln!`, so
//! both are pointed at a pipe whose lines a thread writes to logcat (tag `nex`): read them
//! with `adb logcat -s nex`. Panics reach logcat the same way, through stderr.

use std::io::{BufRead, BufReader};
use std::os::fd::FromRawFd;

const ANDROID_LOG_INFO: i32 = 4;

#[link(name = "log")]
unsafe extern "C" {
    fn __android_log_write(prio: i32, tag: *const std::ffi::c_char, text: *const std::ffi::c_char) -> i32;
}

/// Redirect stdout and stderr to logcat, once.
pub(super) fn to_logcat() {
    let mut fds = [0; 2];
    // SAFETY: `fds` has room for the two descriptors pipe writes.
    if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
        return;
    }
    let [read, write] = fds;
    // SAFETY: plain descriptor calls on descriptors we own; 1 and 2 are replaced, not leaked.
    unsafe {
        libc::dup2(write, libc::STDOUT_FILENO);
        libc::dup2(write, libc::STDERR_FILENO);
        libc::close(write);
    }
    // SAFETY: `read` is a fresh descriptor from `pipe`, owned by this File from here on.
    let pipe = unsafe { std::fs::File::from_raw_fd(read) };
    let _ = std::thread::Builder::new().name("logcat".into()).spawn(move || {
        let tag = c"nex";
        for line in BufReader::new(pipe).split(b'\n').map_while(Result::ok) {
            let mut text: Vec<u8> = line.into_iter().filter(|&b| b != 0).collect();
            text.push(0);
            // SAFETY: both strings are NUL-terminated and live for the call.
            unsafe { __android_log_write(ANDROID_LOG_INFO, tag.as_ptr(), text.as_ptr().cast()) };
        }
    });
}
