//! Word-level highlights: which words differ between a removed line and the added line that
//! replaced it.

use std::ops::Range;

use crate::git_diff::git::diff::{Line, LineKind};

/// Lines longer than this (in chars) get no word-level highlights.
const MAX_EMPHASIS_CHARS: usize = 500;
/// Tokens per line compared for word-level highlights (the LCS is quadratic).
const MAX_EMPHASIS_TOKENS: usize = 240;

/// Byte ranges within a line.
pub type Spans = Vec<Range<usize>>;
/// Pair each run of removed lines with the added lines right after it, and mark the words
/// that differ between each pair.
pub(super) fn add_emphasis(lines: &mut [Line]) {
    let mut i = 0;
    while i < lines.len() {
        if lines[i].kind != LineKind::Removed {
            i += 1;
            continue;
        }
        let del_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Removed {
            i += 1;
        }
        let add_start = i;
        while i < lines.len() && lines[i].kind == LineKind::Added {
            i += 1;
        }
        let pairs = (add_start - del_start).min(i - add_start);
        for k in 0..pairs {
            let (old, new) = (del_start + k, add_start + k);
            if let Some((a, b)) = word_diff(&lines[old].text, &lines[new].text) {
                lines[old].emphasis = a;
                lines[new].emphasis = b;
            }
        }
    }
}

/// Byte ranges that changed in `old` and in `new`, or `None` when the lines are too long or
/// too different for highlights to help (the whole line is then simply changed).
pub fn word_diff(old: &str, new: &str) -> Option<(Spans, Spans)> {
    if old.chars().count() > MAX_EMPHASIS_CHARS || new.chars().count() > MAX_EMPHASIS_CHARS {
        return None;
    }
    let a = tokens(old);
    let b = tokens(new);
    if a.len() > MAX_EMPHASIS_TOKENS || b.len() > MAX_EMPHASIS_TOKENS || a.is_empty() || b.is_empty() {
        return None;
    }
    let ta: Vec<&str> = a.iter().map(|r| &old[r.clone()]).collect();
    let tb: Vec<&str> = b.iter().map(|r| &new[r.clone()]).collect();
    // LCS table, from the end.
    let (n, m) = (ta.len(), tb.len());
    let mut dp = vec![0u16; (n + 1) * (m + 1)];
    let at = |i: usize, j: usize| i * (m + 1) + j;
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            dp[at(i, j)] = if ta[i] == tb[j] { dp[at(i + 1, j + 1)] + 1 } else { dp[at(i + 1, j)].max(dp[at(i, j + 1)]) };
        }
    }
    let mut keep_a = vec![false; n];
    let mut keep_b = vec![false; m];
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if ta[i] == tb[j] {
            keep_a[i] = true;
            keep_b[j] = true;
            i += 1;
            j += 1;
        } else if dp[at(i + 1, j)] >= dp[at(i, j + 1)] {
            i += 1;
        } else {
            j += 1;
        }
    }
    // Mostly rewritten: highlighting nearly every word is just noise.
    let common: usize = keep_a.iter().zip(&ta).filter(|(k, t)| **k && !t.trim().is_empty()).map(|(_, t)| t.len()).sum();
    let longest = old.trim().len().max(new.trim().len()).max(1);
    if common * 10 < longest * 4 {
        return None;
    }
    Some((changed_ranges(&a, &keep_a), changed_ranges(&b, &keep_b)))
}

/// The ranges of tokens not kept, merged where they touch.
fn changed_ranges(spans: &[Range<usize>], keep: &[bool]) -> Vec<Range<usize>> {
    let mut out: Vec<Range<usize>> = Vec::new();
    for (k, span) in spans.iter().enumerate() {
        if keep[k] {
            continue;
        }
        match out.last_mut() {
            Some(last) if last.end == span.start => last.end = span.end,
            _ => out.push(span.clone()),
        }
    }
    out
}

/// Words (letters, digits, `_`), runs of whitespace, and single other characters, as byte
/// ranges.
fn tokens(s: &str) -> Vec<Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<(usize, u8)> = None;
    let class = |c: char| {
        if c.is_alphanumeric() || c == '_' {
            1
        } else if c.is_whitespace() {
            2
        } else {
            0
        }
    };
    for (i, c) in s.char_indices() {
        let k = class(c);
        match start {
            Some((_, sk)) if sk == k && k != 0 => {}
            Some((st, _)) => {
                out.push(st..i);
                start = Some((i, k));
            }
            None => start = Some((i, k)),
        }
    }
    if let Some((st, _)) = start {
        out.push(st..s.len());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rewritten_lines_get_no_word_highlights() {
        assert!(word_diff("let total = items.len();", "return Err(\"nope\".into());").is_none());
        assert!(word_diff("foo(a, b)", "foo(a, c)").is_some());
    }
}
