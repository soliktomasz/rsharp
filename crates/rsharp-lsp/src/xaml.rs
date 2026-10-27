//! RSharp's XAML host (`xaml-host/`, Passenger.Xaml in a process of its own): what an Avalonia or WPF XAML file
//! completes, warns of and where a name in it leads, asked with the file's text as typed and the project it belongs to.

use crate::host::Connection;
use crate::{Location, Point, Problem, ServerFailed, Span, TextEdit};
use futures::channel::mpsc::{UnboundedReceiver, UnboundedSender, unbounded};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

/// Whether a file is XAML Passenger.Xaml may read: Avalonia's .axaml (and its old .paml), or WPF's .xaml.
pub fn is_xaml(path: &Path) -> bool {
    path.extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| ["axaml", "paml", "xaml"].iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// One thing completion offers: what the list shows and what goes in, the byte in it the caret stays at (a snippet's
/// `$0`), and whether completion opens again once it is in (a property's values inside its new quotes).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XamlItem {
    pub label: String,
    pub kind: String,
    pub insert: String,
    pub caret: Option<usize>,
    pub reopen: bool,
    pub detail: String,
}

/// A completion list: where the text it replaces starts (it ends at the caret) and its items.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XamlCompletion {
    pub start: Point,
    pub items: Vec<XamlItem>,
    pub word_characters: String,
}

/// The name at a place: where it is written, the card that says what it is, and where it leads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XamlSymbol {
    pub span: Span,
    pub description: String,
    pub targets: Vec<Location>,
    pub named: Option<XamlNamed>,
}

/// An element's name: the name, where its file names it, and the C# files of its class, where the field Avalonia's
/// generator or WPF's MarkupCompile makes for it is used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct XamlNamed {
    pub name: String,
    pub uses: Vec<Span>,
    pub files: Vec<PathBuf>,
}

/// The folder's XAML host, started on first use and again after it fails.
#[derive(Clone)]
pub struct XamlHost {
    running: Arc<Mutex<Option<Arc<Connection>>>>,
    changes: Arc<Mutex<Vec<UnboundedSender<Option<PathBuf>>>>>,
    dotnet: PathBuf,
    host: PathBuf,
}

impl XamlHost {
    /// None without the host RSharp's build publishes, or without dotnet.
    pub fn carried() -> Option<Self> {
        let host = crate::roslyn::carried_xaml_host()?;
        Some(Self { running: Arc::default(), changes: Arc::default(), dotnet: rsharp_core::programs::dotnet()?, host })
    }

    /// A completed project update; the file that published it need not check itself again.
    pub fn subscribe(&self) -> UnboundedReceiver<Option<PathBuf>> {
        let (send, receive) = unbounded();
        self.changes.lock().unwrap().push(send);
        receive
    }

    fn changed_project(&self, except: Option<&Path>) {
        self.changes.lock().unwrap().retain(|send| send.unbounded_send(except.map(Path::to_path_buf)).is_ok());
    }

    fn ask(&self, method: &str, params: Value) -> Result<Value, ServerFailed> {
        let connection = {
            let mut running = self.running.lock().unwrap();
            if running.as_ref().is_some_and(|c| c.failed().is_some()) {
                *running = None;
            }
            match running.as_ref() {
                Some(connection) => connection.clone(),
                None => {
                    let mut command = Command::new(&self.dotnet);
                    command.arg(&self.host).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
                    running.insert(Connection::start(command, "the XAML host", None)?).clone()
                }
            }
        };
        connection.ask(method, params, |_| {}).map_err(ServerFailed)
    }

    /// The file's problems as typed; None for a project whose XAML is neither Avalonia's nor WPF's.
    /// Blocks for the answer, so call it off the UI thread.
    pub fn check(&self, project: &Path, path: &Path, text: &str) -> Result<Option<Vec<Problem>>, ServerFailed> {
        let found = self.ask("check", json!({ "project": project, "path": path, "text": text }))?;
        if found["changed"] == true {
            self.changed_project(Some(path));
        }
        if found["dialect"].is_null() {
            return Ok(None);
        }
        let problems = found["diagnostics"].as_array().into_iter().flatten();
        Ok(Some(
            problems
                .map(|d| Problem {
                    span: Span::from(&d["range"]),
                    error: d["error"] == true,
                    message: d["message"].as_str().unwrap_or_default().to_string(),
                })
                .collect(),
        ))
    }

    /// What may go at `at`. Blocks for the answer, so call it off the UI thread.
    pub fn complete(
        &self,
        project: &Path,
        path: &Path,
        text: &str,
        at: Point,
    ) -> Result<Option<XamlCompletion>, ServerFailed> {
        let found = self.ask("complete", params(project, path, text, at))?;
        if found.is_null() {
            return Ok(None);
        }
        let items = found["items"].as_array().into_iter().flatten();
        Ok(Some(XamlCompletion {
            start: Point::from(&found["start"]),
            items: items.map(item).collect(),
            word_characters: found["wordCharacters"].as_str().unwrap_or_default().to_string(),
        }))
    }

