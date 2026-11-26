//! The Properties dialog: a file's or folder's name (which can be edited), kind, location,
//! size, dates, permissions and owner. A folder's contents are added up on a background
//! thread, and the dialog shows the running total until that finishes. On the phone, a
//! file on the ssh host has a Download button (see `fs/download.rs`).

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

use egui::{Color32, Id, Key, RichText, Sense, Vec2};

use crate::layout::PaneId;
use crate::terminal::tilde;
use crate::ui::theme::{ACCENT, Theme};

use crate::file_explorer::fs::backend::FsBackend;
use crate::file_explorer::fs::ops::{human_size, thousands};
use crate::file_explorer::fs::remote::Details;
use crate::file_explorer::view::items::{paint_icon, sizes};
use crate::file_explorer::{Entry, FileExplorer};

pub(in crate::file_explorer) struct Properties {
    path: PathBuf,
    /// The name field.
    name: String,
    entry: Entry,
    /// The folder it's in, for people.
    location: Option<String>,
    info: Info,
    slow: Arc<Mutex<Slow>>,
    /// Tells the folder-size thread to stop (the dialog closed).
    stop: Arc<AtomicBool>,
}

impl Drop for Properties {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

/// What's read straight away (here; on an ssh host it comes with the slow part).
#[derive(Default)]
struct Info {
    link_target: Option<PathBuf>,
    modified: Option<String>,
    accessed: Option<String>,
    created: Option<String>,
    mode: Option<u32>,
    owner: Option<(String, String)>,
}

/// What takes a moment: the MIME type (a command) and a folder's contents (a walk).
#[derive(Default)]
struct Slow {
    mime: Option<String>,
    /// `(bytes, items)` so far.
    contents: Option<(u64, usize)>,
    /// The details, for a file on an ssh host.
    info: Option<Info>,
    done: bool,
}

impl From<Details> for Info {
    fn from(d: Details) -> Self {
        Info {
            link_target: d.link_target,
            modified: epoch_time(d.modified),
            accessed: epoch_time(d.accessed),
            created: epoch_time(d.created),
            mode: d.mode,
            owner: d.owner,
        }
    }
}

/// Seconds since 1970 as a local time.
fn epoch_time(secs: Option<u64>) -> Option<String> {
    secs.and_then(|s| crate::platform::format_local_time(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s)))
}

/// `rwxr-xr-x (755)`.
fn permissions(mode: u32) -> String {
    let bits: String = (0..9).map(|i| if mode & (0o400 >> i) != 0 { b"rwx"[i % 3] as char } else { '-' }).collect();
    format!("{bits} ({:o})", mode & 0o777)
}

/// "PNG image" style names for common MIME types.
fn kind(mime: &str, ext: Option<&str>) -> String {
    let ext = ext.map(str::to_uppercase);
    let named = |what: &str| ext.as_ref().map_or_else(|| what.to_string(), |e| format!("{e} {}", what.to_lowercase()));
    let (top, sub) = mime.split_once('/').unwrap_or((mime, ""));
    let label = match (top, sub) {
        ("inode", "directory") => "Folder".into(),
        ("image", _) => named("Image"),
        ("video", _) => named("Video"),
        ("audio", _) => named("Audio"),
        ("text", _) => named("Text"),
        ("application", "pdf") => "PDF document".into(),
        ("application", s) if ["zip", "gzip", "x-tar", "x-xz", "x-bzip2", "zstd", "x-7z-compressed", "vnd.rar"].contains(&s) => named("Archive"),
        ("application", s) if s.contains("executable") || s == "x-sharedlib" => "Program".into(),
        _ => named("File"),
    };
    format!("{label} ({mime})")
}

impl Properties {
    #[cfg(test)]
    pub(in crate::file_explorer) fn path(&self) -> &std::path::Path {
        &self.path
    }

