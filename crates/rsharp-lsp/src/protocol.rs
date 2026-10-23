//! The answers a language server gives the editor, made plain: places as the protocol counts them (a line and a
//! UTF-16 column), which the app turns into offsets with its document's own conversion.

use rsharp_core::paths;
use serde_json::{Value, json};
use std::path::PathBuf;

/// A line and a UTF-16 column, from 0.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct Point {
    pub line: usize,
    pub character: usize,
}

impl Point {
    pub fn new(line: usize, character: usize) -> Self {
        Self { line, character }
    }

    pub fn to_json(self) -> Value {
        json!({ "line": self.line, "character": self.character })
    }

    fn from(value: &Value) -> Self {
        let n = |key: &str| value[key].as_u64().unwrap_or(0) as usize;
        Self { line: n("line"), character: n("character") }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Span {
    pub start: Point,
    pub end: Point,
}

impl Span {
    pub fn to_json(self) -> Value {
        json!({ "start": self.start.to_json(), "end": self.end.to_json() })
    }

    fn from(value: &Value) -> Self {
        Self { start: Point::from(&value["start"]), end: Point::from(&value["end"]) }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub span: Span,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TextEdit {
    pub span: Span,
    pub text: String,
}

/// What a rename or a code action changes: each file's edits, the files it makes and the files it moves, in order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WorkspaceEdit {
    pub files: Vec<(PathBuf, Vec<TextEdit>)>,
    pub created: Vec<PathBuf>,
    pub moves: Vec<(PathBuf, PathBuf)>,
}

impl WorkspaceEdit {
    pub fn is_empty(&self) -> bool {
        self.files.iter().all(|(_, e)| e.is_empty()) && self.created.is_empty() && self.moves.is_empty()
    }
}

#[derive(Clone, Debug)]
pub struct CompletionItem {
    pub label: String,
    /// What it is, as the list says beside it: its type or namespace.
    pub detail: String,
    /// The protocol's CompletionItemKind.
    pub kind: u64,
    pub filter_text: String,
    pub sort_text: String,
    /// What choosing it writes, and where; none writes the label at the word before the caret.
    pub edit: Option<TextEdit>,
    pub additional: Vec<TextEdit>,
    /// The item as the server sent it, its list's defaults merged in, for `completionItem/resolve`.
    pub raw: Value,
}

#[derive(Clone, Debug, Default)]
pub struct CompletionList {
    /// The server will answer differently as more is typed, so the list is asked again rather than filtered.
    pub incomplete: bool,
    pub items: Vec<CompletionItem>,
}

/// The signature the caret is in, and the byte range of its current parameter in `label`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signature {
    pub label: String,
    pub parameter: Option<std::ops::Range<usize>>,
    /// Which of how many overloads this is, from 0.
    pub overload: (usize, usize),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InlayHint {
    pub at: Point,
    pub label: String,
    pub pad_left: bool,
    pub pad_right: bool,
}

/// A run of a semantic token's kind, on one line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Token {
    pub at: Point,
    pub length: usize,
    pub kind: String,
}

#[derive(Clone, Debug)]
pub struct CodeAction {
    pub title: String,
    /// Fixes a problem, as Rider marks a quick fix apart from a context action.
    pub quick_fix: bool,
    pub raw: Value,
}

/// A type in a type hierarchy: its name, where it is declared (the whole and its name), and the item as the server
/// sent it, which its supertypes and subtypes are asked by.
#[derive(Clone, Debug)]
pub struct HierarchyItem {
    pub name: String,
    pub path: PathBuf,
    pub range: Span,
    pub selection: Span,
    pub raw: Value,
}

/// A prepareTypeHierarchy, supertypes or subtypes answer: the types, those outside a file left out.
pub fn hierarchy_items(answer: &Value) -> Vec<HierarchyItem> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|item| {
            Some(HierarchyItem {
                name: item["name"].as_str()?.to_string(),
                path: path_of(&item["uri"])?,
                range: Span::from(&item["range"]),
                selection: Span::from(&item["selectionRange"]),
                raw: item.clone(),
            })
        })
        .collect()
}

