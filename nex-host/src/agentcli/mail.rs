//! Messages: `send`, `done`, `recv` and `read`.

use std::io;

use nex_host::proto::{self, MessageKind, Payload, Request};

use crate::agentcli::args::Args;
use crate::agentcli::{bad, me, text};
use crate::daemon::deliver;
use crate::{client, pipeline};

pub fn send(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--reply-to", "--files", "--verdict", "--kind", "--pr"], &["--ask", "--task", "--note", "--force", "--json"])?;
    let (to, body) = a.words.split_first().ok_or_else(|| bad("usage: nex-host send TO \"TEXT\" [--ask | --task] [--reply-to ID]"))?;
    let body = text(body)?;
    if body.trim().is_empty() {
        return Err(bad("nothing to send"));
    }
    let kind = match (a.value("--kind"), a.flag("--ask"), a.flag("--task"), a.flag("--note")) {
        (Some(k), ..) => MessageKind::parse(k).ok_or_else(|| bad("--kind is task, ask, reply, done or note"))?,
        (_, true, _, _) => MessageKind::Ask,
        (_, _, true, _) => MessageKind::Task,
        (_, _, _, true) => MessageKind::Note,
        _ if a.value("--reply-to").is_some() => MessageKind::Reply,
        _ => MessageKind::Note,
    };
    let payload = Payload { body, reply_to: a.value("--reply-to").map(Into::into), verdict: a.verdict()?, files: a.files(), pr: a.pr() };
    deliver_one(to, kind, payload, a.flag("--force"), a.flag("--json"))
}

pub fn done(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--verdict", "--files", "--to", "--pr"], &["--json"])?;
    let body = text(&a.words)?;
    if body.trim().is_empty() {
        return Err(bad("usage: nex-host done \"what you did\" [--verdict approved|changes|failed] [--files a,b] [--pr N|URL]"));
    }
    let to = a.value("--to").unwrap_or("@task").to_string();
    let payload = Payload { body, reply_to: None, verdict: a.verdict()?, files: a.files(), pr: a.pr() };
    deliver_one(&to, MessageKind::Done, payload, false, a.flag("--json"))
}

fn deliver_one(to: &str, kind: MessageKind, payload: Payload, force: bool, json: bool) -> io::Result<i32> {
    let r = client::request(&Request::Send { from: me(), to: to.into(), kind, payload, force }, true)?;
    let ids = r.id.unwrap_or_default();
    if json {
        println!("{}", serde_json::json!({ "id": ids, "note": r.text }));
        return Ok(0);
    }
    let what = if kind == MessageKind::Done { "Reported" } else { "Sent" };
    println!("{what} ({ids}) to {to}.");
    if let Some(note) = r.text {
        println!("{note}");
    }
    if kind == MessageKind::Ask && me().is_some() {
        println!("The answer will arrive in your input as a <nex-message kind=\"reply\" reply-to=\"{ids}\">.");
    }
    Ok(0)
}

pub fn recv(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--from", "--reply-to", "--kind", "--timeout"], &["--json"])?;
    let kind = a.value("--kind").map(|k| MessageKind::parse(k).ok_or_else(|| bad("--kind is task, ask, reply, done or note"))).transpose()?;
    let timeout = match a.value("--timeout") {
        Some(t) => pipeline::parse_duration(t).ok_or_else(|| bad("--timeout: like 30s, 10m or 1h"))?,
        None => std::time::Duration::from_millis(proto::RECV_DEFAULT_MS),
    };
    let r = client::request(
        &Request::Recv {
            me: me(),
            from: a.value("--from").map(Into::into),
            reply_to: a.value("--reply-to").map(Into::into),
            kind,
            timeout_ms: Some(timeout.as_millis() as u64),
        },
        true,
    );
    let m = match r {
        Ok(r) => r.messages.and_then(|m| m.into_iter().next()),
        Err(e) if e.to_string().starts_with("timed out") => {
            eprintln!("nex-host: nothing yet ({e})");
            return Ok(3);
        }
        Err(e) => return Err(e),
    };
    let Some(m) = m else { return Ok(3) };
    if a.flag("--json") {
        println!("{}", serde_json::to_string(&m).map_err(io::Error::other)?);
    } else {
        println!("{}", deliver::render(&m));
    }
    Ok(0)
}

pub fn read(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--lines"], &[])?;
    let [who] = a.words.as_slice() else { return Err(bad("usage: nex-host read WHO [--lines N]")) };
    let lines = a.value("--lines").map(|n| n.parse().map_err(|_| bad("--lines takes a number"))).transpose()?;
    let r = client::request(&Request::Read { id: who.clone(), lines }, false)?;
    println!("{}", r.text.unwrap_or_default());
    Ok(0)
}
