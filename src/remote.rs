//! Files dropped or pasted into a pane running `ssh`, sent to the remote host.
//!
//! A path typed into an ssh session names a file on *this* machine, which the remote side
//! can't open (and a remote program such as Claude Code can't read this machine's clipboard
//! either). So when a pane's foreground process is `ssh`, nex copies the files over a second
//! ssh connection to the same host, into a fresh folder under the remote `$TMPDIR` (or
//! `/tmp`), and types the *remote* paths instead.
//!
//! The second connection reuses the pane's own `ssh` arguments (port, user, identity, jump
//! hosts, `-o` options), minus forwardings and the remote command. It runs with
//! `BatchMode=yes`: it can't ask for a password, so it works with keys, an agent, or a
//! `ControlMaster` connection to share. When it can't log in, the caller falls back to the
//! local paths.
//!
//! For hosts that want a password, nex offers to add a block to `~/.ssh/config` (see
//! [`enable_sharing`]) that makes every `ssh` started in a nex pane share its connection. The
//! upload then rides on the login typed into the pane, with no password of its own.
//!
//! The files travel as one `tar` stream (so folders work too, and symlinks are followed),
//! unpacked by `sh` on the remote side, whatever the login shell is.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

/// ssh options that take a value (from ssh's `getopt` string).
const VALUE_OPTS: &str = "BbcDEeFIiJLlmOoPpQRSWw";
/// Options that make ssh do something other than open a session (print its config, its
/// version, a query or a control command, a stdio forward, a subsystem): no upload.
const NOT_A_SESSION: &str = "GVQOWs";
/// Options left off the upload's command line: forwardings (`L R D`), session and tty
/// flags, backgrounding, becoming a control master, verbosity, the escape character.
const DROPPED: &str = "LRDNTtfnMve";

/// Options put before the user's own: ssh keeps the first value it sees for an option, so
/// these win over the pane's `-o` options and the config file.
const UPLOAD_OPTS: &[&str] = &[
    "-T",
    "-o", "BatchMode=yes",
    "-o", "ConnectTimeout=8",
    "-o", "ClearAllForwardings=yes",
    // Share an existing master connection, but don't become one.
    "-o", "ControlMaster=no",
    "-o", "RemoteCommand=none",
    "-o", "RequestTTY=no",
    "-o", "PermitLocalCommand=no",
    "-o", "ForwardX11=no",
];

/// The first line of the block nex adds to `~/.ssh/config`, to tell whether it's there.
const SHARING_MARK: &str = "# Added by nex:";

/// Connection sharing for `ssh` run inside nex (panes set `TERM_PROGRAM=nex`, and so does the
/// upload). It goes at the end of the file, so the user's own settings come first and win.
/// `ControlPersist` puts the shared connection in the background, so closing the pane that
/// opened it doesn't wait on other panes using it, and it ends a minute after the last one.
/// The `x` prefix keeps `test` happy when the variable is unset, in any shell.
const SHARING_BLOCK: &str = "# Added by nex: ssh started in a nex pane shares its connection, so files dropped
# into the pane are uploaded over the login you already typed. Safe to remove.
Match exec \"test x$TERM_PROGRAM = xnex\"
  ControlMaster auto
  ControlPath ~/.ssh/nex-%C
  ControlPersist 60
";

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

/// The host a pane's `ssh` is connected to, as the options needed to connect again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SshTarget {
    /// The pane's connection options, in order (`-p 2222`, `-J jump`, `-o …`).
    pub opts: Vec<String>,
    /// `host` or `user@host` (or an `ssh://` URI).
    pub destination: String,
}