/// A document's symbols, each with its depth in the outline, as a hierarchical answer nests them (or a flat one lists
/// them, at depth 0): its name, its kind and where its name is.
pub fn outline(answer: &Value) -> Vec<(usize, String, u64, Span)> {
    fn walk(items: &Value, depth: usize, out: &mut Vec<(usize, String, u64, Span)>) {
        for item in items.as_array().into_iter().flatten() {
            let Some(name) = item["name"].as_str() else { continue };
            let at = item.get("selectionRange").or(item["location"].get("range")).map(Span::from).unwrap_or_default();
            out.push((depth, name.to_string(), item["kind"].as_u64().unwrap_or(0), at));
            walk(&item["children"], depth + 1, out);
        }
    }
    let mut out = Vec::new();
    walk(answer, 0, &mut out);
    out
}

/// A symbol kind as a person names it (the protocol's numbers).
pub fn kind_name(kind: u64) -> &'static str {
    match kind {
        1 => "file",
        2 => "module",
        3 => "namespace",
        4 => "package",
        5 => "class",
        6 => "method",
        7 => "property",
        8 => "field",
        9 => "constructor",
        10 => "enum",
        11 => "interface",
        12 => "function",
        13 => "variable",
        14 => "constant",
        22 => "enum member",
        23 => "struct",
        24 => "event",
        25 => "operator",
        26 => "type parameter",
        _ => "symbol",
    }
}

/// A problem the server found in a file: where, how bad (an error, else a warning) and what.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem {
    pub span: Span,
    pub error: bool,
    pub message: String,
}

/// A diagnostic report's errors and warnings, the rest (hints, suggestions) left out as Rider's Problems leaves them.
pub fn problems(items: &Value) -> Vec<Problem> {
    items
        .as_array()
        .into_iter()
        .flatten()
        .filter(|d| d["severity"].as_u64().is_some_and(|s| s <= 2))
        .map(|d| Problem {
            span: Span::from(&d["range"]),
            error: d["severity"] == 1,
            message: d["message"].as_str().unwrap_or_default().to_string(),
        })
        .collect()
}

/// A workspace diagnostic report: each file's problems, files with none left out.
pub fn workspace_problems(answer: &Value) -> Vec<(PathBuf, Vec<Problem>)> {
    answer["items"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|report| Some((path_of(&report["uri"])?, problems(&report["items"]))))
        .filter(|(_, found)| !found.is_empty())
        .collect()
}

/// A symbol `workspace/symbol` found: its name, what holds it, its kind and where it is.
#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: String,
    pub container: String,
    pub kind: u64,
    pub location: Location,
}

/// The project context chosen for each document, by its URI: the target framework of a multi-targeted project the
/// editor's switcher chose, which every request about the document names.
// shortcut: one map for the process, as a document's URI is one file whatever window shows it; per server if two
// servers ever hold one file.
static CONTEXTS: std::sync::LazyLock<std::sync::Mutex<std::collections::HashMap<String, Value>>> =
    std::sync::LazyLock::new(Default::default);

/// A request's document: its URI, and the project context chosen for it, if any.
pub fn document(uri: &str) -> Value {
    match CONTEXTS.lock().unwrap().get(uri) {
        Some(context) => json!({ "uri": uri, "_vs_projectContext": context }),
        None => json!({ "uri": uri }),
    }
}

/// The project context requests about a document name from now, or none (the server's default).
pub fn set_project_context(uri: &str, context: Option<Value>) {
    let mut contexts = CONTEXTS.lock().unwrap();
    match context {
        Some(context) => drop(contexts.insert(uri.to_string(), context)),
        None => drop(contexts.remove(uri)),
    }
}

/// What asks for a document's project contexts (textDocument/_vs_getProjectContexts).
pub fn contexts_request(uri: &str, version: u64) -> Value {
    json!({ "_vs_textDocument": { "uri": uri, "languageId": "csharp", "version": version, "text": "" } })
}

