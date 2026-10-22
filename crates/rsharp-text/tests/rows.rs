//! Lines into rows: folded lines left out, long lines wrapped, and every offset finding its row.

use rsharp_text::*;

fn shown(text: &Rope, rows: &Rows) -> Vec<String> {
    (0..rows.len(text))
        .map(|i| {
            let r = rows.row(text, i);
            text.byte_slice(r.start..r.end).to_string()
        })
        .collect()
}

#[test]
fn nothing_folded_or_wrapped_builds_nothing() {
    assert!(matches!(Rows::build(&Rope::from_str("a\nb\n"), &[], None), Rows::Lines));
}

#[test]
fn folded_lines_are_left_out_and_their_offsets_show_on_the_folds_first_row() {
    let text = Rope::from_str("a {\n  b\n  c\n}\nd\n");
    let rows = Rows::build(&text, &[1..4], None);
    assert_eq!(shown(&text, &rows), ["a {", "d", ""], "folded lines still showed");
    assert_eq!(rows.row_of(&text, text.line_to_byte(2) + 1), 0, "an offset in folded lines lost its row");
    assert_eq!(rows.row_of(&text, text.line_to_byte(4)), 1);
}

#[test]
fn a_long_line_wraps_after_a_space_else_at_the_edge() {
    let text = Rope::from_str("one two three\nabcdefgh\n");
    let rows = Rows::build(&text, &[], Some(5));
    assert_eq!(shown(&text, &rows), ["one ", "two ", "three", "abcde", "fgh", ""], "a line wrapped in the wrong place");
    let first = rows.row(&text, 0);
    let second = rows.row(&text, 1);
    assert!(first.first && !first.last && !second.first, "the rows of a wrapped line did not say which ends it");
    assert_eq!(rows.row_of(&text, 4), 1, "a caret where a line wraps did not stand at the next row's start");
}
