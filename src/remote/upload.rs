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
//! unpacked by `sh` on the remote side, whatever the login shell is.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use super::{SshTarget, last_line, needs_login, posix_quote};

/// Why an upload failed.
#[derive(Debug, PartialEq, Eq)]
pub enum UploadError {
    /// The host wants a password (or another prompt) the upload can't answer.
    NeedsLogin,
    Other(String),
}

impl From<String> for UploadError {
    fn from(e: String) -> Self {
        Self::Other(e)
    }
}

impl From<&str> for UploadError {
    fn from(e: &str) -> Self {
        Self::Other(e.to_string())
    }
}

/// Make a folder for the upload, unpack the stream into it, and print its path.
const REMOTE_SCRIPT: &str = r#"d=$(mktemp -d "${TMPDIR:-/tmp}/nex.XXXXXX") && tar xf - -C "$d" && printf '%s\n' "$d""#;

/// Copy `paths` (files or folders) to a new folder on the remote host. Returns each one's
/// remote path, quoted for a shell. Blocks until done: run it on a worker thread.
pub fn upload(target: &SshTarget, paths: &[PathBuf]) -> Result<Vec<String>, UploadError> {
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
    let stream = tar.stdout.take().ok_or("couldn't run tar")?;
    let ssh = target
        .batch_command(REMOTE_SCRIPT)
        .stdin(stream)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("couldn't run ssh: {e}"))?;
    let out = ssh.wait_with_output().map_err(|e| e.to_string())?;
    let mut tar_err = String::new();
    if let Some(mut e) = tar.stderr.take() {
        let _ = e.read_to_string(&mut tar_err);
    }
    let tar_ok = tar.wait().is_ok_and(|s| s.success());

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        if needs_login(&err) {
            return Err(UploadError::NeedsLogin);
        }
        return Err(last_line(&err).unwrap_or_else(|| format!("ssh exited with {}", out.status)).into());
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
        let args = SshTarget::from_argv(&argv).unwrap().batch_args(REMOTE_SCRIPT);
        let tail = &args[args.len() - 5..];
        assert_eq!(&tail[..4], ["-p", "2222", "--", "me@host"]);
        assert!(tail[4].starts_with("sh -c 'd=$(mktemp -d"));
        // Our options come first, so they win over the pane's.
        assert_eq!(&args[..3], ["-T", "-o", "BatchMode=yes"]);
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
