//! Razor, cohosted in Roslyn's server, as Passenger's client answers it (`LanguageServer.cs`' Razor handlers and
//! `RazorHtml.cs`): the legend of a Razor file's colours, which Razor registers as it starts, and Razor's questions
//! about a file's HTML. Razor gives the client each open file's HTML (`razor/updateHtml`: the file with its C# blanked
//! out, so places are the file's own) and forwards HTML's part of a request to it, wrapped with the file. With the HTML
//! server running, each file's HTML is a document of its own there, named as VS Code names it (`<uri>__virtual.html`),
//! and the question goes to it; without one (no Node.js), completion answers HTML's element names after a tag's `<`
//! and its global attributes inside a start tag, as Passenger does, a list empty and the rest null, so Razor's own
//! answers (its components, directives and C#) stand on their own.

use crate::LanguageServer;
use serde_json::{Value, json};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const VIRTUAL: &str = "__virtual.html";

/// Whether Razor answers for a file in Roslyn's server: a component or a page.
pub fn is_razor(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| e.eq_ignore_ascii_case("razor") || e.eq_ignore_ascii_case("cshtml"))
}

#[derive(Default)]
pub struct Razor {
    legend: Mutex<Option<Vec<String>>>,
    html: Mutex<Option<Arc<LanguageServer>>>,
    /// Each file's HTML as Razor last gave it, and the version the HTML server has of it, by the file's URI.
    texts: Mutex<HashMap<String, (String, Option<i64>)>>,
}

/// The requests Razor forwards to the client for a file's HTML, and whether their answer is a list.
const FORWARDED: &[(&str, bool)] = &[
    ("textDocument/completion", false),
    ("completionItem/resolve", false),
    ("codeAction/resolve", false),
    ("textDocument/formatting", true),
    ("textDocument/rangeFormatting", true),
    ("textDocument/onTypeFormatting", true),
    ("textDocument/codeAction", true),
    ("textDocument/documentHighlight", true),
    ("textDocument/references", true),
    ("textDocument/foldingRange", true),
    ("textDocument/documentColor", true),
    ("textDocument/colorPresentation", true),
    ("textDocument/hover", false),
    ("textDocument/signatureHelp", false),
    ("textDocument/definition", false),
    ("textDocument/implementation", false),
    ("textDocument/prepareRename", false),
    ("textDocument/rename", false),
    ("textDocument/linkedEditingRange", false),
    ("textDocument/_vs_onAutoInsert", false),
];

impl Razor {
    /// The token types of a Razor file's colours: C#'s, then markup's and Razor's own; empty until Razor registered them.
    pub fn legend(&self) -> Vec<String> {
        self.legend.lock().unwrap().clone().unwrap_or_default()
    }

    /// Where Razor's HTML questions go from now: the folder's HTML server, or none.
    pub fn set_html(&self, server: Option<Arc<LanguageServer>>) {
        *self.html.lock().unwrap() = server;
        // A server started again is sent each file afresh.
        self.texts.lock().unwrap().values_mut().for_each(|(_, version)| *version = None);
    }

    /// A Razor file closed: Razor told, so it lets go of its HTML, and the HTML server's copy closed.
    pub fn closed(&self, roslyn: &LanguageServer, uri: &str) {
        drop(roslyn.request("razor/documentClosed", json!({ "uri": uri })));
        // Razor spells a file's URI its own way: found by its path.
        let path = rsharp_core::paths::from_uri(uri);
        let mut texts = self.texts.lock().unwrap();
        let Some(known) = texts.keys().find(|k| rsharp_core::paths::from_uri(k) == path).cloned() else { return };
        if let Some((_, Some(_))) = texts.remove(&known)
            && let Some(html) = self.html.lock().unwrap().as_ref()
        {
            html.notify("textDocument/didClose", json!({ "textDocument": { "uri": format!("{known}{VIRTUAL}") } }));
        }
    }

