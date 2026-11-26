//! Scrollback search (Ctrl+Shift+F) and link detection (Ctrl+hover / Ctrl+click).

use std::sync::atomic::{AtomicU64, Ordering};

use alacritty_terminal::Term;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Boundary, Column, Direction, Point};
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

use crate::layout::PaneId;

/// Upper bound on matches we track, so a one-letter query in a huge scrollback stays cheap.
const MAX_MATCHES: usize = 10_000;

/// Same URL pattern Alacritty uses for its default hint.
const URL_REGEX: &str = "(ipfs:|ipns:|magnet:|mailto:|gemini://|gopher://|https://|http://|news:|file:|git://|ssh:|ftp://)[^\u{0000}-\u{001F}\u{007F}-\u{009F}<>\"\\s{-}\\^⟨⟩`]+";

pub struct Search {
    pub pane: PaneId,
    pub query: String,
    /// All matches, oldest (top of scrollback) first.
    pub matches: Vec<Match>,
    /// Index of the focused match in `matches`.
    pub index: Option<usize>,
    searched_query: Option<String>,
    searched_generation: u64,
    /// See [`Search::results_generation`].
    results_generation: u64,
}

/// A number no other set of search results has had, in this search or any other.
fn next_results_generation() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
}

impl Search {
    pub fn new(pane: PaneId) -> Self {
        Self {
            pane,
            query: String::new(),
            matches: Vec::new(),
            index: None,
            searched_query: None,
            searched_generation: 0,
            results_generation: next_results_generation(),
        }
    }

    /// Identifies the contents of `matches`: a new number each time they are searched for
    /// again, never shared with another `Search`. With `index`, it stands for everything the
    /// search draws on the grid, so a frame can be compared to the last one without comparing
    /// (or copying) the matches themselves.
    pub fn results_generation(&self) -> u64 {
        self.results_generation
    }

    pub fn current(&self) -> Option<&Match> {
        self.index.and_then(|i| self.matches.get(i))
    }

    /// Re-run the search if the query or the terminal contents changed. Returns the point to
    /// scroll to when the query changed (jump to the most recent match, like Ghostty).
    pub fn refresh<T>(&mut self, term: &Term<T>, generation: u64) -> Option<Point> {
        let query_changed = self.searched_query.as_deref() != Some(self.query.as_str());
        if !query_changed && generation == self.searched_generation {
            return None;
        }
        self.searched_query = Some(self.query.clone());
        self.searched_generation = generation;
        self.matches = find_all(term, &self.query);
        self.results_generation = next_results_generation();

        if query_changed {
            self.index = self.matches.len().checked_sub(1);
            self.current().map(|m| *m.start())
        } else {
            // Content changed under us: keep the same position, clamped.
            self.index = self.index.map(|i| i.min(self.matches.len().saturating_sub(1)));
            if self.matches.is_empty() {
                self.index = None;
            }
            None
        }
    }

    /// Move to the next older (`up`) or newer match, wrapping around. Returns where to scroll.
    pub fn step(&mut self, up: bool) -> Option<Point> {
        let n = self.matches.len();
        if n == 0 {
            return None;
        }
        let i = match self.index {
            None => n - 1,
            Some(i) if up => (i + n - 1) % n,
            Some(i) => (i + 1) % n,
        };
        self.index = Some(i);
        Some(*self.matches[i].start())
    }
}

