//! Which machine an explorer's folder is on, and the one place that picks between doing the
//! work here (`ops.rs`) or on an ssh host (`remote.rs`). The rest of the explorer asks its
//! [`FsBackend`] instead of checking which machine it is on.

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime};

use crate::file_explorer::Entry;
use crate::file_explorer::fs::ops;
use crate::file_explorer::fs::remote::{self, Details, Listing};
use crate::file_explorer::fs::transfer::{Op, Progress};
use crate::remote::exec::{ExecError, Transport};

/// How often an idle listing is re-read so files created elsewhere show up (where the
/// folder can't be watched; see `platform::DirWatch`).
const REFRESH_INTERVAL: Duration = Duration::from_secs(2);
/// The same for a folder on an ssh host (each read is a trip there).
const REMOTE_REFRESH_INTERVAL: Duration = Duration::from_secs(5);

/// Where the files are: this machine, or the ssh host a transport reaches. Cheap to clone,
/// so background jobs take their own.
#[derive(Clone)]
pub(in crate::file_explorer) enum FsBackend {
    Local,
    Remote(Arc<Transport>),
}

/// What reading a folder gave: this machine's, or an ssh host's (which also says where the
/// folder and the home folder are).
pub(in crate::file_explorer) enum Answer {
    Local(std::io::Result<Vec<Entry>>),
    Remote(Result<Listing, ExecError>),
}

impl FsBackend {
    /// The host, if the files aren't on this machine.
    pub(in crate::file_explorer) fn transport(&self) -> Option<&Arc<Transport>> {
        match self {
            Self::Local => None,
            Self::Remote(t) => Some(t),
        }
    }

    pub(in crate::file_explorer) fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }

    /// How often an idle listing is read again.
    pub(in crate::file_explorer) fn refresh_interval(&self) -> Duration {
        if self.is_remote() { REMOTE_REFRESH_INTERVAL } else { REFRESH_INTERVAL }
    }

    /// Copies and moves report progress in bytes here; on a host (where they run out of
    /// sight) in items.
    pub(in crate::file_explorer) fn progress_in_bytes(&self) -> bool {
        !self.is_remote()
    }

    /// Read `dir`. Here hidden entries are left out as it's read; a host's listing has them
    /// all (they're left out once it arrives).
    pub(in crate::file_explorer) fn list(&self, dir: &Path, show_hidden: bool) -> Answer {
        match self {
            Self::Local => Answer::Local(ops::read_entries(dir, show_hidden)),
            Self::Remote(t) => Answer::Remote(remote::list(t, dir)),
        }
    }

    /// Copy or move `sources` into `dest`. Returns how many items were copied or moved.
    pub(in crate::file_explorer) fn run_op(&self, op: Op, sources: &[PathBuf], dest: &Path, progress: &Progress) -> Result<usize, String> {
        match self {
            Self::Local => ops::run_op(op, sources, dest, progress),
            Self::Remote(t) => remote::run_op(t, op, sources, dest, progress),
        }
    }

    /// Permanently delete `paths`. Returns how many were deleted.
    pub(in crate::file_explorer) fn delete_paths(&self, paths: &[PathBuf], progress: &Progress) -> Result<usize, String> {
        match self {
            Self::Local => ops::delete_paths(paths, progress),
            Self::Remote(t) => remote::delete_paths(t, paths, progress),
        }
    }

    /// Rename `path` to `new_name` in its folder. Never replaces an existing file.
    pub(in crate::file_explorer) fn rename_path(&self, path: &Path, new_name: &str) -> Result<PathBuf, String> {
        match self {
            Self::Local => ops::rename_path(path, new_name),
            Self::Remote(t) => remote::rename_path(t, path, new_name),
        }
    }

    /// Make an empty folder ("New Folder", …) or file ("Untitled", …) in `dest`.
    pub(in crate::file_explorer) fn create_new(&self, dest: &Path, folder: bool) -> Result<PathBuf, String> {
        match self {
            Self::Local => ops::create_new(dest, folder),
            Self::Remote(t) => remote::create_new(t, dest, folder),
        }
    }

    /// `path`'s details for the Properties dialog. Here the MIME type is left out (it takes
    /// a command; the dialog asks for it separately).
    pub(in crate::file_explorer) fn details(&self, path: &Path) -> Details {
        match self {
            Self::Local => local_details(path),
            Self::Remote(t) => remote::details(t, path).unwrap_or_default(),
        }
    }

    /// `(bytes, items)` inside the folder `path`, stopping once `stop` is set. Here
    /// `running` sees the totals so far every so often; a host only gives the end result.
    pub(in crate::file_explorer) fn tree_summary(&self, path: &Path, stop: &AtomicBool, running: &mut dyn FnMut(u64, usize)) -> Option<(u64, usize)> {
        match self {
            Self::Local => Some(ops::tree_summary(path, &mut |bytes, items| {
                running(bytes, items);
                !stop.load(Ordering::Relaxed)
            })),
            Self::Remote(t) => remote::tree_summary(t, path, stop),
        }
    }
}

/// [`FsBackend::details`] on this machine, from the file's metadata.
fn local_details(path: &Path) -> Details {
    let Ok(link) = std::fs::symlink_metadata(path) else { return Details::default() };
    let meta = std::fs::metadata(path).unwrap_or_else(|_| link.clone());
    let secs = |t: std::io::Result<SystemTime>| t.ok()?.duration_since(SystemTime::UNIX_EPOCH).ok().map(|d| d.as_secs());
    Details {
        modified: secs(meta.modified()),
        accessed: secs(meta.accessed()),
        created: secs(meta.created()),
        mode: crate::platform::file_mode(&meta),
        owner: crate::platform::owner_names(&meta),
        link_target: link.file_type().is_symlink().then(|| std::fs::read_link(path).ok()).flatten(),
        mime: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn local_dispatch() {
        let dir = crate::file_explorer::tests::fixture("backend-local");
        let b = FsBackend::Local;
        assert!(!b.is_remote() && b.transport().is_none() && b.progress_in_bytes());
        assert_eq!(b.refresh_interval(), REFRESH_INTERVAL);
        let made = b.create_new(&dir, true).unwrap();
        assert_eq!(made, dir.join("New Folder"));
        let renamed = b.rename_path(&made, "made").unwrap();
        assert!(renamed.is_dir());
        let Answer::Local(Ok(entries)) = b.list(&dir, false) else { panic!("not listed here") };
        assert!(entries.iter().any(|e| e.name == "made" && e.is_dir));
        let details = b.details(&renamed);
        assert!(details.modified.is_some() && details.link_target.is_none() && details.mime.is_none());
        let stop = AtomicBool::new(false);
        assert_eq!(b.tree_summary(&renamed, &stop, &mut |_, _| {}), Some((0, 0)));
        assert_eq!(b.delete_paths(&[renamed.clone()], &Progress::default()), Ok(1));
        assert!(!renamed.exists());
        assert_eq!(b.details(&renamed), Details::default());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
