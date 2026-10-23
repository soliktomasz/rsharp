//! Rider's Solution view edits to a solution's file, a .sln or a .slnx (Passenger's `SolutionEdits`): solution folders
//! added, renamed and removed with all they hold, projects and files put in a folder, and taken out again. Each takes
//! the file's text and changes only the lines or elements it is about, so the rest stays byte for byte as it was:
//! `dotnet sln` rewrites a whole .sln, its line endings included. A folder is named by its path in the solution
//! ("/src/tools/"), as `solution` reads it.

use crate::paths;
use regex::Regex;
use std::ops::Range;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

pub enum SolutionEdit<'a> {
    AddFolder {
        parent: Option<&'a str>,
        name: &'a str,
    },
    RenameFolder {
        folder: &'a str,
        name: &'a str,
    },
    /// With every folder, project and file under it; their files stay on disk.
    RemoveFolder {
        folder: &'a str,
    },
    AddProject {
        folder: Option<&'a str>,
        project: &'a Path,
    },
    RemoveProject {
        project: &'a Path,
    },
    /// As Visual Studio's Solution Items are.
    AddFile {
        folder: &'a str,
        path: &'a Path,
    },
    RemoveFile {
        folder: &'a str,
        path: &'a Path,
    },
}

/// The solution's text with the change made, or why it could not be.
pub fn edit(solution: &Path, text: &str, change: SolutionEdit) -> Result<String, String> {
    if let SolutionEdit::AddFolder { name, .. } | SolutionEdit::RenameFolder { name, .. } = &change
        && let Some(why) = invalid_folder_name(name)
    {
        return Err(why);
    }
    let root = solution.parent().map(Path::to_path_buf).unwrap_or_default();
    if solution.extension().is_some_and(|e| e.eq_ignore_ascii_case("slnx")) {
        Slnx { text: text.to_string(), root }.apply(change)
    } else {
        Sln::new(text, root).apply(change)
    }
}

/// Why a name cannot be a solution folder's (Visual Studio refuses the same characters), or none.
pub fn invalid_folder_name(name: &str) -> Option<String> {
    let trimmed = name.trim();
    if trimmed.is_empty() {
        Some("A name is needed".into())
    } else if matches!(trimmed, "." | "..") || name.contains(['/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        Some(format!("{name} is not a solution folder name"))
    } else {
        None
    }
}

fn no_folder(folder: &str) -> String {
    format!("There is no solution folder {}", folder.trim_matches('/'))
}

fn name_of(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A path as the solution names it: relative to its folder, `..` where it lies outside, with '/'.
pub(crate) fn relative(root: &Path, path: &Path) -> String {
    let (from, to): (Vec<Component>, Vec<Component>) = (root.components().collect(), path.components().collect());
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut parts: Vec<String> = from[common..].iter().map(|_| "..".to_string()).collect();
    parts.extend(to[common..].iter().map(|c| c.as_os_str().to_string_lossy().into_owned()));
    parts.join("/")
}

fn resolve(root: &Path, written: &str) -> PathBuf {
    paths::canonical(&root.join(written.replace('\\', "/")))
}

/// A new project or folder id, upper case in braces, as Visual Studio writes one.
fn new_id() -> String {
    use std::hash::{BuildHasher, Hasher};
    // Two std hashers seeded at random by the process, mixed with the time: unique enough for an id in one file.
    let mut bits = [0u64; 2];
    for (i, part) in bits.iter_mut().enumerate() {
        let mut hasher = std::collections::hash_map::RandomState::new().build_hasher();
        hasher.write_u128(
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos(),
        );
        hasher.write_usize(i);
        *part = hasher.finish();
    }
    let hex = format!("{:016X}{:016X}", bits[0], bits[1]);
    format!("{{{}-{}-{}-{}-{}}}", &hex[0..8], &hex[8..12], &hex[12..16], &hex[16..20], &hex[20..32])
}

// ---- .sln ----

const FOLDER_TYPE: &str = "{2150E333-8FDC-42A3-9474-1A3956D46DE8}";

static PROJECT_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^Project\("(?<type>\{[^}]+\})"\)\s*=\s*"(?<name>[^"]*)",\s*"(?<path>[^"]*)",\s*"(?<id>\{[^}]+\})""#)
        .unwrap()
});
static PAIR_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*(?<left>\{[^}]+\})\s*=\s*(?<right>\{[^}]+\})").unwrap());
static ITEM_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[^=]*?\s*=\s*(?<path>.+?)\s*$").unwrap());

