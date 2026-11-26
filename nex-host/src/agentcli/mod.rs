//! The commands agents use to work with each other (`skill.rs` is their manual):
//!
//! ```text
//! nex-host ls                                          who's around, as a tree
//! nex-host spawn NAME [--kind K] [--role R] [--cwd D] [--worktree] TASK [-- CMD …]
//! nex-host send TO TEXT [--ask | --task] [--reply-to ID] [--files a,b] [--pr N|URL] [--force]
//! nex-host done TEXT [--verdict approved|changes|failed] [--files a,b] [--pr N|URL] [--to TO]
//! nex-host recv [--from WHO] [--reply-to ID] [--kind K] [--timeout 10m] [--json]
//! nex-host read WHO [--lines N]
//! nex-host pipe NAME TASK [--bg] | pipe ls
//! nex-host worktree ls | prune [--all] [--dry-run]
//! ```
//!
//! A TEXT or TASK of `-` is read from stdin. Who's sending comes from `$NEX_AGENT_ID`,
//! which nex-host sets in every agent it starts; without it, it's someone at a shell
//! (`user`), whose messages are taken with `recv`.
//!
//! ```text
//! agentcli/
//!   mod.rs        this file: `spawn`, and what the commands share
//!   args.rs       the options and words a command was given
//!   mail.rs       send, done, recv, read
//!   pipe.rs       pipe, pipe-run
//!   worktree.rs   worktrees: --worktree, worktree ls | prune
//!   tree.rs       ls, as a tree
//! ```

use std::io::{self, Read};

use nex_host::proto::{Request, Session};

use crate::client;

mod args;
mod mail;
mod pipe;
mod tree;
mod worktree;

pub(crate) use args::Args;
pub use mail::{done, read, recv, send};
pub use pipe::{pipe, pipe_run};
pub use tree::tree;
pub use worktree::{worktree, worktrees};

/// This agent's session id, when it runs under nex-host.
fn me() -> Option<String> {
    std::env::var("NEX_AGENT_ID").ok().filter(|s| !s.is_empty())
}

fn bad(m: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, m.into())
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

fn session(id: &str) -> Option<Session> {
    client::request(&Request::List, false).ok()?.sessions?.into_iter().find(|s| s.id == id)
}
