//! What the code help offers, shows and goes to at a place in an MSBuild file: MSBuild's schemas and the last
//! evaluation of its project, as Passenger's MSBuildCompletion and MSBuildNavigation give them.

use super::context::{self, Place, Tag};
use super::insight::ProjectInsight;
use super::schema::{self, ITEM_ATTRIBUTES};
use std::ops::Range;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Element,
    Attribute,
    Property,
    Item,
    Metadata,
    Task,
    Target,
    Value,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Suggestion {
    pub label: String,
    pub detail: String,
    /// What choosing it writes over `replace`, and where in it the caret goes.
    pub insert: String,
    pub caret: usize,
    pub replace: Range<usize>,
    pub kind: Kind,
}

/// What only the feeds can answer: the packages a prefix finds, or a package's versions.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Ask {
    Packages { prefix: String, replace: Range<usize> },
    Versions { id: String, replace: Range<usize> },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Completion {
    pub suggestions: Vec<Suggestion>,
    pub ask: Option<Ask>,
}

const PACKAGE_ITEMS: &[&str] = &["PackageReference", "PackageVersion", "GlobalPackageReference", "PackageDownload"];
const WELL_KNOWN_METADATA: &[&str] = &[
    "Identity",
    "FullPath",
    "RootDir",
    "Filename",
    "Extension",
    "RelativeDir",
    "Directory",
    "RecursiveDir",
    "ModifiedTime",
    "CreatedTime",
    "AccessedTime",
    "DefiningProjectFullPath",
    "DefiningProjectDirectory",
    "DefiningProjectName",
    "DefiningProjectExtension",
];

