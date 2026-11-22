//! The `nex-host` program end to end, in a home folder of its own: agents start in the
//! background, panes attach, see what was drawn, type, leave and come back, and agents end.
#![cfg(unix)]

mod common;

use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::time::{Duration, Instant};

use common::{eventually, Host};
use nex_host::proto::{self, Request, Response, Status};

#[test]
fn prints_its_version() {
    let h = Host::new();
    assert_eq!(h.run(&["--version"]).trim(), proto::version_line());
}

#[test]
fn an_agent_outlives_its_pane_and_redraws_when_reattached() {
    let h = Host::new();
    // `cat` echoes what's typed, as an agent's prompt would.
    let id = h.launch(&["cat"]);
    assert_eq!(id.len(), 8);
    let s = h.session(&id).expect("listed");
    assert_eq!(s.kind, "cat");
    assert!(!s.attached);

    let mut pane = h.attach(&id);
    pane.type_keys("hello from the pane\r");
    pane.wait_for("hello from the pane");
    eventually("attached", || h.session(&id).is_some_and(|s| s.attached));

    // Close the pane: the agent keeps running.
    drop(pane);
    eventually("detached", || h.session(&id).is_some_and(|s| !s.attached));
    assert_eq!(h.session(&id).unwrap().status, Status::Idle);

    // Another pane sees what was on the screen.
    let mut again = h.attach(&id);
    again.wait_for("hello from the pane");
    again.type_keys("still here\r");
    again.wait_for("still here");
}

#[test]
fn attaching_elsewhere_lets_the_first_pane_go() {
    let h = Host::new();
    let id = h.launch(&["cat"]);
    let mut first = h.attach(&id);
    let mut second = h.attach(&id);
    first.wait_for("[attached in another pane]");
    second.type_keys("mine now\r");
    second.wait_for("mine now");
}

#[test]
fn an_agent_that_ends_is_listed_as_exited_until_forgotten() {
    let h = Host::new();
    let id = h.launch(&["sh", "-c", "printf 'done\\n'; exit 3"]);
    eventually("exit", || h.session(&id).is_some_and(|s| s.exit == Some(3)));
    assert_eq!(h.session(&id).unwrap().status, Status::Exited);
    h.run(&["forget", &id]);
    assert!(h.session(&id).is_none());
}

#[test]
fn kill_ends_an_agent_and_forgets_it() {
    let h = Host::new();
    let id = h.launch(&["sleep", "60"]);
    let mut pane = h.attach(&id);
    h.run(&["kill", &id]);
    pane.wait_for("exited");
    eventually("forgotten", || h.session(&id).is_none());
}

#[test]
fn starts_in_the_folder_asked_for() {
    let h = Host::new();
    std::fs::create_dir_all(h.home.join("proj")).unwrap();
    let id = h.run(&["launch", "--cwd", "~/proj", "--name", "fix tests", "--", "sh", "-c", "pwd; sleep 30"]).trim().to_string();
    let s = h.session(&id).unwrap();
    assert_eq!(s.name, "fix tests");
    assert!(s.cwd.ends_with("/proj"), "{}", s.cwd);
    let mut pane = h.attach(&id);
    pane.wait_for("/proj");
}

#[test]
fn watch_streams_changes() {
    let h = Host::new();
    let mut watch = h.cmd(&["watch"]).stdout(std::process::Stdio::piped()).spawn().unwrap();
    let mut out = watch.stdout.take().unwrap();
    let id = h.launch(&["sleep", "30"]);
    let mut seen = String::new();
    let mut buf = [0u8; 4096];
    let deadline = Instant::now() + Duration::from_secs(5);
    while !seen.contains(&id) {
        assert!(Instant::now() < deadline, "watch never listed {id}: {seen}");
        let n = out.read(&mut buf).unwrap();
        seen.push_str(&String::from_utf8_lossy(&buf[..n]));
    }
    let line = seen.lines().find(|l| l.contains(&id)).unwrap();
    let r: Response = serde_json::from_str(line).unwrap();
    assert!(r.sessions.unwrap().iter().any(|s| s.id == id));
    let _ = watch.kill();
    let _ = watch.wait();
}

#[test]
fn run_passes_the_shells_environment_on() {
    let h = Host::new();
    let mut s = UnixStream::connect(h.home.join(".nex/run/sock")).or_else(|_| {
        // Start the daemon with any command first.
        h.launch(&["true"]);
        UnixStream::connect(h.home.join(".nex/run/sock"))
    }).unwrap();
    let env = vec![("PATH".to_string(), "/usr/bin:/bin".to_string()), ("NEX_TEST_KEY".to_string(), "from the shell".to_string()), ("TERM".to_string(), "dumb".to_string())];
    let req = Request::Launch { argv: vec!["sh".into(), "-c".into(), "echo \"$NEX_TEST_KEY/$TERM/$NEX_HOST\"; sleep 30".into()], kind: None, name: None, cwd: None, cols: Some(60), rows: Some(10), env: Some(env) };
    s.write_all(format!("{}\n", serde_json::to_string(&req).unwrap()).as_bytes()).unwrap();
    let mut line = String::new();
    std::io::BufRead::read_line(&mut std::io::BufReader::new(s), &mut line).unwrap();
    let id = serde_json::from_str::<Response>(&line).unwrap().id.unwrap();
    let mut pane = h.attach(&id);
    // Its own variables, nex's terminal, and the mark that it runs inside nex-host.
    pane.wait_for("from the shell/xterm-256color/1");
}