    fn open(ctx: &egui::Context, path: PathBuf) -> Option<Self> {
        let link = std::fs::symlink_metadata(&path).ok()?;
        let meta = std::fs::metadata(&path).unwrap_or_else(|_| link.clone());
        let name = path.file_name().map_or_else(|| tilde(&path), |n| n.to_string_lossy().into_owned());
        let entry = Entry {
            name: name.clone(),
            path: path.clone(),
            is_dir: meta.is_dir(),
            size: (!meta.is_dir()).then_some(meta.len()),
            is_symlink: link.file_type().is_symlink(),
            executable: crate::platform::is_executable(&meta),
        };
        let backend = FsBackend::Local;
        let info = Info::from(backend.details(&path));
        let location = path.parent().map(tilde);
        let slow = Arc::new(Mutex::new(Slow::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st, p, ctx, is_dir) = (slow.clone(), stop.clone(), path.clone(), ctx.clone(), entry.is_dir);
        std::thread::spawn(move || {
            let mime = if is_dir { None } else { crate::platform::mime_type(&p) };
            if let Ok(mut s) = s.lock() {
                s.mime = mime;
            }
            ctx.request_repaint();
            if is_dir {
                let contents = backend.tree_summary(&p, &st, &mut |bytes, items| {
                    if let Ok(mut s) = s.lock() {
                        s.contents = Some((bytes, items));
                    }
                    ctx.request_repaint();
                });
                if let Ok(mut s) = s.lock() {
                    s.contents = contents;
                }
            }
            if let Ok(mut s) = s.lock() {
                s.done = true;
            }
            ctx.request_repaint();
        });
        Some(Self { path, name, entry, location, info, slow, stop })
    }

    /// Properties of `entry`, on the machine `backend` works on (an ssh host): everything
    /// comes from there, in the background.
    fn open_in_background(ctx: &egui::Context, backend: FsBackend, entry: Entry, location: Option<String>) -> Self {
        let slow = Arc::new(Mutex::new(Slow::default()));
        let stop = Arc::new(AtomicBool::new(false));
        let (s, st, p, ctx, is_dir) = (slow.clone(), stop.clone(), entry.path.clone(), ctx.clone(), entry.is_dir);
        std::thread::spawn(move || {
            let mut details = backend.details(&p);
            if let Ok(mut s) = s.lock() {
                s.mime = details.mime.take().filter(|_| !is_dir);
                s.info = Some(Info::from(details));
            }
            ctx.request_repaint();
            if is_dir && !st.load(Ordering::Relaxed) {
                let contents = backend.tree_summary(&p, &st, &mut |_, _| {});
                if let Ok(mut s) = s.lock() {
                    s.contents = contents;
                }
            }
            if let Ok(mut s) = s.lock() {
                s.done = true;
            }
            ctx.request_repaint();
        });
        Self { path: entry.path.clone(), name: entry.name.clone(), entry, location, info: Info::default(), slow, stop }
    }

    /// `(label, value)` rows for the dialog.
    fn rows(&self) -> Vec<(&'static str, String)> {
        let slow = self.slow.lock();
        let slow = slow.as_deref().ok();
        let done = slow.is_some_and(|s| s.done);
        let info = slow.and_then(|s| s.info.as_ref()).unwrap_or(&self.info);
        let e = &self.entry;
        let mut rows = Vec::new();
        let ext = e.path.extension().map(|x| x.to_string_lossy().into_owned());
        let kind = match (e.is_dir, slow.and_then(|s| s.mime.as_deref())) {
            (true, _) => "Folder".to_string(),
            (false, Some(mime)) => kind(mime, ext.as_deref()),
            (false, None) if done => ext.map_or_else(|| "File".into(), |x| format!("{} file", x.to_uppercase())),
            (false, None) => "…".into(),
        };
        rows.push(("Kind", if e.is_symlink { format!("Link to {}", kind.to_lowercase()) } else { kind }));
        if let Some(target) = &info.link_target {
            rows.push(("Link target", target.display().to_string()));
        }
        if let Some(location) = &self.location {
            rows.push(("Location", location.clone()));
        }
        if let Some(size) = e.size {
            rows.push(("Size", format!("{} ({} bytes)", human_size(size), thousands(size))));
        } else {
            let contents = slow.and_then(|s| s.contents);
            let text = match contents {
                Some((bytes, items)) => {
                    let items = if items == 1 { "1 item".into() } else { format!("{} items", thousands(items as u64)) };
                    format!("{items}, {}{}", human_size(bytes), if done { "" } else { " …" })
                }
                None if done => "Unknown".into(),
                None => "Calculating…".into(),
            };
            rows.push(("Contents", text));
        }
        for (label, t) in [("Modified", &info.modified), ("Accessed", &info.accessed), ("Created", &info.created)] {
            if let Some(t) = t {
                rows.push((label, t.clone()));
            }
        }
        if let Some(mode) = info.mode {
            rows.push(("Permissions", permissions(mode)));
        }
        if let Some((user, group)) = &info.owner {
            rows.push(("Owner", format!("{user}, group {group}")));
        }
        rows
    }
}

impl FileExplorer {
    /// What the listing says about `path` (the folder being shown is a folder).
    fn entry_for(&self, path: &Path) -> Entry {
        self.entries.iter().find(|e| e.path == path).cloned().unwrap_or_else(|| Entry {
            name: path.file_name().map_or_else(|| self.shown_path(path), |n| n.to_string_lossy().into_owned()),
            path: path.to_path_buf(),
            is_dir: true,
            size: None,
            is_symlink: false,
            executable: false,
        })
    }

    /// Open Properties for the single target, or for the folder being shown (`folder`, or
    /// when several items are selected).
    pub fn show_properties(&mut self, folder: bool) {
        let targets = self.targets();
        let path = match targets.as_slice() {
            [one] if !folder => one.clone(),
            _ => self.dir.clone(),
        };
        self.pending_properties = Some(path);
    }

