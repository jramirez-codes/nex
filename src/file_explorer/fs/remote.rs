//! The explorer's file system work on an ssh host (see `remote::exec`): the same operations
//! as `ops.rs`, as shell scripts run there. Paths are the host's own (absolute, `/`-separated).
//!
//! Listing uses GNU `find -printf` when the host has it (one process for the whole folder),
//! else a plain `sh` loop (BSD and busybox hosts).

use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;

use crate::file_explorer::Entry;
use crate::file_explorer::fs::ops::{invalid_name, sort_entries};
use crate::file_explorer::fs::transfer::{Op, Progress};
use crate::remote::exec::{ExecError, Output, Transport, cd_word, quote};
use crate::remote::posix_quote;

/// A folder as read on the host.
#[derive(Clone, Debug, PartialEq)]
pub struct Listing {
    /// The folder, as an absolute path (what was asked for may have started with `~`).
    pub dir: PathBuf,
    /// The home folder there.
    pub home: PathBuf,
    pub entries: Vec<Entry>,
}

/// Printed after each item an operation is done with (for progress, and the count).
const DONE: &str = "nex-done";
/// Printed for an item a move left where it was.
const SKIPPED: &str = "nex-skip";

/// List `dir` on the host (every entry; hidden ones are left out later).
pub fn list(t: &Transport, dir: &Path) -> Result<Listing, ExecError> {
    let out = t.run(&list_script(&dir.to_string_lossy()))?;
    if !out.ok() {
        return Err(ExecError::Failed(out.reason()));
    }
    parse_listing(&out.out).ok_or_else(|| ExecError::Failed("The host's answer was cut short".into()))
}

fn list_script(dir: &str) -> String {
    let d = cd_word(dir);
    format!(
        r#"[ -d {d} ] || {{ echo "There's no such folder"; exit 2; }}
cd -- {d} 2>/dev/null || {{ echo "Permission denied"; exit 2; }}
pwd
printf '%s\n' "$HOME"
if find . -maxdepth 0 -printf '' >/dev/null 2>&1; then
  find -L . -mindepth 1 -maxdepth 1 \( -xtype l -printf 'l ' -o -printf '- ' \) -printf '%y %s %m %f\0' 2>/dev/null
else
  for f in .* *; do
    case $f in .|..) continue;; esac
    [ -e "$f" ] || [ -L "$f" ] || continue
    l=-; [ -L "$f" ] && l=l
    if [ -d "$f" ]; then y=d; elif [ -e "$f" ]; then y=f; else y=l; fi
    s=0; [ "$y" = f ] && s=$(wc -c <"$f" 2>/dev/null | tr -d ' ')
    m=644; [ -x "$f" ] && m=755
    printf '%s %s %s %s %s\0' "$l" "$y" "${{s:-0}}" "$m" "$f"
  done
fi
exit 0
"#
    )
}

/// Read what [`list_script`] printed: the folder, home, then one NUL-ended record per entry,
/// `<l or -> <type> <size> <octal mode> <name>`, the type, size and mode of what a link
/// points to. Sorted like a local listing: folders first, then by name.
pub(in crate::file_explorer) fn parse_listing(out: &[u8]) -> Option<Listing> {
    let mut lines = out.splitn(3, |&b| b == b'\n');
    let dir = PathBuf::from(String::from_utf8_lossy(lines.next()?).into_owned());
    let home = PathBuf::from(String::from_utf8_lossy(lines.next()?).into_owned());
    let records = lines.next().unwrap_or_default();
    let mut entries: Vec<Entry> = records
        .split(|&b| b == 0)
        .filter(|r| !r.is_empty())
        .filter_map(|r| {
            let r = String::from_utf8_lossy(r);
            let mut parts = r.splitn(5, ' ');
            let (link, kind, size, mode, name) = (parts.next()?, parts.next()?, parts.next()?, parts.next()?, parts.next()?);
            if name.is_empty() || name == "." || name == ".." {
                return None;
            }
            let is_dir = kind == "d";
            let file = kind == "f";
            let mode = u32::from_str_radix(mode, 8).unwrap_or(0);
            Some(Entry {
                name: name.to_string(),
                path: dir.join(name),
                is_dir,
                // What a broken link would point to has no size.
                size: (!is_dir).then(|| if file { size.parse().unwrap_or(0) } else { 0 }),
                is_symlink: link == "l",
                executable: file && mode & 0o111 != 0,
            })
        })
        .collect();
    sort_entries(&mut entries);
    Some(Listing { dir, home, entries })
}

