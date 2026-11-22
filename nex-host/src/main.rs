//! The `nex-host` program. nex runs it over ssh; it works by hand too:
//!
//! ```text
//! nex-host run [--name N] [--cwd DIR] -- claude …   start an agent and show it here
//! nex-host launch [--kind K] [--name N] [--cwd DIR] [--cols C --rows R] -- CMD …
//!                                                    start one in the background; prints its id
//! nex-host attach ID                                 show it here (closing the terminal leaves it running)
//! nex-host list [--json]                             what's running
//! nex-host watch                                     the list as JSON lines, whenever it changes
//! nex-host kill ID                                   end one
//! nex-host forget ID                                 drop an exited one from the list
//! nex-host shim install|remove|status                the `claude` / `codex` shim (see `shim.rs`)
//! nex-host connect                                   one connection to the daemon, over stdin/stdout
//! nex-host spawn|send|done|recv|read|pipe|skill      agents working together (see `agentcli.rs`)
//! nex-host --version
//! ```

#[cfg(unix)]
mod agent;
#[cfg(unix)]
mod agentcli;
#[cfg(unix)]
mod attach;
#[cfg(unix)]
mod broker;
#[cfg(unix)]
mod client;
#[cfg(unix)]
mod daemon;
#[cfg(unix)]
mod deliver;
#[cfg(unix)]
mod locked;
#[cfg(unix)]
mod pipeline;
#[cfg(unix)]
mod redraw;
#[cfg(unix)]
mod shim;
#[cfg(unix)]
mod skill;

#[cfg(not(unix))]
fn main() {
    eprintln!("nex-host runs on Linux and macOS");
    std::process::exit(1);
}

#[cfg(unix)]
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("nex-host: {e}");
            std::process::exit(1);
        }
    }
}

