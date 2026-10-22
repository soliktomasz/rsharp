//! Colour before any language server answers. C# is tree-sitter's grammar, parsed incrementally (a parse
//! reuses the tree before the edit) on any thread, and coloured from syntax alone by the grammar's own
//! highlights query, read only over the lines asked for.
// shortcut: syntax colours only, so a type is coloured only where its syntax shows it; Roslyn's semantic tokens
// (Passenger's SemanticHighlighter) are the layer that colours every type reference.

use rsharp_text::{Change, Rope};
use std::ops::Range;
use std::sync::OnceLock;
use tree_sitter::{InputEdit, Language, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

/// What a piece of C# is, in Rider's colour scheme's terms.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Kind {
    Keyword,
    String,
    StringEscape,
    Number,
    Comment,
    DocComment,
    Type,
    Method,
    Property,
    Namespace,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Highlight {
    pub range: Range<usize>,
    pub kind: Kind,
}

fn language() -> Language {
    tree_sitter_c_sharp::LANGUAGE.into()
}

fn query() -> &'static Query {
    static QUERY: OnceLock<Query> = OnceLock::new();
    QUERY.get_or_init(|| {
        Query::new(&language(), tree_sitter_c_sharp::HIGHLIGHTS_QUERY).expect("the grammar's own query compiles")
    })
}

/// Parses `text`, reusing `old` (already told the edits since, by [`edit`]) where nothing changed.
pub fn parse(text: &Rope, old: Option<&Tree>) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&language()).expect("the grammar matches tree-sitter's version");
    let mut read = |byte: usize, _: Point| -> &[u8] {
        if byte >= text.len_bytes() {
            return &[];
        }
        let (chunk, start, _, _) = text.chunk_at_byte(byte);
        &chunk.as_bytes()[byte - start..]
    };
    parser.parse_with_options(&mut read, old, None).expect("a parse with no timeout or cancellation finishes")
}

/// Tells a tree of an edit made since it was parsed, so the next parse reuses what the edit left alone.
pub fn edit(tree: &mut Tree, change: &Change) {
    let point = |p: &rsharp_text::Position| Point { row: p.line, column: p.column };
    tree.edit(&InputEdit {
        start_byte: change.start.offset,
        old_end_byte: change.old_end.offset,
        new_end_byte: change.new_end.offset,
        start_position: point(&change.start),
        old_end_position: point(&change.old_end),
        new_end_position: point(&change.new_end),
    });
}

/// The colours within `range` (the lines on screen), each piece once, in order.
pub fn highlights(tree: &Tree, text: &Rope, range: Range<usize>) -> Vec<Highlight> {
    let query = query();
    let mut cursor = QueryCursor::new();
    cursor.set_byte_range(range);
    let source = |node: Node| text.byte_slice(node.byte_range()).chunks().map(str::as_bytes);
    let mut captures = cursor.captures(query, tree.root_node(), source);
    let mut out: Vec<Highlight> = Vec::new();
    // An interpolated string's later pieces, held until the captures inside its holes have gone by.
    let mut held: Vec<Highlight> = Vec::new();
    while let Some((found, index)) = captures.next() {
        let capture = found.captures[*index];
        let mut range = capture.node.byte_range();
        let Some(kind) = kind_of(&query.capture_names()[capture.index as usize], &capture.node, text) else {
            continue;
        };
        while held.first().is_some_and(|h| h.range.start <= range.start) {
            out.push(held.remove(0));
        }
        if kind == Kind::String && capture.node.kind() == "interpolated_string_expression" {
            let mut pieces = string_pieces(&capture.node);
            if pieces.is_empty() {
                continue;
            }
            range = pieces.remove(0);
            held = pieces.into_iter().map(|range| Highlight { range, kind }).collect();
        }
        match out.last() {
            // A piece inside the last (an escape in a string) splits it; any other overlap is the same node
            // again, where the query's earlier pattern wins, as tree-sitter's highlighter takes it.
            Some(last) if range.start < last.range.end => {
                if range.end > last.range.end || kind == last.kind {
                    continue;
                }
                let outer = out.pop().expect("there was a last piece");
                out.extend(
                    [
                        Highlight { range: outer.range.start..range.start, kind: outer.kind },
                        Highlight { range: range.clone(), kind },
                        Highlight { range: range.end..outer.range.end, kind: outer.kind },
                    ]
                    .into_iter()
                    .filter(|h| !h.range.is_empty()),
                );
            }
            _ => out.push(Highlight { range, kind }),
        }
    }
    out.extend(held);
    out
}

