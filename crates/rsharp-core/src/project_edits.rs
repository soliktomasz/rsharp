//! Rider's Add Reference, Remove on a project reference, and a project's items following a file moved, made to the
//! project file's text (Passenger's `ProjectEdits`): a reference goes beside the project's others, or in an item
//! group of its own before `</Project>`, written as `dotnet add reference` writes it (relative, with backslashes);
//! taking it out takes the group it leaves empty, so the file is as it was. The rest keeps its bytes.

use crate::paths;
use crate::solution_edits::relative;
use regex::Regex;
use std::path::{Component, Path, PathBuf};
use std::sync::LazyLock;

static REFERENCE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r#"(?s)<ProjectReference\b[^>]*?\bInclude\s*=\s*"(?<include>[^"]*)"[^>]*?(?:/>|>.*?</ProjectReference\s*>)"#,
    )
    .unwrap()
});
static ITEM_GROUP: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)<ItemGroup\b(?<attributes>[^>]*?)>(?<body>.*?)</ItemGroup\s*>").unwrap());
static COMMENT: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?s)<!--.*?-->").unwrap());
static INDENTED_LINE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\n([ \t]+)<").unwrap());
static START_TAG: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"<(?<tag>[A-Za-z_][\w.-]*)\b(?<rest>[^>]*?)(?<empty>/?)>").unwrap());
static ITEM_ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\b(?:Include|Update)\s*=\s*"(?<items>[^"]*)""#).unwrap());
static RELATED_ATTRIBUTE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r#"\b(?:DependentUpon|LastGenOutput)\s*=\s*"(?<value>[^"]+)""#).unwrap());
static RELATED_ELEMENT: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"<(?<name>DependentUpon|LastGenOutput)>(?<value>[^<]+)</(?<close>DependentUpon|LastGenOutput)\s*>")
        .unwrap()
});

/// An item group's tags, by where they start and end in the text.
struct Group {
    open_end: usize,
    close_start: usize,
    start: usize,
    end: usize,
    conditioned: bool,
}

/// A project reference in the text: the project it names, where its element is, and the group holding it.
struct Reference {
    path: PathBuf,
    start: usize,
    end: usize,
    group: Option<usize>,
}

fn stem(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Two paths for one file: the same, or the same but for letter case where the system ignores it.
fn same(a: &Path, b: &Path) -> bool {
    a == b || (!cfg!(target_os = "linux") && a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()))
}

/// A path with `.` and `..` worked out, as .NET's GetFullPath does, without asking the disk.
fn normalized(path: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for part in path.components() {
        match part {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other),
        }
    }
    out
}

fn commented(comments: &[(usize, usize)], at: usize) -> bool {
    comments.iter().any(|&(start, end)| at >= start && at < end)
}

fn comments(text: &str) -> Vec<(usize, usize)> {
    COMMENT.find_iter(text).map(|m| (m.start(), m.end())).collect()
}

fn groups_and_references(project: &Path, text: &str) -> (Vec<Group>, Vec<Reference>) {
    let folder = project.parent().unwrap_or(project);
    let comments = comments(text);
    let groups: Vec<Group> = ITEM_GROUP
        .captures_iter(text)
        .filter(|m| !commented(&comments, m.get(0).unwrap().start()))
        .map(|m| {
            let (whole, body) = (m.get(0).unwrap(), m.name("body").unwrap());
            Group {
                start: whole.start(),
                end: whole.end(),
                open_end: body.start(),
                close_start: body.end(),
                conditioned: m["attributes"].to_lowercase().contains("condition"),
            }
        })
        .collect();
    let references = REFERENCE
        .captures_iter(text)
        .filter(|m| !commented(&comments, m.get(0).unwrap().start()))
        .map(|m| {
            let whole = m.get(0).unwrap();
            Reference {
                path: paths::canonical(&normalized(&folder.join(m["include"].replace('\\', "/")))),
                start: whole.start(),
                end: whole.end(),
                group: groups.iter().position(|g| whole.start() >= g.open_end && whole.start() < g.close_start),
            }
        })
        .collect();
    (groups, references)
}

