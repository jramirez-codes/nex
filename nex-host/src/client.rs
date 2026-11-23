//! Talking to the daemon: connecting (starting it first when it isn't running), and one
//! request with its answer.

use std::io::{self, Read, Write};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::time::Duration;

use nex_host::paths;
use nex_host::proto::{Request, Response};

/// How long to wait between tries at connecting to a daemon that's starting.
const START_RETRY: Duration = Duration::from_millis(60);

/// Connect to the daemon. With `start`, start one if none is running and wait for it.
pub fn connect(start: bool) -> io::Result<UnixStream> {
    let sock = paths::socket().ok_or_else(|| io::Error::other("no home folder ($HOME is unset)"))?;
    match UnixStream::connect(&sock) {
        Ok(s) => return Ok(s),
        Err(e) if !start => return Err(io::Error::new(e.kind(), "nex-host isn't running (no sessions)")),
        Err(_) => {}
    }
    spawn_daemon()?;
    let mut last = None;
    for _ in 0..50 {
        std::thread::sleep(START_RETRY);
        match UnixStream::connect(&sock) {
            Ok(s) => return Ok(s),
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| io::Error::other("the daemon didn't start")))
}

/// Start `nex-host daemon` in a session of its own, with nothing of this process's attached
/// (so the ssh command that ran us can finish).
fn spawn_daemon() -> io::Result<()> {
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("daemon").stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).current_dir("/");
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    // Not waited for: it outlives us, and init adopts it.
    cmd.spawn().map(drop)
}

/// Send `request` on `stream` and read the answer line (byte by byte, so nothing after it is
/// consumed: for `attach` the terminal stream follows).
pub fn ask(stream: &mut UnixStream, request: &Request) -> io::Result<Response> {
    let mut line = serde_json::to_string(request).map_err(io::Error::other)?;
    line.push('\n');
    stream.write_all(line.as_bytes())?;
    read_response(stream)
}

pub fn read_response(stream: &mut impl Read) -> io::Result<Response> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte)? {
            0 => return Err(io::Error::new(io::ErrorKind::UnexpectedEof, "the daemon hung up")),
            _ if byte[0] == b'\n' => break,
            _ => buf.push(byte[0]),
        }
    }
    serde_json::from_slice(&buf).map_err(io::Error::other)
}

/// Connect (starting the daemon if `start`), ask, and fail on an error answer.
pub fn request(request: &Request, start: bool) -> io::Result<Response> {
    let mut stream = connect(start)?;
    let r = ask(&mut stream, request)?;
    if r.ok { Ok(r) } else { Err(io::Error::other(r.error.unwrap_or_else(|| "failed".into()))) }
}