/// A document's project contexts (a target framework of a multi-targeted project each), with the one the server answers
/// for by default; Roslyn's Miscellaneous Files left out.
pub fn project_contexts(answer: &Value) -> (Vec<Value>, usize) {
    let all: Vec<&Value> = answer["_vs_projectContexts"].as_array().into_iter().flatten().collect();
    let default = answer["_vs_defaultIndex"].as_u64().and_then(|i| all.get(i as usize).copied());
    let kept: Vec<Value> =
        all.into_iter().filter(|c| !c["_vs_is_miscellaneous"].as_bool().unwrap_or(false)).cloned().collect();
    let at = default.and_then(|d| kept.iter().position(|c| c == d)).unwrap_or(0);
    (kept, at)
}

/// The `textDocument` and `position` every request about a place sends.
pub fn at(uri: &str, point: Point) -> Value {
    json!({ "textDocument": document(uri), "position": point.to_json() })
}

fn path_of(uri: &Value) -> Option<PathBuf> {
    paths::from_uri(uri.as_str()?)
}

/// A definition's, an implementation's or the references' answer: a location, a list of them, or links.
pub fn locations(answer: &Value) -> Vec<Location> {
    let one = |v: &Value| {
        if let Some(uri) = v.get("targetUri") {
            let span = v.get("targetSelectionRange").unwrap_or(&v["targetRange"]);
            return Some(Location { path: path_of(uri)?, span: Span::from(span) });
        }
        Some(Location { path: path_of(&v["uri"])?, span: Span::from(&v["range"]) })
    };
    match answer {
        Value::Array(items) => items.iter().filter_map(one).collect(),
        Value::Object(_) => one(answer).into_iter().collect(),
        _ => Vec::new(),
    }
}

pub fn text_edits(answer: &Value) -> Vec<TextEdit> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| TextEdit { span: Span::from(&e["range"]), text: e["newText"].as_str().unwrap_or_default().into() })
        .collect()
}

/// A workspace edit, in `changes` or `documentChanges` form.
pub fn workspace_edit(edit: &Value) -> WorkspaceEdit {
    let mut made = WorkspaceEdit::default();
    if let Some(changes) = edit["changes"].as_object() {
        for (uri, edits) in changes {
            if let Some(path) = paths::from_uri(uri) {
                made.files.push((path, text_edits(edits)));
            }
        }
    }
    for change in edit["documentChanges"].as_array().into_iter().flatten() {
        match change["kind"].as_str() {
            Some("rename") => {
                if let (Some(from), Some(to)) = (path_of(&change["oldUri"]), path_of(&change["newUri"])) {
                    made.moves.push((from, to));
                }
            }
            Some("create") => made.created.extend(path_of(&change["uri"])),
            Some(_) => {}
            None => {
                if let Some(path) = path_of(&change["textDocument"]["uri"]) {
                    made.files.push((path, text_edits(&change["edits"])));
                }
            }
        }
    }
    made
}

/// A hover's text, its markdown made plain: code fences dropped, escapes taken out, blank runs one line.
pub fn hover(answer: &Value) -> Option<String> {
    let contents = &answer["contents"];
    let text = match contents {
        Value::String(s) => s.clone(),
        Value::Array(parts) => {
            parts.iter().map(|p| p.as_str().or(p["value"].as_str()).unwrap_or_default()).collect::<Vec<_>>().join("\n")
        }
        _ => contents["value"].as_str()?.to_string(),
    };
    let plain = plain_markdown(&text);
    (!plain.is_empty()).then_some(plain)
}

/// Markdown as a person reads it in a plain popup.
pub fn plain_markdown(text: &str) -> String {
    let mut lines: Vec<String> = Vec::new();
    for line in text.lines().filter(|l| !l.trim_start().starts_with("```")) {
        let mut plain = String::with_capacity(line.len());
        let mut chars = line.chars();
        while let Some(c) = chars.next() {
            match c {
                '\\' => plain.extend(chars.next()),
                // Emphasis only; an underscore is as often part of a name.
                '*' | '`' => {}
                c => plain.push(c),
            }
        }
        let plain = plain.trim_end().replace("&nbsp;", " ");
        if !(plain.is_empty() && lines.last().is_none_or(String::is_empty)) {
            lines.push(plain);
        }
    }
    while lines.last().is_some_and(String::is_empty) {
        lines.pop();
    }
    lines.join("\n")
}

