//! The same `git` commands for a folder on an ssh host, batched so a status is one trip.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::git_diff::git::diff::{FileDiff, diff_args, parse_diff};
use crate::git_diff::git::status::{Change, FileChange, Status, attach_counts, parse_numstat, parse_status};
use crate::git_diff::git::{GIT_ENV, MAX_COUNT_BYTES, NUMSTAT_ARGS, NUMSTAT_STAGED_ARGS, STATUS_ARGS, TOPLEVEL_ARGS};
use crate::remote::exec::{ExecError, Transport};
use crate::remote::posix_quote;
use crate::remote::{RemoteError, cd_word};

/// Exit status of a remote script whose `cd` into the folder failed.
const NO_FOLDER: i32 = 3;

/// A command for the host's shell: `git args…` in `dir` (which may start with `~`), with
/// the same environment as [`run`](crate::git_diff::git::run) gives it here.
fn remote_git(dir: &str, args: &[&str]) -> String {
    let args: Vec<String> = args.iter().map(|a| posix_quote(a)).collect();
    let env: Vec<String> = GIT_ENV.iter().map(|(name, value)| format!("{name}={value}")).collect();
    format!("cd -- {} 2>/dev/null || exit {NO_FOLDER}; {} git {}", cd_word(dir), env.join(" "), args.join(" "))
}

/// What a remote failure looks like in the pane.
fn remote_failure(e: ExecError) -> Status {
    match e {
        ExecError::Remote(RemoteError::NeedsLogin) => Status::NeedsLogin,
        other => Status::Failed(other.to_string()),
    }
}

/// [`load_status`](crate::git_diff::git::status::load_status) for `dir` on the host `t` reaches: one trip for the status and the
/// counts, and one more for untracked files' line counts.
pub fn load_status_remote(t: &Transport, dir: &Path) -> Status {
    let dir = dir.to_string_lossy();
    let commands: Vec<String> =
        [&TOPLEVEL_ARGS[..], &STATUS_ARGS, &NUMSTAT_STAGED_ARGS, &NUMSTAT_ARGS].iter().map(|args| remote_git(&dir, args)).collect();
    let outs = match t.run_many(&commands) {
        Ok(outs) => outs,
        Err(e) => return remote_failure(e),
    };
    let [top, status, staged, unstaged] = outs.as_slice() else { return Status::Failed("The host's answer was cut short".into()) };
    match top.code {
        0 => {}
        127 => return Status::NoGit,
        NO_FOLDER => return Status::Failed(format!("{dir} isn't there any more")),
        _ => return Status::NotRepo,
    }
    if !status.ok() {
        return Status::Failed("git status failed".into());
    }
    let root = PathBuf::from(top.text().trim());
    let mut snap = parse_status(&status.text());
    snap.root = root.clone();
    let numstat = |o: &crate::remote::exec::Output| if o.ok() { parse_numstat(&o.text()) } else { Vec::new() };
    let untracked: Vec<String> =
        snap.unstaged.iter().filter(|f| f.change == Change::Untracked).map(|f| f.path.clone()).take(MAX_REMOTE_COUNTS).collect();
    let counts = count_lines_remote(t, &root, &untracked);
    let lines: HashMap<&str, u32> = untracked.iter().zip(counts).filter_map(|(path, n)| Some((path.as_str(), n?))).collect();
    attach_counts(&mut snap, numstat(staged), numstat(unstaged), |path| lines.get(path).copied());
    Status::Ready(snap)
}

/// Untracked files on a host whose lines are counted (more would make one huge command).
const MAX_REMOTE_COUNTS: usize = 500;

/// [`count_lines`](crate::git_diff::git::status::count_lines) for each of `files` (relative to `root`) on the host, in one trip.
fn count_lines_remote(t: &Transport, root: &Path, files: &[String]) -> Vec<Option<u32>> {
    if files.is_empty() {
        return Vec::new();
    }
    let script = count_script(&root.to_string_lossy(), files);
    let Ok(out) = t.run(&script) else { return Vec::new() };
    parse_counts(&out.text())
}

/// A script printing [`count_lines`](crate::git_diff::git::status::count_lines)'s answer for each of `files` in the folder `root`, one
/// line each: the count, or `-`.
fn count_script(root: &str, files: &[String]) -> String {
    let quoted: Vec<String> = files.iter().map(|f| posix_quote(f)).collect();
    format!(
        r#"cd -- {root} 2>/dev/null || exit {NO_FOLDER}
for f in {files}; do
  if [ -f "$f" ] && [ "$(wc -c <"$f" | tr -d ' ')" -le {MAX_COUNT_BYTES} ] && [ "$(head -c 8000 "$f" | tr -cd '\000' | wc -c | tr -d ' ')" = 0 ]; then
    n=$(wc -l <"$f" | tr -d ' '); [ -n "$(tail -c 1 "$f")" ] && n=$((n+1)); printf '%s\n' "$n"
  else echo -; fi
done"#,
        root = posix_quote(root),
        files = quoted.join(" "),
    )
}

/// One line per file: its line count, or `-` (binary, too big, not a file).
fn parse_counts(text: &str) -> Vec<Option<u32>> {
    text.lines().map(|l| l.trim().parse().ok()).collect()
}

/// [`load_diff`](crate::git_diff::git::diff::load_diff) for a file in the repository at `root` on the host `t` reaches.
pub fn load_diff_remote(t: &Transport, root: &Path, file: &FileChange) -> Result<FileDiff, String> {
    let args = diff_args(file, "/dev/null");
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = t.run(&remote_git(&root.to_string_lossy(), &args)).map_err(|e| e.to_string())?;
    match out.code {
        // `--no-index` exits with 1 when the files differ, which they always do here.
        0 | 1 => Ok(parse_diff(&out.text())),
        127 => Err("git not found on the host".into()),
        NO_FOLDER => Err("The repository isn't there any more".into()),
        code => Err(format!("git diff failed (exit status {code})")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::git_diff::git::status::count_lines;

    #[test]
    fn remote_git_commands() {
        assert_eq!(
            remote_git("~/my repo", &["diff", "--", "a b.txt"]),
            "cd -- ~/'my repo' 2>/dev/null || exit 3; GIT_PAGER=cat GIT_TERMINAL_PROMPT=0 GIT_OPTIONAL_LOCKS=0 LC_ALL=C git diff -- 'a b.txt'"
        );
    }

    #[test]
    fn counts_lines_like_here() {
        let dir = std::env::temp_dir().join(format!("nex-remote-counts-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("two"), "a\nb\n").unwrap();
        std::fs::write(dir.join("no newline"), "a\nb").unwrap();
        std::fs::write(dir.join("bin"), b"a\0b").unwrap();
        let files = ["two", "no newline", "bin", "missing"];
        // The same script as on a host, run here by `sh`.
        let script = count_script(&dir.to_string_lossy(), &files.map(String::from));
        let out = std::process::Command::new("sh").arg("-c").arg(script).output().unwrap();
        let counts = parse_counts(&String::from_utf8_lossy(&out.stdout));
        assert_eq!(counts, [Some(2), Some(2), None, None]);
        for f in ["two", "no newline"] {
            assert_eq!(counts[files.iter().position(|x| *x == f).unwrap()], count_lines(&dir.join(f)));
        }
        std::fs::remove_dir_all(dir).unwrap();
    }
}
