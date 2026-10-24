//! MSBuild's own schemas (Microsoft.Build.Core.xsd and Microsoft.Build.CommonTypes.xsd from dotnet/msbuild, MIT),
//! carried since the .NET SDK does not: CommonTypes read once, on first use, into the properties, item types (with their
//! metadata) and tasks (with their parameters) its substitution groups declare; Core's structural elements a table.
//! A few items the schemas lack (central package management's) are Passenger's, as Rider knows them.

use std::sync::OnceLock;

const COMMON_TYPES: &str = include_str!("../../assets/msbuild/Microsoft.Build.CommonTypes.xsd");

/// A property, item type, piece of metadata, task or task parameter, with what the schema says of it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub doc: Option<String>,
    /// An item type's metadata, or a task's parameters.
    pub children: Vec<Entry>,
}

#[derive(Debug, Default)]
pub struct Schema {
    pub properties: Vec<Entry>,
    pub items: Vec<Entry>,
    pub tasks: Vec<Entry>,
}

impl Schema {
    pub fn item(&self, name: &str) -> Option<&Entry> {
        self.items.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
    pub fn property(&self, name: &str) -> Option<&Entry> {
        self.properties.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
    pub fn task(&self, name: &str) -> Option<&Entry> {
        self.tasks.iter().find(|e| e.name.eq_ignore_ascii_case(name))
    }
}

/// The schemas, read the first time they are asked for: a caller on the UI thread asks off it.
pub fn schema() -> &'static Schema {
    static READ: OnceLock<Schema> = OnceLock::new();
    READ.get_or_init(|| {
        let mut schema = read(COMMON_TYPES);
        for (name, doc) in [
            (
                "PackageVersion",
                "A package's version under central package management (Directory.Packages.props), which the projects' PackageReference items take",
            ),
            ("GlobalPackageReference", "A package every project under central package management references, as a development dependency"),
            ("PackageDownload", "A package restore downloads without the project referencing it"),
            ("AdditionalFiles", "A file passed to the project's analyzers and source generators"),
        ] {
            if schema.item(name).is_none() {
                let version = Entry { name: "Version".into(), doc: Some("Version of the package".into()), children: vec![] };
                let children = if name == "AdditionalFiles" { vec![] } else { vec![version] };
                schema.items.push(Entry { name: name.into(), doc: Some(doc.into()), children });
            }
        }
        for list in [&mut schema.properties, &mut schema.items, &mut schema.tasks] {
            list.sort_by_key(|e| e.name.to_lowercase());
            list.dedup_by(|a, b| a.name.eq_ignore_ascii_case(&b.name));
        }
        schema
    })
}

/// A structural element of Core.xsd: what it holds, the attributes it takes, and whether any property, item or task
/// goes inside it.
pub struct Structural {
    pub children: &'static [&'static str],
    pub attributes: &'static [(&'static str, &'static str)],
    pub properties: bool,
    pub items: bool,
    pub tasks: bool,
}

const CONDITION: (&str, &str) =
    ("Condition", "Optional expression evaluated to determine whether the element is evaluated");
const LABEL: (&str, &str) = ("Label", "Optional expression. Used to identify or order system and user elements");

