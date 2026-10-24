//! Known vulnerabilities of packages, as restore's NuGetAudit reads them: each audited feed's VulnerabilityInfo
//! resource, an index of pages (a base and the updates since), each a map of lower-case ids to advisories.

use super::feed::PackageFeed;
use super::sources::Feed;
use super::version;
use std::collections::HashMap;

/// How grave an advisory says a vulnerability is: NuGet's four levels, 0 to 3 in a feed's data.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Severity {
    Low,
    Moderate,
    High,
    Critical,
}

impl Severity {
    pub fn name(self) -> &'static str {
        match self {
            Severity::Low => "Low",
            Severity::Moderate => "Moderate",
            Severity::High => "High",
            Severity::Critical => "Critical",
        }
    }
}

/// A known vulnerability: how grave, its advisory's page, and the versions it affects as a NuGet range.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Advisory {
    pub severity: Severity,
    pub url: String,
    pub versions: String,
}

impl Advisory {
    /// Whether a version is in the range, read as NuGet reads it: a bare version is the least version, a bracket
    /// includes its end and a parenthesis leaves it out, an empty end is unbounded, `[1.0]` is that version alone. A
    /// version that is not plain (a floating `1.*`) is in none.
    pub fn affects(&self, version: &str) -> bool {
        let range = self.versions.trim();
        if range.is_empty() || version.contains('*') || version.starts_with(['[', '(']) {
            return false;
        }
        if !range.starts_with(['[', '(']) {
            return version::compare(version, range).is_ge();
        }
        let (from_included, to_included) = (range.starts_with('['), range.ends_with(']'));
        let inner = &range[1..range.len().saturating_sub(1).max(1)];
        let Some((from, to)) = inner.split_once(',') else {
            return version::compare(version, inner.trim()).is_eq();
        };
        let (from, to) = (from.trim(), to.trim());
        let above = from.is_empty() || {
            let o = version::compare(version, from);
            o.is_gt() || (o.is_eq() && from_included)
        };
        let below = to.is_empty() || {
            let o = version::compare(version, to);
            o.is_lt() || (o.is_eq() && to_included)
        };
        above && below
    }
}

/// Every advisory the audited feeds know, by package, with the feeds that did not answer.
#[derive(Clone, Debug, Default)]
pub struct Vulnerabilities {
    by_package: HashMap<String, Vec<Advisory>>,
    pub failed: Vec<(String, String)>,
}

impl Vulnerabilities {
    /// The advisories that affect a version of a package, the gravest first.
    pub fn of(&self, id: &str, version: &str) -> Vec<&Advisory> {
        let mut found: Vec<_> =
            self.by_package.get(&id.to_lowercase()).into_iter().flatten().filter(|a| a.affects(version)).collect();
        found.sort_by(|a, b| b.severity.cmp(&a.severity));
        found
    }

    /// What fixes an installed version: the least offered version above it that no advisory affects.
    pub fn fixed_version<'a>(&self, id: &str, installed: &str, offered: &'a [String]) -> Option<&'a String> {
        offered
            .iter()
            .filter(|v| version::compare(v, installed).is_gt() && self.of(id, v).is_empty())
            .min_by(|a, b| version::compare(a, b))
    }

    /// Reads the feeds' advisories, each feed asked at once; a feed without the resource has none to give, as for
    /// NuGet. A feed that does not answer is named in `failed`, never thrown.
    pub fn read(feeds: &[Feed]) -> Vulnerabilities {
        let answers: Vec<_> = std::thread::scope(|scope| {
            let asked: Vec<_> = feeds
                .iter()
                .map(|feed| scope.spawn(move || (feed.name.clone(), read_feed(&PackageFeed::for_source(feed)))))
                .collect();
            asked.into_iter().map(|a| a.join().expect("reading a feed's vulnerabilities panicked")).collect()
        });
        Self::merged(answers)
    }

    /// The same, from feeds already made (a test's own credentials).
    pub fn read_feeds(feeds: &[PackageFeed]) -> Vulnerabilities {
        Self::merged(feeds.iter().map(|f| (f.name.clone(), read_feed(f))).collect())
    }

    fn merged(answers: Vec<(String, Result<HashMap<String, Vec<Advisory>>, String>)>) -> Vulnerabilities {
        let mut found = Vulnerabilities::default();
        for (feed, answer) in answers {
            match answer {
                Err(why) => found.failed.push((feed, why)),
                Ok(pages) => {
                    for (id, advisories) in pages {
                        let known = found.by_package.entry(id).or_default();
                        for advisory in advisories {
                            if !known.contains(&advisory) {
                                known.push(advisory);
                            }
                        }
                    }
                }
            }
        }
        found
    }
}