    /// The work that answers one of Razor's requests to the client, run on a thread of its own; none for the rest.
    pub(crate) fn answer(self: &Arc<Self>, method: &str, params: &Value) -> Option<Box<dyn FnOnce() -> Value + Send>> {
        let razor = self.clone();
        let params = params.clone();
        if method == "client/registerCapability" {
            razor.registered(&params);
            return Some(Box::new(|| Value::Null));
        }
        if method == "razor/updateHtml" {
            return Some(Box::new(move || {
                razor.update(&params);
                Value::Null
            }));
        }
        let &(method, list) = FORWARDED.iter().find(|(m, _)| *m == method)?;
        Some(Box::new(move || {
            match razor.forward(method, &params) {
                Some(answer) if !list || answer.is_array() => answer,
                _ if list => json!([]),
                _ if method == "textDocument/completion" => razor.markup_names(&params),
                // A resolve answers the item it was given: HTML has nothing to add.
                _ if method.ends_with("/resolve") => params["request"].clone(),
                _ => Value::Null,
            }
        }))
    }

    /// Razor's registrations for its own files: the legend of their colours is the one kept.
    fn registered(&self, params: &Value) {
        for registration in params["registrations"].as_array().into_iter().flatten() {
            let options = &registration["registerOptions"];
            let for_razor = options["documentSelector"]
                .as_array()
                .is_some_and(|s| s.iter().any(|f| f["language"] == "aspnetcorerazor"));
            if for_razor && registration["method"] == "textDocument/semanticTokens" {
                let types = options["legend"]["tokenTypes"].as_array().into_iter().flatten();
                *self.legend.lock().unwrap() = Some(types.filter_map(|t| t.as_str().map(str::to_string)).collect());
            }
        }
    }

    fn update(&self, params: &Value) {
        let (Some(uri), Some(text)) = (uri_of(params), params["text"].as_str()) else { return };
        let mut texts = self.texts.lock().unwrap();
        let version = texts.get(&uri).and_then(|(_, v)| *v);
        texts.insert(uri.clone(), (text.to_string(), version));
        drop(texts);
        self.sync(&uri);
    }

    /// The HTML server's copy of a file's HTML brought to what Razor last gave: opened there the first time, its
    /// whole text after.
    fn sync(&self, uri: &str) {
        let Some(html) = self.html.lock().unwrap().clone() else { return };
        let mut texts = self.texts.lock().unwrap();
        let Some((text, version)) = texts.get_mut(uri) else { return };
        let document = format!("{uri}{VIRTUAL}");
        match version {
            Some(v) => {
                *v += 1;
                html.notify(
                    "textDocument/didChange",
                    json!({ "textDocument": { "uri": document, "version": *v }, "contentChanges": [{ "text": text }] }),
                );
            }
            None => {
                *version = Some(1);
                html.notify(
                    "textDocument/didOpen",
                    json!({ "textDocument": { "uri": document, "languageId": "html", "version": 1, "text": text } }),
                );
            }
        }
    }

    /// A question Razor forwarded, asked of the HTML server about the file's HTML, its answer naming the file again;
    /// none without the HTML server or its answer.
    fn forward(&self, method: &str, params: &Value) -> Option<Value> {
        let html = self.html.lock().unwrap().clone()?;
        let mut request = params.get("request")?.clone();
        let uri = uri_of(params);
        if let Some(uri) = &uri {
            if self.texts.lock().unwrap().get(uri).is_some_and(|(_, v)| v.is_none()) {
                self.sync(uri);
            }
            if request["textDocument"].is_object() {
                request["textDocument"]["uri"] = json!(format!("{uri}{VIRTUAL}"));
            }
        }
        let answer = html.request(method, request).recv_timeout(Duration::from_secs(10)).ok()?.ok()?;
        match &uri {
            Some(uri) if !answer.is_null() => {
                let named = serde_json::to_string(&format!("{uri}{VIRTUAL}")).ok()?;
                let file = serde_json::to_string(uri).ok()?;
                serde_json::from_str(&answer.to_string().replace(&named, &file)).ok()
            }
            _ => Some(answer),
        }
    }
}

