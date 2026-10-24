//! The package sources a folder's restore uses, read as NuGet.Configuration reads them: the nuget.config files from the
//! folder up, then the user's and the machine's, a closer file's key winning and a `<clear />` ending the files after
//! it, with the packageSourceMapping that keeps a package to its feeds.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

pub const NUGET_ORG: &str = "https://api.nuget.org/v3/index.json";

/// Where a nuget.config stands in NuGet's order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Place {
    Solution,
    Folder,
    User,
    Machine,
}

impl Place {
    pub fn name(self) -> &'static str {
        match self {
            Place::Solution => "Solution",
            Place::Folder => "Folder",
            Place::User => "User",
            Place::Machine => "Machine",
        }
    }
}

/// A feed as one file writes it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFeed {
    pub name: String,
    pub value: String,
    pub disabled_here: bool,
}

/// A nuget.config in effect: its feeds, whether it clears those of the files after it, or why it does not read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ConfigFile {
    pub path: PathBuf,
    pub place: Place,
    pub feeds: Vec<ConfigFeed>,
    pub clears: bool,
    pub problem: Option<String>,
}

/// A feed in effect: where it is (a folder made absolute, or a URL), whether it is enabled, and the file naming it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Feed {
    pub name: String,
    pub source: String,
    pub enabled: bool,
    pub protocol_version: Option<u32>,
    pub defined_in: Option<PathBuf>,
}

impl Feed {
    pub fn is_http(&self) -> bool {
        let lower = self.source.to_ascii_lowercase();
        lower.starts_with("http://") || lower.starts_with("https://")
    }
}

/// The user's and the machine's NuGet folders: the person's, or a test's own.
#[derive(Clone, Debug)]
pub struct Places {
    pub user: PathBuf,
    pub machine: PathBuf,
}

