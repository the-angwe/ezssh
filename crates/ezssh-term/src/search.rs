//! Find in the terminal buffer (Ctrl+Shift+F): plain or regex, case
//! sensitive or not, with an `index/count` counter. The current match is
//! made the selection, as xterm.js's search addon does, so Ctrl+Shift+C
//! copies it.
//!
//! Match points go stale when new output scrolls the buffer; they are
//! recounted on every query change and every next / previous.

use alacritty_terminal::event::EventListener;
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point, Side};
use alacritty_terminal::selection::{Selection, SelectionType};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// Matches counted at most (the most recent ones). Past this the counter
/// shows `n/1000+`.
pub const MAX_COUNTED: usize = 1000;

/// Escape regex syntax so the query matches literally.
pub fn escape(query: &str) -> String {
    let mut out = String::with_capacity(query.len() * 2);
    for c in query.chars() {
        if "\\.+*?()|[]{}^$#&-~".contains(c) {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

/// The pattern handed to alacritty. Its `RegexSearch` uses smart case
/// (insensitive unless the pattern has an upper-case letter); the inline
/// flag overrides that so the toggle means what it says.
pub fn pattern(query: &str, case_sensitive: bool, regex: bool) -> String {
    let body = if regex {
        query.to_string()
    } else {
        escape(query)
    };
    if case_sensitive {
        format!("(?-i){body}")
    } else {
        format!("(?i){body}")
    }
}

#[derive(Default)]
pub struct Find {
    /// Feed generation the matches were counted at.
    counted_at: Option<u64>,
    pub query: String,
    pub case_sensitive: bool,
    pub regex: bool,
    compiled: Option<(String, RegexSearch)>,
    /// Why the pattern did not compile.
    pub error: Option<String>,
    /// Up to [`MAX_COUNTED`] matches, oldest first.
    pub matches: Vec<Match>,
    pub truncated: bool,
    pub current: Option<usize>,
    pub(crate) focus_input: bool,
}

impl std::fmt::Debug for Find {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Find")
            .field("query", &self.query)
            .field("case_sensitive", &self.case_sensitive)
            .field("regex", &self.regex)
            .field("matches", &self.matches.len())
            .field("current", &self.current)
            .finish()
    }
}

impl Find {
    pub fn new() -> Self {
        Self {
            focus_input: true,
            ..Self::default()
        }
    }

    /// `(1-based index, count)` for the counter.
    pub fn counter(&self) -> (usize, usize) {
        (self.current.map_or(0, |i| i + 1), self.matches.len())
    }

    fn regex(&mut self) -> Option<&mut RegexSearch> {
        if self.query.is_empty() {
            self.compiled = None;
            self.error = None;
            return None;
        }
        let pat = pattern(&self.query, self.case_sensitive, self.regex);
        if self.compiled.as_ref().is_none_or(|(p, _)| *p != pat) {
            match RegexSearch::new(&pat) {
                Ok(r) => {
                    self.compiled = Some((pat, r));
                    self.error = None;
                }
                Err(e) => {
                    tracing::debug!(pattern = %pat, "term.find: bad pattern: {e}");
                    self.compiled = None;
                    self.error = Some(e.to_string());
                }
            }
        }
        self.compiled.as_mut().map(|(_, r)| r)
    }

    /// Recount the matches. Returns the previous current match's start so
    /// the caller can stay near it.
    fn recount<T>(&mut self, term: &Term<T>, feed_gen: u64) -> Option<Point> {
        let previous = self
            .current
            .and_then(|i| self.matches.get(i))
            .map(|m| *m.start());
        self.counted_at = Some(feed_gen);
        self.matches.clear();
        self.truncated = false;
        self.current = None;
        let Some(regex) = self.regex() else {
            return previous;
        };
        let grid = term.grid();
        let start = Point::new(grid.topmost_line(), Column(0));
        let end = Point::new(grid.bottommost_line(), grid.last_column());
        // Walk from the bottom so the most recent matches are the ones kept.
        let mut found: Vec<Match> = Vec::new();
        for m in RegexIter::new(end, start, Direction::Left, term, regex) {
            if found.len() == MAX_COUNTED {
                self.truncated = true;
                break;
            }
            found.push(m);
        }
        found.reverse();
        self.matches = found;
        previous
    }

    /// Query or toggles changed: recount and select the most recent match.
    /// `feed_gen` is `Terminal::feed_generation()`.
    pub fn update<T: EventListener>(&mut self, term: &mut Term<T>, feed_gen: u64) {
        self.recount(term, feed_gen);
        self.current = self.matches.len().checked_sub(1);
        self.select(term);
    }

    /// Enter (down / forward) and Shift+Enter (up / backward), wrapping.
    /// The matches are recounted only when output arrived since the last
    /// count: a full scrollback walk under the terminal lock per Enter
    /// would otherwise stall the reader on big buffers.
    pub fn step<T: EventListener>(&mut self, term: &mut Term<T>, forward: bool, feed_gen: u64) {
        let previous = if self.counted_at == Some(feed_gen) && self.compiled.is_some() {
            self.current
                .and_then(|i| self.matches.get(i))
                .map(|m| *m.start())
        } else {
            self.recount(term, feed_gen)
        };
        let n = self.matches.len();
        if n == 0 {
            term.selection = None;
            return;
        }
        self.current = Some(match previous {
            None => {
                if forward {
                    0
                } else {
                    n - 1
                }
            }
            Some(p) if forward => self
                .matches
                .iter()
                .position(|m| *m.start() > p)
                .unwrap_or(0),
            Some(p) => self
                .matches
                .iter()
                .rposition(|m| *m.start() < p)
                .unwrap_or(n - 1),
        });
        self.select(term);
    }

    fn select<T: EventListener>(&self, term: &mut Term<T>) {
        let Some(m) = self.current.and_then(|i| self.matches.get(i)) else {
            term.selection = None;
            return;
        };
        let mut sel = Selection::new(SelectionType::Simple, *m.start(), Side::Left);
        sel.update(*m.end(), Side::Right);
        term.selection = Some(sel);
        term.scroll_to_point(*m.start());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    use crate::terminal::GridSize;

    fn term_with(text: &str) -> Term<VoidListener> {
        let mut t = Term::new(
            Config::default(),
            &GridSize {
                columns: 40,
                lines: 5,
            },
            VoidListener,
        );
        let mut p: Processor = Processor::new();
        p.advance(&mut t, text.replace('\n', "\r\n").as_bytes());
        t
    }

    fn texts(f: &Find, t: &Term<VoidListener>) -> Vec<String> {
        f.matches
            .iter()
            .map(|m| t.bounds_to_string(*m.start(), *m.end()))
            .collect()
    }

    #[test]
    fn escape_makes_queries_literal() {
        assert_eq!(escape("a.b*(c)"), "a\\.b\\*\\(c\\)");
        assert_eq!(pattern("Foo", false, false), "(?i)Foo");
        assert_eq!(pattern("foo", true, true), "(?-i)foo");
    }

    #[test]
    fn counts_and_wraps_through_scrollback() {
        // Eight lines on a five-line screen: three are in the scrollback.
        let mut t = term_with("err 1\nok\nError 2\nok\nERR 3\nok\nok\nerr 4");
        let mut f = Find::new();
        f.query = "err".into();
        f.update(&mut t, 0);
        assert_eq!(texts(&f, &t), ["err", "Err", "ERR", "err"]);
        // Starts on the most recent match.
        assert_eq!(f.counter(), (4, 4));
        assert_eq!(t.selection_to_string().as_deref(), Some("err"));
        f.step(&mut t, true, 0);
        assert_eq!(f.counter(), (1, 4));
        f.step(&mut t, false, 0);
        assert_eq!(f.counter(), (4, 4));
        f.step(&mut t, false, 0);
        assert_eq!(f.counter(), (3, 4));
    }

    #[test]
    fn case_toggle_and_regex() {
        let mut t = term_with("err 1\nError 2\nERR 3");
        let mut f = Find::new();
        f.query = "err".into();
        f.case_sensitive = true;
        f.update(&mut t, 0);
        assert_eq!(f.counter().1, 1);

        f.case_sensitive = false;
        f.regex = true;
        f.query = r"err(or)? \d".into();
        f.update(&mut t, 0);
        assert_eq!(texts(&f, &t), ["err 1", "Error 2", "ERR 3"]);

        // Literal mode: the dot is not a wildcard.
        f.regex = false;
        f.query = "r.".into();
        f.update(&mut t, 0);
        assert_eq!(f.counter().1, 0);
        assert!(t.selection.is_none());
    }

    #[test]
    fn bad_regex_reports_an_error() {
        let mut t = term_with("abc");
        let mut f = Find::new();
        f.regex = true;
        f.query = "(".into();
        f.update(&mut t, 0);
        assert!(f.error.is_some());
        assert_eq!(f.counter(), (0, 0));
    }
}
