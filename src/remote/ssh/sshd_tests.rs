//! nex's own ssh client against a real `sshd`: a private one on a spare port, run as this
//! user, with its own host key and authorized_keys file, and `HOME` set to a scratch folder
//! for every login, so nothing of the user's own (`~/.ssh`, `~/.nex`) is touched.
//!
//! Ignored by default (it needs `/usr/sbin/sshd`, and nex-host built for musl):
//!
//! ```text
//! cargo build -p nex-host --target x86_64-unknown-linux-musl
//! cargo test -p nex remote::ssh::sshd_tests -- --ignored --test-threads=1
//! ```

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use super::device::{self, Profile};
use super::host::{self, LoginError};
use super::{Client, ConnectError, Login, Server};

/// A private sshd, stopped when dropped.
struct Sshd {
    child: Child,
    dir: PathBuf,
    port: u16,
}

impl Sshd {
    fn start(dir: &Path, port: u16) -> Sshd {
        let host_key = dir.join("host_key");
        if !host_key.exists() {
            let ok = Command::new("ssh-keygen").args(["-q", "-t", "ed25519", "-N", "", "-f"]).arg(&host_key).status().unwrap();
            assert!(ok.success());
        }
        let config = dir.join("sshd_config");
        std::fs::write(
            &config,
            format!(
                "Port {port}\nListenAddress 127.0.0.1\nHostKey {key}\nPidFile {dir}/sshd.pid\nAuthorizedKeysFile {dir}/authorized_keys\n\
                 PasswordAuthentication no\nKbdInteractiveAuthentication no\nUsePAM no\nStrictModes no\nPermitUserEnvironment yes\n",
                key = host_key.display(),
                dir = dir.display()
            ),
        )
        .unwrap();
        let child = Command::new("/usr/sbin/sshd").args(["-D", "-e", "-f"]).arg(&config).stderr(Stdio::null()).spawn().unwrap();
        // Up once it takes connections.
        let deadline = Instant::now() + Duration::from_secs(5);
        while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
            assert!(Instant::now() < deadline, "sshd didn't start");
            std::thread::sleep(Duration::from_millis(50));
        }
        Sshd { child, dir: dir.to_path_buf(), port }
    }

    /// End every connection to it (its per-connection processes are its children), as a
    /// dropped network would.
    fn drop_connections(&self) {
        let _ = Command::new("pkill").args(["-P", &self.child.id().to_string()]).status();
    }

    /// Freeze every connection to it: they stay open but nothing answers, as with a
    /// connection the phone lost while it slept. New connections still work.
    fn freeze_connections(&self) {
        let _ = Command::new("pkill").args(["-STOP", "-P", &self.child.id().to_string()]).status();
    }

    fn authorize(&self, line: &str) {
        let home = self.dir.join("home");
        std::fs::create_dir_all(&home).unwrap();
        let entry = format!("environment=\"HOME={}\" {line}\n", home.display());
        let mut f = std::fs::OpenOptions::new().create(true).append(true).open(self.dir.join("authorized_keys")).unwrap();
        f.write_all(entry.as_bytes()).unwrap();
    }
}

