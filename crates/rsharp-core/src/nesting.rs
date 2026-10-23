//! Rider's file nesting in the Solution view: a file shown under the one it belongs to, one level deep, by its
//! project's DependentUpon and, while "Show files with the same names as nested" is on, by Rider's rules.

use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The languages Rider's CLR rules are made for (its nestingRulesLanguageExtensions).
const LANGUAGES: &[&str] = &[".cs", ".vb"];

/// Rider 2026.2's own rules, (parent suffix, child suffix), as its ProjectViewNestingRulesProvider extensions add them
/// (Passenger's `FileNesting.RiderRules`).
fn rider_rules() -> Vec<(String, String)> {
    let mut rules: Vec<(String, String)> = Vec::new();
    let mut add = |parent: &str, child: &str| rules.push((parent.to_string(), child.to_string()));
    for code in LANGUAGES {
        add(code, &format!(".designer{code}"));
        add(code, ".resx");
        add(code, &format!(".context{code}"));
        add(".xaml", &format!(".xaml{code}"));
        add(".resx", &format!(".designer{code}"));
        add(".settings", &format!(".designer{code}"));
        add(".tt", &format!(".generated{code}"));
        add(".edmx", &format!(".designer{code}"));
        add(".xsd", &format!(".designer{code}"));
        add(".cshtml", &format!(".cshtml{code}"));
        add(".razor", &format!(".razor{code}"));
        for page in [".aspx", ".asax", ".ascx", ".master"] {
            add(page, &format!("{page}{code}"));
            add(page, &format!("{page}.designer{code}"));
        }
    }
    for markup in [".razor", ".cshtml"] {
        for asset in [".css", ".scss", ".sass", ".js"] {
            add(markup, &format!("{markup}{asset}"));
        }
    }
    for child in [".min.js", ".min.js.map", ".map", ".js.map", "-compiled.js", "-compiled.js.map"] {
        add(".js", child);
    }
    for (parent, child) in [
        (".cjs", ".cjs.map"),
        (".mjs", ".mjs.map"),
        (".jsx", ".js"),
        (".jsx", ".js.map"),
        (".es6", ".js"),
        (".es6", ".js.map"),
    ] {
        add(parent, child);
    }
    for (source, outputs) in [
        (".ts", &[".js", ".mjs", ".cjs"][..]),
        (".mts", &[".js", ".mjs"]),
        (".cts", &[".js", ".cjs"]),
        (".tsx", &[".js", ".jsx"]),
    ] {
        for output in outputs {
            add(source, output);
            add(source, &format!("{output}.map"));
        }
    }
    for style in [".less", ".sass", ".scss"] {
        add(style, ".css");
        add(style, ".css.map");
    }
    let mut seen = std::collections::HashSet::new();
    rules.retain(|r| seen.insert(r.clone()));
    rules
}

/// The rules each combined, in lower case as names are matched with those that go on from its child or lead to its parent, as IntelliJ's
/// FileNestingBuilder combines them: .cs to .resx and .resx to .designer.cs make .cs to .designer.cs.
static COMBINED: LazyLock<Vec<(String, String)>> = LazyLock::new(|| {
    let mut all = rider_rules();
    let mut known: std::collections::HashSet<(String, String)> =
        all.iter().map(|(p, c)| (p.to_lowercase(), c.to_lowercase())).collect();
    let mut at = 0;
    while at < all.len() {
        let (parent, child) = all[at].clone();
        for (p, c) in all.clone() {
            if p.eq_ignore_ascii_case(&child) && known.insert((parent.to_lowercase(), c.to_lowercase())) {
                all.push((parent.clone(), c.clone()));
            }
            if c.eq_ignore_ascii_case(&parent) && known.insert((p.to_lowercase(), child.to_lowercase())) {
                all.push((p.clone(), child.clone()));
            }
        }
        at += 1;
    }
    all.into_iter().map(|(p, c)| (p.to_lowercase(), c.to_lowercase())).collect()
});

/// ASP.NET Core's settings for an environment go under its settings for all, as Visual Studio nests them.
static ENVIRONMENT_SETTINGS: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^appsettings\.[^.]+\.json$").unwrap());

fn name_of(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// The files of one folder shown under another of them, each to the file it goes under: what the project says it
/// depends on first, then Rider's rules when `by_name`. Nesting is one level, as Rider shows it: a file whose parent
/// is nested itself goes under that one's parent, and files that go round in a circle nest nowhere.
pub fn parents(
    files: &[PathBuf],
    dependent_upon: impl Fn(&Path) -> Option<PathBuf>,
    by_name: bool,
) -> HashMap<PathBuf, PathBuf> {
    let mut named: HashMap<String, &PathBuf> = HashMap::new();
    for file in files {
        named.entry(name_of(file).to_lowercase()).or_insert(file);
    }
    let mut candidates: HashMap<&PathBuf, Vec<&PathBuf>> = HashMap::new();
    for file in files {
        let name = name_of(file);
        let mut found: Vec<&PathBuf> = Vec::new();
        if let Some(declared) = dependent_upon(file)
            && let Some(owner) = named.get(&name_of(&declared).to_lowercase())
            && **owner == declared
            && *owner != file
        {
            found.push(owner);
        }
        if by_name {
            let lower = name.to_lowercase();
            for (parent, child) in COMBINED.iter() {
                let child = child.to_lowercase();
                if lower.len() > child.len()
                    && lower.ends_with(&child)
                    && let Some(matched) =
                        named.get(&format!("{}{}", &lower[..lower.len() - child.len()], parent.to_lowercase()))
                    && *matched != file
                    && !found.contains(matched)
                {
                    found.push(matched);
                }
            }
            if ENVIRONMENT_SETTINGS.is_match(&name)
                && let Some(settings) = named.get("appsettings.json")
                && !found.contains(settings)
            {
                found.push(settings);
            }
        }
        if !found.is_empty() {
            candidates.insert(file, found);
        }
    }
    candidates
        .iter()
        .filter_map(|(child, found)| top(child, found, &candidates).map(|top| ((*child).clone(), top.clone())))
        .collect()
}

/// The file a nested one goes under: of those it could, the first nested in none, else the one the first goes under.
fn top<'a>(
    child: &'a PathBuf,
    found: &[&'a PathBuf],
    candidates: &HashMap<&'a PathBuf, Vec<&'a PathBuf>>,
) -> Option<&'a PathBuf> {
    if let Some(top) = found.iter().find(|p| !candidates.contains_key(**p)) {
        return Some(top);
    }
    let mut seen = vec![child];
    let mut at = found[0];
    while let Some(above) = candidates.get(at) {
        if seen.contains(&at) {
            return None;
        }
        seen.push(at);
        at = above.iter().find(|p| !candidates.contains_key(**p)).copied().unwrap_or(above[0]);
    }
    (at != child).then_some(at)
}
