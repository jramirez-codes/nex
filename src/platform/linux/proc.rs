//! A shell's foreground process, read from `/proc`.

use crate::platform::ProcInfo;

/// We look at the *foreground* process group of the shell's controlling terminal, so running
/// `node` shows "node" rather than "bash".
pub fn proc_info(pid: u32) -> ProcInfo {
    let shell_cwd = std::fs::read_link(format!("/proc/{pid}/cwd")).ok();
    let fg_pid = std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            // Fields after the `(comm)` part: state ppid pgrp session tty_nr tpgid ...
            let rest = &stat[stat.rfind(')')? + 1..];
            rest.split_whitespace().nth(5)?.parse::<i32>().ok()
        })
        .filter(|&p| p > 0)
        .map(|p| p as u32)
        .unwrap_or(pid);

    let name = std::fs::read_to_string(format!("/proc/{fg_pid}/comm"))
        .or_else(|_| std::fs::read_to_string(format!("/proc/{pid}/comm")))
        .map(|s| s.trim().to_string())
        .ok()
        .filter(|s| !s.is_empty());
    let fg_cwd = std::fs::read_link(format!("/proc/{fg_pid}/cwd")).ok();

    ProcInfo { name, cwd: fg_cwd.or(shell_cwd) }
}
