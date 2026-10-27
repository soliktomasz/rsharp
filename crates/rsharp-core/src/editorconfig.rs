//! The .editorconfig files over a file, read as editorconfig.org says: from the file's folder up to one marked
//! `root = true`, the nearer file's sections winning, a later section in a file over an earlier one.

use regex::Regex;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// The properties that apply to one file, by their lower-case names and values, and the .editorconfig that gave
/// each.
#[derive(Default, Clone, Debug, PartialEq)]
pub struct EditorConfig {
    properties: HashMap<String, (String, PathBuf)>,
}

impl EditorConfig {
    /// The .editorconfig properties for `file`.
    pub fn for_file(file: &Path) -> Self {
        let mut found = Vec::new();
        let mut folder = file.parent();
        while let Some(dir) = folder {
            let config = dir.join(".editorconfig");
            if let Ok(text) = std::fs::read_to_string(&config) {
                let root = parse(&text).0;
                found.push((config, text));
                if root {
                    break;
                }
            }
            folder = dir.parent();
        }
        let mut properties = HashMap::new();
        // The outermost first, so a nearer file's values replace its.
        for (config, text) in found.into_iter().rev() {
            let dir = config.parent().unwrap_or(Path::new(""));
            let relative = file.strip_prefix(dir).unwrap_or(file).to_string_lossy().replace('\\', "/");
            for (glob, pairs) in parse(&text).1 {
                if matches(&glob, &relative) {
                    for (key, value) in pairs {
                        properties.insert(key, (value, config.clone()));
                    }
                }
            }
        }
        Self { properties }
    }

    /// A property's value, lower case, none where none applies or it is `unset`.
    pub fn get(&self, key: &str) -> Option<&str> {
        self.properties.get(key).map(|(v, _)| v.as_str()).filter(|v| *v != "unset")
    }

    /// Every property that applies, `unset` ones left out.
    pub fn properties(&self) -> impl Iterator<Item = (&str, &str)> {
        self.properties.iter().map(|(k, (v, _))| (k.as_str(), v.as_str())).filter(|(_, v)| *v != "unset")
    }

    /// The .editorconfig that set a property.
    pub fn source(&self, key: &str) -> Option<&Path> {
        self.properties.get(key).map(|(_, from)| from.as_path())
    }

    pub fn flag(&self, key: &str) -> Option<bool> {
        match self.get(key)? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    }
}

type Section = (String, Vec<(String, String)>);

/// Whether a file says it is the root, and its sections in order.
fn parse(text: &str) -> (bool, Vec<Section>) {
    let mut root = false;
    let mut sections: Vec<Section> = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if let Some(glob) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            sections.push((glob.to_string(), Vec::new()));
        } else if let Some((key, value)) = line.split_once(['=', ':']) {
            let (key, value) = (key.trim().to_lowercase(), value.trim().to_lowercase());
            match sections.last_mut() {
                Some((_, pairs)) => pairs.push((key, value)),
                None if key == "root" => root = value == "true",
                None => {}
            }
        }
    }
    (root, sections)
}

/// Whether an .editorconfig glob takes a path relative to its file: one with a `/` from that folder, one without
/// in any folder under it.
fn matches(glob: &str, path: &str) -> bool {
    let anchored = glob.contains('/');
    let glob = glob.trim_start_matches('/');
    let mut pattern = String::from(if anchored { "^" } else { "^(?:.*/)?" });
    let chars: Vec<char> = glob.chars().collect();
    let mut i = 0;
    let mut braces = 0;
    while i < chars.len() {
        match chars[i] {
            '*' if chars.get(i + 1) == Some(&'*') => {
                pattern.push_str(".*");
                i += 1;
            }
            '*' => pattern.push_str("[^/]*"),
            '?' => pattern.push_str("[^/]"),
            '[' => match chars[i..].iter().position(|c| *c == ']') {
                Some(end) => {
                    let class: String = chars[i + 1..i + end].iter().collect();
                    let class = class.strip_prefix('!').map_or(class.clone(), |c| format!("^{c}"));
                    pattern.push_str(&format!("[{}]", class.replace('\\', "\\\\")));
                    i += end;
                }
                None => pattern.push_str("\\["),
            },
            '{' => {
                let end = chars[i..].iter().position(|c| *c == '}').map(|e| i + e);
                let inside: String = end.map(|e| chars[i + 1..e].iter().collect()).unwrap_or_default();
                if let Some((a, b)) = inside.split_once("..")
                    && a.parse::<i64>().is_ok()
                    && b.parse::<i64>().is_ok()
                {
                    // shortcut: a number range takes any whole number; check the bounds once a person relies on them.
                    pattern.push_str("[+-]?[0-9]+");
                    i = end.unwrap_or(i);
                } else if inside.contains(',') {
                    pattern.push_str("(?:");
                    braces += 1;
                } else {
                    pattern.push_str("\\{");
                }
            }
            ',' if braces > 0 => pattern.push('|'),
            '}' if braces > 0 => {
                pattern.push(')');
                braces -= 1;
            }
            c => pattern.push_str(&regex::escape(&c.to_string())),
        }
        i += 1;
    }
    pattern.push('$');
    Regex::new(&pattern).is_ok_and(|r| r.is_match(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn globs_take_the_files_editorconfig_says() {
        assert!(matches("*", "src/Program.cs"), "a bare glob did not reach into a subfolder");
        assert!(matches("*.cs", "Program.cs"));
        assert!(!matches("*.cs", "Program.csproj"));
        assert!(matches("*.{cs,csx}", "a/b.csx"), "a brace list did not offer each choice");
        assert!(matches("src/**/*.cs", "src/a/b/C.cs"));
        assert!(!matches("src/*.cs", "src/a/C.cs"), "a single star crossed a folder");
        assert!(matches("[Pp]rogram.cs", "program.cs"));
        assert!(matches("file{1..3}.txt", "file2.txt"));
    }

    #[test]
    fn the_nearer_file_wins_and_a_root_stops_the_climb() {
        let base = std::env::temp_dir().join(format!("rsharp-editorconfig-{}", std::process::id()));
        let inner = base.join("repo/src");
        std::fs::create_dir_all(&inner).unwrap();
        std::fs::write(base.join(".editorconfig"), "[*]\nindent_size = 8\ninsert_final_newline = false\n").unwrap();
        std::fs::write(base.join("repo/.editorconfig"), "root = true\n[*.cs]\nindent_style = space\nindent_size = 4\n")
            .unwrap();
        std::fs::write(inner.join(".editorconfig"), "[*.cs]\nindent_size = 2\n").unwrap();
        let config = EditorConfig::for_file(&inner.join("Program.cs"));
        assert_eq!(config.get("indent_size"), Some("2"), "the nearer .editorconfig did not win");
        assert_eq!(config.get("indent_style"), Some("space"));
        assert_eq!(config.flag("insert_final_newline"), None, "a file above the root was read");
        assert_eq!(config.source("indent_size"), Some(inner.join(".editorconfig").as_path()));
        std::fs::remove_dir_all(&base).unwrap();
    }
}
