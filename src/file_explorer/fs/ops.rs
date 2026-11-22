//! File system work with no UI: reading folders, copying, moving, renaming, creating and
//! deleting files, and turning paths into text (sizes, counts, file:// URIs) and back.

use std::ffi::OsStr;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::terminal::tilde;

use crate::file_explorer::Entry;
use crate::file_explorer::fs::transfer::{Op, Progress};

/// Files are copied in chunks this big, so a copy can report progress and be cancelled.
const CHUNK: usize = 1 << 20;

pub(in crate::file_explorer) fn count(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

/// Copy or move `sources` into the folder `dest`. Name clashes get a " (2)" style suffix
/// instead of overwriting anything. Returns how many items were copied or moved; when
/// `progress` is cancelled it stops early, removes the half-copied item and returns the
/// count so far.
pub fn run_op(op: Op, sources: &[PathBuf], dest: &Path, progress: &Progress) -> Result<usize, String> {
    let dest_real = dest.canonicalize().map_err(|e| format!("{}: {e}", tilde(dest)))?;
    progress.total.store(sources.iter().map(|s| tree_size(s)).sum(), Ordering::Relaxed);
    let mut done = 0;
    for src in sources {
        let Some(name) = src.file_name() else { continue };
        let fail = |e: std::io::Error| format!("{}: {e}", name.to_string_lossy());
        let src_meta = std::fs::symlink_metadata(src).map_err(fail)?;
        let src_real = src.canonicalize().map_err(fail)?;
        if src_meta.is_dir() && dest_real.starts_with(&src_real) {
            return Err(format!("Can't put “{}” inside itself", name.to_string_lossy()));
        }
        let same_folder = src.parent().and_then(|p| p.canonicalize().ok()).as_deref() == Some(dest_real.as_path());
        if op == Op::Move && same_folder {
            progress.done.fetch_add(tree_size(src), Ordering::Relaxed);
            continue;
        }
        let target = unique_target(dest, name);
        let result = match op {
            Op::Copy => copy_recursive(src, &target, progress),
            Op::Move => move_path(src, &target, progress),
        };
        match result {
            Err(_) if progress.cancelled() => {
                // A copy that was cut short (also the copy half of a move across disks).
                let _ = remove_any(&target);
                return Ok(done);
            }
            r => r.map_err(fail)?,
        }
        done += 1;
    }
    Ok(done)
}

/// Permanently delete `paths` (folders with everything in them). Symlinks are removed, not
/// followed. Stops at the first failure, or between items once `progress` is cancelled.
/// Returns how many items were deleted.
pub fn delete_paths(paths: &[PathBuf], progress: &Progress) -> Result<usize, String> {
    progress.total.store(paths.len() as u64, Ordering::Relaxed);
    let mut done = 0;
    for path in paths {
        if progress.cancelled() {
            break;
        }
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| tilde(path));
        progress.set_current(&name);
        remove_any(path).map_err(|e| format!("{name}: {e}"))?;
        done += 1;
        progress.done.fetch_add(1, Ordering::Relaxed);
    }
    Ok(done)
}

fn remove_any(path: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(path)?;
    if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }
}

/// Bytes of the files in `path` (itself, or everything inside a folder). Links count as
/// nothing and aren't followed.
pub(in crate::file_explorer) fn tree_size(path: &Path) -> u64 {
    tree_summary(path, &mut |_, _| true).0
}

/// `(bytes, items)` inside `path`, not counting `path` itself. `keep_going` sees the running
/// totals every so often and can stop the walk by returning false.
pub(in crate::file_explorer) fn tree_summary(path: &Path, keep_going: &mut dyn FnMut(u64, usize) -> bool) -> (u64, usize) {
    let Ok(meta) = std::fs::symlink_metadata(path) else { return (0, 0) };
    if !meta.is_dir() {
        return (if meta.is_file() { meta.len() } else { 0 }, 0);
    }
    let (mut bytes, mut items) = (0, 0);
    let mut stack = vec![path.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.filter_map(Result::ok) {
            items += 1;
            match e.metadata() {
                Ok(m) if m.is_dir() => stack.push(e.path()),
                Ok(m) if m.is_file() => bytes += m.len(),
                _ => {}
            }
            if items % 512 == 0 && !keep_going(bytes, items) {
                return (bytes, items);
            }
        }
    }
    (bytes, items)
}

