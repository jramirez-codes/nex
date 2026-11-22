//! The commands agents use to work with each other (`skill.rs` is their manual):
//!
//! ```text
//! nex-host ls                                          who's around, as a tree
//! nex-host spawn NAME [--kind K] [--role R] [--cwd D] [--worktree] TASK [-- CMD …]
//! nex-host send TO TEXT [--ask | --task] [--reply-to ID] [--files a,b] [--force]
//! nex-host done TEXT [--verdict approved|changes|failed] [--files a,b] [--to TO]
//! nex-host recv [--from WHO] [--reply-to ID] [--kind K] [--timeout 10m] [--json]
//! nex-host read WHO [--lines N]
//! nex-host pipe NAME TASK [--bg] | pipe ls
//! ```
//!
//! A TEXT or TASK of `-` is read from stdin. Who's sending comes from `$NEX_AGENT_ID`,
//! which nex-host sets in every agent it starts; without it, it's someone at a shell
//! (`user`), whose messages are taken with `recv`.

use std::io::{self, Read};
use std::path::{Path, PathBuf};

use nex_host::proto::{self, MessageKind, Request, Session, Verdict};

use crate::{client, deliver, pipeline};

/// This agent's session id, when it runs under nex-host.
fn me() -> Option<String> {
    std::env::var("NEX_AGENT_ID").ok().filter(|s| !s.is_empty())
}

fn bad(m: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, m.into())
}

/// Options, and the words that aren't (all words after `--` go in `rest`).
#[derive(Default)]
struct Args {
    words: Vec<String>,
    values: Vec<(String, String)>,
    flags: Vec<String>,
    rest: Vec<String>,
}

impl Args {
    fn parse(args: &[String], takes_value: &[&str], flags: &[&str]) -> io::Result<Args> {
        let mut out = Args::default();
        let mut it = args.iter();
        while let Some(a) = it.next() {
            match a.as_str() {
                "--" => {
                    out.rest = it.cloned().collect();
                    break;
                }
                o if takes_value.contains(&o) => {
                    let v = it.next().ok_or_else(|| bad(format!("{o} needs a value")))?;
                    out.values.push((o.to_string(), v.clone()));
                }
                o if flags.contains(&o) => out.flags.push(o.to_string()),
                o if o.starts_with("--") => return Err(bad(format!("unknown option {o}"))),
                _ => out.words.push(a.clone()),
            }
        }
        Ok(out)
    }

    fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    fn files(&self) -> Vec<String> {
        self.value("--files").map(|f| f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    }

    fn verdict(&self) -> io::Result<Option<Verdict>> {
        self.value("--verdict").map(|v| Verdict::parse(v).ok_or_else(|| bad("--verdict is approved, changes or failed"))).transpose()
    }
}

/// The words as one text, or stdin when it's `-`.
fn text(words: &[String]) -> io::Result<String> {
    if words == ["-"] {
        let mut s = String::new();
        io::stdin().read_to_string(&mut s)?;
        return Ok(s);
    }
    Ok(words.join(" "))
}

pub fn spawn(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--kind", "--role", "--cwd"], &["--worktree", "--json"])?;
    let (name, task) = a.words.split_first().ok_or_else(|| bad("usage: nex-host spawn NAME [--kind claude|codex] [--role R] [--worktree] \"TASK\""))?;
    let task = text(task)?;
    if task.trim().is_empty() {
        return Err(bad("no task: say what it should do (or pass - and pipe it in)"));
    }
    let mut cwd = match a.value("--cwd") {
        Some(c) => nex_host::paths::expand_home(c),
        None => std::env::current_dir()?,
    };
    if a.flag("--worktree") {
        cwd = worktree(&cwd, name)?;
    }
    let r = client::request(
        &Request::Spawn {
            name: name.clone(),
            kind: a.value("--kind").map(Into::into),
            argv: a.rest.clone(),
            cwd: Some(cwd.to_string_lossy().into_owned()),
            // So it finds what it would here: PATH, API keys.
            env: Some(std::env::vars().collect()),
            parent: me(),
            role: a.value("--role").map(Into::into),
            task,
        },
        true,
    )?;
    let id = r.id.unwrap_or_default();
    if a.flag("--json") {
        println!("{}", serde_json::json!({ "id": id, "cwd": cwd }));
        return Ok(0);
    }
    let name = session(&id).map_or_else(|| name.clone(), |s| s.name);
    println!("Started {name} ({id}) in {}.", nex_host::paths::tilde(&cwd));
    if me().is_some() {
        println!("Its report will arrive in your input as a <nex-message kind=\"done\">: no need to wait or poll for it.");
    } else {
        println!("Wait for its report with: nex-host recv --from {id}   (or watch it: nex-host attach {id})");
    }
    Ok(0)
}

pub fn send(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--reply-to", "--files", "--verdict", "--kind"], &["--ask", "--task", "--note", "--force", "--json"])?;
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
    deliver_one(to, kind, body, a.value("--reply-to").map(Into::into), a.verdict()?, a.files(), a.flag("--force"), a.flag("--json"))
}

pub fn done(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &["--verdict", "--files", "--to"], &["--json"])?;
    let body = text(&a.words)?;
    if body.trim().is_empty() {
        return Err(bad("usage: nex-host done \"what you did\" [--verdict approved|changes|failed] [--files a,b]"));
    }
    let to = a.value("--to").unwrap_or("@task").to_string();
    deliver_one(&to, MessageKind::Done, body, None, a.verdict()?, a.files(), false, a.flag("--json"))
}

