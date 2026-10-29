//! Colour before any language server answers. C# is tree-sitter's grammar, and so is every other file this colours
//! (XML, JSON, CSS, JavaScript and TypeScript, HTML, Markdown, YAML), parsed incrementally (a parse reuses the tree
//! before the edit) on any thread, and coloured from syntax alone by the grammar's own highlights query, read only
//! over the lines asked for.
// shortcut: syntax colours only, so a type is coloured only where its syntax shows it; Roslyn's semantic tokens
// (Passenger's SemanticHighlighter) are the layer that colours every type reference.

use rsharp_text::{Change, Rope};
use std::ops::Range;
use std::path::Path;
use std::sync::OnceLock;
use tree_sitter::{InputEdit, Language, Node, Parser, Point, Query, QueryCursor, StreamingIterator, Tree};

pub mod testing;

/// What a piece of a file is, in Rider's colour scheme's terms: C#'s, then the markup's and data's (Rider's XML_TAG,
/// XML_ATTRIBUTE_NAME and _VALUE, XML_ENTITY_REFERENCE, JSON.PROPERTY_KEY, YAML_SCALAR_KEY, MARKDOWN_HEADER_LEVEL_1,
/// MARKDOWN_CODE_SPAN and MARKDOWN_LINK_DESTINATION).
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
    Tag,
    AttributeName,
    AttributeValue,
    Entity,
    JsonKey,
    YamlKey,
    Heading,
    Code,
    LinkDestination,
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Highlight {
    pub range: Range<usize>,
    pub kind: Kind,
}

/// The tree-sitter grammar a file is parsed and coloured by, chosen by its name as Passenger's `Language.ForPath`.
// shortcut: a language inside another (an HTML file's <script> and <style>, a Markdown fence, Markdown's inline
// emphasis and links) keeps its host's colour; colour it through the grammars' injection queries when one is asked for.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Hash)]
pub enum Grammar {
    CSharp,
    Xml,
    Json,
    Css,
    JavaScript,
    TypeScript,
    Tsx,
    Html,
    Markdown,
    Yaml,
}

impl Grammar {
    const ALL: usize = 10;

    pub fn for_path(path: &Path) -> Option<Grammar> {
        let extension = path.extension()?.to_str()?.to_ascii_lowercase();
        Some(match extension.as_str() {
            "cs" | "csx" => Grammar::CSharp,
            "csproj" | "fsproj" | "vbproj" | "proj" | "props" | "targets" | "xml" | "axaml" | "xaml" | "paml"
            | "slnx" | "resx" | "config" | "nuspec" | "manifest" | "runsettings" | "xsd" | "xslt" | "xsl" | "svg"
            | "plist" | "storyboard" => Grammar::Xml,
            "json" | "jsonc" => Grammar::Json,
            "css" => Grammar::Css,
            "js" | "mjs" | "cjs" | "jsx" => Grammar::JavaScript,
            "ts" | "mts" | "cts" => Grammar::TypeScript,
            "tsx" => Grammar::Tsx,
            "html" | "htm" => Grammar::Html,
            "md" | "markdown" => Grammar::Markdown,
            "yml" | "yaml" => Grammar::Yaml,
            _ => return None,
        })
    }

    fn language(self) -> Language {
        match self {
            Grammar::CSharp => tree_sitter_c_sharp::LANGUAGE.into(),
            Grammar::Xml => tree_sitter_xml::LANGUAGE_XML.into(),
            Grammar::Json => tree_sitter_json::LANGUAGE.into(),
            Grammar::Css => tree_sitter_css::LANGUAGE.into(),
            Grammar::JavaScript => tree_sitter_javascript::LANGUAGE.into(),
            Grammar::TypeScript => tree_sitter_typescript::LANGUAGE_TYPESCRIPT.into(),
            Grammar::Tsx => tree_sitter_typescript::LANGUAGE_TSX.into(),
            Grammar::Html => tree_sitter_html::LANGUAGE.into(),
            Grammar::Markdown => tree_sitter_md::LANGUAGE.into(),
            Grammar::Yaml => tree_sitter_yaml::LANGUAGE.into(),
        }
    }