fn name_end(text: &str, from: usize) -> usize {
    from + text[from..].bytes().take_while(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.')).count()
}

/// The part of a semicolon list the caret is in, from its start to its end.
fn segment(text: &str, start: usize, end: usize, at: usize) -> Range<usize> {
    let from = text[start..at].rfind(';').map_or(start, |s| start + s + 1);
    let from = from + text[from..at].len() - text[from..at].trim_start().len();
    let to = text[at..end].find(';').map_or(end, |s| at + s);
    from..to
}

fn plain(label: &str, detail: Option<&str>, replace: Range<usize>, kind: Kind) -> Suggestion {
    Suggestion {
        label: label.into(),
        detail: detail.unwrap_or("").into(),
        insert: label.into(),
        caret: label.len(),
        replace,
        kind,
    }
}

/// Whether an element inside `parent` is an item (its parent an ItemGroup) or a task (its parent a Target).
fn parent_is(path: &[String], parent: &str) -> bool {
    path.last().is_some_and(|p| p == parent)
}

pub fn complete(text: &str, at: usize, insight: Option<&ProjectInsight>) -> Completion {
    let schema = schema::schema();
    let mut out = Completion::default();
    match context::at(text, at) {
        Place::ElementName { path, start, closing } => {
            let replace = start..name_end(text, at);
            if closing {
                if let Some(open) = path.last() {
                    out.suggestions.push(plain(open, None, replace, Kind::Element));
                }
                return out;
            }
            let Some(parent) = path.last() else {
                out.suggestions.push(plain("Project", Some("An MSBuild project"), replace, Kind::Element));
                return out;
            };
            let grand = path.len().checked_sub(2).map(|i| path[i].as_str());
            let structural = schema::structural(parent);
            for child in structural.iter().flat_map(|s| s.children) {
                out.suggestions.push(plain(child, None, replace.clone(), Kind::Element));
            }
            if structural.as_ref().is_some_and(|s| s.properties) {
                for p in &schema.properties {
                    out.suggestions.push(plain(&p.name, p.doc.as_deref(), replace.clone(), Kind::Property));
                }
                // A leading underscore is MSBuild's mark of a property private to the SDK's own files.
                let private = !text[replace.start..at].starts_with('_');
                for p in insight
                    .iter()
                    .flat_map(|i| &i.properties)
                    .filter(|p| p.origin == 0 && !(private && p.name.starts_with('_')))
                {
                    if schema.property(&p.name).is_none() {
                        out.suggestions.push(plain(&p.name, Some(&p.value), replace.clone(), Kind::Property));
                    }
                }
            }
            if structural.as_ref().is_some_and(|s| s.items) {
                for i in &schema.items {
                    out.suggestions.push(plain(&i.name, i.doc.as_deref(), replace.clone(), Kind::Item));
                }
                for t in insight.iter().flat_map(|i| &i.item_types) {
                    if schema.item(&t.name).is_none() {
                        out.suggestions.push(plain(&t.name, None, replace.clone(), Kind::Item));
                    }
                }
            }
            if structural.as_ref().is_some_and(|s| s.tasks) {
                for t in &schema.tasks {
                    out.suggestions.push(plain(&t.name, t.doc.as_deref(), replace.clone(), Kind::Task));
                }
            }
            // An item's metadata as elements inside it.
            if structural.is_none() && grand.is_some_and(|g| g == "ItemGroup" || g == "ItemDefinitionGroup") {
                for m in metadata_of(parent, insight) {
                    out.suggestions.push(plain(&m.0, m.1.as_deref(), replace.clone(), Kind::Metadata));
                }
            }
        }
        Place::AttributeName { tag, path, start } => {
            let replace = start..name_end(text, at);
            let mut add = |name: &str, doc: Option<&str>, kind: Kind| {
                if tag.attribute(name).is_none() {
                    let insert = format!("{name}=\"\"");
                    out.suggestions.push(Suggestion {
                        label: name.into(),
                        detail: doc.unwrap_or("").into(),
                        caret: insert.len() - 1,
                        insert,
                        replace: replace.clone(),
                        kind,
                    });
                }
            };
            if let Some(structural) = schema::structural(&tag.name) {
                for (name, doc) in structural.attributes {
                    add(name, Some(doc), Kind::Attribute);
                }
            } else if parent_is(&path, "ItemGroup") || parent_is(&path, "ItemDefinitionGroup") {
                for (name, doc) in ITEM_ATTRIBUTES {
                    add(name, Some(doc), Kind::Attribute);
                }
                for (name, doc) in metadata_of(&tag.name, insight) {
                    add(&name, doc.as_deref(), Kind::Metadata);
                }
            } else if parent_is(&path, "Target") {
                for p in schema.task(&tag.name).iter().flat_map(|t| &t.children) {
                    add(&p.name, p.doc.as_deref(), Kind::Attribute);
                }
                add(
                    "Condition",
                    Some("Optional expression evaluated to determine whether the task runs"),
                    Kind::Attribute,
                );
                add(
                    "ContinueOnError",
                    Some("Optional: whether the build goes on after the task fails"),
                    Kind::Attribute,
                );
            } else if parent_is(&path, "PropertyGroup") {
                add(
                    "Condition",
                    Some("Optional expression evaluated to determine whether the property is set"),
                    Kind::Attribute,
                );
            }
        }
        Place::AttributeValue { tag, attribute, start, end, .. } => {
            let replace = segment(text, start, end, at);
            let lists_targets = matches!(
                (tag.name.as_str(), attribute.as_str()),
                ("Target", "DependsOnTargets" | "BeforeTargets" | "AfterTargets")
                    | ("Project", "DefaultTargets" | "InitialTargets")
                    | ("CallTarget", "Targets")
                    | ("MSBuild", "Targets")
            );
            if lists_targets {
                for t in insight.iter().flat_map(|i| &i.targets) {
                    out.suggestions.push(plain(&t.name, None, replace.clone(), Kind::Target));
                }
            } else if PACKAGE_ITEMS.contains(&tag.name.as_str()) {
                // shortcut: under central package management a new PackageReference is completed as any other; write
                // its PackageVersion into Directory.Packages.props in the same edit, as Passenger does, once CPM's
                // projects are edited here.
                match attribute.as_str() {
                    "Include" | "Update" => {
                        out.ask = Some(Ask::Packages { prefix: text[replace.start..at].to_string(), replace });
                    }
                    "Version" | "VersionOverride" => {
                        if let Some(id) =
                            tag.attribute("Include").or_else(|| tag.attribute("Update")).filter(|i| !i.is_empty())
                        {
                            out.ask = Some(Ask::Versions { id: id.to_string(), replace: start..end });
                        }
                    }
                    _ => {}
                }
            }
        }
        Place::Text { path, start } => {
            let Some(element) = path.last() else { return out };
            let end = start + text[start..].find('<').unwrap_or(text.len() - start);
            if matches!(element.as_str(), "TargetFramework" | "TargetFrameworks")
                && parent_is(&path[..path.len() - 1], "PropertyGroup")
            {
                let replace = segment(text, start, end, at);
                for f in insight.iter().flat_map(|i| &i.frameworks) {
                    out.suggestions.push(plain(&f.alias, Some(&f.name), replace.clone(), Kind::Value));
                }
            }
        }
        Place::Reference { kind, start } => {
            let end = name_end(text, at);
            let close = if text[end..].starts_with(')') { "" } else { ")" };
            let mut add = |name: &str, doc: Option<&str>, kind: Kind| {
                let insert = format!("{name}{close}");
                out.suggestions.push(Suggestion {
                    label: name.into(),
                    detail: doc.unwrap_or("").into(),
                    caret: insert.len(),
                    insert,
                    replace: start..end,
                    kind,
                });
            };
            match kind {
                '$' => {
                    let mut seen = Vec::new();
                    for p in insight.iter().flat_map(|i| &i.properties).filter(|p| p.origin != 3) {
                        seen.push(p.name.to_lowercase());
                        add(&p.name, Some(&p.value), Kind::Property);
                    }
                    for p in schema.properties.iter().filter(|p| !seen.contains(&p.name.to_lowercase())) {
                        add(&p.name, p.doc.as_deref(), Kind::Property);
                    }
                }
                '@' => {
                    for t in insight.iter().flat_map(|i| &i.item_types) {
                        add(&t.name, Some(&format!("{} items", t.count)), Kind::Item);
                    }
                }
                _ => {
                    for m in WELL_KNOWN_METADATA {
                        add(m, Some("Well-known item metadata"), Kind::Metadata);
                    }
                }
            }
        }
        Place::None => {}
    }
    out
}

/// An item type's metadata: the schema's, then what the evaluation's items of it carry.
fn metadata_of(item: &str, insight: Option<&ProjectInsight>) -> Vec<(String, Option<String>)> {
    let mut found: Vec<(String, Option<String>)> =
        schema::schema().item(item).iter().flat_map(|e| &e.children).map(|c| (c.name.clone(), c.doc.clone())).collect();
    for m in insight.and_then(|i| i.item_type(item)).iter().flat_map(|t| &t.metadata) {
        if !found.iter().any(|(n, _)| n.eq_ignore_ascii_case(m)) {
            found.push((m.clone(), None));
        }
    }
    found
}

/// What a name at a place is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Symbol {
    Property(String),
    Target(String),
    ItemType(String),
    /// An item element, by its tag.
    Item(Tag),
    /// An import, or a project's or Sdk element's SDK, by its tag.
    Import(Tag),
}

