//! "Open With": the apps whose desktop entries (`.desktop` files) say they open a file's
//! MIME type, and starting one of them on the file.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;

use super::super::AppChoice;
use super::super::unix::{command_output, file_mime, spawn_detached};

/// The file's MIME type, from the desktop's database (`xdg-mime`), else from `file(1)`.
pub fn mime_type(path: &Path) -> Option<String> {
    command_output(Command::new("xdg-mime").args(["query", "filetype"]).arg(path)).or_else(|| file_mime(path))
}

/// Apps that open `path`, the default one first, then by name.
pub fn apps_for(path: &Path) -> Vec<AppChoice> {
    let Some(mime) = mime_type(path) else { return Vec::new() };
    let default = command_output(Command::new("xdg-mime").args(["query", "default", &mime]));
    // Plain-text editors can open any text type (source code is `text/x-rust` and so on).
    let mut wanted = vec![mime.clone()];
    if mime.starts_with("text/") && mime != "text/plain" {
        wanted.push("text/plain".into());
    }
    let mut seen = HashSet::new();
    let mut apps = Vec::new();
    for dir in app_dirs() {
        for (id, file) in desktop_files(&dir) {
            // The first entry with an id wins: one in ~/.local hides the system's.
            if !seen.insert(id.clone()) {
                continue;
            }
            let Some(entry) = std::fs::read_to_string(&file).ok().and_then(|t| parse_entry(&t)) else { continue };
            let is_default = default.as_deref() == Some(id.as_str());
            if !entry.mime_types.iter().any(|m| wanted.contains(m)) || (entry.hidden && !is_default) {
                continue;
            }
            apps.push(AppChoice { name: entry.name, command: entry.exec, default: is_default });
        }
    }
    apps.sort_by(|a, b| b.default.cmp(&a.default).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    apps.dedup_by(|a, b| a.name == b.name && a.command == b.command);
    apps
}

/// Start `app` on `path`.
pub fn open_with(app: &AppChoice, path: &Path) {
    spawn_detached(&exec_argv(&app.command, path), path.parent());
}

/// `$XDG_DATA_HOME/applications`, then each of `$XDG_DATA_DIRS`.
fn app_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("XDG_DATA_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| super::home_dir().map(|h| h.join(".local/share")));
    let data = std::env::var("XDG_DATA_DIRS").ok().filter(|s| !s.is_empty()).unwrap_or_else(|| "/usr/local/share:/usr/share".into());
    home.into_iter().chain(data.split(':').filter(|s| !s.is_empty()).map(PathBuf::from)).map(|d| d.join("applications")).collect()
}

/// `(desktop id, path)` for each `.desktop` file under `dir`. Files in subfolders get ids
/// like `kde4-okular.desktop`.
fn desktop_files(dir: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    let mut stack = vec![(dir.to_path_buf(), String::new())];
    while let Some((d, prefix)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.filter_map(Result::ok) {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if e.file_type().is_ok_and(|t| t.is_dir()) {
                stack.push((path, format!("{prefix}{name}-")));
            } else if name.ends_with(".desktop") {
                out.push((format!("{prefix}{name}"), path));
            }
        }
    }
    out.sort();
    out
}

#[derive(Debug, PartialEq)]
struct DesktopEntry {
    name: String,
    exec: String,
    mime_types: Vec<String>,
    /// `NoDisplay` or `Hidden`: not offered unless it's the default app.
    hidden: bool,
}

/// The `[Desktop Entry]` group of an application that runs outside a terminal.
fn parse_entry(text: &str) -> Option<DesktopEntry> {
    let (mut name, mut exec, mut mimes) = (None, None, Vec::new());
    let (mut app, mut hidden, mut terminal, mut in_group) = (false, false, false, false);
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            in_group = line == "[Desktop Entry]";
            continue;
        }
        let Some((key, value)) = line.split_once('=').filter(|_| in_group) else { continue };
        let value = value.trim();
        match key.trim() {
            "Type" => app = value == "Application",
            "Name" => name = Some(value.to_string()),
            "Exec" => exec = Some(value.to_string()),
            "MimeType" => mimes = value.split(';').filter(|m| !m.is_empty()).map(str::to_string).collect(),
            "NoDisplay" | "Hidden" if value == "true" => hidden = true,
            "Terminal" => terminal = value == "true",
            _ => {}
        }
    }
    if !app || terminal {
        return None;
    }
    Some(DesktopEntry { name: name?, exec: exec?, mime_types: mimes, hidden })
}

