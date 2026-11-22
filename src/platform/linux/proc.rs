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

    let argv = std::fs::read(format!("/proc/{fg_pid}/cmdline")).map(|b| split_cmdline(&b)).unwrap_or_default();

    let job = (fg_pid != pid).then_some(fg_pid);
    ProcInfo { name, cwd: fg_cwd.or(shell_cwd), argv, job }
}

/// `/proc/<pid>/cmdline`: the arguments, each ended by a NUL.
fn split_cmdline(bytes: &[u8]) -> Vec<String> {
    bytes.split(|&b| b == 0).filter(|a| !a.is_empty()).map(|a| String::from_utf8_lossy(a).into_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_cmdline() {
        assert_eq!(split_cmdline(b"ssh\x00-p\x002222\x00host\x00"), ["ssh", "-p", "2222", "host"]);
        assert!(split_cmdline(b"").is_empty());
    }

    #[test]
    fn reads_own_argv() {
        let info = proc_info(std::process::id());
        assert!(!info.argv.is_empty());
    }
}
