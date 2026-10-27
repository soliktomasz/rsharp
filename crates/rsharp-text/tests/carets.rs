//! Several carets, and undo that takes back a run of typing a word at a time, as Rider does.

use rsharp_text::*;

fn typed(document: &mut Document, text: &str) {
    for c in text.chars() {
        let plan = insert(document.text(), document.selections(), &c.to_string());
        document.apply(EditPlan { kind: EditKind::Typing, ..plan });
    }
}

#[test]
fn undo_takes_back_a_run_of_typing_a_word_at_a_time() {
    let mut document = Document::new("");
    typed(&mut document, "var total");
    document.undo();
    assert_eq!(document.text().to_string(), "var", "Undo took back more or less than the last word typed");
    document.undo();
    assert_eq!(document.text().to_string(), "", "Undo did not take back the first word in one step");
    document.redo();
    document.redo();
    assert_eq!(document.text().to_string(), "var total", "Redo did not put both words back");
}

#[test]
fn a_caret_moved_between_keys_starts_a_new_undo_step() {
    let mut document = Document::new("");
    typed(&mut document, "ab");
    document.set_selections(vec![Selection::caret(1)]);
    typed(&mut document, "x");
    document.undo();
    assert_eq!(document.text().to_string(), "ab", "typing after the caret moved joined the typing before it");
}

#[test]
fn a_run_of_backspaces_undoes_as_one() {
    let mut document = Document::new("hello");
    document.set_selections(vec![Selection::caret(5)]);
    for _ in 0..3 {
        document.apply(backspace(document.text(), document.selections()));
    }
    document.undo();
    assert_eq!(document.text().to_string(), "hello", "Undo after three backspaces did not bring back all three");
    assert_eq!(document.selections(), &[Selection::caret(5)], "Undo did not put the caret back where it was");
}

#[test]
fn typing_at_several_carets_types_at_each_and_undoes_as_one() {
    let mut document = Document::new("a\nb\nc\n");
    document.set_selections(vec![Selection::caret(0)]);
    let carets = clone_caret(document.text(), document.selections(), 1);
    let carets = clone_caret(document.text(), &carets, 1);
    document.set_selections(carets);
    typed(&mut document, "//");
    assert_eq!(document.text().to_string(), "//a\n//b\n//c\n", "a caret cloned below did not type");
    document.undo();
    assert_eq!(document.text().to_string(), "a\nb\nc\n");
}

#[test]
fn select_next_occurrence_takes_the_word_then_its_next_copies_wrapping() {
    let text = Rope::from_str("foo bar foo foo");
    let word = select_next_occurrence(&text, &[Selection::caret(9)]);
    assert_eq!(word, vec![Selection { anchor: 8, head: 11 }], "the first press did not select the word at the caret");
    let two = select_next_occurrence(&text, &word);
    assert_eq!(two.last(), Some(&Selection { anchor: 12, head: 15 }), "the next copy after the caret was not added");
    let three = select_next_occurrence(&text, &two);
    assert_eq!(three.last(), Some(&Selection { anchor: 0, head: 3 }), "the search did not wrap to the start");
    assert_eq!(select_next_occurrence(&text, &three), three, "a fourth press added a copy already selected");
    assert_eq!(unselect_last(&three), two, "Unselect Occurrence did not drop the copy added last");
}

#[test]
fn select_all_occurrences_selects_every_copy_with_the_callers_last() {
    let text = Rope::from_str("int a = a + a;");
    let all = select_all_occurrences(&text, &[Selection::caret(9)]);
    assert_eq!(all.len(), 3, "not every copy of 'a' was selected: {all:?}");
    assert_eq!(all.last(), Some(&Selection { anchor: 8, head: 9 }), "the copy at the caret is not the primary");
}

#[test]
fn a_column_selection_spans_the_same_columns_on_every_line() {
    let text = Rope::from_str("abcdef\nab\nabcdef\n");
    let rows = column_selection(&text, (0, 1), (2, 4));
    let selected: Vec<String> = rows.iter().map(|s| text.byte_slice(s.range()).to_string()).collect();
    assert_eq!(selected, ["bcd", "b", "bcd"], "a column selection did not keep its columns line by line");
}