/// The command line for an `Exec=` value: quoted arguments are unquoted, `%f %F %u %U` become
/// the file (added at the end if there's none), `%%` is a percent sign and other field codes
/// are dropped.
fn exec_argv(exec: &str, path: &Path) -> Vec<String> {
    let file = path.to_string_lossy();
    let mut argv = Vec::new();
    let mut used_file = false;
    for token in split_exec(exec) {
        if matches!(token.as_str(), "%f" | "%F" | "%u" | "%U") {
            argv.push(file.to_string());
            used_file = true;
            continue;
        }
        let mut arg = String::new();
        let mut chars = token.chars();
        while let Some(c) = chars.next() {
            if c != '%' {
                arg.push(c);
                continue;
            }
            match chars.next() {
                Some('%') => arg.push('%'),
                Some('f' | 'F' | 'u' | 'U') => {
                    arg.push_str(&file);
                    used_file = true;
                }
                _ => {}
            }
        }
        if !arg.is_empty() {
            argv.push(arg);
        }
    }
    if !used_file && !argv.is_empty() {
        argv.push(file.to_string());
    }
    argv
}

/// Split an `Exec=` value into arguments (double quotes group, backslash escapes).
fn split_exec(exec: &str) -> Vec<String> {
    let (mut out, mut cur, mut quoted, mut any) = (Vec::new(), String::new(), false, false);
    let mut chars = exec.chars();
    while let Some(c) = chars.next() {
        match c {
            '"' => {
                quoted = !quoted;
                any = true;
            }
            '\\' => {
                // `\\` is also how the desktop file format writes a single backslash.
                if let Some(n) = chars.next() {
                    cur.push(n);
                    any = true;
                }
            }
            c if c.is_whitespace() && !quoted => {
                if any {
                    out.push(std::mem::take(&mut cur));
                    any = false;
                }
            }
            c => {
                cur.push(c);
                any = true;
            }
        }
    }
    if any {
        out.push(cur);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exec_field_codes() {
        let p = Path::new("/tmp/a b.png");
        assert_eq!(exec_argv("eog %U", p), ["eog", "/tmp/a b.png"]);
        assert_eq!(exec_argv("gimp-2.10 %f --flag", p), ["gimp-2.10", "/tmp/a b.png", "--flag"]);
        assert_eq!(exec_argv("app --icon %i --name=%c", p), ["app", "--icon", "--name=", "/tmp/a b.png"]);
        assert_eq!(exec_argv("app --file=%f 100%%", p), ["app", "--file=/tmp/a b.png", "100%"]);
        assert_eq!(exec_argv(r#""/opt/My App/run" "%F""#, p), ["/opt/My App/run", "/tmp/a b.png"]);
        assert_eq!(exec_argv("viewer", p), ["viewer", "/tmp/a b.png"]);
    }

    #[test]
    fn parses_desktop_entries() {
        let text = "[Desktop Entry]\nType=Application\nName=Image Viewer\nName[de]=Bildbetrachter\nExec=eog %U\nMimeType=image/png;image/jpeg;\n\n[Desktop Action new]\nName=New Window\nExec=eog --new\n";
        let e = parse_entry(text).unwrap();
        assert_eq!(e.name, "Image Viewer");
        assert_eq!(e.exec, "eog %U");
        assert_eq!(e.mime_types, ["image/png", "image/jpeg"]);
        assert!(!e.hidden);
        assert!(parse_entry("[Desktop Entry]\nType=Application\nName=Vim\nExec=vim %F\nTerminal=true\n").is_none());
        assert!(parse_entry("[Desktop Entry]\nType=Link\nName=Site\nURL=x\n").is_none());
        assert!(parse_entry("[Desktop Entry]\nType=Application\nName=X\nExec=x\nNoDisplay=true\n").unwrap().hidden);
    }
}