/// The indentation one level of the file takes: its first indented element's.
fn indent_unit(text: &str) -> String {
    INDENTED_LINE.captures(text).map(|m| m[1].to_string()).unwrap_or_else(|| "  ".into())
}

fn line_start(text: &str, at: usize) -> usize {
    text[..at].rfind('\n').map_or(0, |i| i + 1)
}

pub fn add_reference(project: &Path, text: &str, referenced: &Path) -> Result<String, String> {
    let (groups, references) = groups_and_references(project, text);
    if references.iter().any(|r| same(&r.path, referenced)) {
        return Err(format!("{} already references {}", stem(project), stem(referenced)));
    }
    let include = relative(project.parent().unwrap_or(project), referenced).replace('/', "\\");
    let eol = if text.contains("\r\n") { "\r\n" } else { "\n" };
    let element = format!("<ProjectReference Include=\"{include}\" />");
    // Beside the project's other references, in an item group of no condition.
    if let Some(last) = references.iter().rev().find(|r| r.group.is_some_and(|g| !groups[g].conditioned)) {
        let at = text[last.end..].find('\n').map_or(text.len(), |i| last.end + i + 1);
        let indent = &text[line_start(text, last.start)..last.start];
        let mut out = text.to_string();
        out.insert_str(at, &format!("{indent}{element}{eol}"));
        return Ok(out);
    }
    let close = text.rfind("</Project>").ok_or_else(|| format!("{} is not a project file", stem(project)))?;
    let close_line = line_start(text, close);
    // A file that leaves a blank line between its groups gets one after the new group too.
    let before = &text[..close_line];
    let spaced = close_line >= 2 && before.trim_end_matches(['\r', '\n']).len() < before.len() - eol.len();
    let indent = indent_unit(text);
    let group = format!(
        "{indent}<ItemGroup>{eol}{indent}{indent}{element}{eol}{indent}</ItemGroup>{eol}{}",
        if spaced { eol } else { "" }
    );
    let mut out = text.to_string();
    out.insert_str(close_line, &group);
    Ok(out)
}

/// The line or lines a span stands on, their indentation and line break included, when nothing else is on them; the
/// span alone otherwise.
fn whole_lines(text: &str, start: usize, end: usize) -> (usize, usize) {
    let from = line_start(text, start);
    if start > 0 && !text[from..start].trim().is_empty() {
        return (start, end);
    }
    let line_end = text[end..].find('\n').map(|i| end + i);
    let rest = &text[end..line_end.unwrap_or(text.len())];
    if !rest.trim().is_empty() {
        return (start, end);
    }
    (from, line_end.map_or(text.len(), |i| i + 1))
}

fn blank_line_before(text: &str, line: usize) -> bool {
    line >= 2 && text[line_start(text, line - 1)..line - 1].trim().is_empty()
}

/// The end of a blank line beginning at a place, line break included, when the line there is blank.
fn blank_line_at(text: &str, at: usize) -> Option<usize> {
    let end = at + text[at..].find('\n')?;
    text[at..end].trim().is_empty().then_some(end + 1)
}

pub fn remove_reference(project: &Path, text: &str, referenced: &Path) -> Result<String, String> {
    let (groups, references) = groups_and_references(project, text);
    let reference = references
        .iter()
        .find(|r| same(&r.path, referenced))
        .ok_or_else(|| format!("{} does not reference {}", stem(project), stem(referenced)))?;
    let (start, end) = whole_lines(text, reference.start, reference.end);
    let mut out = text.to_string();
    if let Some(group) = reference.group.map(|g| &groups[g])
        && (text[group.open_end..start].to_string() + &text[end..group.close_start]).trim().is_empty()
    {
        // The group is left empty: it goes too, with the blank line that set it apart from the next.
        let (from, mut to) = whole_lines(text, group.start, group.end);
        if blank_line_before(text, from)
            && let Some(blank) = blank_line_at(text, to)
        {
            to = blank;
        }
        out.replace_range(from..to, "");
        return Ok(out);
    }
    out.replace_range(start..end, "");
    Ok(out)
}