pub fn structural(element: &str) -> Option<Structural> {
    let s =
        |children, attributes, properties, items, tasks| Structural { children, attributes, properties, items, tasks };
    Some(match element {
        "Project" => s(
            &[
                "PropertyGroup",
                "ItemGroup",
                "Target",
                "Import",
                "ImportGroup",
                "Choose",
                "ItemDefinitionGroup",
                "UsingTask",
                "ProjectExtensions",
                "Sdk",
            ],
            &[
                ("Sdk", "The SDKs the project uses, as Name/Version, separated by semicolons"),
                (
                    "DefaultTargets",
                    "Optional semi-colon separated list of one or more targets that will be built if no targets are otherwise specified",
                ),
                (
                    "InitialTargets",
                    "Optional semi-colon separated list of targets that should always be built before any other targets",
                ),
                (
                    "TreatAsLocalProperty",
                    "Optional semi-colon separated list of properties that a project file may set though they are global",
                ),
                (
                    "ToolsVersion",
                    "Optional string describing the toolset version this project should normally be built with",
                ),
            ],
            false,
            false,
            false,
        ),
        "PropertyGroup" => s(&[], &[CONDITION, LABEL], true, false, false),
        "ItemGroup" | "ItemDefinitionGroup" => s(&[], &[CONDITION, LABEL], false, true, false),
        "Target" => s(
            &["PropertyGroup", "ItemGroup", "OnError"],
            &[
                ("Name", "Name of the target"),
                (
                    "DependsOnTargets",
                    "Optional semi-colon separated list of targets that should be run before this target",
                ),
                ("BeforeTargets", "Optional semi-colon separated list of targets that this target should run before"),
                ("AfterTargets", "Optional semi-colon separated list of targets that this target should run after"),
                ("Inputs", "Optional semi-colon separated list of files that form inputs into this target"),
                ("Outputs", "Optional semi-colon separated list of files that form outputs into this target"),
                ("Returns", "Optional expression evaluated to the items the target returns"),
                ("KeepDuplicateOutputs", "Optional: whether duplicate items in the Returns or Outputs are kept"),
                CONDITION,
                LABEL,
            ],
            false,
            false,
            true,
        ),
        "Import" => s(
            &[],
            &[
                ("Project", "Project file to import"),
                ("Sdk", "Name of the SDK which contains the project file to import"),
                ("Version", "Optional version of the SDK"),
                ("MinimumVersion", "Optional minimum version of the SDK"),
                CONDITION,
                LABEL,
            ],
            false,
            false,
            false,
        ),
        "ImportGroup" => s(&["Import"], &[CONDITION, LABEL], false, false, false),
        "Choose" => s(&["When", "Otherwise"], &[LABEL], false, false, false),
        "When" => s(&["PropertyGroup", "ItemGroup", "Choose"], &[CONDITION], false, false, false),
        "Otherwise" => s(&["PropertyGroup", "ItemGroup", "Choose"], &[], false, false, false),
        "UsingTask" => s(
            &["ParameterGroup", "Task"],
            &[
                ("TaskName", "Name of the task class in the assembly"),
                ("AssemblyFile", "Optional path to the assembly"),
                ("AssemblyName", "Optional name of the assembly"),
                ("TaskFactory", "Optional name of the task factory"),
                CONDITION,
            ],
            false,
            false,
            false,
        ),
        "Sdk" => {
            s(&[], &[("Name", "Name of the SDK"), ("Version", "Optional version of the SDK")], false, false, false)
        }
        "OnError" => s(
            &[],
            &[("ExecuteTargets", "Semi-colon separated list of targets to run on error"), CONDITION, LABEL],
            false,
            false,
            false,
        ),
        _ => return None,
    })
}

/// What every item element takes, whatever its type.
pub const ITEM_ATTRIBUTES: &[(&str, &str)] = &[
    (
        "Include",
        "Semi-colon separated list of files (wildcards are allowed) or other item names to include in this item list",
    ),
    (
        "Exclude",
        "Semi-colon separated list of files (wildcards are allowed) or other item names to exclude from the Include list",
    ),
    (
        "Remove",
        "Semi-colon separated list of files (wildcards are allowed) or other item names to remove from the existing list contents",
    ),
    (
        "Update",
        "Semi-colon separated list of files (wildcards are allowed) or other item names to be updated with the metadata contained in this xml element",
    ),
    ("KeepMetadata", "Optional semi-colon separated list of metadata names to keep, leaving out the rest"),
    ("RemoveMetadata", "Optional semi-colon separated list of metadata names to remove"),
    ("KeepDuplicates", "Optional: whether an item already in the list is added again"),
    ("MatchOnMetadata", "Optional semi-colon separated list of metadata names that Remove matches items on"),
    CONDITION,
    LABEL,
];

