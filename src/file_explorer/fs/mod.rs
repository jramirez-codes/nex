//! File system work behind the explorer: reading folders and copying, moving, renaming and
//! deleting files (`ops` on this machine, `remote` on an ssh host, and `backend`, which picks
//! between them once), the file clipboard and background copy/move/delete jobs
//! (`transfer`), and downloading a file from an ssh host to this device (`download`).

pub(super) mod backend;
pub mod download;
pub mod ops;
pub mod remote;
pub(super) mod transfer;