/// A .sln as lines split at '\n', each keeping a '\r' when the file has them, which a line added carries too.
struct Sln {
    lines: Vec<String>,
    cr: &'static str,
    root: PathBuf,
}

/// One Project ... EndProject block: the lines it starts and ends on.
#[derive(Clone)]
struct Entry {
    kind: String,
    name: String,
    path: String,
    id: String,
    start: usize,
    end: usize,
}

impl Entry {
    fn is_folder(&self) -> bool {
        self.kind.eq_ignore_ascii_case(FOLDER_TYPE)
    }
}

fn same_id(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b)
}

impl Sln {
    fn new(text: &str, root: PathBuf) -> Self {
        let cr = if text.contains("\r\n") { "\r" } else { "" };
        Self { lines: text.split('\n').map(str::to_string).collect(), cr, root }
    }

    fn apply(mut self, change: SolutionEdit) -> Result<String, String> {
        match change {
            SolutionEdit::AddFolder { parent, name } => self.add_folder(parent, name),
            SolutionEdit::RenameFolder { folder, name } => self.rename_folder(folder, name),
            SolutionEdit::RemoveFolder { folder } => self.remove_folder(folder),
            SolutionEdit::AddProject { folder, project } => self.add_project(folder, project),
            SolutionEdit::RemoveProject { project } => self.remove_project(project),
            SolutionEdit::AddFile { folder, path } => self.add_file(folder, path),
            SolutionEdit::RemoveFile { folder, path } => self.remove_file(folder, path),
        }?;
        Ok(self.lines.join("\n"))
    }

    fn entries(&self) -> Vec<Entry> {
        let mut found = Vec::new();
        let mut i = 0;
        while i < self.lines.len() {
            if let Some(m) = PROJECT_LINE.captures(&self.lines[i]) {
                let end = (i + 1..self.lines.len()).find(|&j| self.lines[j].trim() == "EndProject").unwrap_or(i);
                found.push(Entry {
                    kind: m["type"].to_string(),
                    name: m["name"].to_string(),
                    path: m["path"].to_string(),
                    id: m["id"].to_string(),
                    start: i,
                    end,
                });
                i = end;
            }
            i += 1;
        }
        found
    }

    /// A global section's first and last lines.
    fn section(&self, name: &str) -> Option<(usize, usize)> {
        let start = self.lines.iter().position(|l| l.trim_start().starts_with(&format!("GlobalSection({name})")))?;
        let end = (start + 1..self.lines.len()).find(|&i| self.lines[i].trim() == "EndGlobalSection")?;
        Some((start, end))
    }

    /// Each entry's parent folder's id, from NestedProjects.
    fn parents(&self) -> Vec<(String, String)> {
        let Some((start, end)) = self.section("NestedProjects") else { return Vec::new() };
        (start + 1..end)
            .filter_map(|i| PAIR_LINE.captures(&self.lines[i]).map(|m| (m["left"].to_string(), m["right"].to_string())))
            .collect()
    }

    fn parent_of(parents: &[(String, String)], id: &str) -> Option<String> {
        parents.iter().find(|(child, _)| same_id(child, id)).map(|(_, parent)| parent.clone())
    }

    /// The folder a path in the solution names: each name inside the folder NestedProjects puts it in.
    fn folder(&self, path: &str) -> Option<Entry> {
        let (entries, parents) = (self.entries(), self.parents());
        let mut at: Option<Entry> = None;
        for part in path.split('/').filter(|p| !p.is_empty()) {
            let found = entries.iter().find(|e| {
                e.is_folder()
                    && e.name == part
                    && match (Self::parent_of(&parents, &e.id), &at) {
                        (None, None) => true,
                        (Some(parent), Some(at)) => same_id(&parent, &at.id),
                        _ => false,
                    }
            })?;
            at = Some(found.clone());
        }
        at
    }

    fn children(&self, folder: Option<&str>) -> Vec<Entry> {
        let parents = self.parents();
        self.entries()
            .into_iter()
            .filter(|e| match (Self::parent_of(&parents, &e.id), folder) {
                (None, None) => true,
                (Some(parent), Some(folder)) => same_id(&parent, folder),
                _ => false,
            })
            .collect()
    }

