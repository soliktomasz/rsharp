use rsharp_text::{DiffHunk, InlineDiff, MergeChoice, ThreeWayMerge, diff_hunks, rollback_hunk, word_diff};

#[test]
fn hunks_keep_insertions_deletions_and_the_final_newline_visible() {
    for (before, after, expected) in [
        ("", "", vec![]),
        ("", "α\n", vec![DiffHunk { before: 0..0, after: 0..1 }]),
        ("a\nb\nc\n", "a\nc\n", vec![DiffHunk { before: 1..2, after: 1..1 }]),
        ("a\n", "a", vec![DiffHunk { before: 0..1, after: 0..1 }]),
        ("a\n", "a\n\n", vec![DiffHunk { before: 1..1, after: 1..2 }]),
        ("a\r\nb\r\n", "a\r\nB\r\n", vec![DiffHunk { before: 1..2, after: 1..2 }]),
    ] {
        assert_eq!(diff_hunks(before, after), expected, "wrong hunk for {before:?} → {after:?}");
    }
}

#[test]
fn rolling_back_one_hunk_preserves_other_changes_and_exact_line_endings() {
    let before = "α\r\nb\r\nc\r\nd";
    let after = "Α\r\nb\r\nc\r\nD\r\n";
    let hunks = diff_hunks(before, after);
    assert_eq!(hunks.len(), 2);
    let edit = rollback_hunk(before, after, &hunks[1]);
    let mut result = after.to_owned();
    result.replace_range(edit.range, &edit.text);
    assert_eq!(result, "Α\r\nb\r\nc\r\nd");
}

#[test]
fn word_marks_use_utf8_bytes_and_preserve_whole_word_boundaries() {
    let before = "  żółty count;";
    let after = "  żółta count;";
    assert_eq!(word_diff(before, after), [InlineDiff { before: 2..10, after: 2..10 }]);
    assert_eq!(word_diff("a(b);", "a(b, c);"), [InlineDiff { before: 3..3, after: 3..6 }]);
    assert_eq!(word_diff("你好世界", "你好大世界"), [InlineDiff { before: 6..6, after: 6..9 }]);
    assert_eq!(word_diff("int value;", "long value;"), [InlineDiff { before: 0..3, after: 0..4 }]);
    assert!(word_diff("old", "new").is_empty(), "a whole-line rewrite needs no inline mark");
}

#[test]
fn word_marks_limit_large_rewrites() {
    let before = "word ".repeat(10_001);
    let after = "other ".repeat(10_001);
    assert!(word_diff(&before, &after).is_empty());
}

#[test]
fn nonoverlapping_changes_and_agreeing_edits_merge_automatically() {
    let merge = ThreeWayMerge::new("a\nb\nc\nd\n", "A\nb\nc\nd\n", "a\nb\nc\nD\n");
    assert_eq!(merge.result(), "A\nb\nc\nD\n");
    assert_eq!(merge.unresolved_count(), 0);
    let agreeing = ThreeWayMerge::new("a", "same\n", "same\n");
    assert_eq!(agreeing.result(), "same\n");
    assert_eq!(agreeing.unresolved_count(), 0);
}

#[test]
fn conflicts_keep_the_base_until_a_chunk_is_explicitly_resolved() {
    for (choice, expected) in [
        (MergeChoice::Left, "left\n"),
        (MergeChoice::Right, "right"),
        (MergeChoice::Base, "base\n"),
        (MergeChoice::Both, "left\nright"),
    ] {
        let mut merge = ThreeWayMerge::new("base\n", "left\n", "right");
        assert_eq!(merge.unresolved_count(), 1);
        assert_eq!(merge.result(), "base\n");
        assert!(merge.accept(0, choice));
        assert_eq!(merge.result(), expected);
        assert_eq!(merge.unresolved_count(), 0);
        assert!(!merge.accept(99, choice));
    }
}

