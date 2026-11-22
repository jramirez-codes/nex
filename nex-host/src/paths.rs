//! Where `nex-host` lives on a host: everything under `~/.nex` (or `$NEX_HOME`), so removing
//! that folder removes it entirely.
//!
//! ```text
//! ~/.nex/
//!   bin/nex-host     the program, installed by nex
//!   run/             0700: only this user can reach the daemon
//!     sock           the daemon's socket
//!     lock           held by the running daemon
//!     daemon.log     what the daemon had to say (errors only)
//! ```

use std::path::PathBuf;

/// Where nex installs the program, relative to the remote home.
pub const INSTALL_PATH: &str = ".nex/bin/nex-host";

/// `$NEX_HOME`, else `~/.nex`.
pub fn root() -> Option<PathBuf> {
    if let Some(p) = std::env::var_os("NEX_HOME").map(PathBuf::from).filter(|p| p.is_absolute()) {
        return Some(p);
    }
    Some(PathBuf::from(std::env::var_os("HOME")?).join(".nex"))
}

pub fn run_dir() -> Option<PathBuf> {
    Some(root()?.join("run"))
}

pub fn socket() -> Option<PathBuf> {
    Some(run_dir()?.join("sock"))
}

pub fn lock() -> Option<PathBuf> {
    Some(run_dir()?.join("lock"))
}

pub fn log() -> Option<PathBuf> {
    Some(run_dir()?.join("daemon.log"))
}

/// `~/dir` with the home folder filled in; anything else unchanged.
pub fn expand_home(path: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match (path, home) {
        ("~", Some(h)) => h,
        (p, Some(h)) if p.starts_with("~/") => h.join(&p[2..]),
        (p, _) => PathBuf::from(p),
    }
}