    /// Open a Properties request from this frame, then draw the dialog. Enter applies a new
    /// name and closes; Escape or a tap outside closes without renaming.
    pub(in crate::file_explorer) fn properties_dialog(&mut self, ctx: &egui::Context, pane: PaneId, c: &Theme) {
        // The key that opened the dialog (Alt+Enter) mustn't also close it.
        let just_opened = self.pending_properties.is_some();
        if let Some(path) = self.pending_properties.take() {
            // A file here is read right away; one on a host can't be without a trip there.
            self.properties = if self.backend.is_remote() {
                let location = path.parent().map(|p| self.shown_path(p));
                Some(Properties::open_in_background(ctx, self.backend.clone(), self.entry_for(&path), location))
            } else {
                Properties::open(ctx, path)
            };
        }
        let Some(mut props) = self.properties.take() else { return };
        let s = sizes(self.touch, 1.0);
        // The folder being shown can't be renamed from inside itself.
        let editable = props.path != self.dir;
        let changed = editable && props.name.trim() != props.entry.name;
        let (mut confirm, mut cancel) = ctx.input(|i| (i.key_pressed(Key::Enter), i.key_pressed(Key::Escape)));
        if just_opened {
            (confirm, cancel) = (false, false);
        }
        let touch = self.touch;
        // A file on the host can be downloaded to the phone from here too.
        let download = self.entries.iter().position(|e| e.path == props.path).filter(|&i| self.can_download(i));
        let mut download_clicked = false;
        let rows = props.rows();
        let frame = egui::Frame::popup(&ctx.style()).fill(c.card).corner_radius(12).inner_margin(18.0);
        let modal = egui::Modal::new(Id::new(("files-properties", pane))).frame(frame).show(ctx, |ui| {
            ui.set_width(380.0f32.min(ctx.content_rect().width() - 48.0));
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(Vec2::new(40.0, 36.0), Sense::hover());
                let size = if props.entry.is_dir { Vec2::new(36.0, 29.0) } else { Vec2::splat(34.0) };
                paint_icon(ui.painter(), egui::Rect::from_center_size(r.center(), size), &props.entry, 1.0, c, true, false);
                ui.add_space(6.0);
                let field = egui::TextEdit::singleline(&mut props.name)
                    .font(egui::FontId::proportional(s.font + 2.0))
                    .text_color(c.text_bright)
                    .desired_width(f32::INFINITY)
                    .margin(Vec2::new(8.0, 6.0))
                    .interactive(editable);
                ui.add(field);
            });
            ui.add_space(12.0);
            egui::Grid::new(("files-properties-grid", pane)).num_columns(2).spacing([16.0, 7.0]).show(ui, |ui| {
                for (label, value) in &rows {
                    ui.label(RichText::new(*label).size(s.font).color(c.text_dim));
                    ui.add(egui::Label::new(RichText::new(value).size(s.font).color(c.text)).wrap());
                    ui.end_row();
                }
            });
            ui.add_space(16.0);
            let button = |label: &str, fill: Color32, text: Color32| {
                egui::Button::new(RichText::new(label).size(s.font).color(text))
                    .fill(fill)
                    .corner_radius(s.button / 2.0)
                    .min_size(Vec2::new(if touch { 110.0 } else { 88.0 }, s.button))
            };
            // A row just as tall as the buttons (a bare right-to-left layout would take all
            // the height the modal offers, and the modal would grow to fill the window).
            let row = Vec2::new(ui.available_width(), s.button);
            ui.allocate_ui_with_layout(row, egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if changed {
                    confirm |= ui.add(button("Rename", ACCENT, Color32::WHITE)).clicked();
                    ui.add_space(6.0);
                    cancel |= ui.add(button("Cancel", Color32::from_white_alpha(18), c.text_bright)).clicked();
                } else {
                    cancel |= ui.add(button("Close", Color32::from_white_alpha(18), c.text_bright)).clicked();
                    if download.is_some() {
                        ui.add_space(6.0);
                        download_clicked = ui.add(button("Download", ACCENT, Color32::WHITE)).clicked();
                    }
                }
            });
        });
        cancel |= modal.backdrop_response.clicked();
        if cancel {
            return;
        }
        if let (true, Some(i)) = (download_clicked, download) {
            self.download(ctx, i);
            return;
        }
        if confirm {
            if changed {
                let new_name = props.name.trim().to_string();
                match self.rename_item(&props.path, &new_name) {
                    Ok(_) => self.set_status(format!("Renamed to “{new_name}”"), false),
                    Err(e) => {
                        self.set_status(e, true);
                        self.properties = Some(props);
                    }
                }
            }
            return;
        }
        self.properties = Some(props);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn permission_bits() {
        assert_eq!(permissions(0o755), "rwxr-xr-x (755)");
        assert_eq!(permissions(0o640), "rw-r----- (640)");
    }

    #[test]
    fn kinds() {
        assert_eq!(kind("image/png", Some("png")), "PNG image (image/png)");
        assert_eq!(kind("application/pdf", Some("pdf")), "PDF document (application/pdf)");
        assert_eq!(kind("application/zip", None), "Archive (application/zip)");
        assert_eq!(kind("application/octet-stream", None), "File (application/octet-stream)");
    }
}