impl Places {
    /// NuGet's own: on macOS and Linux the user's under DOTNET_CLI_HOME when set, else the home folder.
    pub fn current() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty()).map(PathBuf::from);
        if cfg!(windows) {
            Places {
                user: var("APPDATA").unwrap_or_default().join("NuGet"),
                machine: var("ProgramFiles(x86)")
                    .or_else(|| var("ProgramFiles"))
                    .unwrap_or_default()
                    .join("NuGet/Config"),
            }
        } else {
            Places {
                user: var("DOTNET_CLI_HOME").unwrap_or_else(crate::programs::home).join(".nuget/NuGet"),
                machine: if cfg!(target_os = "macos") {
                    "/Library/Application Support/NuGet/Config".into()
                } else {
                    "/etc/opt/NuGet/Config".into()
                },
            }
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct SourceSet {
    pub root: PathBuf,
    /// NuGet's order: the folder's first, the machine's last.
    pub files: Vec<ConfigFile>,
    pub feeds: Vec<Feed>,
    /// Each feed's patterns, by feed name in lower case; empty when no file maps packages.
    mapping: HashMap<String, Vec<String>>,
}

impl SourceSet {
    pub fn load(root: &Path, places: &Places) -> SourceSet {
        let mut files = Vec::new();
        let mut read_files: Vec<Parsed> = Vec::new();
        let mut add = |files: &mut Vec<ConfigFile>, path: PathBuf, place: Place| {
            let canonical = crate::paths::canonical(&path);
            if !files.iter().any(|f: &ConfigFile| f.path == canonical) {
                let (file, parsed) = read(&canonical, place);
                files.push(file);
                read_files.extend(parsed);
            }
        };
        for (i, folder) in root.ancestors().enumerate() {
            for name in ["nuget.config", "NuGet.config", "NuGet.Config"] {
                let path = folder.join(name);
                if path.is_file() {
                    add(&mut files, path, if i == 0 { Place::Solution } else { Place::Folder });
                    break;
                }
            }
        }
        let user = places.user.join("NuGet.Config");
        for extra in config_files(&places.user.join("config")) {
            add(&mut files, extra, Place::User);
        }
        let has_user = user.is_file();
        if has_user {
            add(&mut files, user.clone(), Place::User);
        }
        for machine in config_files(&places.machine) {
            add(&mut files, machine, Place::Machine);
        }

        // A file's <clear /> ends the files after it, for that section alone.
        let in_effect = |section: &str| -> Vec<&Parsed> {
            let mut kept = Vec::new();
            for parsed in &read_files {
                kept.push(parsed);
                if parsed.cleared.iter().any(|s| s == section) {
                    break;
                }
            }
            kept
        };
        let mut disabled: HashMap<String, bool> = HashMap::new();
        for parsed in in_effect("disabledPackageSources").iter().rev() {
            for (key, value) in parsed.section("disabledPackageSources") {
                disabled.insert(key.to_lowercase(), value.eq_ignore_ascii_case("true"));
            }
        }
        let mut feeds: Vec<Feed> = Vec::new();
        let sources = in_effect("packageSources");
        for parsed in &sources {
            for add in parsed.adds("packageSources") {
                if feeds.iter().any(|f| f.name.eq_ignore_ascii_case(&add.key)) {
                    continue;
                }
                feeds.push(Feed {
                    enabled: !disabled.get(&add.key.to_lowercase()).copied().unwrap_or(false),
                    source: resolve(&add.value, &parsed.path),
                    protocol_version: add.protocol_version,
                    defined_in: Some(parsed.path.clone()),
                    name: add.key.clone(),
                });
            }
        }
        // NuGet writes the user's file naming nuget.org when there is none; it is taken as written, never written here.
        let cleared_before_user = read_files.iter().any(|p| {
            matches!(p.place, Place::Solution | Place::Folder) && p.cleared.iter().any(|s| s == "packageSources")
        });
        if !has_user && !cleared_before_user {
            feeds.push(Feed {
                name: "nuget.org".into(),
                source: NUGET_ORG.into(),
                enabled: !disabled.get("nuget.org").copied().unwrap_or(false),
                protocol_version: Some(3),
                defined_in: None,
            });
        }
        let mut mapping: HashMap<String, Vec<String>> = HashMap::new();
        for parsed in in_effect("packageSourceMapping").iter().rev() {
            for (feed, patterns) in &parsed.mapping {
                mapping.insert(feed.to_lowercase(), patterns.clone());
            }
        }
        SourceSet { root: root.into(), files, feeds, mapping }
    }

    pub fn enabled(&self) -> impl Iterator<Item = &Feed> {
        self.feeds.iter().filter(|f| f.enabled)
    }

    pub fn mapped(&self) -> bool {
        !self.mapping.is_empty()
    }

    /// Whether restore may take a package from a feed: always without a mapping, else when that feed holds the
    /// package's longest matching pattern (an exact id, `Prefix.*` or `*`).
    pub fn allows(&self, feed: &str, id: &str) -> bool {
        if self.mapping.is_empty() {
            return true;
        }
        let id = id.to_lowercase();
        let strength = |pattern: &str| {
            let pattern = pattern.to_lowercase();
            match pattern.strip_suffix('*') {
                Some(prefix) if id.starts_with(prefix) => Some(prefix.len()),
                // An exact id outranks every prefix.
                None if pattern == id => Some(usize::MAX),
                _ => None,
            }
        };
        let best = |patterns: &[String]| patterns.iter().filter_map(|p| strength(p)).max();
        let longest = self.mapping.values().filter_map(|p| best(p)).max();
        longest.is_some() && self.mapping.get(&feed.to_lowercase()).and_then(|p| best(p)) == longest
    }
}

fn config_files(folder: &Path) -> Vec<PathBuf> {
    let mut files: Vec<_> = std::fs::read_dir(folder)
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("config")))
        .collect();
    files.sort();
    files
}

/// A value as NuGet takes it: `%VAR%` expanded, a relative folder made absolute against its file's folder.
fn resolve(value: &str, file: &Path) -> String {
    let expanded = regex::Regex::new(r"%([^%]+)%")
        .unwrap()
        .replace_all(value, |c: &regex::Captures| std::env::var(&c[1]).unwrap_or_else(|_| c[0].to_string()))
        .into_owned();
    let lower = expanded.to_ascii_lowercase();
    if lower.starts_with("http://") || lower.starts_with("https://") {
        return expanded;
    }
    let path = match lower.strip_prefix("file://") {
        Some(_) => crate::paths::from_uri(&expanded).unwrap_or_else(|| PathBuf::from(&expanded)),
        None => PathBuf::from(&expanded),
    };
    let path = if path.is_absolute() { path } else { file.parent().unwrap_or(Path::new("")).join(path) };
    normalized(&path).to_string_lossy().into_owned()
}