    fn add_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), String> {
        let into = match parent {
            Some(parent) => Some(self.folder(parent).ok_or_else(|| no_folder(parent))?),
            None => None,
        };
        if self
            .children(into.as_ref().map(|e| e.id.as_str()))
            .iter()
            .any(|e| e.is_folder() && e.name.eq_ignore_ascii_case(name))
        {
            return Err(format!("{name} is already there"));
        }
        let id = new_id();
        self.insert_entry(FOLDER_TYPE, name, name, &id);
        if let Some(into) = into {
            self.nest(&id, &into.id);
        }
        Ok(())
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> Result<(), String> {
        let renamed = self.folder(folder).ok_or_else(|| no_folder(folder))?;
        let parent = Self::parent_of(&self.parents(), &renamed.id);
        let taken = self
            .children(parent.as_deref())
            .into_iter()
            .any(|e| e.is_folder() && e.id != renamed.id && e.name.eq_ignore_ascii_case(name));
        if taken {
            return Err(format!("{name} is already there"));
        }
        let line = self.lines[renamed.start].clone();
        let m = PROJECT_LINE.captures(&line).expect("an entry's line is a Project line");
        let (old_name, old_path) = (m.name("name").unwrap(), m.name("path").unwrap());
        self.lines[renamed.start] = format!(
            "{}{name}{}{name}{}",
            &line[..old_name.start()],
            &line[old_name.end()..old_path.start()],
            &line[old_path.end()..]
        );
        Ok(())
    }

    fn remove_folder(&mut self, folder: &str) -> Result<(), String> {
        let removed = self.folder(folder).ok_or_else(|| no_folder(folder))?;
        let parents = self.parents();
        let mut gone = vec![removed.id.to_uppercase()];
        // Everything nested under it, however deep, goes with it.
        let mut more = true;
        while more {
            more = false;
            for (child, parent) in &parents {
                if gone.contains(&parent.to_uppercase()) && !gone.contains(&child.to_uppercase()) {
                    gone.push(child.to_uppercase());
                    more = true;
                }
            }
        }
        self.remove_entries(&gone);
        Ok(())
    }

    fn add_project(&mut self, folder: Option<&str>, project: &Path) -> Result<(), String> {
        let into = match folder {
            Some(folder) => Some(self.folder(folder).ok_or_else(|| no_folder(folder))?),
            None => None,
        };
        if self.entries().iter().any(|e| !e.is_folder() && resolve(&self.root, &e.path) == project) {
            return Err(format!("{} is already in the solution", name_of(project)));
        }
        let id = new_id();
        let kind = match project.extension().and_then(|e| e.to_str()).map(str::to_lowercase).as_deref() {
            Some("fsproj") => "{F2A71F9B-5D33-465A-A702-920D77279786}",
            Some("vbproj") => "{F184B08F-C81C-45F6-A57F-5ABD9991F28F}",
            _ => "{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}",
        };
        self.insert_entry(kind, &name_of(project), &relative(&self.root, project).replace('/', "\\"), &id);
        // A project the solution's configurations do not name is never built with it: each builds it as Any CPU, the
        // platform an SDK project builds for, as dotnet sln add maps it.
        let configurations = match self.section("SolutionConfigurationPlatforms") {
            Some(found) => found,
            None => self.add_section(
                "SolutionConfigurationPlatforms",
                "preSolution",
                None,
                &["Debug|Any CPU = Debug|Any CPU", "Release|Any CPU = Release|Any CPU"],
            ),
        };
        let names: Vec<String> = self.lines[configurations.0 + 1..configurations.1]
            .iter()
            .map(|l| l.split('=').next().unwrap_or_default().trim().to_string())
            .filter(|l| !l.is_empty())
            .collect();
        let platforms = match self.section("ProjectConfigurationPlatforms") {
            Some(found) => found,
            None => self.add_section("ProjectConfigurationPlatforms", "postSolution", Some(configurations.1 + 1), &[]),
        };
        let cr = self.cr;
        let added: Vec<String> = names
            .iter()
            .flat_map(|n| {
                let configuration = n.split('|').next().unwrap_or_default();
                [
                    format!("\t\t{id}.{n}.ActiveCfg = {configuration}|Any CPU{cr}"),
                    format!("\t\t{id}.{n}.Build.0 = {configuration}|Any CPU{cr}"),
                ]
            })
            .collect();
        self.lines.splice(platforms.1..platforms.1, added);
        if let Some(into) = into {
            self.nest(&id, &into.id);
        }
        Ok(())
    }

    fn remove_project(&mut self, project: &Path) -> Result<(), String> {
        let removed = self
            .entries()
            .into_iter()
            .find(|e| !e.is_folder() && resolve(&self.root, &e.path) == project)
            .ok_or_else(|| format!("{} is not in the solution", name_of(project)))?;
        self.remove_entries(&[removed.id.to_uppercase()]);
        Ok(())
    }

    /// A folder's ProjectSection(SolutionItems): its first and last lines.
    fn items(&self, folder: &Entry) -> Option<(usize, usize)> {
        let start = (folder.start..folder.end).find(|&i| self.lines[i].contains("ProjectSection(SolutionItems)"))?;
        let end = (start..folder.end).find(|&i| self.lines[i].trim() == "EndProjectSection")?;
        Some((start, end))
    }

    fn item_path(&self, line: usize) -> Option<PathBuf> {
        ITEM_LINE.captures(&self.lines[line]).map(|m| resolve(&self.root, &m["path"]))
    }

    fn add_file(&mut self, folder: &str, path: &Path) -> Result<(), String> {
        let into = self.folder(folder).ok_or_else(|| no_folder(folder))?;
        let item = relative(&self.root, path).replace('/', "\\");
        let cr = self.cr;
        match self.items(&into) {
            Some((start, end)) => {
                if (start + 1..end).any(|i| self.item_path(i).as_deref() == Some(path)) {
                    return Err(format!("{} is already in {}", file_name(path), into.name));
                }
                self.lines.insert(end, format!("\t\t{item} = {item}{cr}"));
            }
            None => {
                let lines = [
                    format!("\tProjectSection(SolutionItems) = preProject{cr}"),
                    format!("\t\t{item} = {item}{cr}"),
                    format!("\tEndProjectSection{cr}"),
                ];
                self.lines.splice(into.end..into.end, lines);
            }
        }
        Ok(())
    }

    fn remove_file(&mut self, folder: &str, path: &Path) -> Result<(), String> {
        let from = self.folder(folder).ok_or_else(|| no_folder(folder))?;
        let not_there = || format!("{} is not in {}", file_name(path), from.name);
        let (start, end) = self.items(&from).ok_or_else(not_there)?;
        let line = (start + 1..end).find(|&i| self.item_path(i).as_deref() == Some(path)).ok_or_else(not_there)?;
        self.lines.remove(line);
        // A folder with no files left has no section for them, as Visual Studio writes one.
        if end - 1 == start + 1 {
            self.lines.drain(start..start + 2);
        }
        Ok(())
    }

    /// A Project block after the last one, before Global.
    fn insert_entry(&mut self, kind: &str, name: &str, path: &str, id: &str) {
        let at = self.entries().last().map(|e| e.end + 1).unwrap_or_else(|| self.global_start());
        let cr = self.cr;
        self.lines.splice(
            at..at,
            [format!("Project(\"{kind}\") = \"{name}\", \"{path}\", \"{id}\"{cr}"), format!("EndProject{cr}")],
        );
    }

    fn nest(&mut self, child: &str, parent: &str) {
        let nested = match self.section("NestedProjects") {
            Some(found) => found,
            None => {
                let at = self.section("ExtensibilityGlobals").map(|s| s.0);
                self.add_section("NestedProjects", "preSolution", at, &[])
            }
        };
        let cr = self.cr;
        self.lines.insert(nested.1, format!("\t\t{child} = {parent}{cr}"));
    }

    /// A new global section, at a line or before EndGlobal, the Global block itself made when there is none.
    fn add_section(&mut self, name: &str, when: &str, at: Option<usize>, content: &[&str]) -> (usize, usize) {
        let cr = self.cr;
        let end = match self.lines.iter().position(|l| l.trim() == "EndGlobal") {
            Some(end) => end,
            None => {
                let start = self.global_start();
                self.lines.splice(start..start, [format!("Global{cr}"), format!("EndGlobal{cr}")]);
                start + 1
            }
        };
        let start = at.unwrap_or(end);
        let mut added = vec![format!("\tGlobalSection({name}) = {when}{cr}")];
        added.extend(content.iter().map(|c| format!("\t\t{c}{cr}")));
        added.push(format!("\tEndGlobalSection{cr}"));
        self.lines.splice(start..start, added);
        (start, start + content.len() + 1)
    }

    /// Where Global begins, or would: before the empty line after the file's last line break.
    fn global_start(&self) -> usize {
        match self.lines.iter().position(|l| l.trim() == "Global") {
            Some(global) => global,
            None if self.lines.last().is_some_and(String::is_empty) => self.lines.len() - 1,
            None => self.lines.len(),
        }
    }

    /// Takes out entries' blocks and every line that names them: their nesting, their configurations and the
    /// dependencies other projects have on them. A section they leave empty goes too.
    fn remove_entries(&mut self, ids: &[String]) {
        let named = |id: &str| ids.contains(&id.to_uppercase());
        let mut gone: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for entry in self.entries().iter().filter(|e| named(&e.id)) {
            gone.extend(entry.start..=entry.end);
        }
        let names = |line: &str| match PAIR_LINE.captures(line) {
            Some(pair) => named(&pair["left"]) || named(&pair["right"]),
            None => {
                let entry = line.trim_start();
                entry.len() > 38 && entry.starts_with('{') && entry.as_bytes()[38] == b'.' && named(&entry[..38])
            }
        };
        for name in ["NestedProjects", "ProjectConfigurationPlatforms"] {
            let Some((start, end)) = self.section(name) else { continue };
            let naming: Vec<usize> = (start + 1..end).filter(|&i| names(&self.lines[i])).collect();
            if !naming.is_empty() && naming.len() == end - start - 1 {
                gone.insert(start);
                gone.insert(end);
            }
            gone.extend(naming);
        }
        let mut i = 0;
        while i < self.lines.len() {
            if self.lines[i].contains("ProjectSection(ProjectDependencies)") {
                i += 1;
                while i < self.lines.len() && self.lines[i].trim() != "EndProjectSection" {
                    if PAIR_LINE.captures(&self.lines[i]).is_some_and(|pair| named(&pair["left"])) {
                        gone.insert(i);
                    }
                    i += 1;
                }
            }
            i += 1;
        }
        for line in gone.into_iter().rev() {
            self.lines.remove(line);
        }
    }
}