/// A path as a project writes it, with the separators the value it replaces had.
fn written(relative: &str, was: &str) -> String {
    if was.contains('\\') { relative.replace('/', "\\") } else { relative.replace('\\', "/") }
}

/// The project following files and folders that moved or were renamed, as Rider's project model does: an item that
/// names a file by itself (its Include or Update, no wildcards) names where it went, and a DependentUpon or
/// LastGenOutput naming one (relative to its item's folder) names it there. None when nothing in it changed.
pub fn move_items(project: &Path, text: &str, moves: &[(PathBuf, PathBuf)]) -> Option<String> {
    let folder = project.parent().unwrap_or(project);
    // A file moved, or one inside a folder that was.
    let moved = |path: &Path| -> PathBuf {
        for (from, to) in moves {
            if same(path, from) {
                return to.clone();
            }
            if let Ok(rest) = path.strip_prefix(from) {
                return to.join(rest);
            }
        }
        path.to_path_buf()
    };
    let comments = comments(text);
    let mut spans: Vec<(std::ops::Range<usize>, String)> = Vec::new();
    for tag in START_TAG.captures_iter(text) {
        let whole = tag.get(0).unwrap();
        let Some(attribute) = ITEM_ATTRIBUTE.captures(&tag["rest"]) else { continue };
        let items = attribute.name("items").unwrap();
        if commented(&comments, whole.start()) || items.as_str().contains(['*', '?', ';', '$', '@', '%']) {
            continue;
        }
        // The element's end: its start tag when empty, else its own end tag.
        let end = if !tag["empty"].is_empty() {
            whole.end()
        } else {
            let close = format!("</{}", &tag["tag"]);
            match text[whole.end()..].find(&close) {
                Some(at) => whole.end() + at + text[whole.end() + at..].find('>').map_or(0, |i| i + 1),
                None => continue,
            }
        };
        let was = normalized(&folder.join(items.as_str().replace('\\', "/")));
        let now = moved(&was);
        if now != was {
            let at = whole.start()
                + 1
                + tag["tag"].len()
                + attribute.get(0).unwrap().start()
                + (items.start() - attribute.get(0).unwrap().start());
            let value = written(&relative(folder, &now), items.as_str());
            spans.push((at..at + items.len(), value));
        }
        // What the item names beside it, relative to its folder: where that is now, from where the item is now.
        let element = &text[whole.start()..end];
        let (was_folder, now_folder) = (was.parent().unwrap_or(folder), now.parent().unwrap_or(folder));
        let mut related = |value: regex::Match| {
            let named = normalized(&was_folder.join(value.as_str().trim().replace('\\', "/")));
            let target = moved(&named);
            if target != named || now != was {
                let at = whole.start() + value.start();
                let replaced = written(&relative(now_folder, &target), value.as_str());
                if replaced != value.as_str() {
                    spans.push((at..at + value.len(), replaced));
                }
            }
        };
        for m in RELATED_ATTRIBUTE.captures_iter(element) {
            related(m.name("value").unwrap());
        }
        for m in RELATED_ELEMENT.captures_iter(element).filter(|m| m["name"] == m["close"]) {
            related(m.name("value").unwrap());
        }
    }
    if spans.is_empty() {
        return None;
    }
    spans.sort_by_key(|(span, _)| std::cmp::Reverse(span.start));
    let mut out = text.to_string();
    for (span, value) in spans {
        out.replace_range(span, &value);
    }
    Some(out)
}

/// The projects a project file references, outside comments, in the file's order.
pub fn references(project: &Path, text: &str) -> Vec<PathBuf> {
    groups_and_references(project, text).1.into_iter().map(|r| r.path).collect()
}
