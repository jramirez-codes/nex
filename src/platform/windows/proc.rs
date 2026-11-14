//! A shell's foreground process. Not implemented on Windows yet: panes show no process name
//! or cwd, and new panes start in the home folder.

use crate::platform::ProcInfo;

pub fn proc_info(_pid: u32) -> ProcInfo {
    ProcInfo::default()
}