/// The shell lines that set `t` to a free name for `name` in `$dest` (`name`, else
/// `stem (2).ext`, `(3)`, …), the way `ops::unique_target` picks one.
fn free_name(name_var: &str) -> String {
    format!(
        r#"case ${name_var} in .*.*|[!.]*.*) stem=${{{name_var}%.*}}; ext=.${{{name_var}##*.}};; *) stem=${name_var}; ext=;; esac
  t="$dest/${name_var}"; n=2
  while [ -e "$t" ] || [ -L "$t" ]; do t="$dest/$stem ($n)$ext"; n=$((n+1)); done"#
    )
}

/// Copy or move `sources` into `dest` on the host (name clashes get a " (2)" suffix), as
/// `ops::run_op` does locally. Returns how many items were copied or moved.
pub fn run_op(t: &Transport, op: Op, sources: &[PathBuf], dest: &Path, progress: &Progress) -> Result<usize, String> {
    progress.total.store(sources.len() as u64, Ordering::Relaxed);
    let srcs: Vec<String> = sources.iter().map(|s| quote(s)).collect();
    let (moving, cmd) = match op {
        Op::Copy => (0, "cp -pPR"),
        Op::Move => (1, "mv"),
    };
    let script = format!(
        r#"exec 2>&1
dest={dest}
for src in {srcs}; do
  name=${{src##*/}}
  case "$dest/" in "$src/"*) printf '%s\n' "Can't put “$name” inside itself"; exit 3;; esac
  [ -e "$src" ] || [ -L "$src" ] || {{ printf '%s\n' "$name: it isn't there any more"; exit 1; }}
  if [ {moving} = 1 ] && [ "${{src%/*}}" = "$dest" ]; then echo {SKIPPED}; continue; fi
  {free}
  {cmd} -- "$src" "$t" || exit 1
  echo {DONE}
done"#,
        dest = quote(dest),
        srcs = srcs.join(" "),
        free = free_name("name"),
    );
    let (done, result) = job(t, &script, progress);
    match result {
        Ok(out) if out.ok() => Ok(done),
        Ok(out) => Err(out.reason()),
        Err(ExecError::Cancelled) => Ok(done),
        Err(e) => Err(e.to_string()),
    }
}

/// Permanently delete `paths` on the host. Returns how many were deleted.
pub fn delete_paths(t: &Transport, paths: &[PathBuf], progress: &Progress) -> Result<usize, String> {
    progress.total.store(paths.len() as u64, Ordering::Relaxed);
    let targets: Vec<String> = paths.iter().map(|p| quote(p)).collect();
    let script = format!("exec 2>&1\nfor p in {}; do rm -rf -- \"$p\" || exit 1; echo {DONE}; done", targets.join(" "));
    let (done, result) = job(t, &script, progress);
    match result {
        Ok(out) if out.ok() => Ok(done),
        Ok(out) => Err(out.reason()),
        Err(ExecError::Cancelled) => Ok(done),
        Err(e) => Err(e.to_string()),
    }
}

/// Run an operation's script as a job, counting the items it reports done into `progress`.
/// Returns the count with the result.
fn job(t: &Transport, script: &str, progress: &Progress) -> (usize, Result<Output, ExecError>) {
    let count = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        let done = text.lines().filter(|l| *l == DONE).count();
        let skipped = text.lines().filter(|l| *l == SKIPPED).count();
        (done, skipped)
    };
    let mut seen = (0, 0);
    let result = t.run_job(script, &progress.cancel, &mut |bytes| {
        seen = count(bytes);
        progress.done.store((seen.0 + seen.1) as u64, Ordering::Relaxed);
    });
    if let Ok(out) = &result {
        seen = count(&out.out);
    }
    (seen.0, result)
}