/// An interpolated string's text, its `{…}` holes left out so what is inside them takes its own colours.
fn string_pieces(node: &Node) -> Vec<Range<usize>> {
    let mut pieces = Vec::new();
    let mut at = node.start_byte();
    let mut cursor = node.walk();
    for hole in node.children(&mut cursor).filter(|c| c.kind() == "interpolation") {
        pieces.push(at..hole.start_byte());
        at = hole.end_byte();
    }
    pieces.push(at..node.end_byte());
    pieces.retain(|r| !r.is_empty());
    pieces
}

fn kind_of(capture: &str, node: &Node, text: &Rope) -> Option<Kind> {
    Some(match capture {
        "keyword" | "type.builtin" | "constant.builtin" => Kind::Keyword,
        "string" => Kind::String,
        "string.escape" => Kind::StringEscape,
        "number" => Kind::Number,
        "comment" if text.byte_slice(node.byte_range()).chars().take(3).eq("///".chars()) => Kind::DocComment,
        "comment" => Kind::Comment,
        "type" | "constructor" | "attribute" => Kind::Type,
        "function" => Kind::Method,
        "property.definition" => Kind::Property,
        "module" => Kind::Namespace,
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsharp_text::{Document, Selection, insert};

    const SOURCE: &str = "namespace App;\n\n/// The entry.\npublic class Program\n{\n    // say hello\n    static void Main() => System.Console.WriteLine(\"hi\\n\" + 42 + $\"at {name} ok\");\n}\n";

    fn kind_at(highlights: &[Highlight], text: &str, word: &str) -> Option<Kind> {
        let at = text.find(word).unwrap();
        highlights.iter().find(|h| h.range.contains(&at)).map(|h| h.kind)
    }

    #[test]
    fn csharp_has_rider_colours_before_any_server_answers() {
        let text = Rope::from_str(SOURCE);
        let tree = parse(&text, None);
        let h = highlights(&tree, &text, 0..text.len_bytes());
        assert_eq!(kind_at(&h, SOURCE, "public"), Some(Kind::Keyword));
        assert_eq!(kind_at(&h, SOURCE, "Program"), Some(Kind::Type));
        assert_eq!(kind_at(&h, SOURCE, "Main"), Some(Kind::Method));
        assert_eq!(kind_at(&h, SOURCE, "\"hi"), Some(Kind::String));
        assert_eq!(kind_at(&h, SOURCE, "\\n"), Some(Kind::StringEscape));
        assert_eq!(kind_at(&h, SOURCE, "42"), Some(Kind::Number));
        assert_eq!(kind_at(&h, SOURCE, "// say"), Some(Kind::Comment));
        assert_eq!(kind_at(&h, SOURCE, "/// The"), Some(Kind::DocComment));
        assert_eq!(kind_at(&h, SOURCE, "at {"), Some(Kind::String));
        assert_eq!(kind_at(&h, SOURCE, "name}"), None, "an interpolation's hole is code, not string");
        assert_eq!(kind_at(&h, SOURCE, " ok"), Some(Kind::String));
    }

    #[test]
    fn typing_never_colours_differently_from_a_fresh_parse() {
        let mut document = Document::new(SOURCE);
        let mut tree = parse(document.text(), None);
        let at = SOURCE.find("    static").unwrap();
        document.set_selections(vec![Selection::caret(at)]);
        for key in ["    int count = 0;\n", "\"unfinished", "\n"] {
            for change in document.apply(insert(document.text(), document.selections(), key)) {
                edit(&mut tree, &change);
            }
            tree = parse(document.text(), Some(&tree));
            let fresh = parse(document.text(), None);
            let all = 0..document.text().len_bytes();
            assert_eq!(
                highlights(&tree, document.text(), all.clone()),
                highlights(&fresh, document.text(), all),
                "after typing {key:?}, the reused parse coloured the file differently"
            );
        }
    }
}
