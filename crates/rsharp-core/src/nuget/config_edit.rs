//! Changes one nuget.config as `dotnet nuget` changes it with `--configfile`: a feed added, edited, removed, enabled or
//! disabled, and its credentials, in packageSources, disabledPackageSources and packageSourceCredentials. Only the
//! elements changed are written, indented as their neighbours are; every other character of the file stays as it
//! was (its declaration or none, comments, quotes, byte order mark and line ends).

use super::secret;
use super::sources::encode_name;
use std::path::Path;

const SOURCES: &str = "packageSources";
const DISABLED: &str = "disabledPackageSources";
const CREDENTIALS: &str = "packageSourceCredentials";
const EMPTY: &str = "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<configuration>\n</configuration>\n";

/// A feed as the Sources tab's form gives it. A password of none keeps the one the file has; no user name takes
/// the credentials out.
#[derive(Clone, PartialEq, Eq)]
pub struct FeedEdit {
    pub name: String,
    pub url: String,
    pub enabled: bool,
    pub allow_insecure: bool,
    pub username: Option<String>,
    pub password: Option<String>,
}

impl std::fmt::Debug for FeedEdit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} ({})", self.name, self.url)
    }
}

pub fn add(text: &str, feed: &FeedEdit) -> Result<String, String> {
    let mut config = Config::new(text)?;
    if config.child(SOURCES, &feed.name).is_some() {
        return Err(format!("The file has a feed named {} already", feed.name));
    }
    let element = feed_element(feed, None);
    config.add_child(SOURCES, &element)?;
    credential(&mut config, feed, None)?;
    enable(&mut config, &feed.name, feed.enabled)?;
    Ok(config.text)
}

/// Changes a feed the file names, under its old name; a new name takes its disabling, credentials and mapping along.
pub fn update(text: &str, name: &str, feed: &FeedEdit) -> Result<String, String> {
    let mut config = Config::new(text)?;
    let element = config.child(SOURCES, name).ok_or_else(|| format!("The file names no feed {name}"))?;
    let was = config.nodes[element].clone();
    config.replace(element, &feed_element(feed, Some(&was)))?;
    if feed.name != name {
        if let Some(disabled) = config.child(DISABLED, name) {
            config.remove(disabled)?;
        }
        while let Some(mapped) =
            config.section("packageSourceMapping").and_then(|m| {
                config.nodes[m].children.iter().copied().find(|c| {
                    config.nodes[*c].name == "packageSource" && config.nodes[*c].attribute("key") == Some(name)
                })
            })
        {
            config.set_key(mapped, &feed.name)?;
        }
    }
    credential(&mut config, feed, Some(name))?;
    enable(&mut config, &feed.name, feed.enabled)?;
    Ok(config.text)
}

/// Takes a feed out of the file, with what the file says of it elsewhere.
pub fn remove(text: &str, name: &str) -> Result<String, String> {
    let mut config = Config::new(text)?;
    while let Some(element) =
        config.child(SOURCES, name).or_else(|| config.child(DISABLED, name)).or_else(|| config.held(name))
    {
        config.remove(element)?;
    }
    Ok(config.text)
}

/// Disables a feed in this file, or takes this file's disabling of it away.
pub fn set_enabled(text: &str, name: &str, enabled: bool) -> Result<String, String> {
    let mut config = Config::new(text)?;
    enable(&mut config, name, enabled)?;
    Ok(config.text)
}

/// Reads a file, changes its text and writes it back as it came: its encoding, byte order mark and line ends,
/// through a temporary file, to a symlink's target, never over a read-only file. A file not there, or empty, starts as
/// NuGet's empty configuration. The bytes before (none for a new file) and after.
pub fn change_file(
    path: &Path,
    change: impl FnOnce(&str) -> Result<String, String>,
) -> Result<(Option<Vec<u8>>, Vec<u8>), String> {
    let path = if path.exists() { crate::paths::canonical(path) } else { path.to_path_buf() };
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let before = path.exists().then(|| std::fs::read(&path)).transpose().map_err(|e| format!("{name}: {e}"))?;
    let (text, charset) = match &before {
        Some(bytes) if !bytes.is_empty() => crate::encodings::decode(bytes, None),
        _ => (EMPTY.to_string(), crate::encodings::Charset::UTF8),
    };
    let after = change(&text)?;
    let bytes = crate::encodings::encode(&after, charset)
        .map_err(|c| format!("'{c}' cannot be written in {}", charset.name()))?;
    if let Some(folder) = path.parent() {
        std::fs::create_dir_all(folder).map_err(|e| format!("{name}: {e}"))?;
    }
    crate::files::write(&path, &bytes).map_err(|e| format!("{name} could not be written: {e}"))?;
    Ok((before, bytes))
}