/// Rename `path` to `new_name` in its folder on the host. Never replaces an existing file.
pub fn rename_path(t: &Transport, path: &Path, new_name: &str) -> Result<PathBuf, String> {
    if let Some(why) = invalid_name(new_name) {
        return Err(why.into());
    }
    let target = path.with_file_name(new_name);
    if target == path {
        return Ok(target);
    }
    // A change of case only is the same file on case-insensitive disks.
    let same_file = path.file_name().is_some_and(|old| old.to_string_lossy().to_lowercase() == new_name.to_lowercase());
    let clash = posix_quote(&format!("“{new_name}” already exists"));
    let script = format!(
        "p={p}; t={t}\nif [ {check} = 1 ] && {{ [ -e \"$t\" ] || [ -L \"$t\" ]; }}; then printf '%s\\n' {clash}; exit 1; fi\nmv -- \"$p\" \"$t\" 2>&1",
        p = quote(path),
        t = quote(&target),
        check = if same_file { 0 } else { 1 },
    );
    let out = t.run(&script).map_err(|e| format!("Can't rename: {e}"))?;
    if out.ok() {
        Ok(target)
    } else if out.text().contains("already exists") {
        Err(out.reason())
    } else {
        Err(format!("Can't rename: {}", out.reason()))
    }
}

/// Make an empty folder ("New Folder", …) or file ("Untitled", …) in `dest` on the host.
pub fn create_new(t: &Transport, dest: &Path, folder: bool) -> Result<PathBuf, String> {
    let make = if folder { "mkdir -- \"$t\" 2>&1" } else { "( set -C; : >\"$t\" ) 2>&1" };
    let script = format!(
        "dest={dest}; name={name}\n  {free}\n{make} || exit 1\nprintf '%s\\n' \"$t\"",
        dest = quote(dest),
        name = if folder { "'New Folder'" } else { "Untitled" },
        free = free_name("name"),
    );
    let out = t.run(&script).map_err(|e| format!("Can't create it here: {e}"))?;
    if out.ok() {
        Ok(PathBuf::from(out.text().trim_end_matches('\n').rsplit('\n').next().unwrap_or_default()))
    } else {
        Err(format!("Can't create it here: {}", out.reason()))
    }
}

/// A file's details on the host, for the Properties dialog.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Details {
    /// Seconds since 1970.
    pub modified: Option<u64>,
    pub accessed: Option<u64>,
    pub created: Option<u64>,
    pub mode: Option<u32>,
    pub owner: Option<(String, String)>,
    pub link_target: Option<PathBuf>,
    pub mime: Option<String>,
}

/// Read `path`'s details (GNU `stat`, else BSD's).
pub fn details(t: &Transport, path: &Path) -> Result<Details, ExecError> {
    let script = format!(
        "p={p}\nstat -L -c '%Y %X %W %a %U %G' -- \"$p\" 2>/dev/null || stat -L -f '%m %a %B %Lp %Su %Sg' -- \"$p\" 2>/dev/null || echo\nif [ -L \"$p\" ]; then readlink -- \"$p\"; else echo; fi\nfile -b --mime-type -- \"$p\" 2>/dev/null || echo\nexit 0",
        p = quote(path)
    );
    let out = t.run(&script)?;
    Ok(parse_details(&out.text()))
}

pub(in crate::file_explorer) fn parse_details(text: &str) -> Details {
    let mut lines = text.lines();
    let stat: Vec<&str> = lines.next().unwrap_or_default().split_whitespace().collect();
    let time = |i: usize| stat.get(i).and_then(|s| s.parse::<u64>().ok()).filter(|&s| s > 0);
    let some = |s: &str| (!s.trim().is_empty()).then(|| s.trim().to_string());
    Details {
        modified: time(0),
        accessed: time(1),
        created: time(2),
        mode: stat.get(3).and_then(|m| u32::from_str_radix(m, 8).ok()),
        owner: stat.get(4).zip(stat.get(5)).map(|(u, g)| (u.to_string(), g.to_string())),
        link_target: lines.next().and_then(some).map(PathBuf::from),
        mime: lines.next().and_then(some),
    }
}

