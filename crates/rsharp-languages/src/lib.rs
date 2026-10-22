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

/// What a region that folds holds, so Rider's Collapse to Definitions can pick bodies and doc comments.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum FoldKind {
    Body,
    Usings,
    Comment,
    DocComment,
    Region,
}

/// A stretch of code that folds: from its start, which stays shown, to its end, over more than one line.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct FoldRegion {
    pub range: Range<usize>,
    pub kind: FoldKind,
}

/// Every region that folds, sorted by start, as Rider's: a body in braces, a run of usings, a comment over several
/// lines (or a run of line comments), and `#region` to `#endregion`.
pub fn folds(tree: &Tree, text: &Rope) -> Vec<FoldRegion> {
    folds_in(tree, text, 0..text.len_bytes())
}

/// The regions that fold among the nodes that meet `range`: what the lines on screen need, found in time that
/// grows with them and the tree's depth, not with the file.
pub fn folds_in(tree: &Tree, text: &Rope, range: Range<usize>) -> Vec<FoldRegion> {
    let mut out = Vec::new();
    visit(tree.root_node(), text, &range, &mut out);
    out.sort_by_key(|f| (f.range.start, std::cmp::Reverse(f.range.end)));
    out
}

fn fold_kind(node: &Node, text: &Rope) -> Option<FoldKind> {
    match node.kind() {
        "using_directive" => Some(FoldKind::Usings),
        "comment" if text.byte_slice(node.byte_range()).chars().take(3).eq("///".chars()) => Some(FoldKind::DocComment),
        "comment" => Some(FoldKind::Comment),
        _ => None,
    }
}

fn visit(node: Node, text: &Rope, range: &Range<usize>, out: &mut Vec<FoldRegion>) {
    let line = |byte: usize| text.byte_to_line(byte.min(text.len_bytes()));
    let close = |run: &mut Option<(FoldKind, Range<usize>)>, out: &mut Vec<FoldRegion>| {
        if let Some((kind, range)) = run.take()
            && line(range.start) < line(range.end)
        {
            out.push(FoldRegion { range, kind });
        }
    };
    // The first child that meets the range, stepped back over a run of usings or comments leading into it.
    // A cursor finds it through the tree's balanced hidden nodes, and steps between siblings without finding their
    // parent from the root each time as a node's own steps do; a class of a thousand members makes those slow.
    let mut cursor = node.walk();
    if cursor.goto_first_child_for_byte(range.start).is_none() {
        return;
    }
    while cursor.goto_previous_sibling() {
        if fold_kind(&cursor.node(), text).is_none() {
            cursor.goto_next_sibling();
            break;
        }
    }
    // Usings and line comments fold as runs of siblings, a line each, nothing but blanks between.
    let mut run: Option<(FoldKind, Range<usize>)> = None;
    let mut regions: Vec<usize> = Vec::new();
    loop {
        let child = cursor.node();
        let span = child.byte_range();
        let past = span.start > range.end;
        if past && regions.is_empty() && run.is_none() {
            break;
        }
        if let Some(kind) = fold_kind(&child, text).filter(|_| !past || run.is_some()) {
            let one_line = line(span.start) == line(span.end.saturating_sub(1));
            match &mut run {
                Some((k, r))
                    if *k == kind
                        && one_line
                        && line(span.start) == line(r.end) + 1
                        && text.byte_slice(r.end..span.start).chars().all(char::is_whitespace) =>
                {
                    r.end = span.end;
                }
                _ => {
                    close(&mut run, out);
                    if past {
                    } else if one_line {
                        run = Some((kind, span));
                    } else {
                        out.push(FoldRegion { range: span, kind });
                    }
                }
            }
        } else {
            close(&mut run, out);
            match child.kind() {
                "block"
                | "declaration_list"
                | "accessor_list"
                | "switch_body"
                | "enum_member_declaration_list"
                | "initializer_expression"
                    if !past && line(span.start) < line(span.end) =>
                {
                    out.push(FoldRegion { range: span.clone(), kind: FoldKind::Body });
                }
                "preproc_region" if !past => regions.push(span.start),
                // Past the range, siblings are read only for the #endregion of a #region seen in it.
                "preproc_endregion" => {
                    if let Some(start) = regions.pop() {
                        out.push(FoldRegion { range: start..span.end, kind: FoldKind::Region });
                    }
                }
                _ => {}
            }
            if !past && child.child_count() > 0 && span.end >= range.start {
                visit(child, text, range, out);
            }
        }
        if !cursor.goto_next_sibling() {
            break;
        }
    }
    close(&mut run, out);
}

const BRACKETS: &[(&str, &str)] = &[("{", "}"), ("(", ")"), ("[", "]")];

/// The bracket at or just before `offset`, and the one it pairs with in the syntax.
pub fn matching_bracket(tree: &Tree, offset: usize) -> Option<(Range<usize>, Range<usize>)> {
    let at = |o: usize| {
        let node = tree.root_node().descendant_for_byte_range(o, o + 1)?;
        (node.start_byte() == o && BRACKETS.iter().any(|(a, b)| node.kind() == *a || node.kind() == *b)).then_some(node)
    };
    let node = at(offset).or_else(|| offset.checked_sub(1).and_then(at))?;
    let parent = node.parent()?;
    let mut cursor = parent.walk();
    let siblings: Vec<Node> = parent.children(&mut cursor).collect();
    let index = siblings.iter().position(|n| n.id() == node.id())?;
    let pair = if let Some((_, close)) = BRACKETS.iter().find(|(open, _)| node.kind() == *open) {
        siblings[index + 1..].iter().rev().find(|n| n.kind() == *close)
    } else {
        let (open, _) = BRACKETS.iter().find(|(_, close)| node.kind() == *close)?;
        siblings[..index].iter().find(|n| n.kind() == *open)
    }?;
    Some((node.byte_range(), pair.byte_range()))
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

    #[test]
    fn bodies_usings_comments_and_regions_fold() {
        let source = "using A;\nusing B;\n\n/// <summary>\n/// Doc\n/// </summary>\nclass C\n{\n    #region Fields\n    int x;\n    #endregion\n    void M()\n    {\n    }\n}\n";
        let text = Rope::from_str(source);
        let found: Vec<(FoldKind, &str)> =
            folds(&parse(&text, None), &text).iter().map(|f| (f.kind, &source[f.range.clone()])).collect();
        let kinds: Vec<FoldKind> = found.iter().map(|f| f.0).collect();
        assert_eq!(
            kinds,
            [FoldKind::Usings, FoldKind::DocComment, FoldKind::Body, FoldKind::Region, FoldKind::Body],
            "not every region folds, or one folds that should not: {found:?}"
        );
        assert_eq!(found[0].1, "using A;\nusing B;", "the usings did not fold as one run");
    }

    #[test]
    fn a_bracket_finds_its_pair_from_either_side() {
        let source = "class C { void M(int a) { var x = new[] { 1 }; } }";
        let tree = parse(&Rope::from_str(source), None);
        let open = source.find("(int").unwrap();
        let close = source.find(") {").unwrap();
        assert_eq!(matching_bracket(&tree, open), Some((open..open + 1, close..close + 1)));
        assert_eq!(
            matching_bracket(&tree, close + 1),
            Some((close..close + 1, open..open + 1)),
            "a caret just after a closing bracket did not find its pair"
        );
        assert_eq!(matching_bracket(&tree, source.find("void").unwrap()), None);
    }
}
