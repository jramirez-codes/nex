//! Typing in the explorer: renaming in place, new folders and files, the location bar, and
//! type-to-find. Also loading the "Open With" list for the right-click menu.

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::platform::AppChoice;
use crate::terminal::tilde;

use super::FileExplorer;
use super::fs_ops::{complete_location, create_new, expand_location, rename_path};
use super::line_edit::LineEdit;
use super::transfer::{FS_GENERATION, clipboard, set_clipboard};

/// Type-to-find forgets what was typed after this long without a key.
pub(super) const FIND_TIMEOUT: Duration = Duration::from_millis(1500);

/// An item being renamed: its path, and the name being typed.
pub(super) struct Rename {
    pub(super) path: PathBuf,
    pub(super) field: LineEdit,
}

pub(super) struct Find {
    pub(super) query: String,
    pub(super) at: Instant,
    /// Nothing in the folder matches.
    pub(super) missed: bool,
}

/// The apps that can open one file; `apps` is `None` while they're still being looked up.
pub(super) struct OpenWith {
    pub(super) path: PathBuf,
    pub(super) apps: Arc<Mutex<Option<Vec<AppChoice>>>>,
}

/// The part of `name` a rename selects first: everything before the extension.
fn stem_len(name: &str, is_dir: bool) -> usize {
    let n = name.chars().count();
    if is_dir {
        return n;
    }
    match name.rfind('.') {
        // ".bashrc" has no extension; "archive.tar.gz" selects "archive.tar".
        Some(0) | None => n,
        Some(i) => name[..i].chars().count(),
    }
}

impl FileExplorer {
    /// Start renaming the single target (F2).
    pub fn start_rename(&mut self) {
        let targets = self.targets();
        let [path] = targets.as_slice() else { return };
        let Some(i) = self.entries.iter().position(|e| &e.path == path) else { return };
        let e = &self.entries[i];
        let field = LineEdit::with_selection(e.name.clone(), 0, stem_len(&e.name, e.is_dir));
        self.rename = Some(Rename { path: path.clone(), field });
        self.marked.clear();
        self.selecting = false;
        self.selected = i;
        self.reveal = true;
        self.find = None;
    }

    /// Apply the typed name. On failure the field stays open with the error shown.
    pub(super) fn commit_rename(&mut self) {
        let Some(r) = self.rename.take() else { return };
        let new_name = r.field.text.trim().to_string();
        if r.path.file_name().is_some_and(|n| n.to_string_lossy() == new_name) {
            return;
        }
        match self.rename_item(&r.path, &new_name) {
            Ok(_) => self.set_status(format!("Renamed to “{new_name}”"), false),
            Err(e) => {
                self.set_status(e, true);
                self.rename = Some(r);
            }
        }
    }

    /// Rename `path`, then show and select it under its new name. Used by the rename field
    /// and the Properties dialog.
    pub(super) fn rename_item(&mut self, path: &Path, new_name: &str) -> Result<PathBuf, String> {
        let new = rename_path(path, new_name)?;
        // Cut or copied files keep pointing at the right file.
        if let Some(mut clip) = clipboard() {
            for p in clip.paths.iter_mut().filter(|p| *p == path) {
                p.clone_from(&new);
            }
            set_clipboard(Some(clip));
        }
        FS_GENERATION.fetch_add(1, Ordering::Relaxed);
        self.refresh();
        self.select_path(&new);
        Ok(new)
    }

    pub(super) fn select_path(&mut self, path: &Path) {
        if let Some(i) = self.entries.iter().position(|e| e.path == path) {
            self.marked.clear();
            self.selected = i;
            self.reveal = true;
        }
    }

