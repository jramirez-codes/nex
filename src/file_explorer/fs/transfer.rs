//! The file clipboard (copy / cut / paste between explorer panes) and copy / move / delete
//! jobs running on background threads, with the progress they report.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use egui::Pos2;

use crate::terminal::tilde;

use crate::file_explorer::FileExplorer;
use crate::file_explorer::fs::ops::{count, delete_paths, human_size, parse_paths, run_op};

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

pub(in crate::file_explorer) static CLIPBOARD: Mutex<Option<Clip>> = Mutex::new(None);

pub fn clipboard() -> Option<Clip> {
    CLIPBOARD.lock().map(|c| c.clone()).unwrap_or(None)
}

pub(in crate::file_explorer) fn set_clipboard(clip: Option<Clip>) {
    if let Ok(mut c) = CLIPBOARD.lock() {
        *c = clip;
    }
}

/// Bumped whenever a file operation finishes, so every explorer re-reads its listing.
pub(in crate::file_explorer) static FS_GENERATION: AtomicU64 = AtomicU64::new(0);

/// Files changed outside the explorers' own jobs (another window moved them away): every
/// explorer re-reads its listing on its next frame.
pub fn files_changed() {
    FS_GENERATION.fetch_add(1, Ordering::Relaxed);
}

/// How far a job has got, written by its thread and read by the action bar. Copies and
/// moves count bytes; deletes count items.
#[derive(Default)]
pub struct Progress {
    pub total: AtomicU64,
    pub done: AtomicU64,
    /// The name of the file being worked on.
    current: Mutex<String>,
    /// Set by the Cancel button; the job stops at the next chunk or item.
    pub cancel: AtomicBool,
}

impl Progress {
    pub fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }

    /// An `Interrupted` error once cancelled, for `?` in copy loops.
    pub fn check(&self) -> std::io::Result<()> {
        if self.cancelled() { Err(std::io::Error::new(std::io::ErrorKind::Interrupted, "cancelled")) } else { Ok(()) }
    }

    pub fn set_current(&self, name: &str) {
        if let Ok(mut c) = self.current.lock() {
            name.clone_into(&mut c);
        }
    }

    fn current(&self) -> String {
        self.current.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// `(done, total)`.
    fn amounts(&self) -> (u64, u64) {
        (self.done.load(Ordering::Relaxed), self.total.load(Ordering::Relaxed))
    }
}

/// A copy, move or delete running on its own thread.
pub(in crate::file_explorer) struct Job {
    rx: Receiver<Result<String, String>>,
    progress: Arc<Progress>,
    /// "Copying 3 items".
    label: String,
    /// Progress counts bytes (copies, moves) rather than items (deletes).
    bytes: bool,
    started: Instant,
}

/// What the action bar shows while jobs run.
pub(in crate::file_explorer) struct JobSummary {
    /// 0..=1.
    pub(in crate::file_explorer) fraction: f32,
    /// "Copying 3 items · 45 M of 120 M · 37% · ~12 s left".
    pub(in crate::file_explorer) text: String,
    /// The file being worked on now.
    pub(in crate::file_explorer) current: String,
}