    /// The grammar's own highlights query, compiled the first time a file of it is coloured. TypeScript's adds to
    /// JavaScript's, as its package lists them; JSX's tags are JavaScript's and TSX's.
    fn query(self) -> &'static Query {
        static QUERIES: [OnceLock<Query>; Grammar::ALL] = [const { OnceLock::new() }; Grammar::ALL];
        QUERIES[self as usize].get_or_init(|| {
            let js = tree_sitter_javascript::HIGHLIGHT_QUERY;
            let jsx = tree_sitter_javascript::JSX_HIGHLIGHT_QUERY;
            let ts = tree_sitter_typescript::HIGHLIGHTS_QUERY;
            let source = match self {
                Grammar::CSharp => tree_sitter_c_sharp::HIGHLIGHTS_QUERY.to_string(),
                Grammar::Xml => tree_sitter_xml::XML_HIGHLIGHT_QUERY.to_string(),
                Grammar::Json => tree_sitter_json::HIGHLIGHTS_QUERY.to_string(),
                Grammar::Css => tree_sitter_css::HIGHLIGHTS_QUERY.to_string(),
                Grammar::JavaScript => format!("{js}\n{jsx}"),
                Grammar::TypeScript => format!("{js}\n{ts}"),
                Grammar::Tsx => format!("{js}\n{jsx}\n{ts}"),
                Grammar::Html => tree_sitter_html::HIGHLIGHTS_QUERY.to_string(),
                Grammar::Markdown => tree_sitter_md::HIGHLIGHT_QUERY_BLOCK.to_string(),
                Grammar::Yaml => tree_sitter_yaml::HIGHLIGHTS_QUERY.to_string(),
            };
            Query::new(&self.language(), &source).expect("the grammar's own query compiles")
        })
    }

    /// The nodes that fold as a body: what is between a pair of braces, an element, a Markdown section.
    fn bodies(self) -> &'static [&'static str] {
        match self {
            Grammar::CSharp => &[
                "block",
                "declaration_list",
                "accessor_list",
                "switch_body",
                "enum_member_declaration_list",
                "initializer_expression",
            ],
            Grammar::Xml | Grammar::Html => &["element", "script_element", "style_element"],
            Grammar::Json => &["object", "array"],
            Grammar::Css => &["block"],
            Grammar::JavaScript | Grammar::TypeScript | Grammar::Tsx => &[
                "statement_block",
                "class_body",
                "object",
                "switch_body",
                "interface_body",
                "enum_body",
                "object_type",
            ],
            Grammar::Markdown => &["section", "fenced_code_block"],
            Grammar::Yaml => &["block_mapping_pair"],
        }
    }
}