fn enable(config: &mut Config, name: &str, enabled: bool) -> Result<(), String> {
    match (enabled, config.child(DISABLED, name)) {
        (true, Some(disabled)) => config.remove(disabled),
        (false, None) => config.add_child(DISABLED, &format!("<add key=\"{}\" value=\"true\" />", escape(name))),
        _ => Ok(()),
    }
}

/// Writes a feed's credentials as the form left them, under its name in packageSourceCredentials.
fn credential(config: &mut Config, feed: &FeedEdit, old_name: Option<&str>) -> Result<(), String> {
    let held = config.held(&feed.name).or_else(|| old_name.and_then(|n| config.held(n)));
    let Some(username) = feed.username.as_deref().filter(|u| !u.is_empty()) else {
        if let Some(held) = held {
            config.remove(held)?;
        }
        return Ok(());
    };
    let kept: Vec<(String, String)> = held
        .map(|h| {
            config.nodes[h]
                .children
                .iter()
                .map(|c| &config.nodes[*c])
                .filter(|c| c.name == "add")
                .map(|c| (c.attribute("key").unwrap_or("").to_string(), c.attribute("value").unwrap_or("").to_string()))
                .collect()
        })
        .unwrap_or_default();
    let password = match feed.password.as_deref().filter(|p| !p.is_empty()) {
        Some(given) => {
            let (key, value) = secret::protect(given);
            Some((key.to_string(), value))
        }
        None => kept.iter().find(|(k, _)| k == "Password" || k == "ClearTextPassword").cloned(),
    };
    let mut values = vec![format!("<add key=\"Username\" value=\"{}\" />", escape(username))];
    if let Some((key, value)) = password {
        values.push(format!("<add key=\"{key}\" value=\"{}\" />", escape(&value)));
    }
    for (key, value) in
        kept.iter().filter(|(k, _)| !matches!(k.as_str(), "Username" | "Password" | "ClearTextPassword"))
    {
        values.push(format!("<add key=\"{}\" value=\"{}\" />", escape(key), escape(value)));
    }
    let indent = config.child_indent(CREDENTIALS);
    let name = encode_name(&feed.name);
    let inner: String = values.iter().map(|v| format!("{}{indent}{}{v}", config.newline, config.unit)).collect();
    let element = format!("<{name}>{inner}{}{indent}</{name}>", config.newline);
    match held {
        Some(held) => config.replace(held, &element),
        None => config.add_child(CREDENTIALS, &element),
    }
}

/// A feed's add element: its key and value, what else the element had, and its insecure switch.
fn feed_element(feed: &FeedEdit, was: Option<&Node>) -> String {
    let mut element = format!("<add key=\"{}\" value=\"{}\"", escape(&feed.name), escape(&feed.url));
    for (key, value) in was.iter().flat_map(|w| &w.attributes) {
        if !matches!(key.as_str(), "key" | "value" | "allowInsecureConnections") {
            element.push_str(&format!(" {key}=\"{}\"", escape(value)));
        }
    }
    if feed.allow_insecure {
        element.push_str(" allowInsecureConnections=\"true\"");
    }
    element + " />"
}

fn escape(value: &str) -> String {
    value.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&apos;")
}

#[derive(Clone, Debug)]
struct Node {
    name: String,
    attributes: Vec<(String, String)>,
    /// From its `<` to after its end tag (or the `/>` of an empty element).
    start: usize,
    end: usize,
    /// Where its children go: after its start tag, and before its end tag; equal for an empty element.
    inner_start: usize,
    inner_end: usize,
    empty: bool,
    children: Vec<usize>,
}

impl Node {
    fn attribute(&self, key: &str) -> Option<&str> {
        self.attributes.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
    }
}

