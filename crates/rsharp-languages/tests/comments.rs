//! The comments a file holds, from its own grammar's tree: what the TODO window scans.

use rsharp_languages::{Grammar, comments_in, parse};
use rsharp_text::Rope;

#[test]
fn a_files_comments_are_its_grammars_comment_nodes() {
    let source = "// first\nclass A\n{\n    string s = \"// not a comment\";\n    /* second */\n    /// third\n}\n";
    let text = Rope::from_str(source);
    let tree = parse(Grammar::CSharp, &text, None);
    let found: Vec<&str> = comments_in(&tree).into_iter().map(|r| &source[r]).collect();
    assert_eq!(found, ["// first", "/* second */", "/// third"], "a comment was missed, or a string taken for one");
}

#[test]
fn an_xml_files_comments_are_found_too() {
    let source = "<Project>\n  <!-- TODO: drop this -->\n  <PropertyGroup />\n</Project>\n";
    let text = Rope::from_str(source);
    let tree = parse(Grammar::Xml, &text, None);
    let found: Vec<&str> = comments_in(&tree).into_iter().map(|r| &source[r]).collect();
    assert_eq!(found, ["<!-- TODO: drop this -->"], "a project file's TODO comment was not found");
}
