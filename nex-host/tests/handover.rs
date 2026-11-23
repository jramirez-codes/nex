//! A newer nex-host installed while an older daemon runs: the old one is replaced at once
//! when nothing runs on it, and otherwise kept (its agents with it) until they finish.
#![cfg(unix)]

mod common;

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Child, Stdio};
use std::time::{Duration, Instant};

use common::{Host, eventually};
use nex_host::proto::{self, Request, Response};

const OLD: &str = "nex-host 0.0.1 (protocol 3)";

/// One request straight to the daemon's socket.
fn ask(h: &Host, r: &Request) -> Option<Response> {
    let mut s = UnixStream::connect(h.home.join(".nex/run/sock")).ok()?;
    s.write_all(format!("{}\n", serde_json::to_string(r).unwrap()).as_bytes()).ok()?;
    let mut line = String::new();
    BufReader::new(s).read_line(&mut line).ok()?;
    serde_json::from_str(&line).ok()
}

fn daemon_version(h: &Host) -> Option<String> {
    ask(h, &Request::Version)?.text
}

/// The old daemon: this nex-host copied to where nex installs it, run as `OLD`. It's then
/// replaced there by a stub that says it's `installed`, as an update would.
struct Old {
    exe: PathBuf,
    daemon: Child,
}

impl Old {
    fn start(h: &Host) -> Old {
        let exe = h.home.join(".nex/bin/nex-host");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::copy(env!("CARGO_BIN_EXE_nex-host"), &exe).unwrap();
        let daemon = h
            .cmd_at(&exe, &["daemon"])
            .env("NEX_HOST_TEST_VERSION", OLD)
            .env("NEX_HOST_IDLE_EXIT", "300")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        eventually("the old daemon", || daemon_version(h).as_deref() == Some(OLD));
        Old { exe, daemon }
    }

    /// Put a program that answers `--version` with `version` where this one was.
    fn install(&self, version: &str) {
        let tmp = self.exe.with_extension("tmp");
        std::fs::write(&tmp, format!("#!/bin/sh\necho '{version}'\n")).unwrap();
        std::fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        std::fs::rename(&tmp, &self.exe).unwrap();
    }

    fn gone(&mut self) -> bool {
        self.daemon.try_wait().unwrap().is_some()
    }
}

impl Drop for Old {
    fn drop(&mut self) {
        let _ = self.daemon.kill();
        let _ = self.daemon.wait();
    }
}

fn launch_raw(h: &Host, argv: &[&str]) -> String {
    let req = Request::Launch { argv: argv.iter().map(|s| s.to_string()).collect(), kind: None, name: None, cwd: None, cols: Some(40), rows: Some(10), env: None, parent: None };
    let r = ask(h, &req).unwrap();
    assert!(r.ok, "{:?}", r.error);
    r.id.unwrap()
}

#[test]
fn an_idle_old_daemon_is_replaced_on_connect() {
    let h = Host::new();
    let mut old = Old::start(&h);
    // Installed again at the same (old) version: it has no reason to go by itself.
    old.install(OLD);
    let out = h.cmd(&["launch", "--cols", "40", "--rows", "10", "--", "sleep", "30"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let id = String::from_utf8(out.stdout).unwrap().trim().to_string();
    eventually("the old daemon gone", || old.gone());
    assert_eq!(daemon_version(&h), Some(proto::version_line()));
    assert!(h.session(&id).is_some_and(|s| s.exit.is_none()), "launched on the new daemon");
    assert!(out.stderr.is_empty(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn an_old_daemon_with_agents_is_kept_until_they_finish() {
    let h = Host::new();
    let mut old = Old::start(&h);
    let first = launch_raw(&h, &["sleep", "60"]);
    old.install("nex-host 99.0.0 (protocol 3)");

    // A command now reuses it, and says why.
    let out = h.cmd(&["launch", "--cols", "40", "--rows", "10", "--", "sleep", "60"]).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let second = String::from_utf8(out.stdout).unwrap().trim().to_string();
    let note = String::from_utf8_lossy(&out.stderr);
    assert!(note.contains("nex-host 0.0.1 is still running 1 agent;"), "{note}");
    assert_eq!(daemon_version(&h).as_deref(), Some(OLD));
    for id in [&first, &second] {
        assert!(h.session(id).is_some_and(|s| s.exit.is_none()), "{id} still running");
    }

    // `watch` marks what it passes on as coming from the old daemon.
    let mut watch = h.cmd(&["watch"]).stdout(Stdio::piped()).spawn().unwrap();
    let mut line = String::new();
    BufReader::new(watch.stdout.take().unwrap()).read_line(&mut line).unwrap();
    let _ = watch.kill();
    let _ = watch.wait();
    let r: Response = serde_json::from_str(&line).unwrap();
    assert_eq!(r.daemon_version.as_deref(), Some(OLD));
    assert_eq!(r.sessions.unwrap().len(), 2);

    // One ending isn't enough; the last one is.
    h.run(&["kill", &first]);
    std::thread::sleep(Duration::from_millis(500));
    assert!(!old.gone());
    h.run(&["kill", &second]);
    let start = Instant::now();
    eventually("the old daemon retired", || old.gone());
    assert!(start.elapsed() < Duration::from_secs(5));
    assert!(!h.home.join(".nex/run/sock").exists());
}
