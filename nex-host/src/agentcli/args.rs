//! The options and words a subcommand was given.

use std::io;

use nex_host::proto::Verdict;

use crate::agentcli::bad;

/// Options, and the words that aren't (all words after `--` go in `rest`).
#[derive(Default)]
pub(super) struct Args {
    pub(super) words: Vec<String>,
    values: Vec<(String, String)>,
    flags: Vec<String>,
    pub(super) rest: Vec<String>,
}

impl Args {
    pub(super) fn parse(args: &[String], takes_value: &[&str], flags: &[&str]) -> io::Result<Args> {
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

    pub(super) fn value(&self, name: &str) -> Option<&str> {
        self.values.iter().rev().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }

    pub(super) fn flag(&self, name: &str) -> bool {
        self.flags.iter().any(|f| f == name)
    }

    pub(super) fn files(&self) -> Vec<String> {
        self.value("--files").map(|f| f.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect()).unwrap_or_default()
    }

    /// `--pr 42`, `--pr '#42'` or a URL; `#` is dropped so the number reads the same anywhere.
    pub(super) fn pr(&self) -> Option<String> {
        self.value("--pr").map(|p| p.trim().trim_start_matches('#').to_string()).filter(|p| !p.is_empty())
    }

    pub(super) fn verdict(&self) -> io::Result<Option<Verdict>> {
        self.value("--verdict").map(|v| Verdict::parse(v).ok_or_else(|| bad("--verdict is approved, changes or failed"))).transpose()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