impl SshTarget {
    /// Read `ssh`'s command line, if it is an interactive session to some host.
    pub fn from_argv(argv: &[String]) -> Option<Self> {
        let (prog, args) = argv.split_first()?;
        if prog.rsplit('/').next()? != "ssh" {
            return None;
        }
        let mut opts = Vec::new();
        let mut i = 0;
        while i < args.len() {
            let arg = &args[i];
            i += 1;
            if arg == "--" {
                let destination = args.get(i)?.clone();
                return Some(Self { opts, destination });
            }
            let Some(flags) = arg.strip_prefix('-').filter(|f| !f.is_empty()) else {
                // The destination; anything after it is the remote command.
                return Some(Self { opts, destination: arg.clone() });
            };
            // A cluster of flags (`-tt`, `-4A`), possibly ending in one that takes a value,
            // either the rest of this argument (`-p2222`) or the next one (`-p 2222`).
            for (at, c) in flags.char_indices() {
                if NOT_A_SESSION.contains(c) {
                    return None;
                }
                if VALUE_OPTS.contains(c) {
                    let rest = &flags[at + c.len_utf8()..];
                    let value = if rest.is_empty() {
                        i += 1;
                        args.get(i - 1)?.clone()
                    } else {
                        rest.to_string()
                    };
                    if !DROPPED.contains(c) {
                        opts.push(format!("-{c}"));
                        opts.push(value);
                    }
                    break;
                }
                if !DROPPED.contains(c) {
                    opts.push(format!("-{c}"));
                }
            }
        }
        None
    }

    /// The host part of the destination, for messages ("Uploaded to devbox").
    pub fn host(&self) -> &str {
        let d = self.destination.strip_prefix("ssh://").unwrap_or(&self.destination);
        let d = d.rsplit('@').next().unwrap_or(d);
        // `[::1]:22`, `host:22` (URI form only), `host`.
        let d = d.split('/').next().unwrap_or(d);
        match d.strip_prefix('[') {
            Some(v6) => v6.split(']').next().unwrap_or(v6),
            None if self.destination.starts_with("ssh://") => d.split(':').next().unwrap_or(d),
            None => d,
        }
    }

    /// The upload's full `ssh` argument list (without the program name).
    fn upload_args(&self) -> Vec<String> {
        let mut args: Vec<String> = UPLOAD_OPTS.iter().map(|s| s.to_string()).collect();
        args.extend(self.opts.iter().cloned());
        args.push("--".into());
        args.push(self.destination.clone());
        // ssh hands this to the remote login shell (which may be fish or csh): one
        // single-quoted word that every shell passes to `sh` unchanged.
        args.push(format!("sh -c {}", posix_quote(REMOTE_SCRIPT)));
        args
    }
}

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
    let ssh = Command::new("ssh")
        // So the sharing block in `~/.ssh/config` finds the pane's connection, whatever
        // terminal nex itself was started from.
        .env("TERM_PROGRAM", "nex")
        .args(target.upload_args())
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

/// ssh's error when every way it could log in without asking was refused:
/// `user@host: Permission denied (publickey,password).`
fn needs_login(stderr: &str) -> bool {
    stderr.lines().any(|l| l.contains("Permission denied ("))
}

/// The user's ssh config file.
fn ssh_config() -> Option<PathBuf> {
    crate::platform::home_dir().map(|h| h.join(".ssh").join("config"))
}

/// Whether `~/.ssh/config` has nex's sharing block (see [`enable_sharing`]).
pub fn sharing_enabled() -> bool {
    ssh_config().and_then(|p| std::fs::read_to_string(p).ok()).is_some_and(|c| has_sharing(&c))
}

fn has_sharing(config: &str) -> bool {
    config.lines().any(|l| l.trim_start().starts_with(SHARING_MARK))
}

/// Connection sharing is a Unix ssh feature; Windows' OpenSSH doesn't have it.
pub const SHARING_SUPPORTED: bool = cfg!(unix);

/// Add the sharing block to the end of `~/.ssh/config`, making the file (private, as ssh
/// requires) and `~/.ssh` if needed. Sessions started from then on share their connection.
pub fn enable_sharing() -> std::io::Result<()> {
    use std::io::Write;
    let path = ssh_config().ok_or_else(|| std::io::Error::other("no home folder"))?;
    let existing = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(e),
    };
    if has_sharing(&existing) {
        return Ok(());
    }
    if let Some(dir) = path.parent() {
        let mut b = std::fs::DirBuilder::new();
        b.recursive(true);
        #[cfg(unix)]
        std::os::unix::fs::DirBuilderExt::mode(&mut b, 0o700);
        b.create(dir)?;
    }
    let mut open = std::fs::OpenOptions::new();
    open.append(true).create(true);
    #[cfg(unix)]
    std::os::unix::fs::OpenOptionsExt::mode(&mut open, 0o600);
    open.open(&path)?.write_all(sharing_addition(&existing).as_bytes())
}

