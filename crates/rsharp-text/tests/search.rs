//! Find and replace as Rider's find bar does them.

use rsharp_text::*;

fn query(text: &str) -> SearchQuery {
    SearchQuery { text: text.into(), ..Default::default() }
}

fn found(text: &str, q: &SearchQuery) -> Vec<String> {
    let rope = Rope::from_str(text);
    find_all(&rope, q, MAX_MATCHES).matches.iter().map(|m| text[m.clone()].to_string()).collect()
}

#[test]
fn a_plain_search_ignores_case_until_match_case_is_on() {
    assert_eq!(found("Count count", &query("count")), ["Count", "count"], "a search did not ignore case");
    let exact = SearchQuery { match_case: true, ..query("count") };
    assert_eq!(found("Count count", &exact), ["count"], "Match Case still found the other case");
    assert_eq!(found("a.b axb", &query("a.b")), ["a.b"], "a plain search read its text as a pattern");
}

#[test]
fn whole_words_skips_a_match_inside_a_longer_word() {
    let q = SearchQuery { whole_words: true, ..query("cat") };
    assert_eq!(found("catalog cat hotcat", &q), ["cat"], "Words found 'cat' inside another word");
}

#[test]
fn a_regex_that_does_not_parse_says_why_and_finds_nothing() {
    let result = find_all(&Rope::from_str("abc"), &SearchQuery { regex: true, ..query("(a") }, MAX_MATCHES);
    assert!(result.matches.is_empty());
    let problem = result.problem.expect("a broken pattern said nothing");
    assert!(problem.starts_with(char::is_uppercase) && !problem.contains('\n'), "the problem reads badly: {problem:?}");
}

#[test]
fn replace_all_fills_a_regexs_groups_from_each_match() {
    let text = Rope::from_str("int a = 1;\nint b = 2;\n");
    let q = SearchQuery { regex: true, ..query(r"int (\w)") };
    let edits = replace_all(&text, &q, "var $1");
    let mut document = Document::new("int a = 1;\nint b = 2;\n");
    document.apply(EditPlan { edits, selections: vec![Selection::caret(0)], ..Default::default() });
    assert_eq!(document.text().to_string(), "var a = 1;\nvar b = 2;\n", "a group was not carried into its match");
    assert_eq!(replacement(&text, &q, &(11..16), "var $1"), "var b", "one match's replacement took another's group");
}

#[test]
fn a_caret_pattern_leaves_the_files_last_newline_alone() {
    let q = SearchQuery { regex: true, ..query("^") };
    let edits = replace_all(&Rope::from_str("a\nb\n"), &q, "// ");
    assert_eq!(edits.len(), 2, "'^' matched the empty end after the last line break");
}

#[test]
fn next_and_previous_wrap_round_the_matches() {
    let result = find_all(&Rope::from_str("x.x.x"), &query("x"), MAX_MATCHES);
    assert_eq!(result.next_from(5), Some(0), "Next past the last match did not wrap to the first");
    assert_eq!(result.previous_from(0), Some(2), "Previous before the first match did not wrap to the last");
    let many = find_all(&Rope::from_str(&"x".repeat(MAX_MATCHES + 5)), &query("x"), MAX_MATCHES);
    assert!(many.truncated && many.matches.len() == MAX_MATCHES, "a search past the most matches did not stop");
}
