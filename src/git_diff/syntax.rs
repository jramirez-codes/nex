//! Syntax colors for the diff: each line's text split into token kinds (keyword, string,
//! comment…) by syntect with bat's grammars. Only the kinds are kept; the view picks their
//! colors from the terminal palette, so highlighting follows the user's theme.
//!
//! Each hunk is read as two streams, the old side (context and removed lines) and the new
//! side (context and added lines), so a string opened on a removed line never bleeds into
//! the added line after it. The parser starts fresh at every hunk: a hunk that begins
//! inside a block comment is colored as code until the comment closes.

use std::ops::Range;
use std::sync::OnceLock;

use syntect::easy::ScopeRangeIterator;
use syntect::parsing::{ParseState, Scope, ScopeStack, SyntaxReference, SyntaxSet};

use crate::git_diff::git::diff::{FileDiff, Hunk, LineKind};

/// What a stretch of code is, for its color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Token {
    Keyword,
    String,
    Comment,
    Number,
    Constant,
    Function,
    Type,
    Attribute,
    Operator,
    Punct,
}

/// Byte ranges of a line's text, with their kinds.
pub type Tokens = Vec<(Range<usize>, Token)>;

/// Lines longer than this (in bytes) aren't colored: minified files stall the regexes.
const MAX_LINE_BYTES: usize = 2_000;

fn syntaxes() -> &'static SyntaxSet {
    static SET: OnceLock<SyntaxSet> = OnceLock::new();
    SET.get_or_init(two_face::syntax::extra_newlines)
}

/// Scope prefixes and the token they mean, most specific first.
fn rules() -> &'static [(Scope, Token)] {
    static RULES: OnceLock<Vec<(Scope, Token)>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            ("comment", Token::Comment),
            ("punctuation.definition.comment", Token::Comment),
            ("string", Token::String),
            ("constant.numeric", Token::Number),
            ("constant.character.escape", Token::Constant),
            ("constant.language", Token::Constant),
            ("constant.character", Token::String),
            ("constant.other", Token::Constant),
            ("keyword.operator", Token::Operator),
            ("keyword", Token::Keyword),
            ("storage.type", Token::Keyword),
            ("storage.modifier", Token::Keyword),
            ("storage", Token::Keyword),
            ("variable.language", Token::Keyword),
            ("entity.name.function", Token::Function),
            ("support.function", Token::Function),
            ("variable.function", Token::Function),
            ("meta.function-call", Token::Function),
            ("entity.name.type", Token::Type),
            ("entity.name.class", Token::Type),
            ("entity.name.struct", Token::Type),
            ("entity.name.enum", Token::Type),
            ("entity.name.trait", Token::Type),
            ("entity.other.inherited-class", Token::Type),
            ("support.type", Token::Type),
            ("support.class", Token::Type),
            ("entity.name.tag", Token::Keyword),
            ("entity.other.attribute-name", Token::Attribute),
            ("meta.attribute", Token::Attribute),
            ("meta.annotation", Token::Attribute),
            ("entity.name.section", Token::Function),
            ("markup.heading", Token::Function),
            ("markup.bold", Token::Constant),
            ("markup.italic", Token::Constant),
            ("markup.raw", Token::String),
            ("markup.underline.link", Token::Type),
            ("punctuation", Token::Punct),
        ]
        .into_iter()
        .filter_map(|(s, t)| Some((Scope::new(s).ok()?, t)))
        .collect()
    })
}

/// The token for the scopes on the stack: the innermost scope that matches a rule wins,
/// except that being inside a comment or string always wins (a `{` in a string is string).
fn classify(stack: &ScopeStack) -> Option<Token> {
    let rules = rules();
    let find = |scope: &Scope| rules.iter().find(|(prefix, _)| prefix.is_prefix_of(*scope)).map(|(_, t)| *t);
    let scopes = stack.as_slice();
    if let Some(t) = scopes.iter().filter_map(find).find(|t| matches!(t, Token::Comment | Token::String)) {
        // An escape or interpolation inside a string still gets its own color.
        let inner = scopes.iter().rev().find_map(find);
        return Some(match inner {
            Some(Token::Constant) if t == Token::String => Token::Constant,
            _ => t,
        });
    }
    scopes.iter().rev().find_map(find)
}

/// The grammar for a file: by extension, then by whole name (`Dockerfile`), then by its
/// first line (a shebang). `None` for plain text and files nothing knows.
pub fn syntax_for(path: &str, first_line: Option<&str>) -> Option<&'static SyntaxReference> {
    let set = syntaxes();
    let name = path.rsplit('/').next().unwrap_or(path);
    let ext = name.rsplit_once('.').map(|(_, e)| e).filter(|e| !e.is_empty());
    let found = ext
        .and_then(|e| set.find_syntax_by_extension(e).or_else(|| set.find_syntax_by_extension(&e.to_lowercase())))
        .or_else(|| set.find_syntax_by_extension(name))
        .or_else(|| set.find_syntax_by_extension(&name.to_lowercase()))
        .or_else(|| first_line.and_then(|l| set.find_syntax_by_first_line(l)));
    found.filter(|s| s.name != "Plain Text")
}