/// The file's text and its elements, changed by splices of the text, after each of which the elements are read again.
struct Config {
    text: String,
    newline: String,
    unit: String,
    nodes: Vec<Node>,
    root: usize,
}

impl Config {
    fn new(text: &str) -> Result<Config, String> {
        let text = if text.trim().is_empty() { EMPTY.to_string() } else { text.to_string() };
        let newline = if text.contains("\r\n") { "\r\n" } else { "\n" }.to_string();
        let mut config = Config { text, newline, unit: "  ".into(), nodes: Vec::new(), root: 0 };
        config.parse()?;
        if let Some(first) = config.nodes[config.root].children.first() {
            let indent = config.indent_of(*first);
            if !indent.is_empty() {
                config.unit = indent;
            }
        }
        Ok(config)
    }

    fn parse(&mut self) -> Result<(), String> {
        use quick_xml::events::Event;
        // quick-xml passes over a byte order mark without counting it, so places are read past it and moved back.
        let bom = if self.text.starts_with('\u{feff}') { '\u{feff}'.len_utf8() } else { 0 };
        let mut reader = quick_xml::Reader::from_str(&self.text[bom..]);
        let mut nodes: Vec<Node> = Vec::new();
        let mut open: Vec<usize> = Vec::new();
        let mut root = None;
        loop {
            let at = bom + reader.buffer_position() as usize;
            let event = reader.read_event().map_err(|e| format!("The file is not XML: {e}"))?;
            let after = bom + reader.buffer_position() as usize;
            let (element, empty) = match &event {
                Event::Start(e) => (e, false),
                Event::Empty(e) => (e, true),
                Event::End(_) => {
                    let node = open.pop().ok_or("The file closes an element it never opened")?;
                    nodes[node].inner_end = at;
                    nodes[node].end = after;
                    continue;
                }
                Event::Eof => break,
                _ => continue,
            };
            let attributes = element
                .attributes()
                .flatten()
                .map(|a| {
                    let key = String::from_utf8_lossy(a.key.as_ref()).into_owned();
                    let value = a
                        .unescape_value()
                        .map(|v| v.into_owned())
                        .unwrap_or_else(|_| String::from_utf8_lossy(&a.value).into());
                    (key, value)
                })
                .collect();
            let index = nodes.len();
            nodes.push(Node {
                name: String::from_utf8_lossy(element.local_name().as_ref()).into_owned(),
                attributes,
                start: at,
                end: after,
                inner_start: after,
                inner_end: after,
                empty,
                children: Vec::new(),
            });
            match open.last() {
                Some(parent) => nodes[*parent].children.push(index),
                None => root = Some(index),
            }
            if !empty {
                open.push(index);
            }
        }
        if !open.is_empty() {
            return Err("The file ends inside an element".into());
        }
        let root = root.filter(|r| nodes[*r].name == "configuration").ok_or("The file has no configuration element")?;
        self.nodes = nodes;
        self.root = root;
        Ok(())
    }

    fn splice(&mut self, from: usize, to: usize, with: &str) -> Result<(), String> {
        self.text.replace_range(from..to, with);
        self.parse()
    }

    fn section(&self, name: &str) -> Option<usize> {
        self.nodes[self.root].children.iter().copied().find(|c| self.nodes[*c].name == name)
    }

    /// The add element a section has for a key, matched as NuGet matches keys, ignoring case.
    fn child(&self, section: &str, key: &str) -> Option<usize> {
        let section = self.section(section)?;
        self.nodes[section].children.iter().copied().find(|c| {
            self.nodes[*c].name == "add" && self.nodes[*c].attribute("key").is_some_and(|k| k.eq_ignore_ascii_case(key))
        })
    }

    fn held(&self, feed: &str) -> Option<usize> {
        let section = self.section(CREDENTIALS)?;
        let name = encode_name(feed);
        self.nodes[section].children.iter().copied().find(|c| self.nodes[*c].name.eq_ignore_ascii_case(&name))
    }

    /// The whitespace before an element on its line.
    fn indent_of(&self, node: usize) -> String {
        let before = &self.text[..self.nodes[node].start];
        let line = before.rsplit_once('\n').map_or(before, |(_, l)| l);
        if line.chars().all(|c| c == ' ' || c == '\t') { line.to_string() } else { String::new() }
    }

