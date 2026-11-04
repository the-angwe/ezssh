//! Clickable links: OSC 8 hyperlinks set by programs, and plain `http(s)`
//! URLs found in the text with the same pattern as xterm.js's web-links
//! addon (so the same things are links in both builds).

use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Direction, Point};
use alacritty_terminal::term::Term;
use alacritty_terminal::term::search::{Match, RegexIter, RegexSearch};

/// `@xterm/addon-web-links` strict URL regex, in regex-automata syntax.
pub const URL_PATTERN: &str =
    r#"(https?|HTTPS?)://[^\s"'!*(){}|\\^<>`]*[^\s"':,.!?{}|\\^~\[\]`()<>]"#;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Link {
    /// Cells the link covers, for the hover underline.
    pub range: Match,
    pub uri: String,
}

pub struct LinkFinder {
    url: RegexSearch,
}

impl LinkFinder {
    pub fn new() -> Option<Self> {
        match RegexSearch::new(URL_PATTERN) {
            Ok(url) => Some(Self { url }),
            Err(e) => {
                tracing::error!("term.links: URL pattern does not compile: {e}");
                None
            }
        }
    }

    /// The link under `point`, if any. OSC 8 wins over a URL in the text.
    pub fn at<T>(&mut self, term: &Term<T>, point: Point) -> Option<Link> {
        if let Some(link) = osc8_at(term, point) {
            return Some(link);
        }
        let start = term.line_search_left(point);
        let end = term.line_search_right(point);
        RegexIter::new(start, end, Direction::Right, term, &mut self.url)
            .find(|m| m.contains(&point))
            .map(|m| Link {
                uri: term.bounds_to_string(*m.start(), *m.end()),
                range: m,
            })
    }
}

/// The OSC 8 hyperlink under `point`, extended over the neighbouring cells
/// that carry the same link on that line.
fn osc8_at<T>(term: &Term<T>, point: Point) -> Option<Link> {
    let grid = term.grid();
    let link = grid[point].hyperlink()?;
    let row = &grid[point.line];
    let same = |col: usize| row[Column(col)].hyperlink().is_some_and(|h| h == link);
    let mut first = point.column.0;
    while first > 0 && same(first - 1) {
        first -= 1;
    }
    let mut last = point.column.0;
    while last + 1 < grid.columns() && same(last + 1) {
        last += 1;
    }
    Some(Link {
        range: Point::new(point.line, Column(first))..=Point::new(point.line, Column(last)),
        uri: link.uri().to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use alacritty_terminal::event::VoidListener;
    use alacritty_terminal::index::Line;
    use alacritty_terminal::term::Config;
    use alacritty_terminal::vte::ansi::Processor;

    use crate::terminal::GridSize;

    fn term_with(bytes: &[u8], columns: usize) -> Term<VoidListener> {
        let mut t = Term::new(
            Config::default(),
            &GridSize { columns, lines: 4 },
            VoidListener,
        );
        let mut p: Processor = Processor::new();
        p.advance(&mut t, bytes);
        t
    }

    fn at(t: &Term<VoidListener>, line: i32, col: usize) -> Option<String> {
        LinkFinder::new()
            .unwrap()
            .at(t, Point::new(Line(line), Column(col)))
            .map(|l| l.uri)
    }

    #[test]
    fn finds_urls_and_drops_trailing_punctuation() {
        let t = term_with(b"see https://example.com/a?b=1. and (http://x.io/p)", 80);
        assert_eq!(at(&t, 0, 10).as_deref(), Some("https://example.com/a?b=1"));
        // The full stop after the URL is not part of it.
        assert_eq!(at(&t, 0, 29), None);
        assert_eq!(at(&t, 0, 40).as_deref(), Some("http://x.io/p"));
        assert_eq!(at(&t, 0, 1), None);
    }

    #[test]
    fn only_http_schemes() {
        let t = term_with(b"ftp://host/x mailto:a@b.c", 80);
        assert_eq!(at(&t, 0, 2), None);
        assert_eq!(at(&t, 0, 15), None);
    }

    #[test]
    fn url_wrapped_over_two_rows() {
        let t = term_with(b"xx https://example.com/very/long/path", 20);
        assert_eq!(
            at(&t, 1, 3).as_deref(),
            Some("https://example.com/very/long/path")
        );
        assert_eq!(
            at(&t, 0, 5).as_deref(),
            Some("https://example.com/very/long/path")
        );
    }

    #[test]
    fn osc8_hyperlinks() {
        let t = term_with(
            b"a \x1b]8;;https://ezssh.dev/doc\x1b\\docs\x1b]8;;\x1b\\ b",
            80,
        );
        let mut f = LinkFinder::new().unwrap();
        let l = f.at(&t, Point::new(Line(0), Column(3))).unwrap();
        assert_eq!(l.uri, "https://ezssh.dev/doc");
        assert_eq!(
            l.range,
            Point::new(Line(0), Column(2))..=Point::new(Line(0), Column(5))
        );
        assert_eq!(at(&t, 0, 7), None);
    }
}
