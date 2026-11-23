//! `pipe` and `pipe-run`: starting a pipeline (or joining the run of it that's going), and
//! running it inside the session `pipe` started.

use std::io;
use std::path::{Path, PathBuf};

use nex_host::proto::{self, MessageKind, Payload, Request};

use crate::agentcli::args::Args;
use crate::agentcli::{bad, me, text};
use crate::{client, pipeline};

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
            let parsed = pipeline::Pipeline::parse(&spec).map_err(|e| bad(format!("{}: {e}", file.display())))?;
            let task = text(task)?;
            let here = std::env::current_dir()?;
            // One run per pipeline and repository: a run that's going takes this task too.
            let going = if parsed.joinable() { client::request(&Request::List, false).ok().and_then(|r| r.sessions).unwrap_or_default() } else { Vec::new() };
            if let Some(run) = pipeline::joins(&going, &file, &here, &repo_top) {
                client::request(
                    &Request::Send { from: me(), to: run.id.clone(), kind: MessageKind::Task, payload: Payload::text(task), force: false },
                    true,
                )?;
                if a.flag("--bg") {
                    println!("{}", run.id);
                    return Ok(0);
                }
                eprintln!("Joined the {} run that's going ({}).", run.name, run.id);
                return crate::attach::run(&run.id);
            }
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
                    // Shown under the agent that asked for it (if one did).
                    parent: me(),
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
    // Enter, as it's pressed (a closed stdin never counts as one).
    let (tx, enter) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut line = String::new();
        while io::stdin().read_line(&mut line).is_ok_and(|n| n > 0) {
            if tx.send(()).is_err() {
                return;
            }
            line.clear();
        }
    });
    let ran = pipeline::run(Path::new(file), task, &enter).unwrap_or_else(|e| {
        println!("\n✗ {e}");
        pipeline::Ran { code: 1, ended: false }
    });
    if !ran.ended {
        println!("\nThe stages' agents are still up (nex-host ls). Press Enter to end them and this.");
        let _ = enter.recv();
    }
    Ok(ran.code)
}

/// The top of the git repository `dir` is in, or `dir` itself.
fn repo_top(dir: &Path) -> PathBuf {
    std::process::Command::new("git")
        .arg("-C")
        .arg(dir)
        .args(["rev-parse", "--path-format=absolute", "--git-common-dir"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| PathBuf::from(String::from_utf8_lossy(&o.stdout).trim()))
        .unwrap_or_else(|| dir.to_path_buf())
}