// ---- .slnx ----

/// An element as it stands in the text: where it starts, where its start tag ends and where it ends, so an edit
/// splices the text and nothing outside the element moves.
struct Element {
    name: String,
    attributes: Vec<(String, String, Range<usize>)>,
    start: usize,
    open_end: usize,
    end: usize,
    empty: bool,
    children: Vec<Element>,
}

impl Element {
    fn attribute(&self, name: &str) -> Option<&str> {
        self.attributes.iter().find(|(n, ..)| n == name).map(|(_, v, _)| v.as_str())
    }
}

static ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"([A-Za-z_][\w.:-]*)\s*=\s*(?:"([^"]*)"|'([^']*)')"#).unwrap());

/// The document's root element, its children within it, from the text as written.
fn parse(text: &str) -> Result<Element, String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(text);
    let mut stack: Vec<Element> = Vec::new();
    let made = |start: usize, end: usize, empty: bool| {
        let tag = &text[start..end];
        let name =
            tag[1..].split(|c: char| c.is_whitespace() || c == '/' || c == '>').next().unwrap_or_default().to_string();
        let attributes = ATTRIBUTE
            .captures_iter(&tag[1 + name.len()..])
            .map(|m| {
                let value = m.get(2).or(m.get(3)).expect("a quoted value");
                let offset = start + 1 + name.len();
                let raw = value.as_str();
                let unescaped =
                    quick_xml::escape::unescape(raw).map(|v| v.into_owned()).unwrap_or_else(|_| raw.to_string());
                (m[1].to_string(), unescaped, offset + value.start()..offset + value.end())
            })
            .collect();
        Element { name, attributes, start, open_end: end, end, empty, children: Vec::new() }
    };
    loop {
        let before = reader.buffer_position() as usize;
        let event = reader.read_event().map_err(|e| format!("the solution is not well-formed XML: {e}"))?;
        let after = reader.buffer_position() as usize;
        match event {
            Event::Start(_) => stack.push(made(before, after, false)),
            Event::Empty(_) => {
                let element = made(before, after, true);
                match stack.last_mut() {
                    Some(parent) => parent.children.push(element),
                    None => return Ok(element),
                }
            }
            Event::End(_) => {
                let mut element = stack.pop().ok_or("the solution's tags do not match")?;
                element.end = after;
                match stack.last_mut() {
                    Some(parent) => parent.children.push(element),
                    None => return Ok(element),
                }
            }
            Event::Eof => return Err("the solution has no root element".into()),
            _ => {}
        }
    }
}

