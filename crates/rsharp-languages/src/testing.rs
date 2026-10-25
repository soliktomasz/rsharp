//! Test declarations read from the editor's existing C# syntax tree.
// shortcut: custom attribute aliases and conditional-compilation symbols need semantic evaluation; source discovery
// shows syntactic candidates until a runner discovers the built tests, upgrade if MSBuild's symbols are needed here.

use rsharp_text::Rope;
use std::collections::BTreeSet;
use tree_sitter::{Node, Tree};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FileTests {
    pub tests: Vec<DeclaredTest>,
    pub methods: BTreeSet<String>,
    pub types: Vec<(String, usize)>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeclaredTest {
    pub type_name: String,
    pub method: String,
    pub line: usize,
    pub abstract_type: bool,
    pub traits: Vec<(String, String)>,
}

/// Declarations only: method bodies never enter the discovery walk. Lines start at one.
pub fn read(tree: &Tree, text: &Rope) -> FileTests {
    let mut found = FileTests::default();
    declarations(tree.root_node(), text, "", "", false, &mut found);
    found
}

const TEST_ATTRIBUTES: &[&str] = &[
    "Fact",
    "Theory",
    "AvaloniaFact",
    "AvaloniaTheory",
    "Test",
    "TestCase",
    "TestCaseSource",
    "TestMethod",
    "DataTestMethod",
];

fn is_type(node: Node) -> bool {
    matches!(node.kind(), "class_declaration" | "record_declaration" | "struct_declaration" | "interface_declaration")
}

fn value(node: Node, text: &Rope) -> String {
    text.byte_slice(node.byte_range()).to_string()
}

fn name(node: Node, text: &Rope) -> Option<(String, usize)> {
    let identifier = node.child_by_field_name("name")?;
    Some((value(identifier, text).trim_start_matches('@').to_string(), text.byte_to_line(identifier.start_byte()) + 1))
}

fn declarations(node: Node, text: &Rope, namespace: &str, owner: &str, generic: bool, found: &mut FileTests) {
    let mut namespace = namespace.to_string();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "file_scoped_namespace_declaration" => {
                if let Some((name, _)) = name(child, text) {
                    namespace = qualified(&namespace, &name, ".");
                }
            }
            "namespace_declaration" => {
                if let Some((name, _)) = name(child, text)
                    && let Some(body) = child.child_by_field_name("body")
                {
                    declarations(body, text, &qualified(&namespace, &name, "."), owner, generic, found);
                }
            }
            _ if is_type(child) => {
                let Some((short, line)) = name(child, text) else { continue };
                let full =
                    if owner.is_empty() { qualified(&namespace, &short, ".") } else { qualified(owner, &short, "+") };
                let mut modifiers = child.walk();
                let mut generic = generic;
                let mut abstract_type = false;
                for part in child.named_children(&mut modifiers) {
                    generic |= part.kind() == "type_parameter_list";
                    abstract_type |= part.kind() == "modifier" && value(part, text) == "abstract";
                }
                let Some(body) = child.child_by_field_name("body") else { continue };
                let before = found.tests.len();
                let type_traits = traits(child, text);
                for method in methods(body) {
                    let Some((method_name, line)) = name(method, text) else { continue };
                    found.methods.insert(method_name.clone());
                    if attributes(method).iter().any(|a| TEST_ATTRIBUTES.contains(&attribute_name(*a, text).as_str())) {
                        let mut traits = type_traits.clone();
                        traits.extend(self::traits(method, text));
                        found.tests.push(DeclaredTest {
                            type_name: full.clone(),
                            method: method_name,
                            line,
                            abstract_type: generic || abstract_type,
                            traits,
                        });
                    }
                }
                if found.tests.len() > before {
                    found.types.push((full.clone(), line));
                }
                declarations(body, text, &namespace, &full, generic, found);
            }
            "preproc_if" | "preproc_elif" | "preproc_else" => {
                declarations(child, text, &namespace, owner, generic, found);
            }
            _ => {}
        }
    }
}

fn methods(node: Node) -> Vec<Node> {
    let mut found = Vec::new();
    let mut cursor = node.walk();
    for child in node.named_children(&mut cursor) {
        match child.kind() {
            "method_declaration" => found.push(child),
            "preproc_if" | "preproc_elif" | "preproc_else" => found.extend(methods(child)),
            _ => {}
        }
    }
    found
}

fn qualified(prefix: &str, name: &str, separator: &str) -> String {
    if prefix.is_empty() { name.into() } else { format!("{prefix}{separator}{name}") }
}

fn attributes(node: Node) -> Vec<Node> {
    let mut attributes = Vec::new();
    let mut cursor = node.walk();
    for list in node.named_children(&mut cursor).filter(|n| n.kind() == "attribute_list") {
        let mut entries = list.walk();
        attributes.extend(list.named_children(&mut entries).filter(|n| n.kind() == "attribute"));
    }
    attributes
}