/// The name under `at` and what it is.
pub fn symbol(text: &str, at: usize) -> Option<Symbol> {
    let bytes = text.as_bytes();
    let is_name = |c: u8| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.');
    let (mut from, mut to) = (at.min(bytes.len()), at.min(bytes.len()));
    while from > 0 && is_name(bytes[from - 1]) {
        from -= 1;
    }
    while to < bytes.len() && is_name(bytes[to]) {
        to += 1;
    }
    if from == to {
        return None;
    }
    let name = text[from..to].to_string();
    if from >= 2 && bytes[from - 1] == b'(' {
        match bytes[from - 2] {
            b'$' => return Some(Symbol::Property(name)),
            b'@' => return Some(Symbol::ItemType(name)),
            _ => {}
        }
    }
    match context::at(text, to) {
        Place::ElementName { path, .. } => {
            let tag_start = text[..from].rfind('<')?;
            let tag = Tag { name: name.clone(), start: tag_start, attributes: tag_attributes(text, tag_start) };
            if parent_is(&path, "PropertyGroup") {
                Some(Symbol::Property(name))
            } else if parent_is(&path, "ItemGroup") {
                Some(Symbol::Item(tag))
            } else if name == "Import" || name == "Sdk" {
                Some(Symbol::Import(tag))
            } else {
                None
            }
        }
        Place::AttributeValue { tag, attribute, .. } => {
            let tag = Tag { attributes: tag_attributes(text, tag.start), ..tag };
            match (tag.name.as_str(), attribute.as_str()) {
                ("Target", "Name" | "DependsOnTargets" | "BeforeTargets" | "AfterTargets")
                | ("CallTarget" | "MSBuild", "Targets")
                | ("Project", "DefaultTargets" | "InitialTargets") => Some(Symbol::Target(name)),
                ("Import", "Project" | "Sdk") | ("Project", "Sdk") | ("Sdk", "Name") => Some(Symbol::Import(tag)),
                _ => None,
            }
        }
        _ => None,
    }
}

