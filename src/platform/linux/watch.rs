//! Watching folders for changes with inotify, so a file explorer updates the moment
//! something in its folder changes (another nex moved a file away, `mv` in a terminal, a
//! file manager) instead of on its next periodic re-read.
//!
//! One inotify instance and one thread serve every watch. inotify hands out one watch per
//! folder, shared by everything watching it, so watches are counted and removed with the last.

use std::collections::HashMap;
use std::ffi::CString;
use std::os::unix::ffi::OsStrExt;
use std::path::Path;
use std::sync::{Mutex, OnceLock};

/// Created, deleted, renamed or moved in or out, finished writing, or permissions changed. Not
/// every write: a file being copied in would wake the app thousands of times.
const MASK: u32 = libc::IN_CREATE
    | libc::IN_DELETE
    | libc::IN_MOVED_FROM
    | libc::IN_MOVED_TO
    | libc::IN_CLOSE_WRITE
    | libc::IN_ATTRIB
    | libc::IN_DELETE_SELF
    | libc::IN_MOVE_SELF;

struct Watcher {
    fd: i32,
    /// Watch descriptor → how many `DirWatch`es use it.
    counts: Mutex<HashMap<i32, usize>>,
}

static WATCHER: OnceLock<Option<Watcher>> = OnceLock::new();
type Callback = Box<dyn Fn() + Send + Sync>;
static ON_CHANGE: Mutex<Vec<Callback>> = Mutex::new(Vec::new());

/// Call `f` (on a background thread) whenever a watched folder changes, from now on.
pub fn on_dir_change(f: impl Fn() + Send + Sync + 'static) {
    ON_CHANGE.lock().unwrap().push(Box::new(f));
}

fn watcher() -> Option<&'static Watcher> {
    WATCHER
        .get_or_init(|| {
            // SAFETY: plain syscall.
            let fd = unsafe { libc::inotify_init1(libc::IN_CLOEXEC) };
            if fd < 0 {
                return None;
            }
            std::thread::Builder::new().name("nex-inotify".into()).spawn(move || read_events(fd)).ok()?;
            Some(Watcher { fd, counts: Mutex::new(HashMap::new()) })
        })
        .as_ref()
}

/// Wait for events; each read (however many events it holds) is one change.
fn read_events(fd: i32) {
    let mut buf = [0u8; 4096];
    loop {
        // SAFETY: reading into a buffer of the given length.
        let n = unsafe { libc::read(fd, buf.as_mut_ptr().cast(), buf.len()) };
        if n < 0 && std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
            continue;
        }
        if n <= 0 {
            return;
        }
        for f in ON_CHANGE.lock().unwrap().iter() {
            f();
        }
        // Let a burst of changes (a build, an archive unpacking) pile up into the next read.
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

/// A folder being watched for as long as this lives.
pub struct DirWatch {
    wd: i32,
}

impl DirWatch {
    /// Watch `dir`; `None` if it can't be (gone, no permission, out of inotify watches).
    pub fn new(dir: &Path) -> Option<Self> {
        let w = watcher()?;
        let path = CString::new(dir.as_os_str().as_bytes()).ok()?;
        // Holding the lock keeps a concurrent drop from removing the shared watch in between.
        let mut counts = w.counts.lock().unwrap();
        // SAFETY: a valid fd and C string.
        let wd = unsafe { libc::inotify_add_watch(w.fd, path.as_ptr(), MASK) };
        if wd < 0 {
            return None;
        }
        *counts.entry(wd).or_default() += 1;
        Some(Self { wd })
    }
}

impl Drop for DirWatch {
    fn drop(&mut self) {
        let Some(w) = watcher() else { return };
        let mut counts = w.counts.lock().unwrap();
        let Some(n) = counts.get_mut(&self.wd) else { return };
        *n -= 1;
        if *n == 0 {
            counts.remove(&self.wd);
            // SAFETY: plain syscall on our own fd and watch.
            unsafe { libc::inotify_rm_watch(w.fd, self.wd) };
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::time::{Duration, Instant};

    static CHANGES: AtomicUsize = AtomicUsize::new(0);

    fn wait_for_change(since: usize) -> bool {
        let deadline = Instant::now() + Duration::from_secs(2);
        while Instant::now() < deadline {
            if CHANGES.load(Ordering::SeqCst) > since {
                return true;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        false
    }

    #[test]
    fn notices_files_moving_out_and_shares_watches() {
        on_dir_change(|| {
            CHANGES.fetch_add(1, Ordering::SeqCst);
        });
        let dir = std::env::temp_dir().join(format!("nex-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::write(dir.join("a.txt"), "a").unwrap();
        let first = DirWatch::new(&dir).unwrap();
        let second = DirWatch::new(&dir).unwrap();
        assert_eq!(first.wd, second.wd);

        let before = CHANGES.load(Ordering::SeqCst);
        std::fs::rename(dir.join("a.txt"), dir.join("sub/a.txt")).unwrap();
        assert!(wait_for_change(before), "no event for a file moved out");

        // Dropping one watcher keeps the folder watched for the other.
        drop(first);
        std::thread::sleep(Duration::from_millis(50));
        let before = CHANGES.load(Ordering::SeqCst);
        std::fs::write(dir.join("b.txt"), "b").unwrap();
        assert!(wait_for_change(before), "watch removed while still in use");
        drop(second);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
