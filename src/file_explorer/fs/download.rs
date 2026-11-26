//! Downloading a file from an ssh host to this device (the phone's Downloads, see
//! `platform::downloads_dir`): the file is read on the host with `cat` and streamed straight
//! into a new local file, under a name that never replaces one already there.
//!
//! The pieces are kept apart so each can be tested here: [`local_name`] makes a host's file
//! name safe for the phone's storage, [`numbered`] and [`create_unique`] pick a free name,
//! [`save`] writes a stream into it (and removes what it wrote when the stream fails), and
//! [`download`] ties them to the host.

use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::file_explorer::FileExplorer;
use crate::file_explorer::fs::transfer::Progress;
use crate::remote::exec::{ExecError, Transport, quote};

/// How many numbered names (`a (2).txt` … ) are tried before giving up.
const MAX_TRIES: u32 = 999;

/// The longest name, in bytes, most file systems (and Android's shared storage) take.
const MAX_NAME: usize = 255;

/// `name` (a file name on the host) as a name the phone's shared storage takes: the
/// characters FAT and Android's storage refuse (`"*/:<>?\|`, control characters) become `_`,
/// leading dots and trailing dots or spaces go, and it's cut to [`MAX_NAME`] bytes, keeping
/// the extension. Nothing left gives `download`.
pub fn local_name(name: &str) -> String {
    let cleaned: String = name.chars().map(|c| if c.is_control() || r#""*/:<>?\|"#.contains(c) { '_' } else { c }).collect();
    let trimmed = cleaned.trim_start_matches(['.', ' ']).trim_end_matches(['.', ' ']);
    if trimmed.is_empty() {
        return "download".into();
    }
    let (stem, ext) = split_ext(trimmed);
    if trimmed.len() <= MAX_NAME {
        return trimmed.into();
    }
    // Too long: shorten the stem, at a character boundary, and keep the extension (unless
    // the extension alone is too long).
    let ext = if ext.len() < MAX_NAME / 2 { ext } else { "" };
    let mut end = MAX_NAME - ext.len();
    while !stem.is_char_boundary(end.min(stem.len())) {
        end -= 1;
    }
    format!("{}{ext}", &stem[..end.min(stem.len())])
}

/// `name` split into stem and extension (with its dot) the way `ops::unique_target` does:
/// `a.tar.gz` is `a.tar` + `.gz`, `.bashrc` and `README` have none.
fn split_ext(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 => name.split_at(i),
        _ => (name, ""),
    }
}

/// The `n`th name to try for `name`: `name` itself for 1, then `stem (2).ext`, `(3)`, …
pub fn numbered(name: &str, n: u32) -> String {
    if n <= 1 {
        return name.into();
    }
    let (stem, ext) = split_ext(name);
    format!("{stem} ({n}){ext}")
}

/// Make a new, empty file for `name` in `dir`: `name`, else the first free numbered name.
/// The file is created only if nothing has that name (`create_new`), so an existing file is
/// never replaced, even one this app can't see (another app's, in shared storage).
pub fn create_unique(dir: &Path, name: &str) -> std::io::Result<(PathBuf, File)> {
    for n in 1..=MAX_TRIES {
        let path = dir.join(numbered(name, n));
        match OpenOptions::new().write(true).create_new(true).open(&path) {
            Ok(file) => return Ok((path, file)),
            Err(e) if e.kind() == ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e),
        }
    }
    Err(std::io::Error::new(ErrorKind::AlreadyExists, format!("too many files named like “{name}”")))
}

/// Save a stream into a new file for `name` in `dir` (see [`create_unique`]). `fill` writes
/// the stream through the function it's given; each chunk also counts into `progress`. If
/// `fill` fails, or `progress` is cancelled, the half-written file is removed. Returns the
/// new file's path.
pub fn save(
    dir: &Path,
    name: &str,
    progress: &Progress,
    fill: impl FnOnce(&mut dyn FnMut(&[u8]) -> std::io::Result<()>) -> Result<(), String>,
) -> Result<PathBuf, String> {
    let (path, mut file) = create_unique(dir, name).map_err(|e| format!("Can't save to {}: {e}", dir.display()))?;
    progress.set_current(&path.file_name().unwrap_or_default().to_string_lossy());
    let result = fill(&mut |chunk| {
        file.write_all(chunk)?;
        progress.done.fetch_add(chunk.len() as u64, Ordering::Relaxed);
        Ok(())
    })
    .and_then(|()| file.sync_all().map_err(|e| e.to_string()));
    drop(file);
    match result {
        Ok(()) if !progress.cancelled() => Ok(path),
        Ok(()) => {
            let _ = std::fs::remove_file(&path);
            Err("Download cancelled".into())
        }
        Err(e) => {
            let _ = std::fs::remove_file(&path);
            Err(e)
        }
    }
}