    /// The indent a section's children have: its last child's, else one more than the section's.
    fn child_indent(&self, section: &str) -> String {
        match self.section(section) {
            Some(s) => match self.nodes[s].children.last() {
                Some(last) => self.indent_of(*last),
                None => self.indent_of(s) + &self.unit,
            },
            None => self.unit.clone() + &self.unit,
        }
    }

    fn add_child(&mut self, section: &str, element: &str) -> Result<(), String> {
        if self.section(section).is_none() {
            let root = self.root;
            let at = self.nodes[root].children.last().map_or(self.nodes[root].inner_start, |c| self.nodes[*c].end);
            let (nl, unit) = (self.newline.clone(), self.unit.clone());
            let empty_root = self.nodes[root].children.is_empty() && self.nodes[root].empty;
            if empty_root {
                let (start, end) = (self.nodes[root].start, self.nodes[root].end);
                self.splice(
                    start,
                    end,
                    &format!("<configuration>{nl}{unit}<{section}>{nl}{unit}</{section}>{nl}</configuration>"),
                )?;
            } else {
                self.splice(at, at, &format!("{nl}{unit}<{section}>{nl}{unit}</{section}>"))?;
            }
        }
        let indent = self.child_indent(section);
        let node = self.section(section).ok_or("The section was not made")?;
        let nl = self.newline.clone();
        let Node { start, end, empty, ref children, inner_start, .. } = self.nodes[node];
        if empty {
            let outer = self.indent_of(node);
            return self.splice(start, end, &format!("<{section}>{nl}{indent}{element}{nl}{outer}</{section}>"));
        }
        let at = children.last().map_or(inner_start, |c| self.nodes[*c].end);
        if children.is_empty() && !self.text[inner_start..self.nodes[node].inner_end].contains('\n') {
            let outer = self.indent_of(node);
            let inner_end = self.nodes[node].inner_end;
            return self.splice(inner_start, inner_end, &format!("{nl}{indent}{element}{nl}{outer}"));
        }
        self.splice(at, at, &format!("{nl}{indent}{element}"))
    }

    fn replace(&mut self, node: usize, with: &str) -> Result<(), String> {
        let (start, end) = (self.nodes[node].start, self.nodes[node].end);
        self.splice(start, end, with)
    }

    /// Takes an element out, with its line when it had one to itself.
    fn remove(&mut self, node: usize) -> Result<(), String> {
        let (mut start, end) = (self.nodes[node].start, self.nodes[node].end);
        let before = &self.text[..start];
        let spaces = before.len() - before.trim_end_matches([' ', '\t']).len();
        let line_starts = before[..before.len() - spaces].ends_with('\n');
        let rest = &self.text[end..];
        let line_ends = rest.trim_start_matches([' ', '\t']).starts_with(['\r', '\n']);
        if line_starts && line_ends {
            start -= spaces;
            start -= if self.text[..start].ends_with("\r\n") { 2 } else { 1 };
        }
        self.splice(start, end, "")
    }