impl Drop for Sshd {
    fn drop(&mut self) {
        // Frozen connections too (a stopped process ignores everything but KILL).
        let _ = Command::new("pkill").args(["-KILL", "-P", &self.child.id().to_string()]).status();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn user() -> String {
    std::env::var("USER").unwrap_or_else(|_| "root".into())
}

#[test]
#[ignore = "runs a private sshd; see the module doc"]
fn logs_in_with_the_device_key_and_keeps_a_shell_through_a_dropped_connection() {
    let dir = std::env::temp_dir().join(format!("nex-ssh-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // nex's own files (known_hosts, the device key, remote.json) go here too.
    // SAFETY: this test runs alone (`--test-threads=1`), before any thread reads these.
    unsafe {
        std::env::set_var("XDG_CONFIG_HOME", dir.join("config"));
        std::env::set_var("NEX_HOST_DIR", &dir);
    }
    // The nex-host just built (a release build there may be an older one).
    let built = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("target/x86_64-unknown-linux-musl/debug/nex-host");
    std::fs::copy(&built, dir.join("nex-host-x86_64-unknown-linux-musl")).expect("cargo build -p nex-host --target x86_64-unknown-linux-musl");
    let sshd = Sshd::start(&dir, 2299);
    let server = Server { host: "127.0.0.1".into(), port: sshd.port, user: user() };

    // A device that set up its key before: the key, its profile, its line on the host.
    let mut profile = Profile::new(server.clone());
    let key = device::new_key(&profile.comment).unwrap();
    profile.key_expires = Some(device::expiry(device::now(), 30));
    device::save_profile(&profile).unwrap();
    sshd.authorize(&device::authorized_line(&key, &profile.comment, None).unwrap());

    // A host nex hasn't seen asks to be trusted, and a wrong fingerprint isn't trust.
    let fingerprint = match host::login(&server, None, None, 30) {
        Err(LoginError::UnknownHost { fingerprint }) => fingerprint,
        other => panic!("expected UnknownHost, got {:?}", other.err()),
    };
    assert!(fingerprint.starts_with("SHA256:"));
    assert!(matches!(host::login(&server, None, Some("SHA256:nope"), 30), Err(LoginError::UnknownHost { .. })));

    // Trusted: logs in with the key, installs nex-host in the scratch home.
    let remote = Arc::new(host::login(&server, None, Some(&fingerprint), 30).expect("key login"));
    assert!(dir.join("home/.nex/bin/nex-host").is_file(), "nex-host installed");
    assert!(remote.idle_shells().is_empty());

    // Known from now on.
    drop(Client::connect(&server, Login::Key(&key), None).expect("known host"));

    // Another key is turned down: without a password that's NeedsPassword.
    let other = device::new_key("someone-else").unwrap();
    assert!(matches!(Client::connect(&server, Login::Key(&other), None), Err(ConnectError::Denied)));
    let mut expired = profile.clone();
    expired.key_expires = Some(0);
    device::save_profile(&expired).unwrap();
    assert!(matches!(host::login(&server, None, None, 30), Err(LoginError::NeedsPassword)));
    device::save_profile(&profile).unwrap();
    // `new_key` replaced the saved key with `other`: put ours back.
    std::fs::write(dir.join("config/nex/device_key"), key.to_openssh(russh::keys::ssh_key::LineEnding::LF).unwrap().as_bytes()).unwrap();

    // A shell over the remote "PTY": type into it, read what it prints.
    let session = Arc::new(Mutex::new(None));
    let start = crate::terminal::remote::Start { host: remote.clone(), session: session.clone(), keep: false, cwd: None, cols: 80, rows: 24 };
    let repaint = Arc::new(std::sync::OnceLock::new());
    let mut pty = crate::terminal::remote::open(start, Default::default(), crate::terminal::osc7::Alert::new(repaint), Default::default()).unwrap();
    let io = pty_io(&mut pty);
    io.set_nonblocking(false).unwrap();
    io.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
    let mut io = io;
    let mut seen = String::new();
    type_and_wait(&mut io, &mut seen, "echo nex-$((6*7))\r", "nex-42");
    let id = session.lock().unwrap().clone().expect("the session's id");

    // The connection drops; the pane says so, gets it back and attaches again.
    sshd.drop_connections();
    wait_for(&mut io, &mut seen, "reconnecting", Duration::from_secs(60));
    type_and_wait(&mut io, &mut seen, "echo back-$((40+2))\r", "back-42");
    assert_eq!(session.lock().unwrap().as_deref(), Some(id.as_str()), "the same session");

    // The connection goes quiet without closing (the phone slept): waking finds it dead at
    // once, well before keepalives would give up on it, and the pane attaches again.
    sshd.freeze_connections();
    super::woke();
    wait_for(&mut io, &mut seen, "reconnecting", Duration::from_secs(8));
    type_and_wait(&mut io, &mut seen, "echo awake-$((40+2))\r", "awake-42");
    assert_eq!(session.lock().unwrap().as_deref(), Some(id.as_str()), "the same session");

    // Closing the pane ends its shell on the host.
    drop(io);
    drop(pty);
    let deadline = Instant::now() + Duration::from_secs(10);
    while !remote.idle_shells().is_empty() || listed(&remote, &id) {
        assert!(Instant::now() < deadline, "the shell outlived its pane");
        std::thread::sleep(Duration::from_millis(200));
    }
    drop(remote);
    drop(sshd);
    let _ = std::fs::remove_dir_all(&dir);
}

/// The closer ends a command that isn't reading its input: one blocked opening a FIFO, as
/// the shared mic's ALSA waiting session is. Ending its input or closing its channel wouldn't
/// unblock it; the `SIGTERM` the closer sends does.
#[test]
#[ignore = "runs a private sshd; see the module doc"]
fn the_closer_ends_a_command_blocked_on_a_fifo() {
    let dir = std::env::temp_dir().join(format!("nex-ssh-close-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // SAFETY: this test runs alone (`--test-threads=1`), before any thread reads these.
    unsafe { std::env::set_var("XDG_CONFIG_HOME", dir.join("config")) };
    let sshd = Sshd::start(&dir, 2298);
    let key = device::new_key("closer-test").unwrap();
    sshd.authorize(&device::authorized_line(&key, "closer-test", None).unwrap());
    let server = Server { host: "127.0.0.1".into(), port: sshd.port, user: user() };
    let fingerprint = match Client::connect(&server, Login::Key(&key), None) {
        Err(ConnectError::UnknownHost { fingerprint }) => fingerprint,
        other => panic!("expected UnknownHost, got {:?}", other.err()),
    };
    let client = Client::connect(&server, Login::Key(&key), Some(&fingerprint)).expect("key login");

    let script = r#"f="$HOME/blocked.fifo"; rm -f "$f"; mkfifo "$f" || exit 1; echo $$; exec 3>"$f""#;
    let exec = client.exec(&format!("sh -c {}", crate::remote::posix_quote(script))).unwrap();
    let super::Exec { mut reader, writer, closer, .. } = exec;
    let mut pid = Vec::new();
    let mut byte = [0u8];
    while reader.read(&mut byte).unwrap() == 1 && byte[0] != b'\n' {
        pid.push(byte[0]);
    }
    let proc = PathBuf::from(format!("/proc/{}", String::from_utf8(pid).unwrap()));
    // Ending its input alone doesn't end it: it never reads it.
    drop(writer);
    std::thread::sleep(Duration::from_millis(300));
    assert!(proc.exists(), "still blocked");

    let (tx, ended) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rest = Vec::new();
        let _ = reader.read_to_end(&mut rest);
        let _ = tx.send(());
    });
    closer.close();
    ended.recv_timeout(Duration::from_secs(5)).expect("its output ends once closed");
    let deadline = Instant::now() + Duration::from_secs(5);
    while proc.exists() {
        assert!(Instant::now() < deadline, "the command outlived its closed channel");
        std::thread::sleep(Duration::from_millis(50));
    }
    drop(client);
    drop(sshd);
    let _ = std::fs::remove_dir_all(&dir);
}

fn listed(remote: &host::RemoteHost, id: &str) -> bool {
    remote.ask(&nex_host::proto::Request::List).ok().and_then(|(r, _)| r.sessions).unwrap_or_default().iter().any(|s| s.id == id)
}

/// The pane's side of the remote PTY (what alacritty's event loop would read and write).
fn pty_io(pty: &mut crate::terminal::remote::RemotePty) -> std::os::unix::net::UnixStream {
    use alacritty_terminal::tty::EventedReadWrite;
    pty.writer().try_clone().unwrap()
}

fn type_and_wait(io: &mut std::os::unix::net::UnixStream, seen: &mut String, keys: &str, want: &str) {
    // The shell may still be starting: type once it has printed something.
    wait_for(io, seen, "", Duration::from_secs(10));
    io.write_all(keys.as_bytes()).unwrap();
    wait_for(io, seen, want, Duration::from_secs(10));
}

fn wait_for(io: &mut std::os::unix::net::UnixStream, seen: &mut String, want: &str, limit: Duration) {
    let deadline = Instant::now() + limit;
    let mut buf = [0u8; 4096];
    let start = seen.len();
    loop {
        if seen[start..].contains(want) && (!want.is_empty() || seen.len() > start) {
            return;
        }
        assert!(Instant::now() < deadline, "never saw {want:?}; got {:?}", &seen[start..]);
        if let Ok(n) = io.read(&mut buf) {
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
        }
    }
}