/// The script that prints the file `path` on the host. It fails with 2 for something that
/// isn't a file there and 3 for a file it can't read, before printing anything.
fn read_script(path: &Path) -> String {
    format!("p={}\n[ -f \"$p\" ] || exit 2\n[ -r \"$p\" ] || exit 3\nexec cat -- \"$p\"", quote(path))
}

/// Why [`read_script`] exited with `code`.
fn read_failure(code: i32) -> String {
    match code {
        2 => "It isn't a file on the host any more".into(),
        3 => "Permission denied on the host".into(),
        code => format!("Reading it on the host failed (exit status {code})"),
    }
}

/// Download the file `remote` on the host into `dir` (on this device), under its own name
/// made safe ([`local_name`]) and never replacing a file there. `size` (from the listing)
/// is the progress total. Returns the new file's path.
pub fn download(t: &Transport, remote: &Path, size: u64, dir: &Path, progress: &Progress) -> Result<PathBuf, String> {
    let name = local_name(&remote.file_name().unwrap_or_default().to_string_lossy());
    progress.total.store(size, Ordering::Relaxed);
    save(dir, &name, progress, |sink| match t.stream(&read_script(remote), &progress.cancel, sink) {
        Ok(0) => Ok(()),
        Ok(code) => Err(read_failure(code)),
        // `save` says it was cancelled (and removes the file).
        Err(ExecError::Cancelled) => Ok(()),
        Err(e) => Err(e.to_string()),
    })
}

impl FileExplorer {
    /// Whether the entry at `index` can be downloaded to this device: a file (not a folder)
    /// on an ssh host, where the device has somewhere to put it (the phone).
    pub(in crate::file_explorer) fn can_download(&self, index: usize) -> bool {
        self.download_dir.is_some() && self.backend.is_remote() && self.entries.get(index).is_some_and(|e| !e.is_dir)
    }