    /// Changes an element's key attribute in place, the rest of its tag as it was.
    fn set_key(&mut self, node: usize, key: &str) -> Result<(), String> {
        let (start, inner) = (self.nodes[node].start, self.nodes[node].inner_start);
        let tag = &self.text[start..inner];
        let found =
            regex::Regex::new(r#"\bkey\s*=\s*("[^"]*"|'[^']*')"#).unwrap().find(tag).ok_or("The element has no key")?;
        let (from, to) = (start + found.start(), start + found.end());
        self.splice(from, to, &format!("key=\"{}\"", escape(key)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\u{feff}<?xml version=\"1.0\" encoding=\"utf-8\"?>\r\n<configuration>\r\n\t<!-- the team's feeds -->\r\n\t<packageSources>\r\n\t\t<add key='nuget.org' value='https://api.nuget.org/v3/index.json' protocolVersion=\"3\" />\r\n\t</packageSources>\r\n\t<packageSourceMapping>\r\n\t\t<packageSource key=\"corp\">\r\n\t\t\t<package pattern=\"Corp.*\" />\r\n\t\t</packageSource>\r\n\t</packageSourceMapping>\r\n</configuration>\r\n";

    fn feed(name: &str) -> FeedEdit {
        FeedEdit {
            name: name.into(),
            url: "https://corp.example/v3/index.json".into(),
            enabled: true,
            allow_insecure: false,
            username: None,
            password: None,
        }
    }

    #[test]
    fn adding_a_feed_writes_only_its_element_indented_as_its_neighbours() {
        let added = add(FILE, &feed("corp")).unwrap();
        let expected = FILE.replace(
            "protocolVersion=\"3\" />\r\n\t</packageSources>",
            "protocolVersion=\"3\" />\r\n\t\t<add key=\"corp\" value=\"https://corp.example/v3/index.json\" />\r\n\t</packageSources>",
        );
        assert_eq!(added, expected, "a byte of the file other than the new feed changed");
        assert!(add(&added, &feed("CORP")).unwrap_err().contains("already"));
    }

    #[test]
    fn disabling_credentials_rename_and_removal_follow_dotnet_nuget() {
        let disabled = add(FILE, &FeedEdit { enabled: false, ..feed("corp") }).unwrap();
        assert!(disabled.contains("\r\n\t<disabledPackageSources>\r\n\t\t<add key=\"corp\" value=\"true\" />\r\n\t</disabledPackageSources>"), "{disabled}");
        let enabled = set_enabled(&disabled, "corp", true).unwrap();
        assert!(!enabled.contains("<add key=\"corp\" value=\"true\" />"), "{enabled}");

        let with_user = update(
            &disabled,
            "corp",
            &FeedEdit { username: Some("ann".into()), password: Some("p&w".into()), ..feed("Corp Feed") },
        )
        .unwrap();
        if !cfg!(windows) {
            assert!(
                with_user.contains("\t<packageSourceCredentials>\r\n\t\t<Corp_x0020_Feed>\r\n\t\t\t<add key=\"Username\" value=\"ann\" />\r\n\t\t\t<add key=\"ClearTextPassword\" value=\"p&amp;w\" />\r\n\t\t</Corp_x0020_Feed>\r\n\t</packageSourceCredentials>"),
                "{with_user}"
            );
        }
        assert!(
            with_user.contains("<packageSource key=\"Corp Feed\">"),
            "the mapping did not follow the rename: {with_user}"
        );
        assert!(!with_user.contains("<add key=\"corp\" value=\"true\" />"), "the old name's disabling stayed");
        let kept =
            update(&with_user, "Corp Feed", &FeedEdit { username: Some("bo".into()), ..feed("Corp Feed") }).unwrap();
        assert!(kept.contains("value=\"bo\""), "{kept}");
        assert_eq!(
            kept.matches("Password").count(),
            1,
            "a password left empty did not keep the one the file had: {kept}"
        );

        let removed = remove(&with_user, "Corp Feed").unwrap();
        // As dotnet nuget remove source, the mapping is left as it was.
        assert!(!removed.contains("<add key=\"Corp Feed\"") && !removed.contains("Corp_x0020_Feed>"), "{removed}");
        assert!(removed.starts_with('\u{feff}') && removed.contains("<!-- the team's feeds -->"));
        assert!(!removed.replace("\r\n", "").contains('\n'), "a bare line end crept into a CRLF file");
    }

    #[test]
    fn a_feeds_other_attributes_stay_and_an_empty_file_starts_as_nugets_own() {
        let edited = update(
            FILE,
            "nuget.org",
            &FeedEdit { url: "https://mirror.example/index.json".into(), ..feed("nuget.org") },
        )
        .unwrap();
        assert!(
            edited.contains(
                "<add key=\"nuget.org\" value=\"https://mirror.example/index.json\" protocolVersion=\"3\" />"
            ),
            "{edited}"
        );
        let made = add("", &FeedEdit { allow_insecure: true, ..feed("local") }).unwrap();
        assert_eq!(
            made,
            "<?xml version=\"1.0\" encoding=\"utf-8\"?>\n<configuration>\n  <packageSources>\n    <add key=\"local\" value=\"https://corp.example/v3/index.json\" allowInsecureConnections=\"true\" />\n  </packageSources>\n</configuration>\n"
        );
        assert!(format!("{:?}", FeedEdit { password: Some("hunter2".into()), ..feed("x") }).find("hunter2").is_none());
    }
}
