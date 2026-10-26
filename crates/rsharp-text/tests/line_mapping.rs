//! A line carried through edits to the text, as a breakpoint follows its code.

use rsharp_text::{Document, Edit, EditPlan, map_line};

/// The changes a plan makes to `text`, applied in order, and the document after.
fn edited(text: &str, edits: Vec<(std::ops::Range<usize>, &str)>) -> (Document, Vec<rsharp_text::Change>) {
    let mut document = Document::new(text);
    let changes = document.apply(EditPlan {
        edits: edits.into_iter().map(|(range, text)| Edit { range, text: text.into() }).collect(),
        ..Default::default()
    });
    (document, changes)
}

#[test]
fn a_line_moves_as_the_edit_carries_its_code() {
    // A line inserted above the file's first line: every line below moves down one.
    let (_, changes) = edited("one\ntwo\nthree\n", vec![(0..0, "zero\n")]);
    assert_eq!(map_line(&changes, 0), 1, "the first line's code is on line 1 now");
    assert_eq!(map_line(&changes, 2), 3, "and the last on line 3");

    // A line break typed in the middle of a line: the text before it stays, the rest moves down.
    let (_, changes) = edited("alpha\nbeta\n", vec![(2..2, "\n")]);
    assert_eq!(map_line(&changes, 0), 0, "the line's start is before what was typed");
    assert_eq!(map_line(&changes, 1), 2, "the next line moved down");

    // Lines deleted: the code after them moves up, and a line deleted with them goes where the deletion began.
    let (_, changes) = edited("one\ntwo\nthree\nfour\n", vec![(4..8, "")]);
    assert_eq!(map_line(&changes, 2), 1, "three is on line 1 now");
    assert_eq!(map_line(&changes, 3), 2, "four is on line 2");
    assert_eq!(map_line(&changes, 1), 1, "the deleted line's breakpoint stands where the deletion began");
}

#[test]
fn a_breakpoint_on_the_moved_statement_stays_with_it() {
    // The greeting line, with a using directive added above: its line moves from 4 to 5.
    let source =
        "static class Greeter\n{\n    public static void Greet()\n    {\n        Console.WriteLine();\n    }\n}\n";
    let using = source.find('s').unwrap();
    let (document, changes) = edited(source, vec![(using..using, "using System;\n")]);
    let line = document.text().to_string().lines().position(|line| line.contains("Console.WriteLine")).unwrap();
    assert_eq!(line, 5);
    assert_eq!(map_line(&changes, 4), 5, "the statement's breakpoint follows it");
}