/// One side of a hunk being read, line by line.
struct Stream {
    state: ParseState,
    stack: ScopeStack,
    /// Set once the parser fails; the rest of the stream is left uncolored.
    broken: bool,
}

impl Stream {
    fn new(syntax: &SyntaxReference) -> Stream {
        Stream { state: ParseState::new(syntax), stack: ScopeStack::new(), broken: false }
    }

    fn line(&mut self, text: &str) -> Tokens {
        if self.broken {
            return Vec::new();
        }
        if text.len() > MAX_LINE_BYTES {
            // Skipping keeps the state as it was; better than stalling on a minified line.
            return Vec::new();
        }
        let fed = format!("{text}\n");
        let Ok(ops) = self.state.parse_line(&fed, syntaxes()) else {
            self.broken = true;
            return Vec::new();
        };
        let mut out: Tokens = Vec::new();
        for (range, op) in ScopeRangeIterator::new(&ops, &fed) {
            if self.stack.apply(op).is_err() {
                self.broken = true;
                return Vec::new();
            }
            let range = range.start.min(text.len())..range.end.min(text.len());
            if range.is_empty() {
                continue;
            }
            let Some(token) = classify(&self.stack) else { continue };
            match out.last_mut() {
                // Neighbors of the same kind become one span.
                Some((r, t)) if *t == token && r.end == range.start => r.end = range.end,
                _ => out.push((range, token)),
            }
        }
        out
    }
}

/// Where coloring one hunk got to: both sides' parser state, and the next line.
struct Progress {
    old: Stream,
    new: Stream,
    next: usize,
}

/// Colors a diff a piece at a time, so the hunks on screen can go first and the rest can
/// follow in chunks. A hunk can be left partway and picked up again later.
pub struct Colorer {
    syntax: &'static SyntaxReference,
    /// Per hunk: its progress, once started.
    progress: Vec<Option<Progress>>,
}

impl Colorer {
    /// A colorer for `diff`, if its language is known (from `path`, or the file's first line).
    pub fn new(diff: &FileDiff, path: &str) -> Option<Colorer> {
        let starts_file = diff.hunks.first().is_some_and(|h| h.new_start <= 1 || h.old_start <= 1);
        let first = diff.hunks.first().and_then(|h| h.lines.first()).filter(|_| starts_file).map(|l| l.text.as_str());
        let syntax = syntax_for(path, first)?;
        Some(Colorer { syntax, progress: diff.hunks.iter().map(|_| None).collect() })
    }

    /// Color up to `max` more lines of hunk `h`. Returns the lines colored (indices into
    /// the hunk); the hunk is marked `colored` once its last line is done.
    pub fn step(&mut self, diff: &mut FileDiff, h: usize, max: usize) -> Range<usize> {
        let Some(hunk) = diff.hunks.get_mut(h) else { return 0..0 };
        if hunk.colored {
            return 0..0;
        }
        let syntax = self.syntax;
        let p = self.progress[h].get_or_insert_with(|| Progress { old: Stream::new(syntax), new: Stream::new(syntax), next: 0 });
        let start = p.next;
        let end = (start + max).min(hunk.lines.len());
        for line in &mut hunk.lines[start..end] {
            line.syntax = match line.kind {
                LineKind::Removed => p.old.line(&line.text),
                LineKind::Added => p.new.line(&line.text),
                LineKind::Context => {
                    p.old.line(&line.text);
                    p.new.line(&line.text)
                }
            };
        }
        p.next = end;
        if end == hunk.lines.len() {
            hunk.colored = true;
            self.progress[h] = None;
        }
        start..end
    }
}

/// The hunk to color next: the first not yet colored at or after `focus` (the topmost
/// hunk on screen, so what's in view and just below it goes first), else the first one
/// above it. `None` once everything is colored.
pub fn next(diff: &FileDiff, focus: usize) -> Option<usize> {
    let open = |i: &usize| !diff.hunks[*i].colored;
    (focus.min(diff.hunks.len())..diff.hunks.len()).find(open).or_else(|| (0..focus.min(diff.hunks.len())).find(open))
}

/// Give hunks that are line for line the same as a colored one in `prev` (the file's last
/// diff) its colors, instead of reading them again: the diff is re-read every couple of
/// seconds, and usually only one hunk changed, if any.
pub fn reuse(diff: &mut FileDiff, prev: &FileDiff) {
    let same = |a: &Hunk, b: &Hunk| a.lines.len() == b.lines.len() && a.lines.iter().zip(&b.lines).all(|(x, y)| x.kind == y.kind && x.text == y.text);
    for hunk in &mut diff.hunks {
        if let Some(old) = prev.hunks.iter().find(|o| o.colored && same(hunk, o)) {
            for (line, o) in hunk.lines.iter_mut().zip(&old.lines) {
                line.syntax = o.syntax.clone();
            }
            hunk.colored = true;
        }
    }
}

/// Color a whole diff at once (reusing `prev`'s colors where it can).
#[cfg(test)]
pub fn highlight_file(diff: &mut FileDiff, path: &str, prev: Option<&FileDiff>) {
    if let Some(prev) = prev {
        reuse(diff, prev);
    }
    let Some(mut colorer) = Colorer::new(diff, path) else { return };
    while let Some(h) = next(diff, 0) {
        colorer.step(diff, h, usize::MAX);
    }
}
