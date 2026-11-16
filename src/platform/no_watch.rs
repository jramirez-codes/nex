//! Folder watching stub for OSes without a backend yet: explorers fall back to re-reading
//! their folder every couple of seconds.

use std::path::Path;

pub fn on_dir_change(_f: impl Fn() + Send + Sync + 'static) {}

pub struct DirWatch(());

impl DirWatch {
    pub fn new(_dir: &Path) -> Option<Self> {
        None
    }
}
