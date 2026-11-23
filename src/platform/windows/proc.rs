//! A shell's foreground process. Not implemented on Windows yet: panes show no process name
//! or cwd, and new panes start in the home folder.

use crate::platform::ProcInfo;

/// When process `pid` started, as Claude Code records it in its session file. Claude Code
/// only records it where there is a `/proc`, so here the pid alone has to match.
pub fn process_start(_pid: u32) -> Option<String> {
    None
}

pub fn proc_info(_pid: u32) -> ProcInfo {
    ProcInfo::default()
}

pub fn foreground_job(_pid: u32) -> Option<u32> {
    None
}
