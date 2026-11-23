//! Files dropped or pasted into a pane running `ssh`, sent to the remote host.
//!
//! A path typed into an ssh session names a file on *this* machine, which the remote side
//! can't open (and a remote program such as Claude Code can't read this machine's clipboard
//! either). So when a pane's foreground process is `ssh`, nex copies the files over a second
//! ssh connection to the same host (see the module above), into a fresh folder under the
//! remote `$TMPDIR` (or `/tmp`), and types the *remote* paths instead. When it can't log in,
//! the caller falls back to the local paths.
//!
//! The files travel as one `tar` stream (so folders work too, and symlinks are followed),
//! unpacked by `sh` on the remote side, whatever the login shell is. The stream is gzipped
//! (quickly) when the files are mostly ones that shrink, such as text and source code; a
//! pasted screenshot or other already-compressed files go as they are.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use crate::remote::error::last_line;
use crate::remote::{RemoteError, SshTarget, posix_quote};

/// Make a folder for the upload, unpack the stream into it, and print its path.
const REMOTE_SCRIPT: &str = r#"d=$(mktemp -d "${TMPDIR:-/tmp}/nex.XXXXXX") && tar xf - -C "$d" && printf '%s\n' "$d""#;

/// [`REMOTE_SCRIPT`] for a gzipped stream (GNU tar, bsdtar and busybox's all read `z`).
fn remote_script(gzip: bool) -> String {
    if gzip { REMOTE_SCRIPT.replace("tar xf", "tar xzf") } else { REMOTE_SCRIPT.to_string() }
}

/// Below this many bytes that would shrink, compressing isn't worth it.
const GZIP_MIN: u64 = 64 * 1024;
/// Files counted before deciding, so a huge folder doesn't hold up the upload.
const SCAN_MAX: usize = 5000;

/// Extensions of files that are compressed already: gzip would only spend time on them.
const COMPRESSED: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "heic", "heif", "avif", "zip", "gz", "tgz", "xz", "zst", "bz2", "7z", "rar", "jar", "mp4",
    "mov", "mkv", "webm", "mp3", "m4a", "aac", "ogg", "opus", "flac", "pdf", "docx", "xlsx", "pptx", "woff", "woff2", "dmg", "deb",
];

/// Whether `paths` (files or folders) are worth gzipping on the way: at least [`GZIP_MIN`]
/// bytes that would shrink, and at least half of all of them.
fn worth_gzipping(paths: &[PathBuf]) -> bool {
    let (mut shrinks, mut total, mut seen) = (0u64, 0u64, 0usize);
    let mut stack: Vec<PathBuf> = paths.to_vec();
    while let Some(p) = stack.pop() {
        if seen >= SCAN_MAX {
            break;
        }
        seen += 1;
        // Following symlinks, as tar does (`-h`).
        let Ok(meta) = std::fs::metadata(&p) else { continue };
        if meta.is_dir() {
            if let Ok(entries) = std::fs::read_dir(&p) {
                stack.extend(entries.flatten().map(|e| e.path()));
            }
            continue;
        }
        total += meta.len();
        let ext = p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
        if !COMPRESSED.contains(&ext.as_str()) {
            shrinks += meta.len();
        }
    }
    shrinks >= GZIP_MIN && shrinks * 2 >= total
}

/// Copy `paths` (files or folders) to a new folder on the remote host. Returns each one's
/// remote path, quoted for a shell. Blocks until done: run it on a worker thread. It fails
/// with [`RemoteError::NeedsLogin`] when the host wants a password (or another prompt) the
/// upload can't answer.
pub fn upload(target: &SshTarget, paths: &[PathBuf]) -> Result<Vec<String>, RemoteError> {
    let mut tar = Command::new("tar");
    // macOS's tar would otherwise add `._name` files for extended attributes.
    tar.env("COPYFILE_DISABLE", "1").arg("-chf").arg("-");
    let mut names = Vec::new();
    for p in paths {
        let (Some(parent), Some(name)) = (p.parent(), p.file_name()) else { continue };
        tar.arg("-C").arg(parent).arg(name);
        names.push(name.to_string_lossy().into_owned());
    }
    if names.is_empty() {
        return Err("nothing to upload".into());
    }
    let mut tar = tar
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run tar: {e}"))?;
    let mut stream = tar.stdout.take().ok_or("couldn't run tar")?;
    let gzip = worth_gzipping(paths);
    let mut ssh = target.batch_command(&remote_script(gzip));
    ssh.stdout(Stdio::piped()).stderr(Stdio::piped());
    let (ssh, pump) = if gzip {
        let mut ssh = ssh.stdin(Stdio::piped()).spawn().map_err(|e| format!("couldn't run ssh: {e}"))?;
        let pipe = ssh.stdin.take().ok_or("couldn't run ssh")?;
        // Compressed on the way, here rather than by a `gzip` that may not be installed.
        let pump = std::thread::spawn(move || -> std::io::Result<()> {
            let mut gz = flate2::write::GzEncoder::new(pipe, flate2::Compression::fast());
            std::io::copy(&mut stream, &mut gz)?;
            gz.finish()?.flush()
        });
        (ssh, Some(pump))
    } else {
        (ssh.stdin(stream).spawn().map_err(|e| format!("couldn't run ssh: {e}"))?, None)
    };
    let out = ssh.wait_with_output().map_err(|e| e.to_string())?;
    // A failed send shows as ssh's or tar's error below.
    if let Some(pump) = pump {
        let _ = pump.join();
    }
    let mut tar_err = String::new();
    if let Some(mut e) = tar.stderr.take() {
        let _ = e.read_to_string(&mut tar_err);
    }
    let tar_ok = tar.wait().is_ok_and(|s| s.success());

    if let Some(failure) = target.batch_failure(&out) {
        return Err(failure);
    }
    if !tar_ok {
        return Err(last_line(&tar_err).unwrap_or_else(|| "tar failed".into()).into());
    }
    let dir = String::from_utf8_lossy(&out.stdout).lines().next_back().map(str::trim).filter(|d| d.starts_with('/')).map(str::to_string);
    let dir = dir.ok_or("the remote host didn't say where it put the files")?;
    Ok(names.iter().map(|n| posix_quote(&format!("{dir}/{n}"))).collect())
}