/// A completion answer: a list or bare items, each item's text and range from the list's defaults where it has
/// none of its own (Roslyn sends one edit range for them all).
pub fn completion(answer: &Value) -> CompletionList {
    let items = answer.as_array().or(answer["items"].as_array());
    let defaults = &answer["itemDefaults"];
    let default_span = |text: String| {
        let range = &defaults["editRange"];
        let range = range.get("insert").unwrap_or(range);
        range.is_object().then(|| TextEdit { span: Span::from(range), text })
    };
    let items = items
        .into_iter()
        .flatten()
        .map(|item| {
            let label = item["label"].as_str().unwrap_or_default().to_string();
            let edit = match item.get("textEdit") {
                Some(edit) => {
                    let range = edit.get("insert").or(edit.get("range")).unwrap_or(&Value::Null);
                    Some(TextEdit {
                        span: Span::from(range),
                        text: edit["newText"].as_str().unwrap_or_default().into(),
                    })
                }
                None => {
                    let text = item["textEditText"].as_str().or(item["insertText"].as_str()).unwrap_or(&label);
                    default_span(text.to_string())
                }
            };
            let mut raw = item.clone();
            if raw.get("data").is_none()
                && let Some(data) = defaults.get("data")
            {
                raw["data"] = data.clone();
            }
            let detail = item["labelDetails"]["description"].as_str().or(item["detail"].as_str()).unwrap_or_default();
            CompletionItem {
                filter_text: item["filterText"].as_str().unwrap_or(&label).to_string(),
                sort_text: item["sortText"].as_str().unwrap_or(&label).to_string(),
                detail: detail.to_string(),
                kind: item["kind"].as_u64().unwrap_or(0),
                edit,
                additional: text_edits(&item["additionalTextEdits"]),
                label,
                raw,
            }
        })
        .collect();
    CompletionList { incomplete: answer["isIncomplete"].as_bool().unwrap_or(false), items }
}

pub fn highlights(answer: &Value) -> Vec<Span> {
    answer.as_array().into_iter().flatten().map(|h| Span::from(&h["range"])).collect()
}

/// The active signature and its active parameter, whose label is a string in the signature's or its UTF-16
/// offsets there.
pub fn signature(answer: &Value) -> Option<Signature> {
    let signatures = answer["signatures"].as_array()?;
    let active = answer["activeSignature"].as_u64().unwrap_or(0) as usize;
    let chosen = signatures.get(active).or(signatures.first())?;
    let label = chosen["label"].as_str()?.to_string();
    let index = chosen["activeParameter"].as_u64().or(answer["activeParameter"].as_u64()).unwrap_or(0) as usize;
    let parameter = chosen["parameters"].as_array().and_then(|p| p.get(index)).and_then(|p| match &p["label"] {
        // A name is found among the parameters, past the method's own name, which may hold it too.
        Value::String(name) => {
            let open = label.find('(').unwrap_or(0);
            label[open..].find(name.as_str()).map(|at| open + at..open + at + name.len())
        }
        Value::Array(range) => {
            let byte = |utf16: u64| {
                let mut units = 0;
                label.char_indices().find(|(_, c)| {
                    let here = units;
                    units += c.len_utf16() as u64;
                    here >= utf16
                })
            };
            let start = byte(range.first()?.as_u64()?).map_or(label.len(), |(at, _)| at);
            let end = byte(range.get(1)?.as_u64()?).map_or(label.len(), |(at, _)| at);
            Some(start..end)
        }
        _ => None,
    });
    Some(Signature { label, parameter, overload: (active.min(signatures.len() - 1), signatures.len()) })
}

pub fn inlay_hints(answer: &Value) -> Vec<InlayHint> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .map(|h| InlayHint {
            at: Point::from(&h["position"]),
            label: match &h["label"] {
                Value::Array(parts) => parts.iter().filter_map(|p| p["value"].as_str()).collect(),
                label => label.as_str().unwrap_or_default().to_string(),
            },
            pad_left: h["paddingLeft"].as_bool().unwrap_or(false),
            pad_right: h["paddingRight"].as_bool().unwrap_or(false),
        })
        .collect()
}

