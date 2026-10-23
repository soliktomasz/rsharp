//! A solution as its file says it is: its folders, the files they list and its projects, from a .slnx or a .sln,
//! read directly. What each project holds is MSBuild's to say (`project`), never this file's.

use crate::paths;
use quick_xml::events::{BytesStart, Event};
use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

/// The solution folder's type in a classic .sln, which marks an entry as a folder.
const SOLUTION_FOLDER_TYPE: &str = "2150E333-8FDC-42A3-9474-1A3956D46DE8";

pub struct Solution {
    pub path: PathBuf,
    pub name: String,
    pub entries: Vec<SolutionEntry>,
}

pub enum SolutionEntry {
    /// `path` is the folder's place in the solution, as a .slnx names it: "/src/tools/".
    Folder {
        name: String,
        path: String,
        children: Vec<SolutionEntry>,
    },
    Project(Project),
    Item {
        name: String,
        path: PathBuf,
        exists: bool,
    },
}

pub struct Project {
    pub path: PathBuf,
    pub name: String,
    pub exists: bool,
}

impl Solution {
    /// Every project, depth first in the file's order.
    pub fn projects(&self) -> Vec<&Project> {
        fn walk<'a>(entries: &'a [SolutionEntry], out: &mut Vec<&'a Project>) {
            for entry in entries {
                match entry {
                    SolutionEntry::Project(project) => out.push(project),
                    SolutionEntry::Folder { children, .. } => walk(children, out),
                    SolutionEntry::Item { .. } => {}
                }
            }
        }
        let mut out = Vec::new();
        walk(&self.entries, &mut out);
        out
    }
}

/// Reads a solution; none when it cannot be read or a .slnx is not well-formed XML.
pub fn read(path: &Path) -> Option<Solution> {
    let path = paths::canonical(path);
    let text = std::fs::read_to_string(&path).ok()?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let root = path.parent()?.to_path_buf();
    let entries = if path.extension().is_some_and(|e| e.eq_ignore_ascii_case("slnx")) {
        read_slnx(text, &root)?
    } else {
        read_sln(text, &root)
    };
    let name = path.file_stem()?.to_string_lossy().into_owned();
    Some(Solution { path, name, entries })
}

/// The solution at a folder's top: the one named for the folder when there are several, and of a .sln and a .slnx
/// the .slnx, which `dotnet sln migrate` writes beside the .sln it leaves.
pub fn find(folder: &Path) -> Option<PathBuf> {
    let named = folder.file_name().map(|n| n.to_string_lossy().to_lowercase()).unwrap_or_default();
    let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("sln") || e.eq_ignore_ascii_case("slnx")))
        .collect();
    found.sort_by_key(|p| {
        let stem = p.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default();
        let slnx = p.extension().is_some_and(|e| e.eq_ignore_ascii_case("slnx"));
        (stem != named, !slnx, p.clone())
    });
    found.into_iter().next()
}

fn at(root: &Path, relative: &str) -> PathBuf {
    paths::canonical(&root.join(relative.replace('\\', "/")))
}

fn project(path: PathBuf) -> SolutionEntry {
    let name = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    SolutionEntry::Project(Project { exists: path.is_file(), path, name })
}

fn item(path: PathBuf) -> SolutionEntry {
    let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    SolutionEntry::Item { exists: path.is_file(), path, name }
}

/// The level of the tree a folder's parts lead to, its folders made on the way.
fn level<'a>(entries: &'a mut Vec<SolutionEntry>, parts: &[String], above: &str) -> &'a mut Vec<SolutionEntry> {
    let Some((first, rest)) = parts.split_first() else { return entries };
    let path = format!("{above}{first}/");
    let index = match entries.iter().position(|e| matches!(e, SolutionEntry::Folder { name, .. } if name == first)) {
        Some(index) => index,
        None => {
            entries.push(SolutionEntry::Folder { name: first.clone(), path: path.clone(), children: Vec::new() });
            entries.len() - 1
        }
    };
    match &mut entries[index] {
        SolutionEntry::Folder { children, .. } => level(children, rest, &path),
        _ => unreachable!("the index is a folder's"),
    }
}

fn attribute(element: &BytesStart, name: &[u8]) -> Option<String> {
    element
        .attributes()
        .flatten()
        .find(|a| a.key.as_ref() == name)
        .and_then(|a| a.unescape_value().ok())
        .map(Into::into)
}