/// Every attribute of the start tag at `start`, the caret's side and after.
fn tag_attributes(text: &str, start: usize) -> Vec<(String, String)> {
    let end = text[start..].find('>').map_or(text.len(), |e| start + e);
    match context::at(text, end) {
        Place::AttributeName { tag, .. } if tag.start == start => tag.attributes,
        _ => Vec::new(),
    }
}

fn line_of(text: &str, at: usize) -> usize {
    text[..at].bytes().filter(|c| *c == b'\n').count() + 1
}

/// The hover's words for the name under `at`.
pub fn hover(text: &str, at: usize, file: &Path, insight: &ProjectInsight) -> Option<String> {
    let place = |p| insight.at(p).map(|(f, l, c)| format!("{}:{}:{}", f.display(), l + 1, c + 1));
    match symbol(text, at)? {
        Symbol::Property(name) => {
            let schema_doc = schema::schema().property(&name).and_then(|p| p.doc.clone());
            let Some(p) = insight.property(&name) else {
                return schema_doc.map(|d| format!("{name}\n{d}\n(not set in this evaluation)"));
            };
            let origin = match p.origin {
                1 => " (a global property)".to_string(),
                2 => " (from the environment)".to_string(),
                3 => " (MSBuild's own)".to_string(),
                _ => p.set.first().and_then(|s| place(*s)).map(|s| format!("\nSet at {s}")).unwrap_or_default(),
            };
            Some(format!("{name} = {}{origin}{}", p.value, schema_doc.map(|d| format!("\n{d}")).unwrap_or_default()))
        }
        Symbol::Target(name) => {
            let t = insight.target(&name)?;
            let at = t.defined.first().and_then(|d| place(*d)).unwrap_or_default();
            Some(format!("Target {}\nDefined at {at}", t.name))
        }
        Symbol::ItemType(name) => {
            let t = insight.item_type(&name)?;
            Some(format!("@({}): {} items\n{}", t.name, t.count, t.includes.join("\n")))
        }
        Symbol::Item(tag) => {
            let line = line_of(text, tag.start);
            let item =
                insight.items.iter().find(|i| insight.is_here(i.element, file, line) && i.item_type == tag.name)?;
            let doc = schema::schema()
                .item(&tag.name)
                .and_then(|e| e.doc.clone())
                .map(|d| format!("\n{d}"))
                .unwrap_or_default();
            Some(format!("{}: {} items\n{}{doc}", tag.name, item.count, item.includes.join("\n")))
        }
        Symbol::Import(tag) => {
            let line = line_of(text, tag.start);
            let imported: Vec<String> = insight
                .imports
                .iter()
                .filter(|i| insight.is_here(i.element, file, line))
                .map(|i| i.imported.display().to_string())
                .collect();
            (!imported.is_empty()).then(|| format!("Imports\n{}", imported.join("\n")))
        }
    }
}

