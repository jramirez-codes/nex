//! `nex-host shim`: make `claude` and `codex`, typed in a nex pane's ssh session, start
//! through `nex-host run`, so they keep running after the pane closes.
//!
//! It's a few marked lines in the shell's config: `~/.bashrc`, `~/.zshrc`, and a file of
//! its own for fish (`~/.config/fish/conf.d/nex-host.fish`). They only act when:
//! - the terminal is nex: `LC_TERMINAL=nex`, which nex sets in its panes and ssh passes on
//!   wherever it passes `LC_*` (Debian and Ubuntu do by default; elsewhere add
//!   `SendEnv LC_TERMINAL` to `~/.ssh/config`). Without it, `claude` runs as before;
//! - it isn't already inside a `nex-host` session (`NEX_HOST` is set there);
//! - `nex-host` is still installed.
//!
//! `command claude` skips it. `nex-host shim remove` takes the lines out again.

use std::io;
use std::path::{Path, PathBuf};

/// The first and last line of the block in bash and zsh files.
const BEGIN: &str = "# >>> nex-host >>>";
const END: &str = "# <<< nex-host <<<";

/// The programs the shim starts through `nex-host`.
const AGENTS: [&str; 2] = ["claude", "codex"];

fn sh_block() -> String {
    let mut s = format!(
        "{BEGIN}\n# Added by nex: in nex's ssh panes, claude and codex keep running after the pane closes.\n\
         # `command claude` skips it; `~/.nex/bin/nex-host shim remove` takes it out.\n\
         if [ -z \"$NEX_HOST\" ] && [ \"$LC_TERMINAL\" = nex ] && [ -x \"$HOME/.nex/bin/nex-host\" ]; then\n"
    );
    for a in AGENTS {
        // `function` so an alias of the same name can't break the definition.
        s.push_str(&format!("  function {a} {{ \"$HOME/.nex/bin/nex-host\" run -- {a} \"$@\"; }}\n"));
    }
    s.push_str(&format!("fi\n{END}\n"));
    s
}

fn fish_file() -> String {
    let mut s = String::from(
        "# Added by nex: in nex's ssh panes, claude and codex keep running after the pane closes.\n\
         # `command claude` skips it; `~/.nex/bin/nex-host shim remove` deletes this file.\n\
         if test -z \"$NEX_HOST\"; and test \"$LC_TERMINAL\" = nex; and test -x \"$HOME/.nex/bin/nex-host\"\n",
    );
    for a in AGENTS {
        s.push_str(&format!("    function {a}; \"$HOME/.nex/bin/nex-host\" run -- {a} $argv; end\n"));
    }
    s.push_str("end\n");
    s
}

/// The config files the shim goes in: each shell's file that exists, plus the login shell's
/// own even if it doesn't yet.
fn targets(home: &Path) -> (Vec<PathBuf>, Option<PathBuf>) {
    let shell = std::env::var("SHELL").unwrap_or_default();
    let shell = shell.rsplit('/').next().unwrap_or("");
    let mut rc = Vec::new();
    for (file, name) in [(".bashrc", "bash"), (".zshrc", "zsh")] {
        let p = home.join(file);
        if p.exists() || shell == name {
            rc.push(p);
        }
    }
    let fish_dir = home.join(".config/fish");
    let fish = (fish_dir.exists() || shell == "fish").then(|| fish_dir.join("conf.d/nex-host.fish"));
    (rc, fish)
}

fn home() -> io::Result<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).ok_or_else(|| io::Error::other("$HOME is unset"))
}

/// `text` without the block (and the blank line before it, if nex-host added one).
fn without_block(text: &str) -> String {
    without_marked(text, BEGIN, END)
}

/// `text` without the lines from `begin` to `end` (and the blank line before them).
pub fn without_marked(text: &str, begin: &str, end_mark: &str) -> String {
    let Some(start) = text.find(begin) else { return text.to_string() };
    let end = text[start..].find(end_mark).map_or(text.len(), |e| {
        let e = start + e + end_mark.len();
        if text[e..].starts_with('\n') { e + 1 } else { e }
    });
    let mut before = text[..start].to_string();
    if before.ends_with("\n\n") {
        before.pop();
    }
    before + &text[end..]
}

/// `text` with the block at its end (replacing an older one).
fn with_block(text: &str) -> String {
    with_marked(text, BEGIN, END, &sh_block())
}

/// `text` with `block` (which starts with `begin` and ends with `end`) at its end, in place
/// of an older one.
pub fn with_marked(text: &str, begin: &str, end: &str, block: &str) -> String {
    let mut out = without_marked(text, begin, end);
    if !out.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push('\n');
    }
    out + block
}

pub fn install() -> io::Result<Vec<PathBuf>> {
    let (rc, fish) = targets(&home()?);
    let mut done = Vec::new();
    for p in rc {
        let text = std::fs::read_to_string(&p).unwrap_or_default();
        let new = with_block(&text);
        if new != text {
            std::fs::write(&p, new)?;
        }
        done.push(p);
    }
    if let Some(p) = fish {
        if let Some(dir) = p.parent() {
            std::fs::create_dir_all(dir)?;
        }
        std::fs::write(&p, fish_file())?;
        done.push(p);
    }
    Ok(done)
}

pub fn remove() -> io::Result<Vec<PathBuf>> {
    let home = home()?;
    let mut done = Vec::new();
    for file in [".bashrc", ".zshrc"] {
        let p = home.join(file);
        let Ok(text) = std::fs::read_to_string(&p) else { continue };
        if text.contains(BEGIN) {
            std::fs::write(&p, without_block(&text))?;
            done.push(p);
        }
    }
    let fish = home.join(".config/fish/conf.d/nex-host.fish");
    if fish.exists() {
        std::fs::remove_file(&fish)?;
        done.push(fish);
    }
    Ok(done)
}

/// Whether the shim is in any of the shell config files.
pub fn installed() -> bool {
    let Ok(home) = home() else { return false };
    [".bashrc", ".zshrc"].iter().any(|f| std::fs::read_to_string(home.join(f)).is_ok_and(|t| t.contains(BEGIN)))
        || home.join(".config/fish/conf.d/nex-host.fish").exists()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_the_installed_program() {
        let program = format!("\"$HOME/{}\"", nex_host::paths::INSTALL_PATH);
        assert!(sh_block().contains(&format!("-x {program}")));
        assert!(fish_file().contains(&format!("-x {program}")));
    }

    #[test]
    fn the_block_goes_at_the_end_once_and_comes_back_out() {
        let mine = "export PATH=$HOME/bin:$PATH\nalias ll='ls -l'\n";
        let added = with_block(mine);
        assert!(added.starts_with(mine));
        assert!(added.ends_with(&sh_block()));
        assert_eq!(with_block(&added), added, "installing again changes nothing");
        assert_eq!(without_block(&added), mine, "removing gives the file back as it was");
        assert_eq!(with_block(""), sh_block());
        assert_eq!(without_block(mine), mine);
    }

    #[test]
    fn the_block_only_acts_in_nex_outside_nex_host() {
        let b = sh_block();
        assert!(b.contains("[ \"$LC_TERMINAL\" = nex ]"));
        assert!(b.contains("[ -z \"$NEX_HOST\" ]"));
        assert!(b.contains("function claude { \"$HOME/.nex/bin/nex-host\" run -- claude \"$@\"; }"));
        assert!(fish_file().contains("function codex; \"$HOME/.nex/bin/nex-host\" run -- codex $argv; end"));
    }
}
