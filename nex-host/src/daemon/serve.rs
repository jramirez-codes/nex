//! One connection: its request, and the answer (or, for `watch` and `attach`, the stream
//! that follows).

use std::io::{self, BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::{Duration, Instant};

use nex_host::paths;
use nex_host::proto::{self, Request, Response, USER};

use crate::daemon::agent::{self, Agent, Spec};
use crate::daemon::broker::{self, Filter};
use crate::daemon::lifecycle::{Hire, launch, spawn};
use crate::daemon::mail::{Draft, post};
use crate::daemon::retire::own_version;
use crate::daemon::{State, new_id};

/// How often `watch` looks for changes.
const WATCH_TICK: Duration = Duration::from_millis(500);
/// The longest a `recv` waits.
const RECV_MAX_MS: u64 = 3600 * 1000;

fn send(stream: &mut UnixStream, r: &Response) -> io::Result<()> {
    stream.write_all(&response_line(r)?)
}

/// `r` as the line it goes over the socket as.
fn response_line(r: &Response) -> io::Result<Vec<u8>> {
    let mut line = serde_json::to_string(r).map_err(io::Error::other)?;
    line.push('\n');
    Ok(line.into_bytes())
}

pub(super) fn serve(state: &Arc<State>, stream: UnixStream) -> io::Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut line = String::new();
    reader.read_line(&mut line)?;
    let mut stream = stream;
    let request: Request = match serde_json::from_str(line.trim()) {
        Ok(r) => r,
        Err(e) => return send(&mut stream, &Response::error(format!("bad request: {e}"))),
    };
    let starts = matches!(request, Request::Launch { .. } | Request::Spawn { .. });
    if starts && state.retiring.load(Ordering::Relaxed) {
        return send(&mut stream, &Response::error(proto::RESTARTING));
    }
    match request {
        Request::Version => send(&mut stream, &Response { ok: true, text: Some(own_version()), ..Response::default() }),
        Request::Launch { argv, kind, name, cwd, cols, rows, env, parent } => {
            let id = new_id(state);
            let spec = Spec {
                argv,
                kind,
                name,
                cwd: cwd.as_deref().map(paths::expand_home).unwrap_or_else(|| paths::expand_home("~")),
                cols: cols.unwrap_or(120),
                rows: rows.unwrap_or(40),
                env,
                parent: parent.and_then(|p| state.get(&p)).map(|p| p.id.clone()),
                role: None,
            };
            match launch(state, id.clone(), spec) {
                Ok(()) => send(&mut stream, &Response { ok: true, id: Some(id), ..Response::default() }),
                Err(e) => send(&mut stream, &Response::error(format!("couldn't start it: {e}"))),
            }
        }
        Request::Spawn { name, kind, argv, cwd, env, parent, role, task } => match spawn(state, Hire { name, kind, argv, cwd, env, parent, role, task }) {
            Ok(id) => send(&mut stream, &Response { ok: true, id: Some(id), ..Response::default() }),
            Err(e) => send(&mut stream, &Response::error(e)),
        },
        Request::Send { from, to, kind, payload, force } => {
            let out = post(state, Draft { from, to, kind, payload }, force);
            match out {
                Ok((ids, notes)) => {
                    let text = (!notes.is_empty()).then(|| notes.join("\n"));
                    send(&mut stream, &Response { ok: true, id: Some(ids.join(",")), text, ..Response::default() })
                }
                Err(e) => send(&mut stream, &Response::error(e)),
            }
        }
        Request::Recv { me, from, reply_to, kind, timeout_ms } => {
            let me = me.filter(|m| state.get(m).is_some()).unwrap_or_else(|| USER.into());
            // Exited agents count: one that ended without reporting still sends its `done`.
            // One forgotten since is matched on the id given.
            let from = from.map(|f| broker::lookup(&state.peers(), Some(&me), &f).unwrap_or(f));
            let filter = Filter { from, reply_to, kind };
            let timeout = Duration::from_millis(timeout_ms.unwrap_or(proto::RECV_DEFAULT_MS).min(RECV_MAX_MS));
            match state.broker.take(&me, &filter, timeout) {
                Some(m) => send(&mut stream, &Response { ok: true, messages: Some(vec![m]), ..Response::default() }),
                None => send(&mut stream, &Response::error("timed out: no message yet")),
            }
        }
        Request::Read { id, lines } => {
            match broker::lookup(&state.peers(), None, &id).and_then(|i| state.get(&i)) {
                Some(a) => send(&mut stream, &Response { ok: true, id: Some(a.id.clone()), text: Some(a.text(lines.unwrap_or(60))), ..Response::default() }),
                None => send(&mut stream, &Response::error(format!("no session {id}"))),
            }
        }
        Request::List => send(&mut stream, &Response::sessions(state.sessions())),
        Request::Watch => watch(state, &mut stream),
        Request::Kill { id } => match state.get(&id) {
            Some(a) if a.exited() => {
                state.remove(&id);
                send(&mut stream, &Response::ok())
            }
            Some(a) => {
                a.kill();
                send(&mut stream, &Response::ok())
            }
            None => send(&mut stream, &Response::error(format!("no session {id}"))),
        },
        Request::Forget { id } => match state.get(&id) {
            Some(a) if a.exited() => {
                state.remove(&id);
                send(&mut stream, &Response::ok())
            }
            Some(_) => send(&mut stream, &Response::error("it's still running: kill it first")),
            None => send(&mut stream, &Response::error(format!("no session {id}"))),
        },
        Request::Attach { id, cols, rows } => {
            let Some(agent) = state.get(&id) else {
                return send(&mut stream, &Response::error(format!("no session {id}")));
            };
            if agent.exited() {
                return send(&mut stream, &Response::error("it has exited"));
            }
            let token = state.next_token.fetch_add(1, Ordering::Relaxed);
            let tx = crate::daemon::pty::spawn_writer(stream.try_clone()?);
            // The reply goes out through the writer, as the agent takes the pane over.
            if agent.attach(&tx, response_line(&Response::ok())?, token, cols, rows).is_none() {
                let _ = tx.send(agent::Out::Bytes(response_line(&Response::error("it has exited"))?));
                return Ok(());
            }
            drop(tx);
            // Read through the buffer: a client may send keys right behind its request.
            let result = pump(&agent, token, &mut reader);
            agent.detach(token);
            result
        }
    }
}