/// A .slnx, its whitespace kept as read: an element added is indented as its neighbours are, and one taken out goes
/// with the line break and indentation before it.
struct Slnx {
    text: String,
    root: PathBuf,
}

/// Where to put an element in its parent: before a child of it, or at its end.
enum Place {
    Before(usize),
    End,
}

fn folder_path(name: &str) -> String {
    let parts: Vec<&str> = name.split('/').filter(|p| !p.is_empty()).collect();
    format!("/{}/", parts.join("/"))
}

fn parent_of(folder: &str) -> &str {
    &folder[..folder.trim_end_matches('/').rfind('/').map_or(0, |i| i + 1)]
}

fn escape(value: &str) -> String {
    quick_xml::escape::escape(value).into_owned()
}

impl Slnx {
    fn apply(mut self, change: SolutionEdit) -> Result<String, String> {
        match change {
            SolutionEdit::AddFolder { parent, name } => self.add_folder(parent, name),
            SolutionEdit::RenameFolder { folder, name } => self.rename_folder(folder, name),
            SolutionEdit::RemoveFolder { folder } => self.remove_folder(folder),
            SolutionEdit::AddProject { folder, project } => self.add_project(folder, project),
            SolutionEdit::RemoveProject { project } => self.remove_project(project),
            SolutionEdit::AddFile { folder, path } => self.add_file(folder, path),
            SolutionEdit::RemoveFile { folder, path } => self.remove_file(folder, path),
        }?;
        Ok(self.text)
    }