/// `a/./b/../c` as `a/c`, without touching the disk (the folder may not exist yet).
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            std::path::Component::CurDir => {}
            std::path::Component::ParentDir => {
                out.pop();
            }
            other => out.push(other),
        }
    }
    out
}

struct Add {
    key: String,
    value: String,
    protocol_version: Option<u32>,
}

struct Parsed {
    path: PathBuf,
    place: Place,
    sections: HashMap<String, Vec<Add>>,
    cleared: Vec<String>,
    mapping: Vec<(String, Vec<String>)>,
}

impl Parsed {
    fn adds(&self, section: &str) -> &[Add] {
        self.sections.get(section).map(Vec::as_slice).unwrap_or(&[])
    }
    fn section(&self, section: &str) -> impl Iterator<Item = (&str, &str)> {
        self.adds(section).iter().map(|a| (a.key.as_str(), a.value.as_str()))
    }
}

fn read(path: &Path, place: Place) -> (ConfigFile, Option<Parsed>) {
    match parse(path, place) {
        Ok(parsed) => {
            let disabled: Vec<String> = parsed
                .section("disabledPackageSources")
                .filter(|(_, v)| v.eq_ignore_ascii_case("true"))
                .map(|(k, _)| k.to_lowercase())
                .collect();
            let feeds = parsed
                .adds("packageSources")
                .iter()
                .map(|a| ConfigFeed {
                    name: a.key.clone(),
                    value: a.value.clone(),
                    disabled_here: disabled.contains(&a.key.to_lowercase()),
                })
                .collect();
            let clears = parsed.cleared.iter().any(|s| s == "packageSources");
            (ConfigFile { path: path.into(), place, feeds, clears, problem: None }, Some(parsed))
        }
        Err(why) => (ConfigFile { path: path.into(), place, feeds: vec![], clears: false, problem: Some(why) }, None),
    }
}

fn parse(path: &Path, place: Place) -> Result<Parsed, String> {
    use quick_xml::events::Event;
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    let (text, _) = crate::encodings::decode(&bytes, None);
    let mut reader = quick_xml::Reader::from_str(&text);
    let mut parsed =
        Parsed { path: path.into(), place, sections: HashMap::new(), cleared: Vec::new(), mapping: Vec::new() };
    // The open elements' names, the root's first.
    let mut open: Vec<String> = Vec::new();
    let mut root_seen = false;
    loop {
        let event = reader.read_event().map_err(|e| format!("{} at byte {}", e, reader.buffer_position()))?;
        let (start, empty) = match &event {
            Event::Start(e) => (Some(e.clone()), false),
            Event::Empty(e) => (Some(e.clone()), true),
            Event::End(_) => {
                open.pop();
                continue;
            }
            Event::Eof if open.is_empty() => break,
            Event::Eof => return Err(format!("the file ends inside <{}>", open.last().unwrap())),
            _ => continue,
        };
        let Some(element) = start else { continue };
        let name = String::from_utf8_lossy(element.local_name().as_ref()).into_owned();
        let attribute = |wanted: &str| {
            element.attributes().flatten().find(|a| a.key.local_name().as_ref() == wanted.as_bytes()).map(|a| {
                a.unescape_value().map(|v| v.into_owned()).unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into())
            })
        };
        match open.len() {
            0 => {
                if name != "configuration" {
                    return Err("the file has no configuration element".into());
                }
                root_seen = true;
            }
            2 if open[1] == "packageSourceMapping" && name == "packageSource" => {
                parsed.mapping.push((attribute("key").unwrap_or_default(), Vec::new()));
            }
            3 if open[1] == "packageSourceMapping" && name == "package" => {
                if let (Some(last), Some(pattern)) = (parsed.mapping.last_mut(), attribute("pattern")) {
                    last.1.push(pattern);
                }
            }
            2 if name == "clear" => parsed.cleared.push(open[1].clone()),
            2 if name == "add" => {
                if let Some(key) = attribute("key") {
                    parsed.sections.entry(open[1].clone()).or_default().push(Add {
                        key,
                        value: attribute("value").unwrap_or_default(),
                        protocol_version: attribute("protocolVersion").and_then(|v| v.parse().ok()),
                    });
                }
            }
            _ => {}
        }
        if !empty {
            open.push(name);
        }
    }
    if !root_seen {
        return Err("the file has no configuration element".into());
    }
    Ok(parsed)
}
