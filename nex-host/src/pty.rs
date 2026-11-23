//! The pty and process an agent runs in: the login shell that starts it, its environment,
//! its terminal's size, and the thread writing to the pane attached to it.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, Write};
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
use std::os::unix::net::UnixStream;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command};
use std::sync::mpsc::{self, Sender};

use nex_host::proto;

use crate::agent::Out;
use crate::redraw;

/// The login shell to start programs under, so they get the user's `PATH` (`~/.local/bin`,
/// nvm, …) even though the daemon was started by a non-interactive ssh command. Shells
/// whose `-l -c` works as in sh; anything else gets sh.
fn login_shell() -> String {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let base = shell.rsplit('/').next().unwrap_or("");
    if matches!(base, "sh" | "bash" | "zsh" | "dash" | "ksh" | "mksh" | "fish") && std::path::Path::new(&shell).exists() {
        shell
    } else {
        "/bin/sh".into()
    }
}

/// Start `argv` in `cwd` under the login shell (which `exec`s it, so the program keeps the
/// shell's pid), or the login shell itself when `argv` is empty, on a new pty, as the leader of its own session with the pty as its terminal.
pub fn spawn_in_pty(
    argv: &[String],
    env: Option<&[(String, String)]>,
    cwd: &std::path::Path,
    cols: u16,
    rows: u16,
    own_env: HashMap<&'static str, String>,
) -> io::Result<(File, Child)> {
    let (mut master, mut slave) = (-1, -1);
    let mut size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    // Raw pointers: macOS declares these `*mut`, Linux `*const`.
    let r = unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null_mut(), &raw mut size) };
    if r != 0 {
        return Err(io::Error::last_os_error());
    }
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        libc::fcntl(slave.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
        // UTF-8 line editing (backspace over multi-byte characters), as terminals do.
        #[cfg(any(target_os = "linux", target_os = "macos"))]
        {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(master.as_raw_fd(), &mut t) == 0 {
                t.c_iflag |= libc::IUTF8;
                libc::tcsetattr(master.as_raw_fd(), libc::TCSANOW, &t);
            }
        }
    }

    let mut cmd = match env {
        // A plain shell: the login shell, interactive (in `env` when one was sent).
        _ if argv.is_empty() => {
            let mut cmd = Command::new(login_shell());
            cmd.arg("-l");
            if let Some(env) = env {
                cmd.env_clear().envs(env.iter().filter(|(k, _)| !skip_var(k)).map(|(k, v)| (k, v)));
            }
            cmd
        }
        // The environment of the shell it was started from: run it directly in that.
        Some(env) => {
            let mut cmd = Command::new(&argv[0]);
            cmd.args(&argv[1..]).env_clear().envs(env.iter().filter(|(k, _)| !skip_var(k)).map(|(k, v)| (k, v)));
            cmd
        }
        None => {
            let script = format!("exec {}", argv.iter().map(|a| crate::agent::quote(a)).collect::<Vec<_>>().join(" "));
            let mut cmd = Command::new(login_shell());
            cmd.args(["-l", "-c", &script]);
            cmd
        }
    };
    // `nex-host` itself on the PATH, for the agent's `nex-host done` and friends.
    let base_path = env.and_then(|e| e.iter().find(|(k, _)| k == "PATH").map(|(_, v)| v.clone())).or_else(|| std::env::var("PATH").ok()).unwrap_or_default();
    let path = match std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.to_string_lossy().into_owned())) {
        Some(dir) if !base_path.split(':').any(|p| p == dir) => format!("{dir}:{base_path}"),
        _ => base_path,
    };
    cmd.current_dir(cwd)
        .stdin(slave.try_clone()?)
        .stdout(slave.try_clone()?)
        .stderr(slave)
        .envs(own_env)
        .env("PATH", path);
    for key in ["SSH_TTY", "TMUX", "TMUX_PANE", "STY"] {
        cmd.env_remove(key);
    }
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() == -1 {
                return Err(io::Error::last_os_error());
            }
            // stdin is the pty: make it this session's terminal.
            if libc::ioctl(0, libc::TIOCSCTTY as _, 0) == -1 {
                return Err(io::Error::last_os_error());
            }
            for sig in [libc::SIGCHLD, libc::SIGHUP, libc::SIGINT, libc::SIGQUIT, libc::SIGTERM, libc::SIGALRM, libc::SIGPIPE] {
                libc::signal(sig, libc::SIG_DFL);
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok((File::from(master), child))
}

/// Variables that describe the terminal a command was typed in, not the one the agent will
/// run in: left out of an environment passed along with `run`.
pub fn skip_var(key: &str) -> bool {
    matches!(
        key,
        "TERM"
            | "COLORTERM"
            | "TERM_PROGRAM"
            | "TERM_PROGRAM_VERSION"
            | "COLUMNS"
            | "LINES"
            | "SHLVL"
            | "PWD"
            | "OLDPWD"
            | "_"
            | "TMUX"
            | "TMUX_PANE"
            | "STY"
            | "WINDOWID"
            | "NEX_HOST"
            | "NEX_AGENT_ID"
            | "NEX_AGENT_NAME"
            | "NEX_SOCK"
    ) || key.starts_with("SSH_TTY")
}

/// What agents see of their terminal (nex's, as in a local pane), and who they are, for
/// `nex-host send` / `done` run inside them.
pub fn agent_env(id: &str, name: &str) -> HashMap<&'static str, String> {
    let sock = nex_host::paths::socket().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
    HashMap::from([
        ("TERM", "xterm-256color".to_string()),
        ("COLORTERM", "truecolor".to_string()),
        ("TERM_PROGRAM", "nex".to_string()),
        ("TERM_PROGRAM_VERSION", proto::VERSION.to_string()),
        ("NEX_HOST", "1".to_string()),
        ("NEX_AGENT_ID", id.to_string()),
        ("NEX_AGENT_NAME", name.to_string()),
        ("NEX_SOCK", sock),
    ])
}

pub fn set_size(fd: i32, cols: u16, rows: u16) {
    let size = libc::winsize { ws_row: rows, ws_col: cols, ws_xpixel: 0, ws_ypixel: 0 };
    unsafe { libc::ioctl(fd, libc::TIOCSWINSZ as _, &size) };
}

/// The connection to an attached pane: sends what [`crate::agent::Agent`] queues for it, in order, so a
/// slow network never holds up reading the agent's output.
pub fn spawn_writer(mut stream: UnixStream) -> Sender<Out> {
    let (tx, rx) = mpsc::channel::<Out>();
    std::thread::spawn(move || {
        for out in rx {
            match out {
                Out::Bytes(b) => {
                    if stream.write_all(&b).is_err() {
                        break;
                    }
                }
                Out::Close(line) => {
                    let _ = stream.write_all(format!("{}\r\n{line}\r\n", redraw::RESET_MODES).as_bytes());
                    break;
                }
            }
        }
        let _ = stream.shutdown(std::net::Shutdown::Both);
    });
    tx
}