    /// Download the file at `index` to this device's Downloads in the background; the action
    /// bar follows it, then says where it went or why it failed.
    pub(in crate::file_explorer) fn download(&mut self, ctx: &egui::Context, index: usize) {
        if !self.can_download(index) {
            return;
        }
        let (Some(dir), Some(t), Some(entry)) = (self.download_dir.clone(), self.transport().cloned(), self.entries.get(index).cloned()) else {
            return;
        };
        let label = format!("Downloading “{}”", entry.name);
        self.spawn_job(ctx, label, true, move |progress| {
            let saved = download(&t, &entry.path, entry.size.unwrap_or(0), &dir, progress)?;
            Ok(format!("Saved “{}” to Downloads", saved.file_name().unwrap_or_default().to_string_lossy()))
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str) -> std::process::Output {
        std::process::Command::new("sh").arg("-c").arg(script).output().unwrap()
    }

    #[test]
    fn names_safe_for_the_phone() {
        assert_eq!(local_name("report.pdf"), "report.pdf");
        assert_eq!(local_name("a:b?c*.txt"), "a_b_c_.txt");
        assert_eq!(local_name("tab\there"), "tab_here");
        assert_eq!(local_name(".bashrc"), "bashrc");
        assert_eq!(local_name("trailing. "), "trailing");
        assert_eq!(local_name("..."), "download");
        assert_eq!(local_name(""), "download");
        assert_eq!(local_name("café ☕.md"), "café ☕.md");
    }

    #[test]
    fn long_names_keep_their_extension() {
        let long = format!("{}.tar.gz", "é".repeat(200));
        let name = local_name(&long);
        assert!(name.len() <= MAX_NAME, "{}", name.len());
        // The stem is cut at a character boundary; the last extension stays.
        assert!(name.ends_with("é.gz"), "{name}");
        let no_ext = "x".repeat(300);
        assert_eq!(local_name(&no_ext).len(), MAX_NAME);
    }

    #[test]
    fn numbered_names_match_the_explorer_s() {
        assert_eq!(numbered("a.txt", 1), "a.txt");
        assert_eq!(numbered("a.txt", 2), "a (2).txt");
        assert_eq!(numbered("archive.tar.gz", 3), "archive.tar (3).gz");
        assert_eq!(numbered("README", 2), "README (2)");
        assert_eq!(numbered(".bashrc", 2), ".bashrc (2)");
        let dir = crate::file_explorer::tests::fixture("download-numbered");
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        for name in ["a.txt", "new.txt", "archive.tar.gz"] {
            let ours = (1..).map(|n| dir.join(numbered(name, n))).find(|p| !p.exists()).unwrap();
            assert_eq!(ours, crate::file_explorer::fs::ops::unique_target(&dir, std::ffi::OsStr::new(name)), "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn never_replaces_a_file() {
        let dir = crate::file_explorer::tests::fixture("download-unique");
        std::fs::write(dir.join("a.txt"), b"keep").unwrap();
        std::fs::write(dir.join("a (2).txt"), b"keep too").unwrap();
        let (path, _) = create_unique(&dir, "a.txt").unwrap();
        assert_eq!(path, dir.join("a (3).txt"));
        let (path, _) = create_unique(&dir, "b.txt").unwrap();
        assert_eq!(path, dir.join("b.txt"));
        assert_eq!(std::fs::read(dir.join("a.txt")).unwrap(), b"keep");
        assert_eq!(std::fs::read(dir.join("a (2).txt")).unwrap(), b"keep too");
        assert!(create_unique(&dir.join("missing"), "c.txt").is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn saves_a_stream_and_counts_it() {
        let dir = crate::file_explorer::tests::fixture("download-save");
        let progress = Progress::default();
        let path = save(&dir, "data.bin", &progress, |sink| {
            sink(b"hello ").map_err(|e| e.to_string())?;
            sink(b"world").map_err(|e| e.to_string())
        })
        .unwrap();
        assert_eq!(path, dir.join("data.bin"));
        assert_eq!(std::fs::read(&path).unwrap(), b"hello world");
        assert_eq!(progress.done.load(Ordering::Relaxed), 11);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn a_failed_or_cancelled_save_leaves_nothing() {
        let dir = crate::file_explorer::tests::fixture("download-fail");
        let progress = Progress::default();
        let err = save(&dir, "half.bin", &progress, |sink| {
            sink(b"half").map_err(|e| e.to_string())?;
            Err("The connection to the host ended".into())
        });
        assert_eq!(err, Err("The connection to the host ended".into()));
        let cancelled = Progress::default();
        cancelled.cancel.store(true, Ordering::Relaxed);
        assert_eq!(save(&dir, "half.bin", &cancelled, |_| Ok(())), Err("Download cancelled".into()));
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
        assert!(!left.iter().any(|n| n.to_string_lossy().starts_with("half")), "{left:?}");
        std::fs::remove_dir_all(dir).unwrap();
    }

    /// The host's side, run here by `sh`: a file comes out byte for byte, and what isn't one
    /// (or can't be read) fails with its own status before printing anything.
    #[test]
    fn reads_on_the_host() {
        let dir = crate::file_explorer::tests::fixture("download-read");
        let file = dir.join("it's here.bin");
        std::fs::write(&file, b"a\0b\nc").unwrap();
        let out = sh(&read_script(&file));
        assert_eq!((out.status.code(), out.stdout), (Some(0), b"a\0b\nc".to_vec()));
        let out = sh(&read_script(&dir));
        assert_eq!((out.status.code(), out.stdout.len()), (Some(2), 0));
        let out = sh(&read_script(&dir.join("gone")));
        assert_eq!(out.status.code(), Some(2));
        assert_eq!(read_failure(2), "It isn't a file on the host any more");
        assert_eq!(read_failure(3), "Permission denied on the host");
        assert_eq!(read_failure(1), "Reading it on the host failed (exit status 1)");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
