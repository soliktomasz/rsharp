//! A key costs what it changed: the UI thread's work after each key (the text edited, the tree told of it, the
//! regions that fold and the colours for the lines on screen) stays the same on a file ten times larger. The parse
//! runs on a background thread and is left out: tree-sitter's incremental parse grows with the members of one
//! class, which a generated file can have by the hundred thousand.

use rsharp_languages::*;
use rsharp_text::*;
use std::time::Instant;

fn source(methods: usize) -> String {
    let mut text = String::from("namespace P1;\npublic class Big\n{\n");
    for m in 0..methods {
        text.push_str(&format!("    public string M{m}(string s) => s + \"{m}\";\n"));
    }
    text.push_str("}\n");
    text
}

/// The median time of a key's work, in microseconds, typing in the middle of a file of `methods` lines.
fn per_key(methods: usize) -> f64 {
    let mut document = Document::new(&source(methods));
    let mut tree = parse(Grammar::CSharp, document.text(), None);
    let caret = document.text().line_to_byte(methods / 2) + 4;
    document.set_selections(vec![Selection::caret(caret)]);
    let mut times = Vec::new();
    for _ in 0..200 {
        let started = Instant::now();
        let changes = document.apply(insert(document.text(), document.selections(), "x"));
        for change in &changes {
            edit(&mut tree, change);
        }
        // The tree as the UI thread has it between a key and the parse that follows: told of the edit.
        let at = document.selections()[0].head;
        let line = document.text().byte_to_line(at);
        let shown = document.text().line_to_byte(line.saturating_sub(30))
            ..document.text().line_to_byte((line + 30).min(document.text().len_lines() - 1));
        std::hint::black_box(folds_in(Grammar::CSharp, &tree, document.text(), shown.clone()));
        std::hint::black_box(highlights(Grammar::CSharp, &tree, document.text(), shown));
        times.push(started.elapsed().as_secs_f64() * 1e6);
    }
    times.sort_by(f64::total_cmp);
    times[times.len() / 2]
}

#[test]
fn a_key_costs_the_same_on_a_file_ten_times_larger() {
    let small = per_key(20_000);
    let large = per_key(200_000);
    eprintln!("a key's work: {small:.0} µs on 20,000 lines, {large:.0} µs on 200,000");
    // Work in proportion to the file would be ten times; what stays near the caret grows only with the tree's depth.
    assert!(large < small * 3., "a key on 200,000 lines cost {large:.0} µs against {small:.0} µs on 20,000");
}
