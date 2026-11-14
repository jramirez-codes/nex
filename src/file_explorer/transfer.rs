//! The file clipboard (copy / cut / paste between explorer panes) and copy / move jobs
//! running on background threads.

use std::path::PathBuf;
use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, TryRecvError};
use std::time::{Duration, Instant};

use egui::Pos2;

use crate::terminal::tilde;

use super::FileExplorer;
use super::fs_ops::{count, delete_paths, parse_paths, run_op};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Op {
    Copy,
    Move,
}

/// Files picked up by Copy or Cut, waiting to be pasted. Shared by every explorer pane.
#[derive(Clone, Debug, PartialEq)]
pub struct Clip {
    pub paths: Vec<PathBuf>,
    pub cut: bool,
}

pub(super) static CLIPBOARD: Mutex<Option<Clip>> = Mutex::new(None);

pub fn clipboard() -> Option<Clip> {
    CLIPBOARD.lock().map(|c| c.clone()).unwrap_or(None)
}

pub(super) fn set_clipboard(clip: Option<Clip>) {
    if let Ok(mut c) = CLIPBOARD.lock() {
        *c = clip;
    }
}

/// Bumped whenever a file operation finishes, so every explorer re-reads its listing.
pub(super) static FS_GENERATION: AtomicU64 = AtomicU64::new(0);

impl FileExplorer {
    /// Put the targets on the file clipboard (and their paths on the system clipboard as text).
    pub fn copy(&mut self, ctx: &egui::Context, cut: bool) {
        let paths = self.targets();
        if paths.is_empty() {
            return;
        }
        ctx.copy_text(paths.iter().map(|p| p.display().to_string()).collect::<Vec<_>>().join("\n"));
        let verb = if cut { "Cut" } else { "Copied" };
        self.set_status(format!("{verb} {} — paste in any folder", count(paths.len())), false);
        set_clipboard(Some(Clip { paths, cut }));
        self.clear_selection();
    }

    /// Paste the file clipboard into `dest` (default: the current folder). Without a file
    /// clipboard, `text` holding absolute paths (or file:// URIs) is pasted as a copy.
    pub fn paste(&mut self, ctx: &egui::Context, dest: Option<PathBuf>, text: Option<&str>) {
        let dest = dest.unwrap_or_else(|| self.dir.clone());
        match clipboard() {
            Some(clip) => {
                if clip.cut {
                    set_clipboard(None);
                }
                self.start_op(ctx, if clip.cut { Op::Move } else { Op::Copy }, clip.paths, dest);
            }
            None => {
                let paths = text.map(parse_paths).unwrap_or_default();
                if !paths.is_empty() {
                    self.start_op(ctx, Op::Copy, paths, dest);
                }
            }
        }
    }

    /// Files dropped from another application: copy them into the folder under `pos`
    /// (or the current folder).
    pub fn drop_external(&mut self, ctx: &egui::Context, paths: Vec<PathBuf>, pos: Option<Pos2>) {
        let dest = pos.and_then(|p| self.folder_at(p, &[])).unwrap_or_else(|| self.dir.clone());
        self.start_op(ctx, Op::Copy, paths, dest);
    }

    /// The folder entry under `pos`, unless it's one of the files being dragged.
    pub(super) fn folder_at(&self, pos: Pos2, dragged: &[PathBuf]) -> Option<PathBuf> {
        let layout = self.layout?;
        let i = layout.item_at(pos, self.scroll, self.entries.len())?;
        let e = &self.entries[i];
        (e.is_dir && !dragged.contains(&e.path)).then(|| e.path.clone())
    }

    /// Copy or move `paths` into `dest` on a background thread.
    pub fn start_op(&mut self, ctx: &egui::Context, op: Op, paths: Vec<PathBuf>, dest: PathBuf) {
        if paths.is_empty() {
            return;
        }
        let verb = if op == Op::Copy { "Copying" } else { "Moving" };
        self.set_status(format!("{verb} {}…", count(paths.len())), false);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = run_op(op, &paths, &dest).map(|n| {
                let verb = if op == Op::Copy { "Copied" } else { "Moved" };
                if n == 0 { "Nothing to do — already here".to_string() } else { format!("{verb} {} to {}", count(n), tilde(&dest)) }
            });
            FS_GENERATION.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.jobs.push(rx);
    }

    /// Ask to delete the targets; nothing is deleted until `confirm_delete` is called.
    pub fn request_delete(&mut self) {
        let paths = self.targets();
        if !paths.is_empty() {
            self.confirm_delete = Some(paths);
        }
    }

    /// The delete confirmation is showing.
    pub fn confirming_delete(&self) -> bool {
        self.confirm_delete.is_some()
    }

    pub(super) fn cancel_delete(&mut self) {
        self.confirm_delete = None;
    }

    /// The user confirmed: permanently delete on a background thread.
    pub(super) fn confirm_delete(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.confirm_delete.take() else { return };
        // Deleted files can't be pasted any more.
        if let Some(mut clip) = clipboard() {
            clip.paths.retain(|p| !paths.contains(p));
            set_clipboard((!clip.paths.is_empty()).then_some(clip));
        }
        self.clear_selection();
        self.set_status(format!("Deleting {}…", count(paths.len())), false);
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let result = delete_paths(&paths).map(|n| format!("Deleted {}", count(n)));
            FS_GENERATION.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.jobs.push(rx);
    }

    pub(super) fn set_status(&mut self, text: String, error: bool) {
        self.status = Some((text, error, Instant::now()));
    }

    pub(super) fn poll_jobs(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.jobs.retain(|rx| match rx.try_recv() {
            Ok(r) => {
                finished.push(r);
                false
            }
            Err(TryRecvError::Empty) => true,
            Err(TryRecvError::Disconnected) => false,
        });
        for r in finished {
            match r {
                Ok(msg) => self.set_status(msg, false),
                Err(msg) => self.set_status(msg, true),
            }
        }
        let generation = FS_GENERATION.load(Ordering::Relaxed);
        if generation != self.seen_generation {
            self.seen_generation = generation;
            self.refresh();
        }
        if !self.jobs.is_empty() {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