fn read_feed(feed: &PackageFeed) -> Result<HashMap<String, Vec<Advisory>>, String> {
    let Some((index, client)) = feed.vulnerability_index()? else { return Ok(HashMap::new()) };
    let pages = client.json(&index)?.unwrap_or_default();
    let mut read: HashMap<String, Vec<Advisory>> = HashMap::new();
    for page in pages.as_array().into_iter().flatten() {
        let Some(at) = page["@id"].as_str() else { continue };
        let url = if at.contains("://") {
            at.to_string()
        } else {
            format!("{}/{}", index.rsplit_once('/').map_or(index.as_str(), |(dir, _)| dir), at)
        };
        let entries = client.json(&url)?.unwrap_or_default();
        for (id, advisories) in entries.as_object().into_iter().flatten() {
            let known = read.entry(id.to_lowercase()).or_default();
            for entry in advisories.as_array().into_iter().flatten() {
                let (Some(url), Some(versions)) = (entry["url"].as_str(), entry["versions"].as_str()) else { continue };
                let severity = match entry["severity"].as_i64().unwrap_or(0).clamp(0, 3) {
                    0 => Severity::Low,
                    1 => Severity::Moderate,
                    2 => Severity::High,
                    _ => Severity::Critical,
                };
                let advisory = Advisory { severity, url: url.into(), versions: versions.into() };
                if !known.contains(&advisory) {
                    known.push(advisory);
                }
            }
        }
    }
    Ok(read)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn advisory(versions: &str) -> Advisory {
        Advisory { severity: Severity::High, url: "https://advisory.example".into(), versions: versions.into() }
    }

    #[test]
    fn ranges_read_as_nuget_reads_them() {
        let cases = [
            ("(, 1.4.1)", "1.4.0", true),
            ("(, 1.4.1)", "1.4.1", false),
            ("[2.0.0, 2.3.1)", "2.0.0", true),
            ("[2.0.0, 2.3.1)", "2.3.1", false),
            ("(2.0.0, 2.3.1]", "2.0.0", false),
            ("(2.0.0, 2.3.1]", "2.3.1", true),
            ("[1.0]", "1.0.0", true),
            ("[1.0]", "1.0.1", false),
            ("1.2.0", "1.1.9", false),
            ("1.2.0", "5.0.0", true),
            ("[3.0.0, )", "3.0.0-beta", false),
            ("[1.0, 2.0)", "1.*", false),
        ];
        for (range, version, affected) in cases {
            assert_eq!(advisory(range).affects(version), affected, "{version} in {range}");
        }
    }

    #[test]
    fn the_fix_is_the_least_offered_version_no_advisory_affects() {
        let mut data = Vulnerabilities::default();
        data.by_package.insert(
            "demo.core".into(),
            vec![advisory("[1.0.0, 1.2.0)"), Advisory { severity: Severity::Critical, ..advisory("[1.3.0]") }],
        );
        let offered: Vec<String> = ["2.0.0", "1.3.0", "1.2.0", "1.1.0", "1.0.0"].map(String::from).into();
        assert_eq!(data.of("Demo.Core", "1.0.0").len(), 1);
        assert_eq!(data.fixed_version("Demo.Core", "1.0.0", &offered).map(String::as_str), Some("1.2.0"));
        assert_eq!(
            data.fixed_version("Demo.Core", "1.2.0", &offered).map(String::as_str),
            Some("2.0.0"),
            "the fix offered is itself affected"
        );
        assert_eq!(data.of("demo.core", "1.3.0")[0].severity, Severity::Critical);
    }
}
