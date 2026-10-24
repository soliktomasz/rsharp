//! NuGet's order of versions, SemVer 2's precedence as NuGet applies it: up to four numbers (a missing one is 0, so
//! 1.0 is 1.0.0.0), a prerelease before its release, labels dot by dot (numbers by value and below words, words
//! ignoring case), build metadata after `+` ignored.

use std::cmp::Ordering;

pub fn is_prerelease(version: &str) -> bool {
    version.split('+').next().unwrap_or("").contains('-')
}

pub fn compare(a: &str, b: &str) -> Ordering {
    let (a, b) = (parse(a), parse(b));
    a.0.cmp(&b.0).then_with(|| match (a.1.is_empty(), b.1.is_empty()) {
        (true, true) => Ordering::Equal,
        (true, false) => Ordering::Greater,
        (false, true) => Ordering::Less,
        (false, false) => labels(&a.1, &b.1),
    })
}

/// The numbers (a word where a number should be counts as 0, so a malformed version still sorts) and the labels.
fn parse(version: &str) -> ([u64; 4], Vec<&str>) {
    let version = version.trim().split('+').next().unwrap_or("");
    let (numbers, release) = version.split_once('-').unwrap_or((version, ""));
    let mut parsed = [0; 4];
    for (slot, part) in parsed.iter_mut().zip(numbers.split('.')) {
        *slot = part.parse().unwrap_or(0);
    }
    (parsed, if release.is_empty() { Vec::new() } else { release.split('.').collect() })
}

fn labels(a: &[&str], b: &[&str]) -> Ordering {
    for (x, y) in a.iter().zip(b) {
        let order = match (x.parse::<u64>(), y.parse::<u64>()) {
            (Ok(x), Ok(y)) => x.cmp(&y),
            (Ok(_), Err(_)) => Ordering::Less,
            (Err(_), Ok(_)) => Ordering::Greater,
            (Err(_), Err(_)) => x.to_ascii_lowercase().cmp(&y.to_ascii_lowercase()),
        };
        if order != Ordering::Equal {
            return order;
        }
    }
    a.len().cmp(&b.len())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_sort_as_nuget_sorts_them() {
        let mut versions = vec![
            "1.0.0",
            "1.0.0-beta.11",
            "1.0.0-alpha",
            "2.0",
            "1.0.0-beta.2",
            "1.0.0-alpha.1",
            "1.0.0-RC.1",
            "1.0.0.1",
            "1.0.0-alpha.beta",
            "10.0.0",
            "1.0.0-beta",
        ];
        versions.sort_by(|a, b| compare(a, b));
        assert_eq!(
            versions,
            [
                "1.0.0-alpha",
                "1.0.0-alpha.1",
                "1.0.0-alpha.beta",
                "1.0.0-beta",
                "1.0.0-beta.2",
                "1.0.0-beta.11",
                "1.0.0-RC.1",
                "1.0.0",
                "1.0.0.1",
                "2.0",
                "10.0.0",
            ]
        );
    }

    #[test]
    fn missing_numbers_case_and_build_metadata_make_no_difference() {
        assert_eq!(compare("1.0", "1.0.0.0"), Ordering::Equal);
        assert_eq!(compare("1.0.0-Beta", "1.0.0-beta"), Ordering::Equal);
        assert_eq!(compare("1.0.0+abc", "1.0.0+def"), Ordering::Equal);
        assert!(is_prerelease("1.0.0-rc.1+build"));
        assert!(!is_prerelease("1.0.0+build-1"));
    }
}
