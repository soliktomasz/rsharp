//! Two texts' lines side by side in one list, as a diff shows them: those both have, and those only one has.

use imara_diff::intern::InternedInput;
use imara_diff::{Algorithm, diff};
use std::ops::Range;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum LineChange {
    Same,
    /// Only in the text before.
    Removed,
    /// Only in the text after.
    Added,
}

/// Every line of `before` and `after` in order, each with whether both have it, its break left off.
pub fn line_diff<'a>(before: &'a str, after: &'a str) -> Vec<(LineChange, &'a str)> {
    // imara-diff reads a text's lines with their breaks, as split_inclusive does, so the indices agree.
    let old: Vec<&str> = before.split_inclusive('\n').collect();
    let new: Vec<&str> = after.split_inclusive('\n').collect();
    let input = InternedInput::new(before, after);
    let mut hunks: Vec<(Range<u32>, Range<u32>)> = Vec::new();
    diff(Algorithm::Histogram, &input, |b: Range<u32>, a: Range<u32>| hunks.push((b, a)));
    let line = |s: &'a str| s.strip_suffix('\n').unwrap_or(s);
    let mut out = Vec::with_capacity(old.len().max(new.len()));
    let mut at = 0;
    for (b, a) in hunks {
        out.extend(old[at..b.start as usize].iter().map(|l| (LineChange::Same, line(l))));
        out.extend(old[b.start as usize..b.end as usize].iter().map(|l| (LineChange::Removed, line(l))));
        out.extend(new[a.start as usize..a.end as usize].iter().map(|l| (LineChange::Added, line(l))));
        at = b.end as usize;
    }
    out.extend(old[at..].iter().map(|l| (LineChange::Same, line(l))));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_changed_line_shows_as_removed_then_added_between_the_lines_both_have() {
        let lines = line_diff("a\nb\nc\n", "a\nB\nc\nd\n");
        assert_eq!(
            lines,
            [
                (LineChange::Same, "a"),
                (LineChange::Removed, "b"),
                (LineChange::Added, "B"),
                (LineChange::Same, "c"),
                (LineChange::Added, "d"),
            ]
        );
    }
}
