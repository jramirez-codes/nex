//! File system work behind the explorer: reading folders and copying, moving, renaming and
//! deleting files (`ops`, or `remote` on an ssh host), and the file clipboard and background
//! copy/move/delete jobs (`transfer`).

pub mod ops;
pub mod remote;
pub(super) mod transfer;