/// Semantic tokens in the protocol's relative form, each named by the server's legend.
pub fn semantic_tokens(answer: &Value, legend: &[String]) -> Vec<Token> {
    let data: Vec<usize> =
        answer["data"].as_array().into_iter().flatten().filter_map(|n| n.as_u64()).map(|n| n as usize).collect();
    let mut tokens = Vec::with_capacity(data.len() / 5);
    let (mut line, mut start) = (0, 0);
    for chunk in data.chunks_exact(5) {
        if chunk[0] > 0 {
            line += chunk[0];
            start = chunk[1];
        } else {
            start += chunk[1];
        }
        if let Some(kind) = legend.get(chunk[3]) {
            tokens.push(Token { at: Point::new(line, start), length: chunk[2], kind: kind.clone() });
        }
    }
    tokens
}

/// The code actions offered, commands left out: Roslyn's come as actions to resolve.
pub fn code_actions(answer: &Value) -> Vec<CodeAction> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter(|a| a.get("command").is_none_or(|c| c.is_object()))
        .filter_map(|a| {
            Some(CodeAction {
                title: a["title"].as_str()?.to_string(),
                quick_fix: a["kind"].as_str().is_some_and(|k| k.starts_with("quickfix")),
                raw: a.clone(),
            })
        })
        .collect()
}

pub fn symbols(answer: &Value) -> Vec<Symbol> {
    answer
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let location = &s["location"];
            Some(Symbol {
                name: s["name"].as_str()?.to_string(),
                container: s["containerName"].as_str().unwrap_or_default().to_string(),
                kind: s["kind"].as_u64().unwrap_or(0),
                location: Location {
                    path: path_of(&location["uri"])?,
                    span: location.get("range").map(Span::from).unwrap_or_default(),
                },
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_completion_item_without_an_edit_of_its_own_writes_its_text_over_the_lists_range() {
        let answer = json!({
            "isIncomplete": true,
            "itemDefaults": { "editRange": { "start": { "line": 1, "character": 4 }, "end": { "line": 1, "character": 6 } }, "data": { "id": 7 } },
            "items": [{ "label": "WriteLine", "kind": 2, "sortText": "a" }],
        });
        let list = completion(&answer);
        assert!(list.incomplete);
        let edit = list.items[0].edit.clone().expect("the item writes over the list's range");
        assert_eq!(
            (edit.span.start, edit.span.end, edit.text.as_str()),
            (Point::new(1, 4), Point::new(1, 6), "WriteLine")
        );
        assert_eq!(list.items[0].raw["data"]["id"], 7, "resolve is sent the list's data");
    }

    #[test]
    fn semantic_tokens_are_read_relative_to_the_one_before() {
        let legend = ["class".to_string(), "method".to_string()];
        let tokens = semantic_tokens(&json!({ "data": [1, 4, 3, 0, 0, 0, 6, 5, 1, 0, 2, 2, 1, 0, 0] }), &legend);
        let at: Vec<_> = tokens.iter().map(|t| (t.at.line, t.at.character, t.length, t.kind.as_str())).collect();
        assert_eq!(at, [(1, 4, 3, "class"), (1, 10, 5, "method"), (3, 2, 1, "class")]);
    }

    #[test]
    fn a_parameter_named_by_its_utf16_offsets_is_its_bytes_in_the_label() {
        let answer = json!({ "signatures": [{ "label": "void Ŝay(int ā, int b)", "parameters": [{ "label": [9, 14] }, { "label": [16, 21] }] }], "activeParameter": 1 });
        let signature = signature(&answer).unwrap();
        assert_eq!(&signature.label[signature.parameter.unwrap()], "int b");
    }

    #[test]
    fn a_hover_reads_as_plain_text() {
        let answer = json!({ "contents": { "kind": "markdown", "value": "```csharp\nvoid Program.Say(int count)\n```\n  \n\nSays it **count** times\\." } });
        assert_eq!(hover(&answer).unwrap(), "void Program.Say(int count)\n\nSays it count times.");
    }
}