#[cfg(unix)]
const HELP: &str = "nex-host keeps coding agents running after the nex pane they were started in closes.

  nex-host run [--name N] [--cwd DIR] -- CMD ...   start an agent and show it here
  nex-host launch [--kind K] [--name N] [--cwd DIR] [--cols C --rows R] -- CMD ...
                                                   start one in the background; prints its id
  nex-host attach ID                               show it here (closing the terminal leaves it running)
  nex-host list [--json]                           what's running
  nex-host watch                                   the list as JSON lines, whenever it changes
  nex-host kill ID                                 end one
  nex-host forget ID                               drop an exited one from the list
  nex-host shim install|remove|status              make `claude` and `codex` typed in nex's ssh
                                                   panes start through nex-host
  nex-host connect                                 pass one connection to the daemon through
                                                   stdin and stdout (nex's own ssh client uses it)

Agents working together (nex-host skill prints the manual for agents):
  nex-host spawn NAME [--kind claude|codex] [--role R] [--cwd DIR] [--worktree] TASK [-- CMD ...]
                                                   start an agent on a task; it reports back with done
  nex-host send TO TEXT [--ask | --task] [--reply-to ID] [--files a,b] [--force]
                                                   message an agent (typed into its input)
  nex-host done TEXT [--verdict approved|changes|failed] [--files a,b]
                                                   report on your task to whoever gave it
  nex-host recv [--from WHO] [--reply-to ID] [--kind K] [--timeout 10m] [--json]
                                                   wait for a message to you (for scripts)
  nex-host read WHO [--lines N]                    an agent's screen as text
  nex-host pipe NAME TASK [--bg] | pipe ls         run a pipeline (.nex/pipelines/NAME.toml)
  nex-host skill [install|remove]                  the manual, or install it for Claude Code and Codex
  TO / WHO: an id, a name, @parent, @children, @task or user. TEXT `-`: read from stdin.

Everything lives in ~/.nex (or $NEX_HOME); remove that folder to remove nex-host.";

#[cfg(unix)]
const USAGE: &str =
    "usage: nex-host run|launch|attach|list|spawn|send|done|recv|read|pipe|skill|watch|kill|forget|shim|connect|--version (see nex-host help)";

#[cfg(unix)]
fn run(args: &[String]) -> std::io::Result<i32> {
    use std::io;

    use nex_host::proto::{self, Request};

    let (cmd, rest) = match args.split_first() {
        Some((c, r)) => (c.as_str(), r),
        None => {
            eprintln!("{USAGE}");
            return Ok(2);
        }
    };
    match cmd {
        "--version" | "-V" | "version" => {
            println!("{}", proto::version_line());
            Ok(0)
        }
        "help" | "--help" | "-h" => {
            println!("{HELP}");
            Ok(0)
        }
        "daemon" => daemon::run().map(|_| 0),
        "launch" | "run" => {
            let opts = Launch::parse(rest)?;
            let (cols, rows) = if cmd == "run" { terminal_size().unwrap_or((120, 40)) } else { (opts.cols.unwrap_or(120), opts.rows.unwrap_or(40)) };
            let cwd = opts.cwd.or_else(|| std::env::current_dir().ok().map(|d| d.to_string_lossy().into_owned()));
            // `run` is typed into a shell: the agent gets that shell's environment.
            let env = (cmd == "run").then(|| std::env::vars().collect());
            let r = client::request(&Request::Launch { argv: opts.argv, kind: opts.kind, name: opts.name, cwd, cols: Some(cols), rows: Some(rows), env }, true)?;
            let id = r.id.unwrap_or_default();
            if cmd == "launch" {
                println!("{id}");
                return Ok(0);
            }
            attach::run(&id)
        }
        "attach" => attach::run(one_id(rest)?),
        "list" | "ls" => {
            let sessions = match client::request(&Request::List, false) {
                Ok(r) => r.sessions.unwrap_or_default(),
                // No daemon: nothing is running.
                Err(e) if e.kind() != io::ErrorKind::Other => Vec::new(),
                Err(e) => return Err(e),
            };
            if rest.iter().any(|a| a == "--json") {
                println!("{}", serde_json::to_string(&proto::Response::sessions(sessions)).map_err(io::Error::other)?);
                return Ok(0);
            }
            if sessions.is_empty() {
                println!("No agents running.");
            }
            for line in agentcli::tree(&sessions) {
                println!("{line}");
            }
            Ok(0)
        }
        "spawn" => agentcli::spawn(rest),
        "send" => agentcli::send(rest),
        "done" => agentcli::done(rest),
        "recv" => agentcli::recv(rest),
        "read" => agentcli::read(rest),
        "pipe" => agentcli::pipe(rest),
        "pipe-run" => agentcli::pipe_run(rest),
        "skill" => skill::run(rest),
        "watch" => watch(),
        "connect" => connect(),
        "shim" => {
            let paths = match rest.first().map(String::as_str) {
                Some("install") => shim::install()?,
                Some("remove") => shim::remove()?,
                Some("status") | None => {
                    println!("{}", if shim::installed() { "on" } else { "off" });
                    return Ok(0);
                }
                Some(other) => return Err(io::Error::new(io::ErrorKind::InvalidInput, format!("shim {other}? (install, remove or status)"))),
            };
            for p in paths {
                println!("{}", p.display());
            }
            Ok(0)
        }
        "kill" => client::request(&Request::Kill { id: one_id(rest)?.to_string() }, false).map(|_| 0),
        "forget" => client::request(&Request::Forget { id: one_id(rest)?.to_string() }, false).map(|_| 0),
        other => {
            eprintln!("nex-host: unknown command `{other}`\n{USAGE}");
            Ok(2)
        }
    }
}

/// `watch`: the session list as JSON lines, forever. It doesn't start the daemon (that would
/// keep one running for nothing): while none runs it reports no sessions and looks again
/// every couple of seconds, and when the daemon exits it goes back to that.
#[cfg(unix)]
fn watch() -> std::io::Result<i32> {
    use std::io::{self, Write};
    use std::time::{Duration, Instant};

    use nex_host::proto::{self, Request, Response};

    let mut out = io::stdout().lock();
    let mut idle_sent: Option<Instant> = None;
    loop {
        match client::connect(false) {
            Ok(mut stream) => {
                idle_sent = None;
                let first = client::ask(&mut stream, &Request::Watch)?;
                writeln!(out, "{}", serde_json::to_string(&first).map_err(io::Error::other)?)?;
                out.flush()?;
                // Line by line, so each one reaches nex as soon as it's sent.
                let mut lines = io::BufReader::new(stream);
                let mut line = String::new();
                while io::BufRead::read_line(&mut lines, &mut line)? > 0 {
                    out.write_all(line.as_bytes())?;
                    out.flush()?;
                    line.clear();
                }
            }
            Err(_) => {
                if idle_sent.is_none_or(|t| t.elapsed() >= Duration::from_secs(proto::HEARTBEAT_SECS)) {
                    writeln!(out, "{}", serde_json::to_string(&Response::sessions(Vec::new())).map_err(io::Error::other)?)?;
                    out.flush()?;
                    idle_sent = Some(Instant::now());
                }
                std::thread::sleep(Duration::from_secs(2));
            }
        }
    }
}

/// `connect`: one connection to the daemon (started if it isn't running), carried over stdin
/// and stdout, for a client on an ssh channel without a terminal. It speaks the protocol
/// itself (see `proto`); this ends when either side hangs up.
#[cfg(unix)]
fn connect() -> std::io::Result<i32> {
    use std::io::{self, Read, Write};

    let stream = client::connect(true)?;
    let mut to_daemon = stream.try_clone()?;
    std::thread::spawn(move || {
        let _ = io::copy(&mut io::stdin().lock(), &mut to_daemon);
        // The client is done sending; the daemon may still have output for it.
        let _ = to_daemon.shutdown(std::net::Shutdown::Write);
    });
    let mut from_daemon = stream;
    let mut out = io::stdout().lock();
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        match from_daemon.read(&mut buf) {
            Ok(0) => return Ok(0),
            Ok(n) => {
                // Each chunk goes out at once: it's a terminal's output.
                out.write_all(&buf[..n])?;
                out.flush()?;
            }
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
}

#[cfg(unix)]
fn one_id(rest: &[String]) -> std::io::Result<&str> {
    match rest {
        [id] => Ok(id),
        _ => Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "expected one session id")),
    }
}

