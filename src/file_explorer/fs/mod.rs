//! File system work behind the explorer: reading folders and copying, moving, renaming and
//! deleting files (`ops` on this machine, `remote` on an ssh host, and `backend`, which picks
//! between them once), and the file clipboard and background copy/move/delete jobs
//! (`transfer`).

pub(super) mod backend;
pub mod ops;
pub mod remote;
pub(super) mod transfer;