#[test]
fn carets_that_meet_become_one() {
    let merged = merge_overlapping(vec![Selection::caret(3), Selection { anchor: 1, head: 4 }, Selection::caret(3)]);
    assert_eq!(merged, vec![Selection { anchor: 1, head: 4 }]);
}

#[test]
fn paste_of_one_line_per_caret_deals_a_line_to_each() {
    let mut document = Document::new("x\nx\n");
    let carets = vec![Selection::caret(2), Selection::caret(0)];
    document.apply(insert_each(&carets, &["1", "2"]));
    assert_eq!(document.text().to_string(), "1x\n2x\n", "lines were not dealt to the carets in the text's order");
}

#[test]
fn caret_per_line_puts_a_caret_at_the_end_of_each_line_selected() {
    let text = Rope::from_str("ab\ncd\nef\n");
    let carets = caret_per_line(&text, &[Selection { anchor: 1, head: 6 }]);
    assert_eq!(
        carets,
        vec![Selection::caret(2), Selection::caret(5)],
        "a selection ending at a line's start still put a caret on that line, or missed one it covers"
    );
}

#[test]
fn the_primary_caret_stays_last_through_a_merge_and_an_edit() {
    // The primary, last, comes first in the text; sorting must not take its place.
    let carets = vec![Selection::caret(4), Selection::caret(4), Selection::caret(0)];
    assert_eq!(merge_overlapping(carets), [Selection::caret(4), Selection::caret(0)], "a merge lost the primary");
    let text = Rope::from_str("ab\ncd\n");
    let plan = insert(&text, &[Selection::caret(3), Selection::caret(0)], "x");
    assert_eq!(plan.selections.last(), Some(&Selection::caret(1)), "typing at several carets lost the primary");
}

#[test]
fn the_word_at_the_caret_finds_whole_words_and_a_part_of_one_finds_every_copy() {
    let text = Rope::from_str("count counter count");
    let all = select_all_occurrences(&text, &[Selection::caret(1)]);
    assert_eq!(all.len(), 2, "the word at the caret matched inside a longer word: {all:?}");
    let word = select_next_occurrence(&text, &[Selection::caret(1)]);
    let next = select_next_occurrence(&text, &word);
    assert_eq!(next.last(), Some(&Selection { anchor: 14, head: 19 }), "Next stopped inside 'counter'");
    let part = select_all_occurrences(&text, &[Selection { anchor: 0, head: 3 }]);
    assert_eq!(part.len(), 3, "a selection of part of a word did not find every copy");
}

/// What a XAML file shows after typing, a quote at a time as the editor types one there, the caret as `|`.
fn typed_in_markup(start: &str, keys: &str) -> String {
    let at = start.find('|').unwrap();
    let mut document = Document::new(&start.replace('|', ""));
    document.set_selections(vec![Selection::caret(at)]);
    for c in keys.chars() {
        let plan = match c {
            '"' | '\'' => type_quote(document.text(), document.selections(), c),
            '\u{8}' => backspace_quotes(document.text(), document.selections()),
            _ => EditPlan { kind: EditKind::Typing, ..insert(document.text(), document.selections(), &c.to_string()) },
        };
        document.apply(plan);
    }
    let mut text = document.text().to_string();
    text.insert(document.selections()[0].head, '|');
    text
}

#[test]
fn a_quote_typed_after_an_attributes_equals_comes_with_its_closer_and_the_closer_typed_steps_over_it() {
    assert_eq!(
        typed_in_markup("<Button Width=|", "\""),
        "<Button Width=\"|\"",
        "the value's quote came without its closer"
    );
    assert_eq!(
        typed_in_markup("<Button Width=|", "\"40\" "),
        "<Button Width=\"40\" |",
        "the closing quote typed went in beside the one there instead of stepping over it"
    );
    assert_eq!(
        typed_in_markup("<Button Width=|", "'x'"),
        "<Button Width='x'|",
        "a single quote did not pair as a double does"
    );
    assert_eq!(typed_in_markup("<Button Width=|", "\"\u{8}"), "<Button Width=|", "Backspace left half of the pair");
    assert_eq!(
        typed_in_markup("<TextBlock Text=\"don|\" />", "'"),
        "<TextBlock Text=\"don'|\" />",
        "an apostrophe after a word came with a closer"
    );
}
