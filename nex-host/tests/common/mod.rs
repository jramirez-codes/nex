//! What the end-to-end tests share: `nex-host` in a home folder of its own.
#![allow(dead_code)]

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use nex_host::proto::{self, Request, Response, Session};

pub struct Host {
    pub home: PathBuf,
}

impl Host {
    pub fn new() -> Host {
        static N: AtomicUsize = AtomicUsize::new(0);
        // Short: the socket's path has to fit in ~100 bytes.
        let home = std::env::temp_dir().join(format!("nexh-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(&home).unwrap();
        Host { home }
    }

    pub fn cmd(&self, args: &[&str]) -> Command {
        self.cmd_at(std::path::Path::new(env!("CARGO_BIN_EXE_nex-host")), args)
    }

    /// `cmd`, running the program at `exe`.
    pub fn cmd_at(&self, exe: &std::path::Path, args: &[&str]) -> Command {
        let mut c = Command::new(exe);
        c.args(args)
            .env("HOME", &self.home)
            .env("NEX_HOME", self.home.join(".nex"))
            .env("SHELL", "/bin/sh")
            .env("NEX_HOST_IDLE_EXIT", "1")
            .env_remove("CLAUDE_CONFIG_DIR")
            // CI runners set it, and pipelines are looked for there before $HOME/.config.
            .env_remove("XDG_CONFIG_HOME");
        c
    }

    pub fn run(&self, args: &[&str]) -> String {
        let out = self.cmd(args).output().unwrap();
        assert!(out.status.success(), "nex-host {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8(out.stdout).unwrap()
    }

    pub fn launch(&self, argv: &[&str]) -> String {
        let mut args = vec!["launch", "--cols", "40", "--rows", "10", "--"];
        args.extend(argv);
        self.run(&args).trim().to_string()
    }

    pub fn sessions(&self) -> Vec<Session> {
        let r: Response = serde_json::from_str(&self.run(&["list", "--json"])).unwrap();
        r.sessions.unwrap()
    }

    pub fn session(&self, id: &str) -> Option<Session> {
        self.sessions().into_iter().find(|s| s.id == id)
    }

    pub fn attach(&self, id: &str) -> Pane {
        let mut s = UnixStream::connect(self.home.join(".nex/run/sock")).unwrap();
        let req = serde_json::to_string(&Request::Attach { id: id.into(), cols: 40, rows: 10 }).unwrap();
        s.write_all(format!("{req}\n").as_bytes()).unwrap();
        let mut line = Vec::new();
        let mut b = [0u8; 1];
        while s.read(&mut b).unwrap() == 1 && b[0] != b'\n' {
            line.push(b[0]);
        }
        let r: Response = serde_json::from_slice(&line).unwrap();
        assert!(r.ok, "attach: {:?}", r.error);
        s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
        Pane { stream: s, seen: String::new() }
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        for s in self.sessions() {
            let _ = self.cmd(&["kill", &s.id]).output();
        }
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

/// An attached pane: what it has been sent so far.
pub struct Pane {
    pub stream: UnixStream,
    pub seen: String,
}

impl Pane {
    /// Read until `text` has arrived (or fail after a few seconds).
    pub fn wait_for(&mut self, text: &str) {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut buf = [0u8; 4096];
        while !self.seen.contains(text) {
            assert!(Instant::now() < deadline, "never saw {text:?}; got {:?}", self.seen);
            match self.stream.read(&mut buf) {
                Ok(0) => panic!("hung up before {text:?}; got {:?}", self.seen),
                Ok(n) => self.seen.push_str(&String::from_utf8_lossy(&buf[..n])),
                Err(_) => {}
            }
        }
    }

    pub fn type_keys(&mut self, keys: &str) {
        self.stream.write_all(&proto::frame(proto::FRAME_INPUT, keys.as_bytes())).unwrap();
    }
}

pub fn eventually(what: &str, mut ok: impl FnMut() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(8);
    while !ok() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(100));
    }
}
