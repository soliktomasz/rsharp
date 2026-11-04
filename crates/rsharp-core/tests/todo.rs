//! The TODO window's scan: Rider's TODO and FIXME patterns, inside a file's comments only.

use rsharp_core::todo::scan;

fn comments(text: &str) -> Vec<std::ops::Range<usize>> {
    // The comments as a parse would give them: each `//` to its line's end and each `/* … */`.
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        let rest = &text[at..];
        if rest.starts_with("//") {
            let end = rest.find('\n').map_or(text.len(), |n| at + n);
            found.push(at..end);
            at = end;
        } else if rest.starts_with("/*") {
            let end = rest.find("*/").map_or(text.len(), |n| at + n + 2);
            found.push(at..end);
            at = end;
        } else {
            at += rest.chars().next().unwrap().len_utf8();
        }
    }
    found
}

#[test]
fn todos_are_found_in_comments_only() {
    let text = "// TODO: pay the bill\nvar s = \"TODO not this\";\n/* fixme later */\nvar TodoList = 1; // todos are not TODO-words\n";
    let found = scan(text, &comments(text));
    let said: Vec<(&str, &str)> = found.iter().map(|t| (t.kind, t.text.as_str())).collect();
    assert_eq!(
        said,
        [("TODO", "TODO: pay the bill"), ("FIXME", "fixme later"), ("TODO", "TODO-words")],
        "a TODO outside a comment was listed, or one inside was missed"
    );
    assert_eq!(found[0].at, 3, "a TODO's place is not where its word starts");
}

#[test]
fn a_comment_with_no_todo_lists_nothing() {
    let text = "// nothing to do here\n";
    assert!(scan(text, &comments(text)).is_empty(), "\"to do\" in words was taken for a TODO");
}
