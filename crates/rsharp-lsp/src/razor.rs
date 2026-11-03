//! Razor, cohosted in Roslyn's server, as Passenger's client answers it (`LanguageServer.cs`' Razor handlers and
//! `RazorHtml.cs`): the legend of a Razor file's colours, which Razor registers as it starts, and Razor's questions
//! about a file's HTML. Razor gives the client each open file's HTML (`razor/updateHtml`: the file with its C# blanked
//! out, so places are the file's own) and forwards HTML's part of a request to it, wrapped with the file. With the HTML
//! server running, each file's HTML is a document of its own there, named as VS Code names it (`<uri>__virtual.html`),
//! and the question goes to it; without one, a list answers empty and the rest null, so Razor's own answers (its
//! components, directives and C#) stand on their own.
// shortcut: no HTML element names or attributes without the HTML server (Passenger lists them itself); add them when a
// Razor file should complete markup on a machine without Node.js.

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
                _ if method == "textDocument/completion" => json!({ "isIncomplete": false, "items": [] }),
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

/// The Razor file a message is about: VS Code's Razor names it `textDocument`, or `uri` at the top.
fn uri_of(params: &Value) -> Option<String> {
    params["textDocument"]["uri"].as_str().or(params["uri"].as_str()).map(str::to_string)
}