/// Save clipboard image bytes to a local temp file, named like a screenshot, to upload.
/// The caller removes its folder once the upload is done.
pub fn save_png(png: &[u8]) -> std::io::Result<PathBuf> {
    save_image(png, "pasted", "png")
}

/// Save image bytes to a fresh temp folder as `<prefix>-<local time>.<ext>`
/// ("pasted-2026-11-17-1405.png"), for a paste or a drop that carried an image but no file.
pub fn save_image(bytes: &[u8], prefix: &str, ext: &str) -> std::io::Result<PathBuf> {
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default();
    let dir = std::env::temp_dir().join(format!("nex-paste-{}-{}", std::process::id(), stamp.as_nanos()));
    std::fs::create_dir_all(&dir)?;
    let name = match crate::platform::format_local_time(std::time::SystemTime::now()) {
        Some(t) => format!("{prefix}-{}.{ext}", t.replace(' ', "-").replace(':', "")),
        None => format!("{prefix}-{}.{ext}", stamp.as_secs()),
    };
    let path = dir.join(name);
    std::fs::write(&path, bytes)?;
    Ok(path)
}

/// Remove a folder made by [`save_png`].
pub fn remove_saved(path: &Path) {
    if let Some(dir) = path.parent().filter(|d| d.file_name().is_some_and(|n| n.to_string_lossy().starts_with("nex-paste-"))) {
        let _ = std::fs::remove_dir_all(dir);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn upload_command() {
        let argv = "ssh -p 2222 -t me@host htop".split_whitespace().map(String::from).collect::<Vec<_>>();
        let args = SshTarget::from_argv(&argv).unwrap().batch_args(&remote_script(false));
        let tail = &args[args.len() - 5..];
        assert_eq!(&tail[..4], ["-p", "2222", "--", "me@host"]);
        assert!(tail[4].starts_with("sh -c 'd=$(mktemp -d"));
        // Our options come first, so they win over the pane's.
        assert_eq!(&args[..3], ["-T", "-o", "BatchMode=yes"]);
    }

    #[test]
    fn gzips_only_what_shrinks() {
        let dir = std::env::temp_dir().join(format!("nex-gzip-choice-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("shot.png"), vec![0u8; 500_000]).unwrap();
        std::fs::write(dir.join("src/main.rs"), "fn main() {}\n".repeat(10_000)).unwrap();
        std::fs::write(dir.join("small.txt"), "hi").unwrap();
        assert!(!worth_gzipping(&[dir.join("shot.png")]), "a screenshot goes as it is");
        assert!(!worth_gzipping(&[dir.join("small.txt")]), "too small to bother");
        assert!(worth_gzipping(&[dir.join("src")]), "source code shrinks");
        assert!(!worth_gzipping(&[dir.join("shot.png"), dir.join("src/main.rs")]), "mostly png");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The gzipped stream, unpacked by the remote half run here.
    #[test]
    fn remote_script_unpacks_gzip() {
        let src = std::env::temp_dir().join(format!("nex-remote-gz-test-{}", std::process::id()));
        let tmp = src.join("remote-tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(src.join("a.txt"), "hello ".repeat(1000)).unwrap();
        let tar = Command::new("tar").args(["-chf", "-", "-C"]).arg(&src).arg("a.txt").output().unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(&tar.stdout).unwrap();
        let packed = gz.finish().unwrap();
        let mut sh = Command::new("sh").args(["-c", &remote_script(true)]).env("TMPDIR", &tmp).stdin(Stdio::piped()).stdout(Stdio::piped()).spawn().unwrap();
        sh.stdin.take().unwrap().write_all(&packed).unwrap();
        let out = sh.wait_with_output().unwrap();
        assert!(out.status.success());
        let dir = String::from_utf8(out.stdout).unwrap();
        assert_eq!(std::fs::read_to_string(Path::new(dir.trim()).join("a.txt")).unwrap(), "hello ".repeat(1000));
        std::fs::remove_dir_all(&src).unwrap();
    }

    /// The remote half, run here with a local `sh` standing in for ssh.
    #[test]
    fn remote_script_unpacks() {
        let src = std::env::temp_dir().join(format!("nex-remote-test-{}", std::process::id()));
        let tmp = src.join("remote-tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(src.join("a b.txt"), "hello").unwrap();
        let mut tar = Command::new("tar").args(["-chf", "-", "-C"]).arg(&src).arg("a b.txt").stdout(Stdio::piped()).spawn().unwrap();
        let out = Command::new("sh")
            .args(["-c", REMOTE_SCRIPT])
            .env("TMPDIR", &tmp)
            .stdin(tar.stdout.take().unwrap())
            .output()
            .unwrap();
        assert!(tar.wait().unwrap().success());
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let dir = String::from_utf8(out.stdout).unwrap();
        let got = std::fs::read_to_string(Path::new(dir.trim()).join("a b.txt")).unwrap();
        assert_eq!(got, "hello");
        std::fs::remove_dir_all(&src).unwrap();
    }
}