fn attribute_name(node: Node, text: &Rope) -> String {
    let Some(name) = node.child_by_field_name("name") else { return String::new() };
    let name = value(name, text);
    let name = name.rsplit(['.', ':']).next().unwrap_or(&name).trim().trim_start_matches('@');
    name.strip_suffix("Attribute").unwrap_or(name).into()
}

fn traits(node: Node, text: &Rope) -> Vec<(String, String)> {
    let mut traits = Vec::new();
    for attribute in attributes(node) {
        let name = attribute_name(attribute, text);
        if !matches!(name.as_str(), "Trait" | "Category" | "TestCategory") {
            continue;
        }
        let mut cursor = attribute.walk();
        let Some(arguments) = attribute.named_children(&mut cursor).find(|n| n.kind() == "attribute_argument_list")
        else {
            continue;
        };
        let mut args = arguments.walk();
        let values: Vec<String> = arguments
            .named_children(&mut args)
            .filter(|n| n.kind() == "attribute_argument" && n.child_by_field_name("name").is_none())
            .filter_map(|n| {
                let mut children = n.walk();
                let expression = n.named_children(&mut children).next()?;
                matches!(expression.kind(), "string_literal" | "verbatim_string_literal" | "raw_string_literal")
                    .then(|| string_literal(&value(expression, text)))
                    .flatten()
            })
            .collect();
        match (name.as_str(), values.as_slice()) {
            ("Trait", [key, value]) => traits.push((key.clone(), value.clone())),
            ("Category" | "TestCategory", [value]) => traits.push((name, value.clone())),
            _ => {}
        }
    }
    traits
}

fn string_literal(literal: &str) -> Option<String> {
    if let Some(verbatim) = literal.strip_prefix("@\"").and_then(|s| s.strip_suffix('"')) {
        return Some(verbatim.replace("\"\"", "\""));
    }
    if literal.starts_with("\"\"\"") {
        let quotes = literal.bytes().take_while(|b| *b == b'"').count();
        let content = literal.get(quotes..literal.len().checked_sub(quotes)?)?;
        let Some(multiline) = content.strip_prefix("\r\n").or_else(|| content.strip_prefix('\n')) else {
            return Some(content.to_string());
        };
        let (body, indent) = multiline.rsplit_once('\n')?;
        let body = body.strip_suffix('\r').unwrap_or(body);
        return body
            .split_inclusive('\n')
            .map(|line| {
                line.strip_prefix(indent)
                    .or_else(|| line.trim().is_empty().then_some(line.trim_start_matches([' ', '\t'])))
            })
            .collect::<Option<String>>();
    }
    let mut chars = literal.strip_prefix('"')?.strip_suffix('"')?.chars();
    let mut result = String::new();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            result.push(ch);
            continue;
        }
        let escaped = chars.next()?;
        result.push(match escaped {
            '\'' => '\'',
            '"' => '"',
            '\\' => '\\',
            '0' => '\0',
            'a' => '\u{7}',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'v' => '\u{b}',
            'u' | 'U' | 'x' => {
                let digits = if escaped == 'U' { 8 } else { 4 };
                let mut hex = String::new();
                for _ in 0..digits {
                    match chars.clone().next().filter(|c| c.is_ascii_hexdigit()) {
                        Some(c) => {
                            chars.next();
                            hex.push(c);
                        }
                        None if escaped == 'x' => break,
                        None => return None,
                    }
                }
                char::from_u32(u32::from_str_radix(&hex, 16).ok()?)?
            }
            _ => return None,
        });
    }
    Some(result)
}

/// The type and optional method enclosing a one-based caret line.
pub fn at(tree: &Tree, text: &Rope, line: usize) -> Option<(String, Option<String>)> {
    let line = line.checked_sub(1).filter(|l| *l < text.len_lines())?;
    let content = text.line(line);
    let indent: usize = content.chars().take_while(|c| matches!(c, ' ' | '\t')).map(char::len_utf8).sum();
    let byte = (text.line_to_byte(line) + indent).min(text.len_bytes());
    let mut node = Some(tree.root_node().descendant_for_byte_range(byte, byte)?);
    let mut method = None;
    let mut types = Vec::new();
    let mut namespaces = Vec::new();
    while let Some(current) = node {
        if current.kind() == "method_declaration" {
            method = name(current, text).map(|(name, _)| name);
        } else if is_type(current) {
            types.push(name(current, text)?.0);
        } else if current.kind() == "namespace_declaration" {
            namespaces.push(name(current, text)?.0);
        }
        node = current.parent();
    }
    if types.is_empty() {
        return None;
    }
    let mut cursor = tree.root_node().walk();
    if let Some(namespace) = tree
        .root_node()
        .named_children(&mut cursor)
        .find(|n| n.kind() == "file_scoped_namespace_declaration" && n.start_byte() <= byte)
    {
        namespaces.push(name(namespace, text)?.0);
    }
    types.reverse();
    namespaces.reverse();
    Some((qualified(&namespaces.join("."), &types.join("+"), "."), method))
}