    /// Make "New Folder" (or an empty "Untitled" file) here and start naming it.
    pub fn create_item(&mut self, folder: bool) {
        match create_new(&self.dir, folder) {
            Ok(path) => {
                FS_GENERATION.fetch_add(1, Ordering::Relaxed);
                self.refresh();
                self.select_path(&path);
                self.start_rename();
                // A new item's whole name is the placeholder: select all of it.
                if let Some(r) = self.rename.as_mut() {
                    r.field = LineEdit::new(r.field.text.clone());
                }
            }
            Err(e) => self.set_status(e, true),
        }
    }

    /// Type a location instead of clicking breadcrumbs (Ctrl+L).
    pub fn edit_location(&mut self) {
        let mut text = tilde(&self.dir);
        if !text.ends_with('/') {
            text.push('/');
        }
        self.location = Some(LineEdit::new(text));
        self.rename = None;
        self.find = None;
    }

    /// Go to the typed folder, or open the typed file. Unknown paths keep the field open.
    pub(super) fn commit_location(&mut self) {
        let Some(field) = self.location.take() else { return };
        let path = expand_location(&field.text, &self.dir);
        if path.is_dir() {
            self.navigate(path);
        } else if path.exists() {
            if let Some(parent) = path.parent().filter(|p| *p != self.dir) {
                self.navigate(parent.to_path_buf());
            }
            self.select_path(&path);
            crate::platform::open_uri(path.as_os_str());
        } else {
            self.set_status(format!("Can't find “{}”", field.text.trim()), true);
            self.location = Some(field);
        }
    }

    pub(super) fn complete_location(&mut self) {
        let dir = self.dir.clone();
        if let Some(field) = self.location.as_mut() {
            if let Some(text) = complete_location(&field.text, &dir) {
                field.set_text(text);
            }
        }
    }

    /// Add typed text to the find query and select the first matching name.
    pub(super) fn find_type(&mut self, text: &str) {
        let find = self.find.get_or_insert_with(|| Find { query: String::new(), at: Instant::now(), missed: false });
        find.query.push_str(text);
        find.at = Instant::now();
        self.find_step(0);
    }

    /// Backspace in the query. Returns false when there was no query to edit.
    pub(super) fn find_backspace(&mut self) -> bool {
        let Some(find) = self.find.as_mut() else { return false };
        find.query.pop();
        find.at = Instant::now();
        if find.query.is_empty() {
            self.find = None;
        } else {
            self.find_step(0);
        }
        true
    }

    /// Select the next match at or after the cursor (`step` 0), or the next / previous one
    /// after it (`step` 1 / -1), wrapping around.
    pub(super) fn find_step(&mut self, step: isize) {
        let Some(find) = self.find.as_mut() else { return };
        find.at = Instant::now();
        let found = find_match(&self.entries.iter().map(|e| e.name.as_str()).collect::<Vec<_>>(), &find.query, self.selected, step);
        find.missed = found.is_none();
        if let Some(i) = found {
            self.marked.clear();
            self.selected = i;
            self.reveal = true;
        }
    }

    /// Forget the query once the user stops typing.
    pub(super) fn expire_find(&mut self, ctx: &egui::Context) {
        if let Some(find) = &self.find {
            let left = FIND_TIMEOUT.saturating_sub(find.at.elapsed());
            if left.is_zero() {
                self.find = None;
            } else {
                ctx.request_repaint_after(left);
            }
        }
    }

    /// Look up the apps for `path` on a thread (reading desktop entries takes a moment).
    pub(super) fn load_open_with(&mut self, ctx: &egui::Context, path: PathBuf) {
        if self.open_with.as_ref().is_some_and(|o| o.path == path) {
            return;
        }
        let apps = Arc::new(Mutex::new(None));
        let (slot, p, ctx) = (apps.clone(), path.clone(), ctx.clone());
        std::thread::spawn(move || {
            let found = crate::platform::apps_for(&p);
            if let Ok(mut s) = slot.lock() {
                *s = Some(found);
            }
            ctx.request_repaint();
        });
        self.open_with = Some(OpenWith { path, apps });
    }