#[allow(clippy::too_many_arguments)]
fn deliver_one(to: &str, kind: MessageKind, body: String, reply_to: Option<String>, verdict: Option<Verdict>, files: Vec<String>, force: bool, json: bool) -> io::Result<i32> {
    let r = client::request(&Request::Send { from: me(), to: to.into(), kind, body, reply_to, verdict, files, force }, true)?;
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
        None => std::time::Duration::from_secs(600),
    };
    let r = client::request(
        &Request::Recv { me: me(), from: a.value("--from").map(Into::into), reply_to: a.value("--reply-to").map(Into::into), kind, timeout_ms: Some(timeout.as_millis() as u64) },
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

/// `pipe NAME TASK`: run a pipeline in a session of its own and show it here (`--bg`: just
/// start it). `pipe ls`: the pipelines there are.
pub fn pipe(args: &[String]) -> io::Result<i32> {
    let a = Args::parse(args, &[], &["--bg"])?;
    match a.words.as_slice() {
        [ls] if ls == "ls" || ls == "list" => {
            let found = pipeline::list();
            if found.is_empty() {
                println!("No pipelines: put them in .nex/pipelines/NAME.toml or ~/.config/nex/pipelines/NAME.toml");
            }
            for (name, path) in found {
                let stages = std::fs::read_to_string(&path).ok().and_then(|t| pipeline::Pipeline::parse(&t).ok()).map_or_else(
                    || "(doesn't parse)".to_string(),
                    |p| p.groups().iter().map(|g| g.iter().map(|&i| p.stages[i].name.as_str()).collect::<Vec<_>>().join(" + ")).collect::<Vec<_>>().join(" → "),
                );
                println!("{name:<16} {stages}    {}", nex_host::paths::tilde(&path));
            }
            Ok(0)
        }
        [name, task @ ..] if !task.is_empty() => {
            let file = pipeline::find(name).ok_or_else(|| bad(format!("no pipeline {name} (see nex-host pipe ls)")))?;
            let spec = std::fs::read_to_string(&file)?;
            pipeline::Pipeline::parse(&spec).map_err(|e| bad(format!("{}: {e}", file.display())))?;
            let task = text(task)?;
            let exe = std::env::current_exe()?.to_string_lossy().into_owned();
            let cwd = std::env::current_dir()?.to_string_lossy().into_owned();
            let (cols, rows) = crate::terminal_size().unwrap_or((120, 40));
            let r = client::request(
                &Request::Launch {
                    argv: vec![exe, "pipe-run".into(), file.to_string_lossy().into_owned(), task],
                    kind: Some(proto::PIPELINE_KIND.into()),
                    name: Some(name.rsplit('/').next().unwrap_or(name).trim_end_matches(".toml").into()),
                    cwd: Some(cwd),
                    cols: Some(cols),
                    rows: Some(rows),
                    env: Some(std::env::vars().collect()),
                },
                true,
            )?;
            let id = r.id.unwrap_or_default();
            if a.flag("--bg") {
                println!("{id}");
                return Ok(0);
            }
            crate::attach::run(&id)
        }
        _ => Err(bad("usage: nex-host pipe NAME \"TASK\" [--bg]  |  nex-host pipe ls")),
    }
}

/// `pipe-run FILE TASK`, inside the session `pipe` started: run it, then keep the stages'
/// agents up until Enter (they end with this).
pub fn pipe_run(args: &[String]) -> io::Result<i32> {
    let [file, task] = args else { return Err(bad("usage: nex-host pipe-run FILE TASK")) };
    let code = pipeline::run(Path::new(file), task).unwrap_or_else(|e| {
        println!("\n✗ {e}");
        1
    });
    println!("\nThe stages' agents are still up (nex-host ls). Press Enter to end them and this.");
    let mut line = String::new();
    let _ = io::stdin().read_line(&mut line);
    Ok(code)
}

/// A new git worktree of the repository `dir` is in, on a branch of its own, for `name`.
pub fn worktree(dir: &Path, name: &str) -> io::Result<PathBuf> {
    let git = |args: &[&str]| std::process::Command::new("git").arg("-C").arg(dir).args(args).output();
    let top = git(&["rev-parse", "--show-toplevel"])?;
    if !top.status.success() {
        return Err(bad(format!("--worktree: {} isn't in a git repository", dir.display())));
    }
    let top = PathBuf::from(String::from_utf8_lossy(&top.stdout).trim());
    let repo = top.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| "repo".into());
    let stamp = crate::agent::now_ms() / 1000;
    let branch = format!("nex/{name}-{stamp}");
    let path = nex_host::paths::worktrees().ok_or_else(|| io::Error::other("no home folder"))?.join(format!("{repo}-{name}-{stamp}"));
    let out = git(&["worktree", "add", "-b", &branch, &path.to_string_lossy()])?;
    if !out.status.success() {
        return Err(io::Error::other(format!("git worktree add: {}", String::from_utf8_lossy(&out.stderr).trim())));
    }
    Ok(path)
}

