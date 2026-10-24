//! The links a run's output carries: a URL, which the browser opens, and a stack frame's place in the code, which the
//! editor opens. Compiler locations are the Build console's, and are found by `build::Parser`.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::LazyLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url {
        at: Range<usize>,
        url: String,
    },
    /// A stack frame's file and its line from 1, as .NET writes one.
    File {
        at: Range<usize>,
        path: PathBuf,
        line: usize,
    },
}

impl Link {
    /// Whether the click was on this link.
    pub fn hits(&self, offset: usize) -> bool {
        match self {
            Link::Url { at, .. } | Link::File { at, .. } => at.contains(&offset),
        }
    }
}

/// The link a click at `offset` in `line` follows: a URL there, else the stack frame the line is.
pub fn find(line: &str, offset: usize) -> Option<Link> {
    if offset > line.len() {
        return None;
    }
    if let Some(url) = url_at(line, offset) {
        return Some(url);
    }
    frame(line)
}

/// A URL at the offset, with the punctuation that ended it left out: `see http://localhost:5000/a.` links the address.
fn url_at(line: &str, offset: usize) -> Option<Link> {
    static URL: LazyLock<regex::Regex> =
        LazyLock::new(|| regex::Regex::new(r"https?://[^\s<>()\[\]{}'`\x22]+").unwrap());
    URL.find_iter(line).find_map(|found| {
        let text = found.as_str().trim_end_matches(['.', ',', ';', ':', '!', '?']);
        let at = found.start()..found.start() + text.len();
        at.contains(&offset).then(|| Link::Url { at, url: text.to_string() })
    })
}

/// A stack frame as .NET writes one: `   at Namespace.Type.Method() in /path/File.cs:line 42`. The path may hold
/// spaces and parentheses, so the frame is cut at its `:line ` and its path taken from the last ` in ` before it.
fn frame(line: &str) -> Option<Link> {
    let marker = line.rfind(":line ")?;
    let number = line[marker + 6..].split_whitespace().next()?;
    let number_end = marker + 6 + number.len();
    let line_number: usize = number.parse().ok()?;
    let path_start = line[..marker].rfind(" in ").map(|found| found + 4)?;
    let path = line[path_start..marker].trim();
    if path.is_empty() || line_number == 0 {
        return None;
    }
    Some(Link::File { at: path_start..number_end, path: PathBuf::from(path), line: line_number - 1 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_url_is_found_where_the_click_was() {
        let line = "Now listening on: http://localhost:5000/swagger.";
        let url = match find(line, 30).unwrap() {
            Link::Url { url, at } => {
                assert_eq!(at, 18..47);
                url
            }
            other => panic!("{other:?}"),
        };
        assert_eq!(url, "http://localhost:5000/swagger", "the stop that ended the address is not part of it");
        assert!(find(line, 2).is_none(), "a click away from the address is no link");
        let https = find("see https://example.test/a?b=c now", 12).unwrap();
        assert_eq!(https, Link::Url { at: 4..30, url: "https://example.test/a?b=c".into() });
        assert!(find("no address here", 3).is_none());
        assert!(find("a sentence with a https:// dot", 24).is_none(), "a scheme alone is no address");
    }

    #[test]
    fn a_stack_frames_place_is_found_with_paths_that_hold_spaces_and_parentheses() {
        let line = "   at Shop.Program.Main(String[] args) in /work/My App/(src)/Program.cs:line 12";
        let at = line.find("/work").unwrap() + 3;
        let found = find(line, at).unwrap();
        assert_eq!(
            found,
            Link::File { at: 42..79, path: PathBuf::from("/work/My App/(src)/Program.cs"), line: 11 },
            "the path is the frame's, from the last ' in ' before its :line, and the line is counted from 0"
        );
        assert!(!found.hits(2), "a click on the method is not on its place");
        assert!(find("Unhandled exception. System.Exception: boom", 10).is_none());
        assert!(find("at Foo() in /nowhere.cs:line 0", 5).is_none(), "a line counted from 0 is no place");
    }

    #[test]
    fn a_url_wins_over_a_frame_in_the_same_line() {
        let line = "   at X.Y() in /work/P.cs:line 3 for http://localhost:5000/x";
        assert_eq!(find(line, line.len() - 3), Some(Link::Url { at: 37..60, url: "http://localhost:5000/x".into() }));
        assert!(matches!(find(line, 6), Some(Link::File { .. })));
    }
}