fn read_slnx(text: &str, root: &Path) -> Option<Vec<SolutionEntry>> {
    let mut reader = quick_xml::Reader::from_str(text);
    let mut entries = Vec::new();
    // The folder being read: a .slnx writes each folder's whole path ("/src/tools/") on it, never nesting them.
    let mut folder: Vec<String> = Vec::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(element) | Event::Empty(element) if element.name().as_ref() == b"Folder" => {
                folder = attribute(&element, b"Name")
                    .unwrap_or_default()
                    .split('/')
                    .filter(|p| !p.is_empty())
                    .map(str::to_string)
                    .collect();
                level(&mut entries, &folder, "/");
            }
            Event::End(element) if element.name().as_ref() == b"Folder" => folder.clear(),
            Event::Start(element) | Event::Empty(element) => {
                let path = attribute(&element, b"Path");
                let entry = match (element.name().as_ref(), path) {
                    (b"Project", Some(path)) => project(at(root, &path)),
                    (b"File", Some(path)) => item(at(root, &path)),
                    _ => continue,
                };
                level(&mut entries, &folder, "/").push(entry);
            }
            Event::Eof => return Some(entries),
            _ => {}
        }
    }
}

static PROJECT_LINE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r#"^Project\("\{(?<type>[^}]+)\}"\)\s*=\s*"(?<name>[^"]*)",\s*"(?<path>[^"]*)",\s*"\{(?<id>[^}]+)\}""#)
        .unwrap()
});
static NESTING_LINE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^\s*\{(?<child>[^}]+)\}\s*=\s*\{(?<parent>[^}]+)\}").unwrap());
static ITEM_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"^\s*[^=]*?\s*=\s*(?<path>.+?)\s*$").unwrap());

fn read_sln(text: &str, root: &Path) -> Vec<SolutionEntry> {
    // Each entry by its id, upper case, as NestedProjects may write ids in either case.
    let mut by_id: Vec<(String, SolutionEntry)> = Vec::new();
    let mut parents: Vec<(String, String)> = Vec::new();
    let (mut declaring_folder, mut in_items, mut in_nesting) = (false, false, false);
    for line in text.lines() {
        if let Some(m) = PROJECT_LINE.captures(line) {
            let folder = m["type"].eq_ignore_ascii_case(SOLUTION_FOLDER_TYPE);
            let entry = if folder {
                SolutionEntry::Folder { name: m["name"].to_string(), path: String::new(), children: Vec::new() }
            } else {
                project(at(root, &m["path"]))
            };
            by_id.push((m["id"].to_uppercase(), entry));
            declaring_folder = folder;
        } else if declaring_folder && line.contains("ProjectSection(SolutionItems)") {
            in_items = true;
        } else if in_items && line.contains("EndProjectSection") {
            in_items = false;
        } else if in_items && let Some(m) = ITEM_LINE.captures(line) {
            // Visual Studio's Solution Items: "README.md = README.md", relative to the solution on each side.
            if let Some((_, SolutionEntry::Folder { children, .. })) = by_id.last_mut() {
                children.push(item(at(root, &m["path"])));
            }
        } else if line.starts_with("EndProject") && !line.starts_with("EndProjectSection") {
            declaring_folder = false;
        } else if line.contains("GlobalSection(NestedProjects)") {
            in_nesting = true;
        } else if in_nesting && line.contains("EndGlobalSection") {
            in_nesting = false;
        } else if in_nesting && let Some(m) = NESTING_LINE.captures(line) {
            parents.push((m["child"].to_uppercase(), m["parent"].to_uppercase()));
        }
    }
    // Children by their parent's index, in the file's order; an entry whose parent is not a folder stays at the top.
    let parent = |id: &str| {
        let parent = &parents.iter().find(|(child, _)| child == id)?.1;
        by_id.iter().position(|(id, e)| id == parent && matches!(e, SolutionEntry::Folder { .. }))
    };
    let parent_of: Vec<Option<usize>> = by_id.iter().map(|(id, _)| parent(id)).collect();
    let mut slots: Vec<Option<SolutionEntry>> = by_id.into_iter().map(|(_, e)| Some(e)).collect();
    fn build(i: usize, parent_of: &[Option<usize>], slots: &mut [Option<SolutionEntry>]) -> Option<SolutionEntry> {
        // Taken once, so nesting that goes round in a circle ends.
        let mut entry = slots[i].take()?;
        if let SolutionEntry::Folder { children, .. } = &mut entry {
            for child in (0..parent_of.len()).filter(|&c| parent_of[c] == Some(i)) {
                children.extend(build(child, parent_of, slots));
            }
        }
        Some(entry)
    }
    let mut entries: Vec<SolutionEntry> = (0..parent_of.len())
        .filter(|&i| parent_of[i].is_none())
        .filter_map(|i| build(i, &parent_of, &mut slots))
        .collect();
    place(&mut entries, "/");
    entries
}

/// Gives each folder its path in the solution, known only once the whole .sln is read.
fn place(entries: &mut [SolutionEntry], above: &str) {
    for entry in entries {
        if let SolutionEntry::Folder { name, path, children } = entry {
            *path = format!("{above}{name}/");
            place(children, &path.clone());
        }
    }
}