fn session(id: &str) -> Option<Session> {
    client::request(&Request::List, false).ok()?.sessions?.into_iter().find(|s| s.id == id)
}

/// The sessions as a tree: each agent under the one that spawned it.
pub fn tree(sessions: &[Session]) -> Vec<String> {
    let home = std::env::var("HOME").unwrap_or_default();
    let me = me();
    let ids: Vec<&str> = sessions.iter().map(|s| s.id.as_str()).collect();
    let mut out = Vec::new();
    fn walk(s: &Session, all: &[Session], depth: usize, home: &str, me: Option<&str>, out: &mut Vec<String>) {
        let cwd = match s.cwd.strip_prefix(home) {
            Some(rest) if !home.is_empty() => format!("~{rest}"),
            _ => s.cwd.clone(),
        };
        let indent = if depth == 0 { String::new() } else { format!("{}└ ", "  ".repeat(depth - 1)) };
        let role = s.role.as_ref().filter(|r| **r != s.name).map(|r| format!(" ({r})")).unwrap_or_default();
        let mut marks = String::new();
        if s.attached {
            marks.push_str("  (attached)");
        }
        if me == Some(s.id.as_str()) {
            marks.push_str("  ← you");
        }
        out.push(format!("{}  {:<9}  {:<8}  {indent}{}{role}  {cwd}{marks}", s.id, s.status.label(), s.kind, s.name));
        for c in all.iter().filter(|c| c.parent.as_deref() == Some(&s.id)) {
            walk(c, all, depth + 1, home, me, out);
        }
    }
    for s in sessions.iter().filter(|s| s.parent.as_deref().is_none_or(|p| !ids.contains(&p))) {
        walk(s, sessions, 0, &home, me.as_deref(), &mut out);
    }
    out
}

#[cfg(test)]
mod tests {
    use nex_host::proto::Status;

    use super::*;

    fn s(id: &str, name: &str, parent: Option<&str>) -> Session {
        Session {
            id: id.into(),
            kind: "claude".into(),
            name: name.into(),
            cwd: "/x".into(),
            argv: vec![],
            pid: 1,
            started: 0,
            title: None,
            bell: false,
            attached: false,
            last_output: 0,
            last_input: 0,
            exit: None,
            claude: None,
            status: Status::Idle,
            parent: parent.map(Into::into),
            role: None,
            last_message: 0,
        }
    }

    #[test]
    fn children_sit_under_their_parent() {
        let t = tree(&[s("a", "lead", None), s("b", "other", None), s("c", "build", Some("a")), s("d", "tests", Some("c")), s("e", "orphan", Some("gone"))]);
        let order: Vec<bool> = ["  lead  ", "  └ build  ", "    └ tests  ", "  other  ", "  orphan  "].iter().zip(&t).map(|(want, line)| line.contains(want)).collect();
        assert_eq!(order, [true; 5], "{t:#?}");
    }

    #[test]
    fn options_and_words() {
        let v = |s: &[&str]| s.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        let a = Args::parse(&v(&["rev", "look", "--ask", "at", "--files", "a, b", "--", "codex", "--x"]), &["--files"], &["--ask"]).unwrap();
        assert_eq!(a.words, v(&["rev", "look", "at"]));
        assert!(a.flag("--ask"));
        assert_eq!(a.files(), v(&["a", "b"]));
        assert_eq!(a.rest, v(&["codex", "--x"]));
        assert!(Args::parse(&v(&["--nope"]), &[], &[]).is_err());
        assert!(Args::parse(&v(&["--files"]), &["--files"], &[]).is_err());
    }
}
