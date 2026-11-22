//! `nex-host attach <id>`: what runs in a pane's `ssh -t` to show an agent. The terminal
//! goes raw and becomes the agent's: its screen first, then its output; keys and resizes go
//! back. Closing the pane (ssh hanging up) just ends this; the agent keeps running.

use std::io::{self, Read, Write};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

use nex_host::proto::{self, Request};

use crate::{client, redraw};

/// How often the terminal's size is checked (sshd passes the pane's resizes on to it).
const SIZE_POLL: Duration = Duration::from_millis(200);

/// The terminal settings to put back on the way out.
static SAVED: OnceLock<libc::termios> = OnceLock::new();

pub fn run(id: &str) -> io::Result<i32> {
    let (cols, rows) = size().unwrap_or((80, 24));
    let mut stream = client::connect(false)?;
    let r = client::ask(&mut stream, &Request::Attach { id: id.to_string(), cols, rows })?;
    if !r.ok {
        return Err(io::Error::other(r.error.unwrap_or_else(|| "couldn't attach".into())));
    }
    raw_mode();
    let _ = io::stdout().write_all(proto::agent_tag(id).as_bytes());

    // The agent's output to the terminal, until the daemon hangs up (the agent ended, or
    // another pane took it): then we're done.
    let mut from = stream.try_clone()?;
    std::thread::spawn(move || {
        let mut out = io::stdout().lock();
        let mut buf = vec![0u8; 64 * 1024];
        loop {
            match from.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if out.write_all(&buf[..n]).and_then(|_| out.flush()).is_err() {
                        break;
                    }
                }
            }
        }
        drop(out);
        leave(0);
    });

    let to = Arc::new(Mutex::new(stream));
    // Resizes.
    let resize = to.clone();
    std::thread::spawn(move || {
        let mut last = (cols, rows);
        loop {
            std::thread::sleep(SIZE_POLL);
            if let Some(now) = size().filter(|&s| s != last) {
                last = now;
                if resize.lock().unwrap().write_all(&proto::resize_frame(now.0, now.1)).is_err() {
                    break;
                }
            }
        }
    });

    // Keys.
    let mut stdin = io::stdin().lock();
    let mut buf = vec![0u8; 16 * 1024];
    loop {
        match stdin.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if to.lock().unwrap().write_all(&proto::frame(proto::FRAME_INPUT, &buf[..n])).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    leave(0)
}

/// Put the terminal back and exit.
fn leave(code: i32) -> ! {
    if let Some(t) = SAVED.get() {
        unsafe { libc::tcsetattr(0, libc::TCSANOW, t) };
    }
    let _ = io::stdout().write_all(redraw::RESET_MODES.as_bytes());
    let _ = io::stdout().write_all(proto::agent_tag("").as_bytes());
    let _ = io::stdout().flush();
    std::process::exit(code)
}

fn raw_mode() {
    unsafe {
        let mut t: libc::termios = std::mem::zeroed();
        if libc::tcgetattr(0, &mut t) != 0 {
            return;
        }
        let _ = SAVED.set(t);
        libc::cfmakeraw(&mut t);
        libc::tcsetattr(0, libc::TCSANOW, &t);
    }
}

/// The terminal's size, columns by rows.
fn size() -> Option<(u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::ioctl(0, libc::TIOCGWINSZ as _, &mut ws) };
    (r == 0 && ws.ws_col > 0 && ws.ws_row > 0).then_some((ws.ws_col, ws.ws_row))
}