    fn solution(&self) -> Result<Element, String> {
        parse(&self.text)
    }

    /// The Folder elements, by index among the solution's children, each with its path in the solution.
    fn folders(solution: &Element) -> Vec<(usize, String)> {
        solution
            .children
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name == "Folder")
            .filter_map(|(i, e)| e.attribute("Name").map(|n| (i, folder_path(n))))
            .collect()
    }

    /// Whether a folder is in the solution: written, or implied by a folder written inside it.
    fn has(&self, folder: &str) -> Result<bool, String> {
        Ok(Self::folders(&self.solution()?).iter().any(|(_, p)| p.starts_with(folder)))
    }

    fn taken(&self, path: &str, except: Option<&str>) -> Result<bool, String> {
        let lower = path.to_lowercase();
        Ok(Self::folders(&self.solution()?)
            .iter()
            .any(|(_, p)| p.to_lowercase().starts_with(&lower) && !except.is_some_and(|e| p.starts_with(e))))
    }

    fn add_folder(&mut self, parent: Option<&str>, name: &str) -> Result<(), String> {
        if let Some(parent) = parent
            && !self.has(parent)?
        {
            return Err(no_folder(parent));
        }
        let path = format!("{}{name}/", parent.unwrap_or("/"));
        if self.taken(&path, None)? {
            return Err(format!("{name} is already there"));
        }
        self.element(&path)?;
        Ok(())
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> Result<(), String> {
        if !self.has(folder)? {
            return Err(no_folder(folder));
        }
        let renamed = format!("{}{name}/", parent_of(folder));
        if self.taken(&renamed, Some(folder))? {
            return Err(format!("{name} is already there"));
        }
        // A .slnx names each folder by its whole path, so the folders inside it are renamed too; from the end, so the
        // spans before stay where they are.
        let solution = self.solution()?;
        let mut spans: Vec<(Range<usize>, String)> = Self::folders(&solution)
            .into_iter()
            .filter(|(_, p)| p.starts_with(folder))
            .filter_map(|(i, p)| {
                let span = solution.children[i].attributes.iter().find(|(n, ..)| n == "Name")?.2.clone();
                Some((span, format!("{renamed}{}", &p[folder.len()..])))
            })
            .collect();
        spans.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
        for (span, value) in spans {
            self.text.replace_range(span, &escape(&value));
        }
        Ok(())
    }

    fn remove_folder(&mut self, folder: &str) -> Result<(), String> {
        let solution = self.solution()?;
        let mut gone: Vec<usize> =
            Self::folders(&solution).into_iter().filter(|(_, p)| p.starts_with(folder)).map(|(i, _)| i).collect();
        if gone.is_empty() {
            return Err(no_folder(folder));
        }
        gone.reverse();
        for i in gone {
            let solution = self.solution()?;
            self.remove(&solution, &[i]);
        }
        Ok(())
    }

    /// The projects anywhere in the solution, each by the child indices that lead to it.
    fn projects(&self, solution: &Element) -> Vec<(Vec<usize>, PathBuf)> {
        let mut found = Vec::new();
        for (i, child) in solution.children.iter().enumerate() {
            if child.name == "Project" {
                if let Some(path) = child.attribute("Path") {
                    found.push((vec![i], resolve(&self.root, path)));
                }
            }
            for (j, inner) in child.children.iter().enumerate() {
                if inner.name == "Project"
                    && let Some(path) = inner.attribute("Path")
                {
                    found.push((vec![i, j], resolve(&self.root, path)));
                }
            }
        }
        found
    }

    fn add_project(&mut self, folder: Option<&str>, project: &Path) -> Result<(), String> {
        if let Some(folder) = folder
            && !self.has(folder)?
        {
            return Err(no_folder(folder));
        }
        if self.projects(&self.solution()?).iter().any(|(_, p)| p == project) {
            return Err(format!("{} is already in the solution", name_of(project)));
        }
        let into = match folder {
            Some(folder) => vec![self.element(folder)?],
            None => vec![],
        };
        let solution = self.solution()?;
        let parent = Self::at(&solution, &into);
        let place = match parent.children.iter().rposition(|e| e.name == "Project") {
            Some(last) if last + 1 < parent.children.len() => Place::Before(last + 1),
            _ => Place::End,
        };
        let added = format!("<Project Path=\"{}\" />", escape(&relative(&self.root, project)));
        self.insert(&solution, &into, &added, place);
        Ok(())
    }

    fn remove_project(&mut self, project: &Path) -> Result<(), String> {
        let solution = self.solution()?;
        let (at, _) = self
            .projects(&solution)
            .into_iter()
            .find(|(_, p)| p == project)
            .ok_or_else(|| format!("{} is not in the solution", name_of(project)))?;
        self.remove(&solution, &at);
        Ok(())
    }

    fn files(&self, folder: &Element) -> Vec<(usize, PathBuf)> {
        folder
            .children
            .iter()
            .enumerate()
            .filter(|(_, e)| e.name == "File")
            .filter_map(|(i, e)| e.attribute("Path").map(|p| (i, resolve(&self.root, p))))
            .collect()
    }

    fn add_file(&mut self, folder: &str, path: &Path) -> Result<(), String> {
        if !self.has(folder)? {
            return Err(no_folder(folder));
        }
        let into = self.element(folder)?;
        let solution = self.solution()?;
        let parent = &solution.children[into];
        if self.files(parent).iter().any(|(_, p)| p == path) {
            let name = folder.trim_end_matches('/').rsplit('/').next().unwrap_or_default();
            return Err(format!("{} is already in {name}", file_name(path)));
        }
        // Files come before projects in a folder, as Visual Studio writes them.
        let place = match parent.children.iter().rposition(|e| e.name == "File") {
            Some(last) if last + 1 < parent.children.len() => Place::Before(last + 1),
            Some(_) => Place::End,
            None => match parent.children.iter().position(|e| e.name == "Project") {
                Some(first) => Place::Before(first),
                None => Place::End,
            },
        };
        let added = format!("<File Path=\"{}\" />", escape(&relative(&self.root, path)));
        self.insert(&solution, &[into], &added, place);
        Ok(())
    }

    fn remove_file(&mut self, folder: &str, path: &Path) -> Result<(), String> {
        let solution = self.solution()?;
        let found = Self::folders(&solution).into_iter().filter(|(_, p)| p == folder).find_map(|(i, _)| {
            self.files(&solution.children[i]).into_iter().find(|(_, p)| p == path).map(|(j, _)| vec![i, j])
        });
        let at = found.ok_or_else(|| format!("{} is not in {}", file_name(path), folder.trim_matches('/')))?;
        self.remove(&solution, &at);
        Ok(())
    }

    fn at<'e>(solution: &'e Element, path: &[usize]) -> &'e Element {
        path.iter().fold(solution, |element, &i| &element.children[i])
    }

    /// A folder's own element, by its index among the solution's children, written now if the folder was only implied:
    /// among the folders in order of their paths, before the solution's own projects, as Visual Studio writes them.
    fn element(&mut self, folder: &str) -> Result<usize, String> {
        let solution = self.solution()?;
        let folders = Self::folders(&solution);
        if let Some((i, _)) = folders.iter().find(|(_, p)| p == folder) {
            return Ok(*i);
        }
        let lower = folder.to_lowercase();
        let place = match folders.iter().find(|(_, p)| p.to_lowercase() > lower) {
            Some((i, _)) => Place::Before(*i),
            None => match folders.last() {
                Some((i, _)) if i + 1 < solution.children.len() => Place::Before(i + 1),
                Some(_) => Place::End,
                None => match solution.children.iter().position(|e| e.name == "Project") {
                    Some(first) => Place::Before(first),
                    None => Place::End,
                },
            },
        };
        self.insert(&solution, &[], &format!("<Folder Name=\"{}\" />", escape(folder)), place);
        let made = Self::folders(&self.solution()?).into_iter().find(|(_, p)| p == folder).map(|(i, _)| i);
        made.ok_or_else(|| "the folder was not written".into())
    }

    /// The whitespace an element is indented by: what follows the last line break before it, when only whitespace
    /// stands between it and what comes before.
    fn indent_of(&self, element: &Element, after: usize) -> String {
        let before = &self.text[after..element.start];
        match before.rfind('\n') {
            Some(at) if before.trim().is_empty() => before[at + 1..].to_string(),
            _ => String::new(),
        }
    }

    /// Where the text before a child begins: its previous sibling's end, else the parent's start tag's end.
    fn before_child(parent: &Element, index: usize) -> usize {
        if index == 0 { parent.open_end } else { parent.children[index - 1].end }
    }

    /// One level of the file's indentation: what its first element is indented by.
    fn unit(&self, solution: &Element) -> String {
        match solution.children.first() {
            Some(first) => {
                let indent = self.indent_of(first, solution.open_end);
                if indent.is_empty() { "  ".into() } else { indent }
            }
            None => "  ".into(),
        }
    }

    /// Puts an element into another, on a line of its own indented as its siblings are.
    fn insert(&mut self, solution: &Element, path: &[usize], element: &str, place: Place) {
        let into = Self::at(solution, path);
        let outer = match path.split_last() {
            Some((&last, above)) => {
                let parent = Self::at(solution, above);
                self.indent_of(into, Self::before_child(parent, last))
            }
            None => String::new(),
        };
        let inner = match into.children.first() {
            Some(sibling) => self.indent_of(sibling, into.open_end),
            None => format!("{outer}{}", self.unit(solution)),
        };
        let newline = if self.text.contains("\r\n") { "\r\n" } else { "\n" };
        if into.empty {
            // `<Folder Name="/x/" />` opens to hold it.
            let tag = &self.text[into.start..into.end];
            let open = tag.trim_end_matches('>').trim_end_matches('/').trim_end();
            let replaced = format!("{open}>{newline}{inner}{element}{newline}{outer}</{}>", into.name);
            self.text.replace_range(into.start..into.end, &replaced);
            return;
        }
        match place {
            Place::Before(index) => {
                let at = into.children[index].start;
                self.text.insert_str(at, &format!("{element}{newline}{inner}"));
            }
            Place::End => {
                let close = into.children.last().map_or(into.open_end, |c| c.end);
                self.text.insert_str(close, &format!("{newline}{inner}{element}"));
            }
        }
    }

    /// Takes an element out with the whitespace before it; a folder left with nothing in it closes itself.
    fn remove(&mut self, solution: &Element, path: &[usize]) {
        let (&last, above) = path.split_last().expect("never the solution itself");
        let parent = Self::at(solution, above);
        let element = &parent.children[last];
        let before = Self::before_child(parent, last);
        let from = if self.text[before..element.start].trim().is_empty() { before } else { element.start };
        let close = parent.end - (parent.name.len() + 3);
        let alone = parent.children.len() == 1
            && self.text[parent.open_end..element.start].trim().is_empty()
            && self.text[element.end..close].trim().is_empty();
        if !above.is_empty() && alone {
            let tag = &self.text[parent.start..parent.open_end];
            let closed = format!("{} />", tag.trim_end_matches('>').trim_end());
            self.text.replace_range(parent.start..parent.end, &closed);
            return;
        }
        self.text.replace_range(from..element.end, "");
    }
}