impl FileExplorer {
    /// Put the targets on the file clipboard (and their paths on the system clipboard as text).
    pub fn copy(&mut self, ctx: &egui::Context, cut: bool) {
        self.find = None;
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
        self.find = None;
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

    /// Files dropped from another window: move or copy them into the folder under `pos`
    /// (or the current folder).
    pub fn drop_external(&mut self, ctx: &egui::Context, op: Op, paths: Vec<PathBuf>, pos: Option<Pos2>) {
        let dest = pos.and_then(|p| self.folder_at(p, &paths)).unwrap_or_else(|| self.dir.clone());
        self.start_op(ctx, op, paths, dest);
    }

    /// The folder entry under `pos`, unless it's one of the files being dragged.
    pub(in crate::file_explorer) fn folder_at(&self, pos: Pos2, dragged: &[PathBuf]) -> Option<PathBuf> {
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
        let (verb, past) = if op == Op::Copy { ("Copying", "copied") } else { ("Moving", "moved") };
        let label = format!("{verb} {}", count(paths.len()));
        self.spawn_job(ctx, label, true, move |progress| {
            let total = paths.len();
            run_op(op, &paths, &dest, progress).map(|n| {
                if progress.cancelled() {
                    format!("Cancelled — {past} {n} of {}", count(total))
                } else if n == 0 {
                    "Nothing to do — already here".to_string()
                } else {
                    let verb = if op == Op::Copy { "Copied" } else { "Moved" };
                    format!("{verb} {} to {}", count(n), tilde(&dest))
                }
            })
        });
    }

    /// Run `work` on a new thread as a job the action bar follows.
    fn spawn_job(
        &mut self,
        ctx: &egui::Context,
        label: String,
        bytes: bool,
        work: impl FnOnce(&Progress) -> Result<String, String> + Send + 'static,
    ) {
        let progress = Arc::new(Progress::default());
        let (tx, rx) = mpsc::channel();
        let ctx = ctx.clone();
        let p = progress.clone();
        std::thread::spawn(move || {
            let result = work(&p);
            FS_GENERATION.fetch_add(1, Ordering::Relaxed);
            let _ = tx.send(result);
            ctx.request_repaint();
        });
        self.set_status(format!("{label}…"), false);
        self.jobs.push(Job { rx, progress, label, bytes, started: Instant::now() });
    }

    /// Stop every running job (each stops at its next chunk or item).
    pub(in crate::file_explorer) fn cancel_jobs(&mut self) {
        for job in &self.jobs {
            job.progress.cancel.store(true, Ordering::Relaxed);
        }
    }

    /// Ask to delete the targets; nothing is deleted until `confirm_delete` is called.
    pub fn request_delete(&mut self) {
        let paths = self.targets();
        if !paths.is_empty() {
            self.confirm_delete = Some(paths);
        }
    }

    pub(in crate::file_explorer) fn cancel_delete(&mut self) {
        self.confirm_delete = None;
    }

    /// The user confirmed: permanently delete on a background thread.
    pub(in crate::file_explorer) fn confirm_delete(&mut self, ctx: &egui::Context) {
        let Some(paths) = self.confirm_delete.take() else { return };
        // Deleted files can't be pasted any more.
        if let Some(mut clip) = clipboard() {
            clip.paths.retain(|p| !paths.contains(p));
            set_clipboard((!clip.paths.is_empty()).then_some(clip));
        }
        self.clear_selection();
        let label = format!("Deleting {}", count(paths.len()));
        self.spawn_job(ctx, label, false, move |progress| {
            delete_paths(&paths, progress).map(|n| {
                if progress.cancelled() { format!("Cancelled — deleted {n} of {}", count(paths.len())) } else { format!("Deleted {}", count(n)) }
            })
        });
    }

    pub(in crate::file_explorer) fn set_status(&mut self, text: String, error: bool) {
        self.status = Some((text, error, Instant::now()));
    }

    pub(in crate::file_explorer) fn poll_jobs(&mut self, ctx: &egui::Context) {
        let mut finished = Vec::new();
        self.jobs.retain(|job| match job.rx.try_recv() {
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

    /// The running jobs, added up into one line and one bar.
    pub(in crate::file_explorer) fn job_summary(&self) -> Option<JobSummary> {
        let first = self.jobs.first()?;
        let fraction_of = |job: &Job| {
            let (done, total) = job.progress.amounts();
            if total == 0 { 0.0 } else { (done as f64 / total as f64).min(1.0) }
        };
        let byte_jobs: Vec<&Job> = self.jobs.iter().filter(|j| j.bytes).collect();
        let (done, total) = byte_jobs.iter().map(|j| j.progress.amounts()).fold((0, 0), |(d, t), (jd, jt)| (d + jd, t + jt));
        // Mixed jobs (a copy and a delete) can't be added up by bytes: average them.
        let fraction = if byte_jobs.len() == self.jobs.len() && total > 0 {
            done as f64 / total as f64
        } else {
            self.jobs.iter().map(fraction_of).sum::<f64>() / self.jobs.len() as f64
        };
        let mut parts = vec![if self.jobs.len() == 1 { first.label.clone() } else { format!("{} operations", self.jobs.len()) }];
        if !byte_jobs.is_empty() && total > 0 {
            parts.push(format!("{} of {}", human_size(done), human_size(total)));
        }
        parts.push(format!("{:.0}%", fraction * 100.0));
        let elapsed = self.jobs.iter().map(|j| j.started).min().map(|t| t.elapsed()).unwrap_or_default();
        if let Some(eta) = eta(elapsed, fraction) {
            parts.push(eta);
        }
        if self.jobs.iter().any(|j| j.progress.cancelled()) {
            parts = vec!["Cancelling…".into()];
        }
        Some(JobSummary { fraction: fraction.clamp(0.0, 1.0) as f32, text: parts.join(" · "), current: first.progress.current() })
    }
}

/// "~12 s left", once a job has run long enough for its rate to mean something.
fn eta(elapsed: Duration, fraction: f64) -> Option<String> {
    if elapsed < Duration::from_secs(1) || fraction <= 0.01 || fraction >= 1.0 {
        return None;
    }
    let left = elapsed.as_secs_f64() * (1.0 - fraction) / fraction;
    Some(match left {
        s if s < 60.0 => format!("~{:.0} s left", s.max(1.0)),
        s if s < 3600.0 => format!("~{:.0} min left", s / 60.0),
        s => format!("~{:.1} h left", s / 3600.0),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eta_waits_then_estimates() {
        assert_eq!(eta(Duration::from_millis(500), 0.5), None);
        assert_eq!(eta(Duration::from_secs(10), 0.5).as_deref(), Some("~10 s left"));
        assert_eq!(eta(Duration::from_secs(60), 0.25).as_deref(), Some("~3 min left"));
        assert_eq!(eta(Duration::from_secs(10), 1.0), None);
    }
}