#[test]
fn adjacent_line_edits_are_one_conflict_that_simple_word_resolution_can_resolve() {
    let mut merge = ThreeWayMerge::new("one\ntwo\n", "ONE\ntwo\n", "one\nTWO\n");
    assert_eq!(merge.chunks.len(), 1);
    assert_eq!(merge.unresolved_count(), 1);
    assert_eq!(merge.resolve_simple_conflicts(), 1);
    assert_eq!(merge.result(), "ONE\nTWO\n");
    assert_eq!(merge.unresolved_count(), 0);
}

#[test]
fn simple_resolution_combines_disjoint_words_but_refuses_overlapping_edits() {
    let mut merge = ThreeWayMerge::new("int count = 1;\n", "long count = 1;\n", "int count = 2;\n");
    assert!(merge.resolve_simple(0));
    assert_eq!(merge.result(), "long count = 2;\n");
    let mut conflict = ThreeWayMerge::new("int count;\n", "long count;\n", "short count;\n");
    assert!(!conflict.resolve_simple(0));
    assert_eq!(conflict.unresolved_count(), 1);
    assert_eq!(conflict.result(), "int count;\n");
    assert!(conflict.replace_chunk(0, "decimal count;".into()));
    assert_eq!(conflict.result(), "decimal count;");
    assert_eq!(conflict.unresolved_count(), 0);
}

#[test]
fn merges_preserve_empty_sides_insertions_deletions_crlf_and_final_newlines() {
    for (base, left, right, expected) in [
        ("", "α", "", "α"),
        ("a\n", "", "a\n", ""),
        ("a\r\nb", "A\r\nb", "a\r\nb", "A\r\nb"),
        ("a", "a\n", "a", "a\n"),
        ("a\n", "a", "a\n", "a"),
    ] {
        let merge = ThreeWayMerge::new(base, left, right);
        assert_eq!(merge.result(), expected);
        assert_eq!(merge.unresolved_count(), 0);
    }
    let mut add_add = ThreeWayMerge::new("", "left", "right");
    assert_eq!(add_add.unresolved_count(), 1);
    assert!(add_add.accept(0, MergeChoice::Both));
    assert_eq!(add_add.result(), "left\nright");
}

#[test]
fn accepting_both_unterminated_sides_uses_the_original_crlf_separator() {
    let mut merge = ThreeWayMerge::new("first\r\nbase", "first\r\nleft", "first\r\nright");
    assert!(merge.accept(0, MergeChoice::Both));
    assert_eq!(merge.result(), "first\r\nleft\r\nright");
}

#[test]
fn resolving_one_chunk_leaves_the_other_conflicts_explicit() {
    let mut merge = ThreeWayMerge::new("a\nb\nc\nd\ne\n", "A\nb\nc\nd\nE\n", "α\nb\nc\nd\nε\n");
    assert_eq!(merge.unresolved_count(), 2);
    assert!(merge.replace_chunk(0, "edited\n".into()));
    assert_eq!(merge.result(), "edited\nb\nc\nd\ne\n");
    assert_eq!(merge.unresolved_count(), 1);
    assert!(!merge.replace_chunk(99, String::new()));
}

#[test]
fn choosing_every_chunk_from_one_side_reproduces_that_side_exactly() {
    let texts = ["", "a", "a\n", "a\nb\n", "b\na", "a\na\na\n", "α\r\nβ", "\n\n", "a\nb\nc\n"];
    for base in texts {
        for left in texts {
            for right in texts {
                for (choice, expected) in
                    [(MergeChoice::Left, left), (MergeChoice::Right, right), (MergeChoice::Base, base)]
                {
                    let mut merge = ThreeWayMerge::new(base, left, right);
                    for index in 0..merge.chunks.len() {
                        assert!(merge.accept(index, choice));
                    }
                    assert_eq!(merge.result(), expected, "lost content with {base:?}, {left:?}, {right:?}, {choice:?}");
                }
            }
        }
    }
}