    /// The name at `at`. Blocks for the answer, so call it off the UI thread.
    pub fn symbol(
        &self,
        project: &Path,
        path: &Path,
        text: &str,
        at: Point,
    ) -> Result<Option<XamlSymbol>, ServerFailed> {
        let found = self.ask("symbol", params(project, path, text, at))?;
        if found.is_null() {
            return Ok(None);
        }
        let targets = found["targets"].as_array().into_iter().flatten();
        Ok(Some(XamlSymbol {
            span: Span::from(&found["range"]),
            description: found["description"].as_str().unwrap_or_default().to_string(),
            targets: targets
                .filter_map(|t| {
                    let at = Point::from(&t["at"]);
                    Some(Location { path: PathBuf::from(t["path"].as_str()?), span: Span { start: at, end: at } })
                })
                .collect(),
            named: found.get("named").filter(|n| n.is_object()).map(|n| XamlNamed {
                name: n["name"].as_str().unwrap_or_default().to_string(),
                uses: n["uses"].as_array().into_iter().flatten().map(Span::from).collect(),
                files: n["files"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|f| f.as_str())
                    .map(PathBuf::from)
                    .collect(),
            }),
        }))
    }

    /// The fixes of the problems on `line`, as LSP's code actions with their edits in them. Blocks, so call it off the
    /// UI thread.
    /// `texts` are the open files' unsaved texts, which a fix's edits to them are made from.
    pub fn fixes(
        &self,
        project: &Path,
        path: &Path,
        text: &str,
        line: usize,
        texts: &std::collections::HashMap<PathBuf, String>,
    ) -> Result<Value, ServerFailed> {
        self.ask("fixes", json!({ "project": project, "path": path, "text": text, "line": line, "texts": texts }))
    }

    /// Reformat Code: the edits that lay `text` out by Rider's XAML code style with `editorconfig`'s properties over it,
    /// only those in `range`'s lines when given; or why it was left as it is. Blocks, so call it off the UI thread.
    pub fn format(
        &self,
        text: &str,
        editorconfig: &serde_json::Map<String, Value>,
        indent: (usize, bool),
        range: Option<Span>,
    ) -> Result<Result<Vec<TextEdit>, String>, ServerFailed> {
        let found = self.ask(
            "format",
            json!({
                "text": text, "editorconfig": editorconfig, "indentSize": indent.0, "tabs": indent.1, "eol": "\n",
                "range": range.map(Span::to_json),
            }),
        )?;
        Ok(match found["problem"].as_str() {
            Some(problem) => Err(problem.to_string()),
            None => Ok(crate::text_edits(&found["edits"])),
        })
    }

    /// What Surround With wraps for `selection` in `text`: the element at a caret, or the elements a selection touches
    /// under one parent, and how many. Blocks, so call it off the UI thread.
    pub fn surround_target(&self, text: &str, selection: Span) -> Result<Option<(Span, usize)>, ServerFailed> {
        let found = self.ask(
            "surround",
            json!({ "text": text, "start": selection.start.to_json(), "end": selection.end.to_json() }),
        )?;
        Ok((!found.is_null()).then(|| (Span::from(&found), found["count"].as_u64().unwrap_or(1) as usize)))
    }

    /// Extract Resource (`kind` "resource") or Extract Style ("style") at `at`: what is there and where it can go, or
    /// with `more`'s `name` why that name will not do, or with `make` the changes (`edit`). Blocks, so call it off the UI
    /// thread.
    pub fn extract(
        &self,
        project: &Path,
        path: &Path,
        text: &str,
        at: Point,
        kind: &str,
        more: Value,
    ) -> Result<Value, ServerFailed> {
        let mut asked = params(project, path, text, at);
        asked["kind"] = json!(kind);
        if let (Some(asked), Value::Object(more)) = (asked.as_object_mut(), more) {
            asked.extend(more);
        }
        self.ask("extract", asked)
    }

    /// Where the XAML of `class` (its full name) names the element `name`. Blocks, so call it off the UI thread.
    pub fn element(&self, project: &Path, class: &str, name: &str) -> Result<Option<Location>, ServerFailed> {
        let found = self.ask("element", json!({ "project": project, "class": class, "name": name }))?;
        let at = Point::from(&found["at"]);
        Ok(found["path"].as_str().map(|path| Location { path: PathBuf::from(path), span: Span { start: at, end: at } }))
    }

    /// Files written on disk, which the projects that hold them read again. Blocks, so call it off the UI thread.
    pub fn changed(&self, paths: &[PathBuf]) -> Result<(), ServerFailed> {
        self.ask("changed", json!({ "paths": paths }))?;
        self.changed_project(None);
        Ok(())
    }

    /// The last editor of a file closed: discard its overlay without starting an unused host.
    /// Blocks, so call it off the UI thread.
    pub fn closed(&self, path: &Path) -> Result<(), ServerFailed> {
        let running = self.running.lock().unwrap().clone();
        if let Some(connection) = running.filter(|c| c.failed().is_none()) {
            connection.ask("closed", json!({ "path": path }), |_| {}).map_err(ServerFailed)?;
            self.changed_project(None);
        }
        Ok(())
    }
}

fn params(project: &Path, path: &Path, text: &str, at: Point) -> Value {
    json!({ "project": project, "path": path, "text": text, "line": at.line, "character": at.character })
}

fn item(found: &Value) -> XamlItem {
    let mut insert = found["insert"].as_str().unwrap_or_default().to_string();
    let caret = (found["snippet"] == true).then(|| insert.find("$0")).flatten();
    if let Some(at) = caret {
        insert.replace_range(at..at + 2, "");
    }
    XamlItem {
        label: found["label"].as_str().unwrap_or_default().to_string(),
        kind: found["kind"].as_str().unwrap_or_default().to_string(),
        insert,
        caret,
        reopen: found["reopen"] == true,
        detail: found["detail"].as_str().unwrap_or_default().to_string(),
    }
}

#[cfg(test)]
mod tests;