#[cfg(unix)]
fn terminal_size() -> Option<(u16, u16)> {
    let mut ws: libc::winsize = unsafe { std::mem::zeroed() };
    let r = unsafe { libc::ioctl(0, libc::TIOCGWINSZ as _, &mut ws) };
    (r == 0 && ws.ws_col > 0).then_some((ws.ws_col, ws.ws_row))
}

/// `launch` / `run`'s options, then the command (after `--`, or from the first word that
/// isn't an option).
#[cfg(unix)]
#[derive(Debug, Default, PartialEq)]
struct Launch {
    kind: Option<String>,
    name: Option<String>,
    cwd: Option<String>,
    cols: Option<u16>,
    rows: Option<u16>,
    argv: Vec<String>,
}

#[cfg(unix)]
impl Launch {
    fn parse(args: &[String]) -> std::io::Result<Launch> {
        let bad = |m: String| std::io::Error::new(std::io::ErrorKind::InvalidInput, m);
        let mut o = Launch::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            let mut value = |name: &str| it.next().cloned().ok_or_else(|| bad(format!("{name} needs a value")));
            match a.as_str() {
                "--" => {
                    o.argv = it.cloned().collect();
                    break;
                }
                "--kind" => o.kind = Some(value("--kind")?),
                "--name" => o.name = Some(value("--name")?),
                "--cwd" => o.cwd = Some(value("--cwd")?),
                "--cols" => o.cols = Some(value("--cols")?.parse().map_err(|_| bad("--cols takes a number".into()))?),
                "--rows" => o.rows = Some(value("--rows")?.parse().map_err(|_| bad("--rows takes a number".into()))?),
                s if s.starts_with("--") => return Err(bad(format!("unknown option {s}"))),
                _ => {
                    o.argv = std::iter::once(a).chain(it).cloned().collect();
                    break;
                }
            }
        }
        if o.argv.is_empty() {
            return Err(bad("nothing to run (e.g. nex-host run -- claude)".into()));
        }
        Ok(o)
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn v(s: &[&str]) -> Vec<String> {
        s.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn launch_options_then_the_command() {
        let o = Launch::parse(&v(&["--name", "fix", "--cwd", "~/x", "--", "claude", "--model", "opus"])).unwrap();
        assert_eq!(o.name.as_deref(), Some("fix"));
        assert_eq!(o.cwd.as_deref(), Some("~/x"));
        assert_eq!(o.argv, v(&["claude", "--model", "opus"]));
        let o = Launch::parse(&v(&["codex", "--full-auto"])).unwrap();
        assert_eq!(o.argv, v(&["codex", "--full-auto"]), "options after the command are its own");
        assert!(Launch::parse(&v(&["--name", "x"])).is_err());
        assert!(Launch::parse(&v(&["--bogus", "claude"])).is_err());
    }
}