/// Escape regex metacharacters so the query is matched literally.
fn escape(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

fn find_all<T>(term: &Term<T>, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let Ok(mut regex) = RegexSearch::new(&escape(query)) else { return Vec::new() };
    let start = Point::new(term.topmost_line(), Column(0));
    let end = Point::new(term.bottommost_line(), term.last_column());
    RegexIter::new(start, end, Direction::Right, term, &mut regex).take(MAX_MATCHES).collect()
}

pub struct Link {
    pub range: Match,
    pub uri: String,
}

pub struct LinkFinder {
    regex: Option<RegexSearch>,
}

impl LinkFinder {
    pub fn new() -> Self {
        Self { regex: RegexSearch::new(URL_REGEX).ok() }
    }

    /// Find an OSC 8 hyperlink or a plain-text URL under `point`.
    pub fn link_at<T>(&mut self, term: &Term<T>, point: Point) -> Option<Link> {
        let grid = term.grid();
        if point.line < term.topmost_line() || point.line > term.bottommost_line() {
            return None;
        }

        // Explicit hyperlinks (OSC 8), e.g. from `ls --hyperlink` or gcc diagnostics.
        if let Some(link) = grid[point].hyperlink() {
            let same = |p: Point| grid[p].hyperlink().is_some_and(|h| h.id() == link.id());
            let mut start = point;
            while start.column.0 > 0 && same(Point::new(start.line, start.column - 1)) {
                start.column -= 1;
            }
            let mut end = point;
            while end.column < term.last_column() && same(Point::new(end.line, end.column + 1)) {
                end.column += 1;
            }
            return Some(Link { range: start..=end, uri: link.uri().to_string() });
        }

        // Plain URLs; search a few lines around the point so wrapped URLs are found.
        let regex = self.regex.as_mut()?;
        let start = Point::new(std::cmp::max(point.line - 3i32, term.topmost_line()), Column(0));
        let end = Point::new(std::cmp::min(point.line + 3i32, term.bottommost_line()), term.last_column());
        let m = RegexIter::new(start, end, Direction::Right, term, regex).find(|m| m.contains(&point))?;

        // Drop trailing punctuation that's almost never part of the URL ("see https://x.org.").
        let mut uri = term.bounds_to_string(*m.start(), *m.end());
        let mut end = *m.end();
        while uri.ends_with(['.', ',', ';', ':', '!', '?', ')', '\'', '"']) && uri.len() > 8 {
            // Keep a closing paren if the URL has a matching opening one (Wikipedia links).
            if uri.ends_with(')') && uri.matches('(').count() >= uri.matches(')').count() {
                break;
            }
            uri.pop();
            end = end.sub(term, Boundary::None, 1);
        }
        if end < point {
            return None;
        }
        Some(Link { range: *m.start()..=end, uri })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Line;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::term::test::TermSize;
    use alacritty_terminal::vte::ansi::{Processor, StdSyncHandler};

    fn term_with(text: &str) -> Term<VoidListener> {
        let mut term = Term::new(Config::default(), &TermSize::new(60, 10), VoidListener);
        let mut parser: Processor<StdSyncHandler> = Processor::new();
        parser.advance(&mut term, text.replace('\n', "\r\n").as_bytes());
        term
    }

    #[test]
    fn literal_search_finds_all_matches() {
        let term = term_with("foo.bar\nfoo bar\nfooXbar\n");
        let matches = find_all(&term, "foo.bar");
        assert_eq!(matches.len(), 1, "query must be matched literally, not as a regex");
        assert_eq!(find_all(&term, "foo").len(), 3);
    }

    /// The frame cache keys on the generation instead of the matches, so it has to change
    /// exactly when the matches may have.
    #[test]
    fn results_generation_changes_only_when_the_search_is_rerun() {
        let term = term_with("foo\nfoo\nbar\n");
        let mut search = Search::new(1);
        search.query = "foo".into();
        let fresh = search.results_generation();
        search.refresh(&term, 1);
        let first = search.results_generation();
        assert_ne!(first, fresh, "new matches, new generation");
        assert_eq!(search.matches.len(), 2);

        // Nothing changed: no search, same generation. Stepping only moves the index.
        search.refresh(&term, 1);
        search.step(true);
        assert_eq!(search.results_generation(), first);
        assert_eq!(search.index, Some(0));

        // New terminal contents, then a new query: searched again each time.
        search.refresh(&term, 2);
        let second = search.results_generation();
        assert_ne!(second, first);
        search.query = "bar".into();
        search.refresh(&term, 2);
        assert_ne!(search.results_generation(), second);

        // Another search never reuses a generation, so one replacing this one between two
        // frames can't be mistaken for it.
        let mut other = Search::new(1);
        other.query = "bar".into();
        other.refresh(&term, 2);
        assert_ne!(other.results_generation(), search.results_generation());
    }

    #[test]
    fn finds_url_and_strips_trailing_period() {
        let term = term_with("see https://example.com/path. ok\n");
        let mut finder = LinkFinder::new();
        let link = finder.link_at(&term, Point::new(Line(0), Column(10))).expect("url");
        assert_eq!(link.uri, "https://example.com/path");
        assert!(finder.link_at(&term, Point::new(Line(0), Column(1))).is_none());
    }
}
