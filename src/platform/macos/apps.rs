//! "Open With": the installed apps (`/Applications` and friends), started with `open -a`.

use std::path::{Path, PathBuf};

use super::super::AppChoice;
use super::super::unix::{file_mime, spawn_detached};

pub fn mime_type(path: &Path) -> Option<String> {
    file_mime(path)
}

/// Every installed app, by name. macOS has no simple per-type list to read, so this isn't
/// filtered by the file's type.
pub fn apps_for(_path: &Path) -> Vec<AppChoice> {
    let mut dirs: Vec<PathBuf> = ["/Applications", "/Applications/Utilities", "/System/Applications", "/System/Applications/Utilities"]
        .into_iter()
        .map(PathBuf::from)
        .collect();
    if let Some(home) = super::home_dir() {
        dirs.push(home.join("Applications"));
    }
    let mut apps: Vec<AppChoice> = dirs
        .iter()
        .filter_map(|d| std::fs::read_dir(d).ok())
        .flatten()
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.extension().is_some_and(|x| x == "app"))
        .filter_map(|p| {
            let name = p.file_stem()?.to_string_lossy().into_owned();
            Some(AppChoice { name, command: p.to_string_lossy().into_owned(), default: false })
        })
        .collect();
    apps.sort_by_key(|a| a.name.to_lowercase());
    apps.dedup_by(|a, b| a.name == b.name);
    apps
}

pub fn open_with(app: &AppChoice, path: &Path) {
    let argv = ["open".to_string(), "-a".into(), app.command.clone(), path.to_string_lossy().into_owned()];
    spawn_detached(&argv, None);
}