/// The HTML living standard's elements, less the obsolete (Passenger's list).
const ELEMENTS: &[&str] = &[
    "a",
    "abbr",
    "address",
    "area",
    "article",
    "aside",
    "audio",
    "b",
    "base",
    "bdi",
    "bdo",
    "blockquote",
    "body",
    "br",
    "button",
    "canvas",
    "caption",
    "cite",
    "code",
    "col",
    "colgroup",
    "data",
    "datalist",
    "dd",
    "del",
    "details",
    "dfn",
    "dialog",
    "div",
    "dl",
    "dt",
    "em",
    "embed",
    "fieldset",
    "figcaption",
    "figure",
    "footer",
    "form",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "head",
    "header",
    "hgroup",
    "hr",
    "html",
    "i",
    "iframe",
    "img",
    "input",
    "ins",
    "kbd",
    "label",
    "legend",
    "li",
    "link",
    "main",
    "map",
    "mark",
    "menu",
    "meta",
    "meter",
    "nav",
    "noscript",
    "object",
    "ol",
    "optgroup",
    "option",
    "output",
    "p",
    "picture",
    "pre",
    "progress",
    "q",
    "rp",
    "rt",
    "ruby",
    "s",
    "samp",
    "script",
    "search",
    "section",
    "select",
    "slot",
    "small",
    "source",
    "span",
    "strong",
    "style",
    "sub",
    "summary",
    "sup",
    "table",
    "tbody",
    "td",
    "template",
    "textarea",
    "tfoot",
    "th",
    "thead",
    "time",
    "title",
    "tr",
    "track",
    "u",
    "ul",
    "var",
    "video",
    "wbr",
];

/// The attributes every HTML element takes.
const GLOBAL_ATTRIBUTES: &[&str] = &[
    "accesskey",
    "autocapitalize",
    "autofocus",
    "class",
    "contenteditable",
    "dir",
    "draggable",
    "enterkeyhint",
    "hidden",
    "id",
    "inert",
    "inputmode",
    "lang",
    "nonce",
    "popover",
    "role",
    "slot",
    "spellcheck",
    "style",
    "tabindex",
    "title",
    "translate",
];

impl Razor {
    /// HTML's part of a Razor file's completion with no HTML server: the elements where a tag's name is typed, the
    /// global attributes inside a start tag, else none.
    fn markup_names(&self, params: &Value) -> Value {
        let nothing = json!({ "isIncomplete": false, "items": [] });
        let Some(uri) = uri_of(params) else { return nothing };
        let Some(html) = self.texts.lock().unwrap().get(&uri).map(|(text, _)| text.clone()) else { return nothing };
        let position = &params["request"]["position"];
        let (Some(line), Some(column)) = (position["line"].as_u64(), position["character"].as_u64()) else {
            return nothing;
        };
        let Some(at) = offset(&html, line as usize, column as usize) else { return nothing };
        let bytes = html.as_bytes();
        let mut start = at;
        while start > 0 && (bytes[start - 1].is_ascii_alphanumeric() || matches!(bytes[start - 1], b'-' | b':')) {
            start -= 1;
        }
        let names: &[&str] = match start.checked_sub(1).map(|i| bytes[i]) {
            Some(b'<') => ELEMENTS,
            Some(c) if c.is_ascii_whitespace() && in_start_tag(bytes, start) => GLOBAL_ATTRIBUTES,
            _ => return nothing,
        };
        let items: Vec<Value> = names.iter().map(|n| json!({ "label": n, "kind": 10, "insertText": n })).collect();
        json!({ "isIncomplete": false, "items": items })
    }
}

/// Whether a place is inside a start tag, past its name: a `<` and a letter before it, with no `>` between, outside a
/// quoted value.
fn in_start_tag(html: &[u8], at: usize) -> bool {
    let mut quote = None;
    for i in (0..at).rev() {
        let c = html[i];
        match quote {
            Some(open) if c == open => quote = None,
            Some(_) => {}
            None if c == b'"' || c == b'\'' => quote = Some(c),
            None if c == b'>' => return false,
            None if c == b'<' => return i + 1 < at && html[i + 1].is_ascii_alphabetic(),
            None => {}
        }
    }
    false
}

/// The byte offset of a line and a UTF-16 column in `text`, its lines ended by `\n`.
fn offset(text: &str, line: usize, column: usize) -> Option<usize> {
    let start = if line == 0 { 0 } else { text.match_indices('\n').nth(line - 1)?.0 + 1 };
    let mut units = 0;
    for (i, c) in text[start..].char_indices() {
        if units >= column || c == '\n' {
            return Some(start + i);
        }
        units += c.len_utf16();
    }
    Some(text.len())
}

/// The Razor file a message is about: VS Code's Razor names it `textDocument`, or `uri` at the top.
fn uri_of(params: &Value) -> Option<String> {
    params["textDocument"]["uri"].as_str().or(params["uri"].as_str()).map(str::to_string)
}
