//! How a typed query finds a name, as Rider's popups match: the query's letters in order, spaces ignored,
//! case ignored, scored higher where they start words or run together.

/// The match's score, higher is better, or `None` when the letters are not all there in order.
pub fn score(query: &str, text: &str) -> Option<i32> {
    let wanted: Vec<char> = query.chars().filter(|c| !c.is_whitespace()).flat_map(char::to_lowercase).collect();
    if wanted.is_empty() {
        return Some(0);
    }
    let chars: Vec<char> = text.chars().collect();
    let starts_word = |i: usize| {
        i == 0 || {
            let (before, here) = (chars[i - 1], chars[i]);
            !before.is_alphanumeric() || (before.is_lowercase() && here.is_uppercase())
        }
    };
    let mut score = 0;
    let mut at = 0;
    let mut last: Option<usize> = None;
    for want in wanted {
        let is = |i: usize| chars[i].to_lowercase().eq(std::iter::once(want));
        let next = (at..chars.len()).find(|&i| is(i))?;
        // A run going on beats a jump to a later word start ("sav" in "Save"); otherwise word starts win.
        let found = if last.is_some_and(|l| next == l + 1) {
            next
        } else {
            (at..chars.len()).find(|&i| starts_word(i) && is(i)).unwrap_or(next)
        };
        score += match (starts_word(found), last.is_some_and(|l| found == l + 1)) {
            (true, _) => 10,
            (false, true) => 5,
            (false, false) => 1,
        };
        score -= (found - at) as i32 / 4;
        last = Some(found);
        at = found + 1;
    }
    if text.to_lowercase().starts_with(&query.to_lowercase()) {
        score += 20;
    }
    Some(score)
}

#[cfg(test)]
mod tests {
    use super::score;

    #[test]
    fn a_query_finds_a_name_by_its_word_starts_and_ranks_the_closer_one_first() {
        assert!(score("cltab", "Close Tab").is_some());
        assert!(score("close tab", "Close Tab").is_some());
        assert!(score("xyz", "Close Tab").is_none());
        assert!(score("tc", "Close Tab").is_none(), "letters out of order matched");
        assert!(
            score("ct", "Close Tab") > score("ct", "Select All Text"),
            "word starts ranked below scattered letters"
        );
        assert!(score("sav", "Save") > score("sav", "Close Solution and Save"), "a prefix ranked below a later match");
        assert!(score("gtf", "GoToFile").is_some(), "camel-case humps are word starts");
    }
}
