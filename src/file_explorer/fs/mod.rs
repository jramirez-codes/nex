//! File system work behind the explorer: reading folders and copying, moving, renaming and
//! deleting files (`ops`), and the file clipboard and background copy/move/delete jobs
//! (`transfer`).

pub mod ops;
pub(super) mod transfer;