/// `(bytes, items)` inside the folder `path` on the host. Stops when `stop` is set (the
/// Properties dialog closed).
pub fn tree_summary(t: &Transport, path: &Path, stop: &std::sync::atomic::AtomicBool) -> Option<(u64, usize)> {
    let script = format!(
        "p={p}\nb=$(du -sb -- \"$p\" 2>/dev/null | cut -f1) || b=\nif [ -z \"$b\" ]; then k=$(du -sk -- \"$p\" 2>/dev/null | cut -f1); b=$(( ${{k:-0}} * 1024 )); fi\nn=$(find \"$p\" -mindepth 1 2>/dev/null | wc -l | tr -d ' ')\nprintf '%s %s\\n' \"$b\" \"$n\"",
        p = quote(path)
    );
    let out = t.run_job(&script, stop, &mut |_| {}).ok()?;
    let text = out.text();
    let (bytes, items) = text.trim().split_once(' ')?;
    Some((bytes.trim().parse().ok()?, items.trim().parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sh(script: &str, cwd: &Path) -> Vec<u8> {
        std::process::Command::new("sh").arg("-c").arg(script).current_dir(cwd).output().unwrap().stdout
    }

    #[test]
    fn reads_records() {
        let out = b"/srv/app\n/home/me\n- d 4096 755 src\0- f 12 644 b c.txt\0l f 5 755 run\0l l 7 777 broken\0- f 0 644 line\nbreak\0";
        let l = parse_listing(out).unwrap();
        assert_eq!(l.dir, PathBuf::from("/srv/app"));
        assert_eq!(l.home, PathBuf::from("/home/me"));
        let names: Vec<&str> = l.entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "b c.txt", "broken", "line\nbreak", "run"]);
        let src = &l.entries[0];
        assert!(src.is_dir && src.size.is_none() && !src.executable);
        assert_eq!(src.path, PathBuf::from("/srv/app/src"));
        let run = l.entries.iter().find(|e| e.name == "run").unwrap();
        assert!(run.is_symlink && run.executable && run.size == Some(5));
        let broken = l.entries.iter().find(|e| e.name == "broken").unwrap();
        assert!(broken.is_symlink && !broken.is_dir && !broken.executable);
    }

    #[test]
    fn an_empty_folder() {
        let l = parse_listing(b"/tmp/x\n/root\n").unwrap();
        assert!(l.entries.is_empty());
        assert_eq!(parse_listing(b"/tmp/x"), None);
    }

    /// Both ways of listing (GNU `find` and the `sh` loop) agree, run here by `sh`.
    #[test]
    fn listing_scripts_agree() {
        let dir = crate::file_explorer::tests::fixture("remote-list");
        std::fs::write(dir.join("has space"), b"12345").unwrap();
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("has space", dir.join("link")).unwrap();
            std::os::unix::fs::symlink("nowhere", dir.join("broken")).unwrap();
        }
        let script = list_script(&dir.to_string_lossy());
        let gnu = parse_listing(&sh(&script, &dir)).unwrap();
        // Without `-printf`, the same as on a BSD host.
        let fallback = parse_listing(&sh(&script.replace("-printf ''", "-no-such-test"), &dir)).unwrap();
        assert_eq!(gnu.dir, dir);
        assert_eq!(gnu.entries, fallback.entries);
        let spaced = gnu.entries.iter().find(|e| e.name == "has space").unwrap();
        assert_eq!(spaced.size, Some(5));
        assert!(gnu.entries.iter().any(|e| e.name == "beta" && e.is_dir));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_folder() {
        let out = sh(&list_script("/definitely/not/here"), Path::new("/"));
        assert_eq!(String::from_utf8_lossy(&out).trim(), "There's no such folder");
    }

    #[test]
    fn free_names_match_local_ones() {
        let dir = crate::file_explorer::tests::fixture("remote-free");
        std::fs::write(dir.join("a.txt"), b"").unwrap();
        std::fs::write(dir.join("a (2).txt"), b"").unwrap();
        let name_for = |name: &str| {
            let script = format!("dest={}; name={}\n  {}\nprintf '%s' \"$t\"", quote(&dir), posix_quote(name), free_name("name"));
            PathBuf::from(String::from_utf8(sh(&script, &dir)).unwrap())
        };
        for name in ["a.txt", "new", ".bashrc", "archive.tar.gz"] {
            assert_eq!(name_for(name), crate::file_explorer::fs::ops::unique_target(&dir, std::ffi::OsStr::new(name)), "{name}");
        }
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn reads_details() {
        let d = parse_details("1700000000 1700000100 0 755 me staff\n/target/file\ntext/plain\n");
        assert_eq!(d.modified, Some(1_700_000_000));
        assert_eq!(d.created, None);
        assert_eq!(d.mode, Some(0o755));
        assert_eq!(d.owner, Some(("me".into(), "staff".into())));
        assert_eq!(d.link_target, Some(PathBuf::from("/target/file")));
        assert_eq!(d.mime.as_deref(), Some("text/plain"));
        assert_eq!(parse_details("\n\n\n"), Details::default());
    }
}