/// Frames from an attached pane, until it hangs up.
fn pump(agent: &Agent, token: u64, stream: &mut impl Read) -> io::Result<()> {
    let mut head = [0u8; 5];
    let mut payload = Vec::new();
    loop {
        if stream.read_exact(&mut head).is_err() {
            return Ok(());
        }
        let len = u32::from_be_bytes([head[1], head[2], head[3], head[4]]) as usize;
        if len > proto::FRAME_MAX {
            return Err(io::Error::other("frame too large"));
        }
        payload.resize(len, 0);
        stream.read_exact(&mut payload)?;
        match head[0] {
            proto::FRAME_INPUT => agent.input(token, &payload),
            proto::FRAME_RESIZE if len == 4 => {
                let cols = u16::from_be_bytes([payload[0], payload[1]]);
                let rows = u16::from_be_bytes([payload[2], payload[3]]);
                agent.resize(token, cols, rows);
            }
            _ => {}
        }
    }
}

/// Send the session list now, again whenever it changes, and every heartbeat regardless.
fn watch(state: &State, stream: &mut UnixStream) -> io::Result<()> {
    let mut last = String::new();
    let mut sent = Instant::now();
    loop {
        let line = serde_json::to_string(&Response::sessions(state.sessions())).map_err(io::Error::other)?;
        if line != last || sent.elapsed() >= Duration::from_secs(proto::HEARTBEAT_SECS) {
            stream.write_all(line.as_bytes())?;
            stream.write_all(b"\n")?;
            last = line;
            sent = Instant::now();
        }
        std::thread::sleep(WATCH_TICK);
    }
}