    /// Open the right-clicked file with the `k`th app of its "Open With" list.
    pub(super) fn open_with_app(&mut self, k: usize) {
        let Some(o) = &self.open_with else { return };
        let app = o.apps.lock().ok().and_then(|a| a.as_ref().and_then(|a| a.get(k).cloned()));
        if let Some(app) = app {
            crate::platform::open_with(&app, &o.path);
            self.set_status(format!("Opening with {}…", app.name), false);
        }
    }
}

/// The index of the name matching `query` (case-insensitive): names starting with it win
/// over names containing it. Searches from `from` (`step` 0) or the item after / before it
/// (`step` 1 / -1), wrapping around.
fn find_match(names: &[&str], query: &str, from: usize, step: isize) -> Option<usize> {
    let n = names.len();
    if n == 0 || query.is_empty() {
        return None;
    }
    let q = query.to_lowercase();
    let dir = if step < 0 { -1 } else { 1 };
    let start = (from as isize + step).rem_euclid(n as isize);
    let order = (0..n as isize).map(|k| (start + k * dir).rem_euclid(n as isize) as usize);
    let lower: Vec<String> = names.iter().map(|s| s.to_lowercase()).collect();
    let by = |test: &dyn Fn(&str) -> bool| order.clone().find(|&i| test(&lower[i]));
    by(&|s| s.starts_with(&q)).or_else(|| by(&|s| s.contains(&q)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_explorer::tests::fixture;

    #[test]
    fn stems() {
        assert_eq!(stem_len("report.pdf", false), 6);
        assert_eq!(stem_len("archive.tar.gz", false), 11);
        assert_eq!(stem_len(".bashrc", false), 7);
        assert_eq!(stem_len("my.folder", true), 9);
        assert_eq!(stem_len("naïve.txt", false), 5);
    }

    #[test]
    fn find_prefers_prefixes_and_steps() {
        let names = ["Alpha", "beta", "Cargo.lock", "Cargo.toml", "scargo"];
        assert_eq!(find_match(&names, "car", 0, 0), Some(2));
        assert_eq!(find_match(&names, "CAR", 2, 1), Some(3));
        assert_eq!(find_match(&names, "car", 3, 1), Some(2));
        assert_eq!(find_match(&names, "car", 2, -1), Some(3));
        assert_eq!(find_match(&names, "arg", 0, 0), Some(2));
        assert_eq!(find_match(&names, "zzz", 0, 0), None);
    }

    #[test]
    fn rename_and_create_in_place() {
        let dir = fixture("edit");
        let mut b = FileExplorer::new(dir.clone());
        b.create_item(true);
        assert!(dir.join("New Folder").is_dir());
        let r = b.rename.as_ref().unwrap();
        assert_eq!(r.field.selected_text(), "New Folder");
        b.text("Projects");
        b.commit_rename();
        assert!(dir.join("Projects").is_dir() && !dir.join("New Folder").exists());
        assert_eq!(b.entries[b.selected].name, "Projects");

        // A clash keeps the field open.
        b.select_path(&dir.join("zeta.txt"));
        b.start_rename();
        assert_eq!(b.rename.as_ref().unwrap().field.selected_text(), "zeta");
        b.text("Apple");
        b.rename.as_mut().unwrap().field.set_text("Apple.md".into());
        b.commit_rename();
        assert!(b.rename.is_some() && dir.join("zeta.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn typed_location() {
        let dir = fixture("typed");
        let mut b = FileExplorer::new(dir.clone());
        b.edit_location();
        b.location.as_mut().unwrap().set_text("be".into());
        b.complete_location();
        assert_eq!(b.location.as_ref().unwrap().text, "beta/");
        b.commit_location();
        assert_eq!(b.dir, dir.canonicalize().unwrap().join("beta"));
        b.edit_location();
        b.location.as_mut().unwrap().set_text("/definitely/missing".into());
        b.commit_location();
        assert!(b.location.is_some());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
