//! File system work with no UI: reading folders, copying and moving files, and turning
//! paths into text (sizes, counts, file:// URIs) and back.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};

use crate::terminal::tilde;

use super::Entry;
use super::transfer::Op;

pub(super) fn count(n: usize) -> String {
    if n == 1 { "1 item".into() } else { format!("{n} items") }
}

/// Copy or move `sources` into the folder `dest`. Name clashes get a " (2)" style suffix
/// instead of overwriting anything. Returns how many items were copied or moved.
pub fn run_op(op: Op, sources: &[PathBuf], dest: &Path) -> Result<usize, String> {
    let dest_real = dest.canonicalize().map_err(|e| format!("{}: {e}", tilde(dest)))?;
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
            continue;
        }
        let target = unique_target(dest, name);
        match op {
            Op::Copy => copy_recursive(src, &target),
            Op::Move => move_path(src, &target),
        }
        .map_err(fail)?;
        done += 1;
    }
    Ok(done)
}

/// Permanently delete `paths` (folders with everything in them). Symlinks are removed, not
/// followed. Stops at the first failure. Returns how many items were deleted.
pub fn delete_paths(paths: &[PathBuf]) -> Result<usize, String> {
    let mut done = 0;
    for path in paths {
        let name = path.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_else(|| tilde(path));
        let fail = |e: std::io::Error| format!("{name}: {e}");
        let meta = std::fs::symlink_metadata(path).map_err(fail)?;
        if meta.is_dir() { std::fs::remove_dir_all(path) } else { std::fs::remove_file(path) }.map_err(fail)?;
        done += 1;
    }
    Ok(done)
}

/// `dest/name`, or `dest/name (2).ext`, `(3)`, … if that's taken.
pub(super) fn unique_target(dest: &Path, name: &OsStr) -> PathBuf {
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

pub(super) fn copy_recursive(src: &Path, dst: &Path) -> std::io::Result<()> {
    let meta = std::fs::symlink_metadata(src)?;
    if meta.file_type().is_symlink() {
        crate::platform::symlink(&std::fs::read_link(src)?, dst)
    } else if meta.is_dir() {
        std::fs::create_dir(dst)?;
        for entry in std::fs::read_dir(src)? {
            let entry = entry?;
            copy_recursive(&entry.path(), &dst.join(entry.file_name()))?;
        }
        std::fs::set_permissions(dst, meta.permissions())
    } else {
        std::fs::copy(src, dst).map(|_| ())
    }
}

pub(super) fn move_path(src: &Path, dst: &Path) -> std::io::Result<()> {
    match std::fs::rename(src, dst) {
        Ok(()) => Ok(()),
        // Across filesystems a rename isn't possible: copy, then remove the original.
        Err(e) if e.kind() == std::io::ErrorKind::CrossesDevices => {
            copy_recursive(src, dst)?;
            let meta = std::fs::symlink_metadata(src)?;
            if meta.is_dir() { std::fs::remove_dir_all(src) } else { std::fs::remove_file(src) }
        }
        Err(e) => Err(e),
    }
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

pub(super) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' && i + 2 < bytes.len() {
            if let Some(b) = std::str::from_utf8(&bytes[i + 1..i + 3]).ok().and_then(|h| u8::from_str_radix(h, 16).ok()) {
                out.push(b);
                i += 3;
                continue;
            }
        }
        out.push(bytes[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// List `dir`: folders first, then files, each sorted case-insensitively by name.
pub(super) fn read_entries(dir: &Path, show_hidden: bool) -> std::io::Result<Vec<Entry>> {
    let mut entries: Vec<Entry> = std::fs::read_dir(dir)?
        .filter_map(Result::ok)
        .filter_map(|de| {
            let name = de.file_name().to_string_lossy().to_string();
            if !show_hidden && name.starts_with('.') {
                return None;
            }
            let path = de.path();
            // Follow symlinks so a link to a folder behaves like a folder.
            let meta = std::fs::metadata(&path).or_else(|_| de.metadata()).ok();
            let is_dir = meta.as_ref().is_some_and(|m| m.is_dir());
            let size = meta.filter(|m| !m.is_dir()).map(|m| m.len());
            Some(Entry { name, path, is_dir, size })
        })
        .collect();
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    Ok(entries)
}

pub(super) fn human_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "K", "M", "G", "T"];
    let mut v = bytes as f64;
    let mut unit = 0;
    while v >= 1024.0 && unit < UNITS.len() - 1 {
        v /= 1024.0;
        unit += 1;
    }
    if unit == 0 { format!("{bytes} B") } else if v < 10.0 { format!("{v:.1} {}", UNITS[unit]) } else { format!("{v:.0} {}", UNITS[unit]) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::file_explorer::tests::fixture;

    #[test]
    fn sizes() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(1536), "1.5 K");
        assert_eq!(human_size(50 * 1024 * 1024), "50 M");
    }

    #[test]
    fn copy_renames_on_clash_and_recurses() {
        let dir = fixture("copy");
        std::fs::write(dir.join("beta/inner.txt"), "in").unwrap();
        // Copying into the same folder makes a numbered duplicate.
        assert_eq!(run_op(Op::Copy, &[dir.join("Apple.md")], &dir), Ok(1));
        assert_eq!(std::fs::read_to_string(dir.join("Apple (2).md")).unwrap(), "apple");
        // Folders are copied with their contents.
        assert_eq!(run_op(Op::Copy, &[dir.join("beta")], &dir.join("Alpha")), Ok(1));
        assert_eq!(std::fs::read_to_string(dir.join("Alpha/beta/inner.txt")).unwrap(), "in");
        assert!(dir.join("beta/inner.txt").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn move_skips_same_folder_and_refuses_self_nesting() {
        let dir = fixture("move");
        assert_eq!(run_op(Op::Move, &[dir.join("zeta.txt")], &dir), Ok(0));
        assert_eq!(run_op(Op::Move, &[dir.join("zeta.txt"), dir.join("Apple.md")], &dir.join("beta")), Ok(2));
        assert!(dir.join("beta/zeta.txt").exists() && !dir.join("zeta.txt").exists());
        std::fs::create_dir(dir.join("beta/sub")).unwrap();
        assert!(run_op(Op::Move, &[dir.join("beta")], &dir.join("beta/sub")).is_err());
        assert!(dir.join("beta").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn delete_removes_files_folders_and_links_only() {
        let dir = fixture("delete");
        std::fs::write(dir.join("beta/inner.txt"), "in").unwrap();
        crate::platform::symlink(&dir.join("Alpha"), &dir.join("link")).unwrap();
        assert_eq!(delete_paths(&[dir.join("zeta.txt"), dir.join("beta"), dir.join("link")]), Ok(3));
        assert!(!dir.join("zeta.txt").exists() && !dir.join("beta").exists());
        assert!(std::fs::symlink_metadata(dir.join("link")).is_err());
        // The link's target is untouched.
        assert!(dir.join("Alpha").is_dir());
        assert!(delete_paths(&[dir.join("missing")]).is_err());
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