/// What to append to a config file holding `existing`: the block, set off by a blank line.
fn sharing_addition(existing: &str) -> String {
    let sep = match existing {
        "" => "",
        e if e.ends_with("\n\n") => "",
        e if e.ends_with('\n') => "\n",
        _ => "\n\n",
    };
    format!("{sep}{SHARING_BLOCK}")
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

/// The last non-empty line of an error output.
fn last_line(s: &str) -> Option<String> {
    s.lines().map(str::trim).rfind(|l| !l.is_empty()).map(str::to_string)
}

/// Quote for a POSIX shell (only when needed).
fn posix_quote(s: &str) -> String {
    let safe = |c: char| c.is_ascii_alphanumeric() || "_./-+:,@%=~".contains(c);
    if !s.is_empty() && s.chars().all(safe) { s.to_string() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(cmd: &str) -> Option<SshTarget> {
        SshTarget::from_argv(&cmd.split_whitespace().map(String::from).collect::<Vec<_>>())
    }

    fn target(opts: &[&str], dest: &str) -> Option<SshTarget> {
        Some(SshTarget { opts: opts.iter().map(|s| s.to_string()).collect(), destination: dest.into() })
    }

    #[test]
    fn plain_hosts() {
        assert_eq!(parse("ssh host"), target(&[], "host"));
        assert_eq!(parse("/usr/bin/ssh user@host"), target(&[], "user@host"));
        assert_eq!(parse("ssh ssh://user@host:2222"), target(&[], "ssh://user@host:2222"));
    }

    #[test]
    fn keeps_connection_options() {
        assert_eq!(parse("ssh -p 2222 user@host"), target(&["-p", "2222"], "user@host"));
        assert_eq!(parse("ssh -p2222 -i ~/.ssh/k host"), target(&["-p", "2222", "-i", "~/.ssh/k"], "host"));
        assert_eq!(parse("ssh -o Port=22 -J jump host"), target(&["-o", "Port=22", "-J", "jump"], "host"));
        assert_eq!(parse("ssh -4A host"), target(&["-4", "-A"], "host"));
        assert_eq!(parse("ssh -Cl me host"), target(&["-C", "-l", "me"], "host"));
    }

    #[test]
    fn drops_forwardings_tty_and_command() {
        assert_eq!(parse("ssh -t host tmux attach"), target(&[], "host"));
        assert_eq!(parse("ssh -tt -v host"), target(&[], "host"));
        assert_eq!(parse("ssh -L 8080:localhost:80 -R9000:x:9000 host"), target(&[], "host"));
        assert_eq!(parse("ssh -J jump host -- ls -la"), target(&["-J", "jump"], "host"));
        assert_eq!(parse("ssh -p 22 -- host"), target(&["-p", "22"], "host"));
    }

    #[test]
    fn not_a_session() {
        assert_eq!(parse("ssh"), None);
        assert_eq!(parse("ssh -V"), None);
        assert_eq!(parse("ssh -G host"), None);
        assert_eq!(parse("ssh -W host:22 jump"), None);
        assert_eq!(parse("ssh -O check host"), None);
        assert_eq!(parse("ssh -p"), None);
        assert_eq!(parse("mosh host"), None);
        assert_eq!(parse("sshd -D"), None);
    }

    #[test]
    fn hosts() {
        assert_eq!(parse("ssh me@devbox").unwrap().host(), "devbox");
        assert_eq!(parse("ssh ssh://me@devbox:2222").unwrap().host(), "devbox");
        assert_eq!(parse("ssh ssh://[::1]:22").unwrap().host(), "::1");
        assert_eq!(parse("ssh fe80::1").unwrap().host(), "fe80::1");
    }

    #[test]
    fn upload_command() {
        let args = parse("ssh -p 2222 -t me@host htop").unwrap().upload_args();
        let tail = &args[args.len() - 5..];
        assert_eq!(&tail[..4], ["-p", "2222", "--", "me@host"]);
        assert!(tail[4].starts_with("sh -c 'd=$(mktemp -d"));
        // Our options come first, so they win over the pane's.
        assert_eq!(&args[..3], ["-T", "-o", "BatchMode=yes"]);
    }

    #[test]
    fn quoting() {
        assert_eq!(posix_quote("/tmp/nex.ab12/a.png"), "/tmp/nex.ab12/a.png");
        assert_eq!(posix_quote("/tmp/x/it's here"), r"'/tmp/x/it'\''s here'");
        assert_eq!(last_line("warning\nPermission denied (publickey).\n\n").as_deref(), Some("Permission denied (publickey)."));
    }

    #[test]
    fn login_errors() {
        assert!(needs_login("jr@10.0.0.2: Permission denied (publickey,password).\n"));
        assert!(!needs_login("ssh: connect to host x port 22: Connection refused\n"));
        assert!(!needs_login("tar: a.txt: Cannot open: Permission denied\n"));
    }

    #[test]
    fn sharing_block() {
        assert!(!has_sharing("Host x\n  User me\n"));
        assert!(has_sharing(&format!("Host x\n  User me\n{}", sharing_addition("Host x\n  User me\n"))));
        assert_eq!(sharing_addition(""), SHARING_BLOCK);
        assert_eq!(sharing_addition("Host x"), format!("\n\n{SHARING_BLOCK}"));
        assert!(sharing_addition("Host x\n").starts_with("\n# Added by nex:"));
        assert!(sharing_addition("Host x\n\n").starts_with("# Added by nex:"));
    }

    /// ssh itself reads the block: the sharing options apply inside nex only.
    #[test]
    fn ssh_reads_sharing_block() {
        let dir = std::env::temp_dir().join(format!("nex-sharing-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let cfg = dir.join("config");
        std::fs::write(&cfg, format!("Host *\n  Port 2200\n{}", sharing_addition("Host *\n  Port 2200\n"))).unwrap();
        let resolved = |term: &str| {
            let out = Command::new("ssh").args(["-G", "-F"]).arg(&cfg).arg("example.invalid").env("TERM_PROGRAM", term).output();
            out.ok().filter(|o| o.status.success()).map(|o| String::from_utf8_lossy(&o.stdout).into_owned())
        };
        if let (Some(inside), Some(outside)) = (resolved("nex"), resolved("xterm")) {
            assert!(inside.lines().any(|l| l == "controlmaster auto"), "{inside}");
            assert!(inside.lines().any(|l| l == "controlpersist 60"), "{inside}");
            assert!(outside.lines().any(|l| l == "controlmaster false"), "{outside}");
            assert!(inside.lines().any(|l| l == "port 2200"));
        }
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The remote half, run here with a local `sh` standing in for ssh.
    #[test]
    fn remote_script_unpacks() {
        let src = std::env::temp_dir().join(format!("nex-remote-test-{}", std::process::id()));
        let tmp = src.join("remote-tmp");
        std::fs::create_dir_all(&tmp).unwrap();
        std::fs::write(src.join("a b.txt"), "hello").unwrap();
        let tar = Command::new("tar").args(["-chf", "-", "-C"]).arg(&src).arg("a b.txt").stdout(Stdio::piped()).spawn().unwrap();
        let out = Command::new("sh")
            .args(["-c", REMOTE_SCRIPT])
            .env("TMPDIR", &tmp)
            .stdin(tar.stdout.unwrap())
            .output()
            .unwrap();
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
        let dir = String::from_utf8(out.stdout).unwrap();
        let got = std::fs::read_to_string(Path::new(dir.trim()).join("a b.txt")).unwrap();
        assert_eq!(got, "hello");
        std::fs::remove_dir_all(&src).unwrap();
    }
}