fn read(xsd: &str) -> Schema {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xsd);
    let mut schema = Schema::default();
    // The open elements, each as its local name and, for an xs:element or xs:attribute, its name.
    let mut open: Vec<(String, Option<String>)> = Vec::new();
    let mut current: Option<(&'static str, Entry)> = None;
    let mut doc_of: Option<(usize, String)> = None;
    let mut doc = String::new();
    loop {
        let Ok(event) = reader.read_event() else { break };
        match event {
            Event::Start(ref e) | Event::Empty(ref e) => {
                let empty = matches!(event, Event::Empty(_));
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                let attribute = |k: &str| {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == k.as_bytes())
                        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
                };
                let name = attribute("name");
                if local == "element" && open.len() == 1 {
                    let group = attribute("substitutionGroup").unwrap_or_default();
                    let kind = match group.rsplit(':').next().unwrap_or("") {
                        "Property" => Some("property"),
                        "Item" => Some("item"),
                        "Task" => Some("task"),
                        _ => None,
                    };
                    current = kind.zip(name.clone()).map(|(k, n)| (k, Entry { name: n, doc: None, children: vec![] }));
                } else if let (Some((kind, entry)), Some(name)) = (&mut current, &name) {
                    let child = (*kind == "item" && local == "element") || (*kind == "task" && local == "attribute");
                    if child && !entry.children.iter().any(|c| &c.name == name) {
                        entry.children.push(Entry { name: name.clone(), doc: None, children: vec![] });
                    }
                }
                if local == "documentation" && current.is_some() {
                    // The documentation of the nearest named element above it: the entry's own, or its last child's.
                    let at = open.iter().rposition(|(_, n)| n.is_some()).unwrap_or(0);
                    doc_of = Some((at, open[at].1.clone().unwrap_or_default()));
                    doc.clear();
                }
                if !empty {
                    open.push((local, name));
                } else if local == "element" && open.len() == 1 {
                    // An element written as <xs:element … /> has no end to be taken at.
                    match current.take() {
                        Some(("property", e)) => schema.properties.push(e),
                        Some(("item", e)) => schema.items.push(e),
                        Some(("task", e)) => schema.tasks.push(e),
                        _ => {}
                    }
                }
            }
            Event::Text(t) if doc_of.is_some() => {
                doc.push_str(&t.unescape().map(|t| t.into_owned()).unwrap_or_default())
            }
            Event::End(e) => {
                let local = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if local == "documentation"
                    && let (Some((at, named)), Some((_, entry))) = (doc_of.take(), &mut current)
                {
                    let words = doc.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !words.is_empty() {
                        // Depth 1 is the entry's own element; anything deeper is the child of that name's, if one.
                        let target = if at <= 1 {
                            Some(&mut *entry)
                        } else {
                            entry.children.iter_mut().find(|c| c.name == named)
                        };
                        if let Some(target) = target.filter(|t| t.doc.is_none()) {
                            target.doc = Some(words);
                        }
                    }
                }
                open.pop();
                if local == "element" && open.len() == 1 {
                    match current.take() {
                        Some(("property", e)) => schema.properties.push(e),
                        Some(("item", e)) => schema.items.push(e),
                        Some(("task", e)) => schema.tasks.push(e),
                        _ => {}
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    schema
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_schemas_give_properties_items_metadata_and_tasks_with_their_documentation() {
        let schema = schema();
        assert!(schema.properties.len() > 400, "only {} properties were read", schema.properties.len());
        let framework = schema.property("TargetFramework").unwrap();
        assert!(
            framework.doc.as_deref().is_some_and(|d| d.starts_with("Framework that this project targets")),
            "{framework:?}"
        );
        let package = schema.item("PackageReference").unwrap();
        assert_eq!(package.doc.as_deref(), Some("Reference to a package"));
        let assets = package.children.iter().find(|c| c.name == "IncludeAssets").unwrap();
        assert_eq!(assets.doc.as_deref(), Some("Assets to include from this reference"));
        let copy = schema.task("Copy").unwrap();
        assert!(
            copy.children.iter().any(|p| p.name == "DestinationFolder"),
            "the Copy task's parameters: {:?}",
            copy.children
        );
        assert!(schema.item("PackageVersion").is_some(), "central package management's item is missing");
    }
}