/// `dest/name`, or `dest/name (2).ext`, `(3)`, … if that's taken.
pub(in crate::file_explorer) fn unique_target(dest: &Path, name: &OsStr) -> PathBuf {
    let first = dest.join(name);
    if std::fs::symlink_metadata(&first).is_err() {
        return first;
    }
    let name = Path::new(name);
    let stem = name.file_stem().unwrap_or(name.as_os_str()).to_string_lossy();
    let ext = name.extension().map(|e| format!(".{}", e.to_string_lossy())).unwrap_or_default();
    (2..)
        .map(|n| dest.join(format!("{stem} ({n}){ext}")))
        .find(|p| std::fs::symlink_metadata(p).is_err())
        .unwrap_or(first)
}

pub(in crate::file_explorer) fn copy_recursive(src: &Path, dst: &Path, progress: &Progress) -> std::io::Result<()> {
    progress.check()?;
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        crate::platform::symlink(&std::fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()), progress)?;
        }
        std::fs::set_permissions(dst, meta.permissions())
    } else {
        copy_file(src, dst, &meta, progress)
    }
}

/// Copy one file's bytes chunk by chunk, counting them into `progress`.
fn copy_file(src: &Path, dst: &Path, meta: &std::fs::Metadata, progress: &Progress) -> std::io::Result<()> {
    progress.set_current(&src.file_name().unwrap_or_default().to_string_lossy());
    let mut from = std::fs::File::open(src)?;
    let mut to = std::fs::OpenOptions::new().write(true).create_new(true).open(dst)?;
    let mut buf = vec![0; CHUNK];
    loop {
        if let Err(e) = progress.check() {
            drop(to);
            let _ = std::fs::remove_file(dst);
            return Err(e);
        }
        let n = match from.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(e),
        };
        to.write_all(&buf[..n])?;
        progress.done.fetch_add(n as u64, Ordering::Relaxed);
    }
    std::fs::set_permissions(dst, meta.permissions())
}

pub(in crate::file_explorer) fn move_path(src: &Path, dst: &Path, progress: &Progress) -> std::io::Result<()> {
    let size = tree_size(src);
    progress.set_current(&src.file_name().unwrap_or_default().to_string_lossy());
    match std::fs::rename(src, dst) {
        Ok(()) => {
            progress.done.fetch_add(size, Ordering::Relaxed);
            Ok(())
        }
        // Across filesystems a rename isn't possible: copy, then remove the original.
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_recursive(src, dst, progress)?;
            remove_any(src)
        }
        Err(e) => Err(e),
    }
}

/// Why `name` can't be a file name, if it can't.
pub(in crate::file_explorer) fn invalid_name(name: &str) -> Option<&'static str> {
    if name.trim().is_empty() {
        Some("A name can't be empty")
    } else if name == "." || name == ".." {
        Some("That name is reserved")
    } else if name.contains('/') || name.contains('\0') {
        Some("A name can't contain “/”")
    } else {
        None
    }
}

/// Rename `path` to `new_name` in the same folder. Never replaces an existing file.
pub(in crate::file_explorer) fn rename_path(path: &Path, new_name: &str) -> Result<PathBuf, String> {
    if let Some(why) = invalid_name(new_name) {
        return Err(why.into());
    }
    let target = path.with_file_name(new_name);
    if target == path {
        return Ok(target);
    }
    // A change of case only is the same file on case-insensitive disks (macOS).
    let same_file = path.file_name().is_some_and(|old| old.to_string_lossy().to_lowercase() == new_name.to_lowercase());
    if !same_file && std::fs::symlink_metadata(&target).is_ok() {
        return Err(format!("“{new_name}” already exists"));
    }
    std::fs::rename(path, &target).map_err(|e| format!("Can't rename: {e}"))?;
    Ok(target)
}

/// Make an empty folder ("New Folder", "New Folder (2)", …) or file ("Untitled", …) in
/// `dest`. Returns its path.
pub(in crate::file_explorer) fn create_new(dest: &Path, folder: bool) -> Result<PathBuf, String> {
    let target = unique_target(dest, OsStr::new(if folder { "New Folder" } else { "Untitled" }));
    let result = if folder {
        std::fs::create_dir(&target)
    } else {
        std::fs::OpenOptions::new().write(true).create_new(true).open(&target).map(|_| ())
    };
    result.map(|_| target).map_err(|e| format!("Can't create it here: {e}"))
}

/// A typed location as a path: `~` is the home folder and relative paths start at `cwd`.
pub(in crate::file_explorer) fn expand_location(text: &str, cwd: &Path) -> PathBuf {
    let text = text.trim();
    let home = || crate::platform::home_dir().unwrap_or_else(|| PathBuf::from("/"));
    let path = if text == "~" {
        home()
    } else if let Some(rest) = text.strip_prefix("~/") {
        home().join(rest)
    } else if let Some(rest) = text.strip_prefix("file://") {
        PathBuf::from(percent_decode(rest))
    } else {
        cwd.join(text)
    };
    // `..` and `.` resolve against the real folders when they exist.
    path.canonicalize().unwrap_or(path)
}