/// Parses `text`, reusing `old` (already told the edits since, by [`edit`]) where nothing changed.
pub fn parse(grammar: Grammar, text: &Rope, old: Option<&Tree>) -> Tree {
    let mut parser = Parser::new();
    parser.set_language(&grammar.language()).expect("the grammar matches tree-sitter's version");
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
pub fn folds(grammar: Grammar, tree: &Tree, text: &Rope) -> Vec<FoldRegion> {
    folds_in(grammar, tree, text, 0..text.len_bytes())
}

/// The regions that fold among the nodes that meet `range`: what the lines on screen need, found in time that
/// grows with them and the tree's depth, not with the file.
pub fn folds_in(grammar: Grammar, tree: &Tree, text: &Rope, range: Range<usize>) -> Vec<FoldRegion> {
    let mut out = Vec::new();
    visit(grammar.bodies(), tree.root_node(), text, &range, &mut out);
    out.sort_by_key(|f| (f.range.start, std::cmp::Reverse(f.range.end)));
    out
}

fn fold_kind(node: &Node, text: &Rope) -> Option<FoldKind> {
    match node.kind() {
        "using_directive" => Some(FoldKind::Usings),
        "comment" if text.byte_slice(node.byte_range()).chars().take(3).eq("///".chars()) => Some(FoldKind::DocComment),
        "comment" | "Comment" => Some(FoldKind::Comment),
        _ => None,
    }
}

fn visit(bodies: &[&str], node: Node, text: &Rope, range: &Range<usize>, out: &mut Vec<FoldRegion>) {
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
                body if bodies.contains(&body) && !past && line(span.start) < line(span.end) => {
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
                visit(bodies, child, text, range, out);
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

/// What Safe Delete would take out for a name declared at `offset`: the declaration's bytes (its attributes and the
/// `///` comment above it included, from its line's start) and what it is; or why it takes out none.
pub struct Declaration {
    pub range: Range<usize>,
    pub kind: &'static str,
}

/// The declaration a name at `offset` declares: a type, a member, an enum member, a local or a local function.
pub fn declaration_at(tree: &Tree, text: &Rope, offset: usize) -> Result<Declaration, &'static str> {
    let at = tree.root_node().descendant_for_byte_range(offset, offset).ok_or("Nothing to delete here")?;
    let mut node = Some(at);
    while let Some(n) = node {
        let kind = match n.kind() {
            "class_declaration" | "record_declaration" => "class",
            "struct_declaration" | "record_struct_declaration" => "struct",
            "interface_declaration" => "interface",
            "enum_declaration" => "enum",
            "delegate_declaration" => "delegate",
            "method_declaration" | "local_function_statement" => "method",
            "constructor_declaration" | "destructor_declaration" => "constructor",
            "property_declaration" | "indexer_declaration" => "property",
            "event_declaration" | "event_field_declaration" => "event",
            "field_declaration" | "local_declaration_statement" => {
                // One of several declared together would take the others with it.
                let declarators = count(n, "variable_declarator");
                if declarators > 1 {
                    return Err(
                        "Safe Delete removes one of several declared together only with them: split them first",
                    );
                }
                if n.kind() == "field_declaration" { "field" } else { "local" }
            }
            "enum_member_declaration" => "enum member",
            "parameter" => return Err("Safe Delete does not delete a parameter: Change Signature does"),
            _ => {
                node = n.parent();
                continue;
            }
        };
        let mut start = n.start_byte();
        // The `///` comments just above it go with it.
        let mut above = n.prev_sibling();
        while let Some(comment) =
            above.filter(|c| c.kind() == "comment" && text.byte_slice(c.byte_range()).to_string().starts_with("///"))
        {
            start = comment.start_byte();
            above = comment.prev_sibling();
        }
        return Ok(Declaration { range: start..n.end_byte(), kind });
    }
    Err("Safe Delete removes a type, a member or a local, and none is declared here")
}

fn count(node: Node, kind: &str) -> usize {
    let mut cursor = node.walk();
    let mut found = usize::from(node.kind() == kind);
    for child in node.children(&mut cursor) {
        found += count(child, kind);
    }
    found
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
pub fn highlights(grammar: Grammar, tree: &Tree, text: &Rope, range: Range<usize>) -> Vec<Highlight> {
    let query = grammar.query();
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
        let Some(kind) = kind_of(grammar, query.capture_names()[capture.index as usize], &capture.node, text) else {
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

fn kind_of(grammar: Grammar, capture: &str, node: &Node, text: &Rope) -> Option<Kind> {
    let node_kind = node.kind();
    Some(match (grammar, capture) {
        (_, "comment") if text.byte_slice(node.byte_range()).chars().take(3).eq("///".chars()) => Kind::DocComment,
        (_, "comment") => Kind::Comment,
        (Grammar::CSharp, _) => match capture {
            "keyword" | "type.builtin" | "constant.builtin" => Kind::Keyword,
            "string" => Kind::String,
            "string.escape" => Kind::StringEscape,
            "number" => Kind::Number,
            "type" | "constructor" | "attribute" => Kind::Type,
            "function" => Kind::Method,
            "property.definition" => Kind::Property,
            "module" => Kind::Namespace,
            _ => return None,
        },
        // The whole tag in the tag's colour (its brackets and its =), as Rider's XML_TAG; a quote is its value's.
        (Grammar::Xml | Grammar::Html, _) => match capture {
            "tag" | "keyword" | "constant" if node_kind == "doctype" => Kind::Tag,
            "tag" | "keyword" => Kind::Tag,
            "punctuation.delimiter" | "punctuation.bracket" | "operator"
                if matches!(node_kind, "<" | ">" | "</" | "/>" | "<?" | "?>" | "<!" | "=") =>
            {
                Kind::Tag
            }
            "property" | "attribute" => Kind::AttributeName,
            "string" | "string.special" | "number" | "boolean" => Kind::AttributeValue,
            "constant" | "constant.builtin" => Kind::Entity,
            _ => return None,
        },
        (Grammar::Json, _) => match capture {
            "string.special.key" => Kind::JsonKey,
            "string" if is_key(node) => Kind::JsonKey,
            "string" => Kind::String,
            "escape" => Kind::StringEscape,
            "number" => Kind::Number,
            "constant.builtin" => Kind::Keyword,
            _ => return None,
        },
        // Selectors in the tag's colour, property names in the keyword's, values plain, units with their numbers.
        (Grammar::Css, _) => match capture {
            "property" if node_kind == "property_name" => Kind::Keyword,
            "tag" | "property" | "attribute" => Kind::Tag,
            "keyword" => Kind::Keyword,
            "string" => Kind::String,
            "number" | "type" | "string.special" => Kind::Number,
            "function" => Kind::Method,
            _ => return None,
        },
        (Grammar::JavaScript | Grammar::TypeScript | Grammar::Tsx, _) => match capture {
            "keyword" | "constant.builtin" | "type.builtin" => Kind::Keyword,
            "variable.builtin" if matches!(node_kind, "this" | "super") => Kind::Keyword,
            "string" | "string.special" => Kind::String,
            "number" => Kind::Number,
            "function" | "function.method" | "function.builtin" => Kind::Method,
            "type" | "constructor" => Kind::Type,
            "property" => Kind::Property,
            "tag" => Kind::Tag,
            "attribute" => Kind::AttributeName,
            _ => return None,
        },
        (Grammar::Markdown, _) => match capture {
            "text.title" => Kind::Heading,
            "punctuation.special" => Kind::Keyword,
            "text.literal" => Kind::Code,
            "text.uri" => Kind::LinkDestination,
            _ => return None,
        },
        // Keys in their colour, quoted scalars as strings, every plain scalar plain, as Rider's lexer.
        (Grammar::Yaml, _) => match capture {
            "property" => Kind::YamlKey,
            "string" if matches!(node_kind, "double_quote_scalar" | "single_quote_scalar") && is_key(node) => {
                Kind::YamlKey
            }
            "string" if matches!(node_kind, "double_quote_scalar" | "single_quote_scalar") => Kind::String,
            "attribute" => Kind::Keyword,
            _ => return None,
        },
    })
}

/// Whether a JSON string or a YAML scalar is a key: its pair's `key`, itself or through YAML's flow node.
fn is_key(node: &Node) -> bool {
    let Some(parent) = node.parent() else { return false };
    let (holder, pair) = if parent.kind() == "flow_node" { (parent, parent.parent()) } else { (*node, Some(parent)) };
    pair.and_then(|p| p.child_by_field_name("key")).is_some_and(|key| key.id() == holder.id())
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_declaration_is_found_from_a_name_in_it_with_its_doc_comment_and_a_parameter_refused() {
        let code = "class A\n{\n    /// Says it.\n    void Say(int times) { }\n    int a, b;\n}\n";
        let text = super::Rope::from(code);
        let tree = super::parse(super::Grammar::CSharp, &text, None);
        let method = super::declaration_at(&tree, &text, code.find("Say(").unwrap()).ok().unwrap();
        assert_eq!((&code[method.range.clone()], method.kind), ("/// Says it.\n    void Say(int times) { }", "method"));
        assert!(
            super::declaration_at(&tree, &text, code.find("times").unwrap()).is_err(),
            "a parameter was offered to delete"
        );
        assert!(
            super::declaration_at(&tree, &text, code.find("a, b").unwrap()).is_err(),
            "one of two fields took the other"
        );
    }

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
        let tree = parse(Grammar::CSharp, &text, None);
        let h = highlights(Grammar::CSharp, &tree, &text, 0..text.len_bytes());
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
        let mut tree = parse(Grammar::CSharp, document.text(), None);
        let at = SOURCE.find("    static").unwrap();
        document.set_selections(vec![Selection::caret(at)]);
        for key in ["    int count = 0;\n", "\"unfinished", "\n"] {
            for change in document.apply(insert(document.text(), document.selections(), key)) {
                edit(&mut tree, &change);
            }
            tree = parse(Grammar::CSharp, document.text(), Some(&tree));
            let fresh = parse(Grammar::CSharp, document.text(), None);
            let all = 0..document.text().len_bytes();
            assert_eq!(
                highlights(Grammar::CSharp, &tree, document.text(), all.clone()),
                highlights(Grammar::CSharp, &fresh, document.text(), all),
                "after typing {key:?}, the reused parse coloured the file differently"
            );
        }
    }

    /// The kind each `word` (its first occurrence in `source`) is coloured as, by the grammar for `name`.
    fn colours(name: &str, source: &str, words: &[&str]) -> Vec<Option<Kind>> {
        let grammar = Grammar::for_path(std::path::Path::new(name)).expect("the file has a grammar");
        let text = Rope::from_str(source);
        let h = highlights(grammar, &parse(grammar, &text, None), &text, 0..text.len_bytes());
        words.iter().map(|w| kind_at(&h, source, w)).collect()
    }

    #[test]
    fn other_files_have_rider_colours_from_their_own_grammars() {
        use Kind::*;
        let xml = "<?xml version=\"1.0\"?>\n<!-- note -->\n<Project Sdk=\"Microsoft.NET.Sdk\">&amp;</Project>\n";
        assert_eq!(
            colours("App.csproj", xml, &["<Project", "Project", "Sdk", "\"Microsoft", "&amp;", "<!--"]),
            [Some(Tag), Some(Tag), Some(AttributeName), Some(AttributeValue), Some(Entity), Some(Comment)],
            "an MSBuild file's tags, attributes, values, entities and comments are not Rider's"
        );
        let json = "{ \"name\": \"app\", \"n\": 4, \"on\": true } // c\n";
        assert_eq!(
            colours("tsconfig.json", json, &["\"name\"", "\"app\"", "4", "true", "// c"]),
            [Some(JsonKey), Some(String), Some(Number), Some(Keyword), Some(Comment)],
            "JSON's keys are not told apart from its values"
        );
        let css = ".card > a:hover { color: #fff; margin: 4px; }\n";
        assert_eq!(
            colours("site.css", css, &["card", "a:", "hover", "color", "#fff", "4px"]),
            [Some(Tag), Some(Tag), Some(Tag), Some(Keyword), Some(Number), Some(Number)],
            "CSS's selectors, property names and values are not Rider's"
        );
        let ts = "class Box { size: number = 3; open(): void { this.show(\"x\"); } }\n";
        assert_eq!(
            colours("box.ts", ts, &["class", "Box", "number", "3", "open", "this", "show", "\"x\""]),
            [
                Some(Keyword),
                Some(Type),
                Some(Keyword),
                Some(Number),
                Some(Method),
                Some(Keyword),
                Some(Method),
                Some(String)
            ],
            "TypeScript is not coloured as JavaScript with its types"
        );
        let tsx = "const a = <Card title=\"t\" />;\n";
        assert_eq!(colours("a.tsx", tsx, &["const", "title"]), [Some(Keyword), Some(AttributeName)]);
        assert_eq!(colours("a.js", tsx, &["const", "title"]), [Some(Keyword), Some(AttributeName)]);
        let html = "<!DOCTYPE html>\n<div class=\"x\">hi</div>\n";
        assert_eq!(
            colours("index.html", html, &["div", "class", "x\"", "hi"]),
            [Some(Tag), Some(AttributeName), Some(AttributeValue), None],
            "HTML's tags and attributes are not Rider's, or its text is coloured"
        );
        let md = "# Title\n\n- item [a](http://x)\n\n```\ncode\n```\n";
        assert_eq!(
            colours("README.md", md, &["# ", "Title", "- ", "code"]),
            [Some(Keyword), Some(Heading), Some(Keyword), Some(Code)],
            "Markdown's headings, markers and code are not Rider's"
        );
        let yaml = "name: build\n\"on\": [push]\nsteps: 3\n";
        assert_eq!(
            colours("ci.yml", yaml, &["name", "build", "\"on\"", "3"]),
            [Some(YamlKey), None, Some(YamlKey), None],
            "YAML's keys are not told apart, or a plain scalar is coloured"
        );
        assert_eq!(Grammar::for_path(std::path::Path::new("notes.txt")), None, "plain text took a grammar");
    }

    #[test]
    fn markup_and_data_fold_by_their_elements_and_objects() {
        let fold = |name: &str, source: &str| {
            let grammar = Grammar::for_path(std::path::Path::new(name)).unwrap();
            let text = Rope::from_str(source);
            folds(grammar, &parse(grammar, &text, None), &text).into_iter().map(|f| f.range).collect::<Vec<_>>()
        };
        let xml = "<Project>\n  <PropertyGroup>\n    <A>1</A>\n  </PropertyGroup>\n</Project>\n";
        assert_eq!(fold("a.props", xml).len(), 2, "an XML file's elements over several lines did not fold");
        let json = "{\n  \"a\": [\n    1\n  ]\n}\n";
        assert_eq!(fold("a.json", json).len(), 2, "a JSON object and array over several lines did not fold");
    }

    #[test]
    fn bodies_usings_comments_and_regions_fold() {
        let source = "using A;\nusing B;\n\n/// <summary>\n/// Doc\n/// </summary>\nclass C\n{\n    #region Fields\n    int x;\n    #endregion\n    void M()\n    {\n    }\n}\n";
        let text = Rope::from_str(source);
        let found: Vec<(FoldKind, &str)> = folds(Grammar::CSharp, &parse(Grammar::CSharp, &text, None), &text)
            .iter()
            .map(|f| (f.kind, &source[f.range.clone()]))
            .collect();
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
        let tree = parse(Grammar::CSharp, &Rope::from_str(source), None);
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
