//! A long random editing session, every key checked against a plain string: what keeps the rope, the carets
//! and undo honest. Keep it passing rather than narrowing it.

use rsharp_text::*;

/// A small deterministic generator, so a failure names a seed that reproduces it.
struct Random(u64);

impl Random {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

const PIECES: &[&str] = &["a", "x", " ", "\n", "é", "😀", "{", "}", "var x = 1;", "\n    ", "ż"];

fn boundary(text: &str, random: &mut Random) -> usize {
    let mut at = random.below(text.len() + 1);
    while !text.is_char_boundary(at) {
        at -= 1;
    }
    at
}

#[test]
fn every_key_of_a_long_session_leaves_the_text_a_plain_string_would_hold() {
    for seed in 1..=20u64 {
        let mut random = Random(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15));
        let mut document = Document::new("class C\n{\n}\n");
        let mut model = String::from("class C\n{\n}\n");
        let mut history: Vec<String> = Vec::new();
        let mut future: Vec<String> = Vec::new();

        let mut typing: Option<&str> = None;
        for key in 0..2_000 {
            let op = random.below(11);
            let before = model.clone();
            let carets_before = document.selections().to_vec();
            match op {
                10 => {
                    let mut at: Vec<usize> = document.selections().iter().map(|s| s.head).collect();
                    at.sort();
                    at.dedup();
                    let piece = ["a", "z", "é", " "][random.below(4)];
                    let joins = typing.is_some_and(|last| !(piece == " " && last != " "));
                    document.set_selections(at.iter().map(|o| Selection::caret(*o)).collect());
                    let plan = insert(document.text(), document.selections(), piece);
                    document.apply(EditPlan { kind: EditKind::Typing, ..plan });
                    for o in at.iter().rev() {
                        model.insert_str(*o, piece);
                    }
                    if !joins {
                        history.push(before.clone());
                    }
                    future.clear();
                    typing = Some(piece);
                }
                0..=3 => {
                    let mut at: Vec<usize> = (0..1 + random.below(3)).map(|_| boundary(&model, &mut random)).collect();
                    at.sort();
                    at.dedup();
                    let piece = PIECES[random.below(PIECES.len())];
                    document.set_selections(at.iter().map(|o| Selection::caret(*o)).collect());
                    document.apply(insert(document.text(), document.selections(), piece));
                    for o in at.iter().rev() {
                        model.insert_str(*o, piece);
                    }
                }
                4 => {
                    let (a, b) = (boundary(&model, &mut random), boundary(&model, &mut random));
                    document.set_selections(vec![Selection { anchor: a, head: b }]);
                    // Runs of deleting join in their own test; here each one is a step the model can count.
                    let plan = backspace(document.text(), document.selections());
                    document.apply(EditPlan { kind: EditKind::Other, ..plan });
                    if a != b {
                        model.replace_range(a.min(b)..a.max(b), "");
                    } else if let Some(c) = model[..a].chars().next_back() {
                        model.replace_range(a - c.len_utf8()..a, "");
                    }
                }
                5 => {
                    let a = boundary(&model, &mut random);
                    document.set_selections(vec![Selection::caret(a)]);
                    let plan = delete(document.text(), document.selections());
                    document.apply(EditPlan { kind: EditKind::Other, ..plan });
                    if let Some(c) = model[a..].chars().next() {
                        model.replace_range(a..a + c.len_utf8(), "");
                    }
                }
                6..=7 => {
                    if let Some(previous) = history.pop() {
                        document.undo().expect("undo had a step to take back");
                        future.push(std::mem::replace(&mut model, previous));
                    }
                }
                _ => {
                    if let Some(next) = future.pop() {
                        document.redo().expect("redo had a step to put back");
                        history.push(std::mem::replace(&mut model, next));
                    }
                }
            }
            let changed = model != before;
            if op <= 5 && changed {
                history.push(before);
                future.clear();
            }
            // A key that changed nothing, as Redo with nothing to redo, leaves a run of typing going.
            if op != 10 && (changed || document.selections() != carets_before) {
                typing = None;
            }
            assert_eq!(
                document.text().to_string(),
                model,
                "seed {seed}, key {key}: the text drifted from what was typed"
            );
        }
    }
}

#[test]
fn a_file_saved_keeps_its_line_endings() {
    let mut document = Document::new("a\r\nb\r\n");
    document.set_selections(vec![Selection::caret(1)]);
    document.apply(newline(document.text(), document.selections(), &Indent::default()));
    assert_eq!(document.contents(), "a\r\n\r\nb\r\n");
}

#[test]
fn enter_after_an_opening_brace_indents_the_new_line() {
    let mut document = Document::new("    if (x) {");
    document.set_selections(vec![Selection::caret(12)]);
    document.apply(newline(document.text(), document.selections(), &Indent::default()));
    assert_eq!(document.text().to_string(), "    if (x) {\n        ");
}

#[test]
fn a_place_is_counted_in_utf16_for_the_language_server() {
    let document = Document::new("😀é = 1;\nx");
    let place = document.position("😀é".len());
    assert_eq!((place.line, place.utf16_column), (0, 3), "an emoji is two UTF-16 units, é one");
    assert_eq!(document.offset_of_utf16(0, 3), "😀é".len());
    assert_eq!(document.offset_of_utf16(1, 99), document.text().len_bytes(), "a column past the line's end is its end");
}

#[test]
fn a_classic_mac_file_keeps_its_lone_carriage_returns_and_a_changed_separator_is_a_change() {
    let mut document = Document::new("a\rb\r");
    assert_eq!(document.line_ending, LineEnding::Cr);
    assert_eq!(document.text().len_lines(), 3, "a CR file's breaks were not read as line breaks");
    assert_eq!(document.contents(), "a\rb\r");
    document.line_ending = LineEnding::Lf;
    assert!(document.is_modified(), "a new line separator did not count as a change to save");
    assert_eq!(document.contents(), "a\nb\n");
}

#[test]
fn a_files_indent_is_read_from_its_own_lines() {
    let two = Rope::from_str("class C\n{\n  void M()\n  {\n    x();\n  }\n}\n");
    assert_eq!(detect_indent(&two), Some(Indent { tabs: false, size: 2 }), "two-space indents were not seen");
    let tabs = Rope::from_str("class C\n{\n\tvoid M()\n\t{\n\t\tx();\n\t}\n}\n");
    assert_eq!(detect_indent(&tabs).map(|i| i.tabs), Some(true), "tab indents were not seen");
    assert_eq!(detect_indent(&Rope::from_str("x\ny\n")), None, "a file with no indents was given one");
    let plan = tab(&two, &[Selection::caret(0)], &Indent { tabs: false, size: 2 });
    assert_eq!(plan.edits[0].text, "  ", "Tab did not insert the file's own indent");
}