/// Tab completion for a typed location: extend the last part as far as all matches agree,
/// adding a `/` after a single folder. `None` when there's nothing to add.
pub(in crate::file_explorer) fn complete_location(text: &str, cwd: &Path) -> Option<String> {
    if text == "~" {
        return Some("~/".into());
    }
    let (head, prefix) = match text.rfind('/') {
        Some(i) => (&text[..=i], &text[i + 1..]),
        None => ("", text),
    };
    let base = if head.is_empty() { cwd.to_path_buf() } else { expand_location(head, cwd) };
    let matches: Vec<(String, bool)> = std::fs::read_dir(base)
        .ok()?
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let shown = prefix.starts_with('.') || !name.starts_with('.');
            (shown && name.starts_with(prefix)).then(|| (name, e.path().is_dir()))
        })
        .collect();
    let first = &matches.first()?.0;
    let common = matches.iter().fold(first.as_str(), |acc, (name, _)| {
        let n = acc.char_indices().zip(name.chars()).take_while(|((_, a), b)| a == b).last().map_or(0, |((i, c), _)| i + c.len_utf8());
        &acc[..n]
    });
    let mut out = format!("{head}{common}");
    if matches.len() == 1 && matches[0].1 {
        out.push('/');
    }
    (out != text).then_some(out)
}

/// Absolute paths (or file:// URIs), one per line, that exist.
pub fn parse_paths(text: &str) -> Vec<PathBuf> {
    text.lines()
        .map(str::trim)
        .filter_map(|l| {
            let p = l.strip_prefix("file://").map(percent_decode).unwrap_or_else(|| l.to_string());
            let p = PathBuf::from(p);
            (p.is_absolute() && std::fs::symlink_metadata(&p).is_ok()).then_some(p)
        })
        .collect()
}

pub fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len()
            && let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// List `dir`: folders first, then files, each sorted case-insensitively by name.
pub(in crate::file_explorer) fn read_entries(dir: &Path, show_hidden: bool) -> std::io::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|de| {
            let name = de.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                return None;
            }
            let path = de.path();
            let is_symlink = de.file_type().is_ok_and(|t| t.is_symlink());
            // Follow symlinks so a link to a folder behaves like a folder.
            let meta = std::fs::metadata(&path).or_else(|_| de.metadata()).ok();
            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            let executable = meta.as_ref().is_some_and(crate::platform::is_executable);
            let size = meta.filter(|m| !m.is_dir()).map(|m| m.len());
            Some(Entry { name, path, is_dir, size, is_symlink, executable })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

pub(in crate::file_explorer) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else if v < 10.0 { format!("{v:.1} {}", UNITS[unit]) } else { format!("{v:.0} {}", UNITS[unit]) }
}