/// Where the name under `at` is declared: a property where it was last set, a target where it is defined, an item
/// type where the project first adds to it, an import the file it brought in. The file, line and column from 0.
pub fn declaration(text: &str, at: usize, file: &Path, insight: &ProjectInsight) -> Option<(PathBuf, usize, usize)> {
    match symbol(text, at)? {
        Symbol::Property(name) => insight.at(*insight.property(&name)?.set.first()?),
        Symbol::Target(name) => insight.at(*insight.target(&name)?.defined.first()?),
        Symbol::ItemType(name) => insight.at(*insight.item_type(&name)?.defined.first()?),
        Symbol::Item(_) => None,
        Symbol::Import(tag) => {
            let line = line_of(text, tag.start);
            let import = insight.imports.iter().find(|i| insight.is_here(i.element, file, line))?;
            Some((import.imported.clone(), 0, 0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::msbuild::insight::*;

    fn insight(file: &Path) -> ProjectInsight {
        ProjectInsight {
            project: file.into(),
            evaluated: true,
            files: vec![file.into(), "/sdk/Sdk.props".into(), "/sdk/Microsoft.Common.targets".into()],
            properties: vec![
                InsightProperty {
                    name: "TargetFramework".into(),
                    value: "net10.0".into(),
                    origin: 0,
                    set: vec![InsightPlace { file: 0, line: 3, column: 5 }],
                },
                InsightProperty { name: "Mine".into(), value: "yes".into(), origin: 0, set: vec![] },
            ],
            item_types: vec![InsightItemType {
                name: "Compile".into(),
                count: 2,
                includes: vec!["A.cs".into()],
                metadata: vec![],
                defined: vec![],
            }],
            targets: vec![InsightTarget {
                name: "Build".into(),
                defined: vec![InsightPlace { file: 2, line: 10, column: 3 }],
            }],
            imports: vec![InsightImport {
                element: InsightPlace { file: 0, line: 1, column: 1 },
                imported: "/sdk/Sdk.props".into(),
                sdk: Some("Microsoft.NET.Sdk".into()),
            }],
            frameworks: vec![InsightFramework { alias: "net10.0".into(), name: ".NETCoreApp,Version=v10.0".into() }],
            ..Default::default()
        }
    }

    fn at(marked: &str) -> (String, usize) {
        (marked.replace('|', ""), marked.find('|').unwrap())
    }

    const PROJECT: &str =
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n";

    #[test]
    fn completion_offers_what_belongs_where_the_caret_is() {
        let file = Path::new("/p/Demo.csproj");
        let i = insight(file);
        let labels = |marked: &str| {
            let (text, caret) = at(marked);
            complete(&text, caret, Some(&i)).suggestions.into_iter().map(|s| s.label).collect::<Vec<_>>()
        };
        let properties = labels(&format!("{PROJECT}    <Nulla|"));
        assert!(
            properties.contains(&"Nullable".to_string()) && properties.contains(&"Mine".to_string()),
            "{properties:?}"
        );
        assert!(!properties.contains(&"PackageReference".to_string()), "an item was offered among properties");
        let items = labels(&format!("{PROJECT}  </PropertyGroup>\n  <ItemGroup>\n    <|"));
        assert!(items.contains(&"PackageReference".to_string()) && items.contains(&"Compile".to_string()));
        let attributes = labels(&format!("{PROJECT}  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference |"));
        assert!(
            attributes.contains(&"Include".to_string()) && attributes.contains(&"PrivateAssets".to_string()),
            "{attributes:?}"
        );
        assert_eq!(
            labels(&format!("{PROJECT}    <Out>$(Target|")).first().map(String::as_str),
            Some("TargetFramework")
        );
        assert_eq!(
            labels(&format!("{PROJECT}  </PropertyGroup>\n  <Target Name=\"A\" DependsOnTargets=\"Restore;B|")),
            ["Build"]
        );
        let frameworks = labels("<Project>\n  <PropertyGroup>\n    <TargetFrameworks>net8.0;|</TargetFrameworks>");
        assert_eq!(frameworks, ["net10.0"]);
    }

    #[test]
    fn package_ids_and_versions_are_asked_of_the_feeds() {
        let (text, caret) = at("<Project><ItemGroup><PackageReference Include=\"Newton|\" />");
        assert_eq!(complete(&text, caret, None).ask, Some(Ask::Packages { prefix: "Newton".into(), replace: 47..53 }));
        let (text, caret) = at("<Project><ItemGroup><PackageReference Include=\"Demo.Core\" Version=\"1.|\" />");
        assert!(matches!(complete(&text, caret, None).ask, Some(Ask::Versions { id, .. }) if id == "Demo.Core"));
    }

    #[test]
    fn hover_and_declaration_follow_the_evaluation() {
        let file = Path::new("/p/Demo.csproj");
        let i = insight(file);
        let (text, caret) = at(&format!("{PROJECT}    <Out>$(TargetFr|amework)</Out>"));
        let shown = hover(&text, caret, file, &i).unwrap();
        assert!(shown.starts_with("TargetFramework = net10.0\nSet at /p/Demo.csproj:3:5"), "{shown}");
        assert_eq!(declaration(&text, caret, file, &i), Some((file.into(), 2, 4)));
        let (text, caret) = at("<Project><Target Name=\"A\" DependsOnTargets=\"Bu|ild\" />");
        assert_eq!(declaration(&text, caret, file, &i), Some(("/sdk/Microsoft.Common.targets".into(), 9, 2)));
        let (text, caret) = at("<Project Sdk=\"Microsoft.N|ET.Sdk\">\n");
        assert_eq!(declaration(&text, caret, file, &i), Some(("/sdk/Sdk.props".into(), 0, 0)));
    }
}
