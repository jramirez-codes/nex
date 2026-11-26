//! Where pipeline files are found, and which run of one a new `pipe` joins.

use std::path::{Path, PathBuf};

/// The pipeline file `name` means: a path, else `.nex/pipelines/<name>.toml` in this folder
/// or one above it, else `~/.config/nex/pipelines/<name>.toml`.
pub fn find(name: &str) -> Option<PathBuf> {
    let p = Path::new(name);
    if p.is_file() {
        return std::fs::canonicalize(p).ok();
    }
    let file = format!("{name}.toml");
    let cwd = std::env::current_dir().ok()?;
    for dir in cwd.ancestors() {
        let candidate = dir.join(".nex/pipelines").join(&file);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    let config = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?;
    let candidate = config.join("nex/pipelines").join(&file);
    candidate.is_file().then_some(candidate)
}

/// Every pipeline `find` can see, by name.
pub fn list() -> Vec<(String, PathBuf)> {
    let mut dirs: Vec<PathBuf> = std::env::current_dir().map(|c| c.ancestors().map(|d| d.join(".nex/pipelines")).collect()).unwrap_or_default();
    if let Some(config) =
        std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from).or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))
    {
        dirs.push(config.join("nex/pipelines"));
    }
    let mut out: Vec<(String, PathBuf)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for e in entries.flatten() {
            let path = e.path();
            if path.extension().is_some_and(|x| x == "toml")
                && let Some(stem) = path.file_stem().map(|s| s.to_string_lossy().into_owned())
                && !out.iter().any(|(n, _)| *n == stem)
            {
                out.push((stem, path));
            }
        }
    }
    out.sort();
    out
}

/// Of `sessions`, the run of the pipeline in `file` that a `pipe` from `here` joins: a
/// live one, from the same repository (`top` says which a folder is in), the latest.
pub fn joins<'a>(
    sessions: &'a [nex_host::proto::Session],
    file: &Path,
    here: &Path,
    top: &dyn Fn(&Path) -> PathBuf,
) -> Option<&'a nex_host::proto::Session> {
    let file = file.to_string_lossy();
    let repo = top(here);
    sessions
        .iter()
        .filter(|s| s.kind == nex_host::proto::PIPELINE_KIND && s.exit.is_none())
        .filter(|s| s.argv.get(1).is_some_and(|a| a == "pipe-run") && s.argv.get(2).is_some_and(|f| *f == file))
        .filter(|s| top(Path::new(&s.cwd)) == repo)
        .max_by_key(|s| s.started)
}