/// `1234567` → `1,234,567`.
pub(in crate::file_explorer) fn thousands(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::with_capacity(s.len() + s.len() / 3);
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_explorer::tests::fixture;

    fn run(op: Op, sources: &[PathBuf], dest: &Path) -> Result<usize, String> {
        run_op(op, sources, dest, &Progress::default())
    }

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 K");
        assert_eq!(human_size(50 * 1024 * 1024), "50 M");
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1234567), "1,234,567");
    }

    #[test]
    fn copy_renames_on_clash_and_recurses() {
        let dir = fixture("copy");
        std::fs::write(dir.join("beta/inner.txt"), "in").unwrap();
        // Copying into the same folder makes a numbered duplicate.
        assert_eq!(run(Op::Copy, &[dir.join("Apple.md")], &dir), Ok(1));
        assert_eq!(std::fs::read_to_string(dir.join("Apple (2).md")).unwrap(), "apple");
        // Folders are copied with their contents.
        assert_eq!(run(Op::Copy, &[dir.join("beta")], &dir.join("Alpha")), Ok(1));
        assert_eq!(std::fs::read_to_string(dir.join("Alpha/beta/inner.txt")).unwrap(), "in");
        assert!(dir.join("beta/inner.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn copy_reports_progress_and_cancel_cleans_up() {
        let dir = fixture("progress");
        std::fs::write(dir.join("beta/inner.txt"), "inner").unwrap();
        let p = Progress::default();
        assert_eq!(run_op(Op::Copy, &[dir.join("Apple.md"), dir.join("beta")], &dir.join("Alpha"), &p), Ok(2));
        assert_eq!(p.total.load(Ordering::Relaxed), 10);
        assert_eq!(p.done.load(Ordering::Relaxed), 10);
        // Cancelled before it starts: nothing is copied and no partial file is left.
        let p = Progress::default();
        p.cancel.store(true, Ordering::Relaxed);
        assert_eq!(run_op(Op::Copy, &[dir.join("zeta.txt")], &dir.join("beta"), &p), Ok(0));
        assert!(!dir.join("beta/zeta.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn move_skips_same_folder_and_refuses_self_nesting() {
        let dir = fixture("move");
        assert_eq!(run(Op::Move, &[dir.join("zeta.txt")], &dir), Ok(0));
        assert_eq!(run(Op::Move, &[dir.join("zeta.txt"), dir.join("Apple.md")], &dir.join("beta")), Ok(2));
        assert!(dir.join("beta/zeta.txt").exists() && !dir.join("zeta.txt").exists());
        std::fs::create_dir(dir.join("beta/sub")).unwrap();
        assert!(run(Op::Move, &[dir.join("beta")], &dir.join("beta/sub")).is_err());
        assert!(dir.join("beta").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn delete_removes_files_folders_and_links_only() {
        let dir = fixture("delete");
        std::fs::write(dir.join("beta/inner.txt"), "in").unwrap();
        crate::platform::symlink(&dir.join("Alpha"), &dir.join("link")).unwrap();
        let p = Progress::default();
        assert_eq!(delete_paths(&[dir.join("zeta.txt"), dir.join("beta"), dir.join("link")], &p), Ok(3));
        assert_eq!(p.done.load(Ordering::Relaxed), 3);
        assert!(!dir.join("zeta.txt").exists() && !dir.join("beta").exists());
        assert!(std::fs::symlink_metadata(dir.join("link")).is_err());
        // The link's target is untouched.
        assert!(dir.join("Alpha").is_dir());
        assert!(delete_paths(&[dir.join("missing")], &Progress::default()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn rename_checks_names_and_never_overwrites() {
        let dir = fixture("rename");
        assert_eq!(rename_path(&dir.join("zeta.txt"), "omega.txt"), Ok(dir.join("omega.txt")));
        assert!(dir.join("omega.txt").exists() && !dir.join("zeta.txt").exists());
        assert!(rename_path(&dir.join("omega.txt"), "Apple.md").is_err());
        assert!(rename_path(&dir.join("omega.txt"), "a/b").is_err());
        assert!(rename_path(&dir.join("omega.txt"), "  ").is_err());
        assert!(rename_path(&dir.join("omega.txt"), "..").is_err());
        assert!(dir.join("omega.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn create_new_picks_free_names() {
        let dir = fixture("create");
        assert_eq!(create_new(&dir, true), Ok(dir.join("New Folder")));
        assert_eq!(create_new(&dir, true), Ok(dir.join("New Folder (2)")));
        assert_eq!(create_new(&dir, false), Ok(dir.join("Untitled")));
        assert!(dir.join("New Folder (2)").is_dir() && dir.join("Untitled").is_file());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn locations_expand_and_complete() {
        let dir = fixture("location");
        let real = dir.canonicalize().unwrap();
        assert_eq!(expand_location("beta", &dir), real.join("beta"));
        assert_eq!(expand_location("beta/..", &dir), real);
        assert_eq!(complete_location("be", &dir).as_deref(), Some("beta/"));
        // "Alpha" and "Apple.md" share only "A".
        assert_eq!(complete_location("A", &dir), None);
        assert_eq!(complete_location("Ap", &dir).as_deref(), Some("Apple.md"));
        // Hidden names only complete when asked for.
        assert_eq!(complete_location(".h", &dir).as_deref(), Some(".hidden_dir/"));
        let abs = format!("{}/ze", dir.display());
        assert_eq!(complete_location(&abs, &dir), Some(format!("{}/zeta.txt", dir.display())));
        assert_eq!(complete_location("~", &dir).as_deref(), Some("~/"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn parses_paths() {
        let dir = fixture("parse");
        let uri = format!("file://{}", dir.join("Apple.md").display()).replace(' ', "%20");
        assert_eq!(parse_paths(&format!("{uri}\nrelative/path\n/definitely/missing")), [dir.join("Apple.md")]);
        assert_eq!(percent_decode("/a%20b/c%2"), "/a b/c%2");
        std::fs::remove_dir_all(dir).unwrap();
    }
}
