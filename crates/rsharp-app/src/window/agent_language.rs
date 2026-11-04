//! The language tools an agent calls in a window (Passenger's `PassengerTools.Languages`): diagnostics, symbols, an
//! outline, usages, definitions, implementations, hover text and rename, answered from the window's own Roslyn over the
//! texts its editors hold, so an agent sees what the person sees, unsaved changes included.

use super::agent::said;
use super::*;
use gpui::{AsyncApp, WeakEntity};
use rsharp_lsp as lsp;
use rsharp_lsp::Point;
use serde_json::{Value, json};
use std::time::Duration;

type Reply = Task<Result<Value, String>>;

/// How long a tool waits for Roslyn to load the projects before it answers without them.
const LOAD_WAIT: Duration = Duration::from_secs(180);

/// A tool's answer in the window's words, or why not.
type Said = Result<String, String>;

impl MainWindow {
    /// The language tool `tool` if this module answers it.
    pub(super) fn agent_language_tool(
        &mut self,
        tool: &str,
        arguments: &Value,
        cx: &mut Context<Self>,
    ) -> Option<Reply> {
        if !matches!(
            tool,
            "diagnostics"
                | "find_symbol"
                | "outline"
                | "usages"
                | "definition"
                | "implementations"
                | "symbol_info"
                | "rename"
        ) {
            return None;
        }
        let (tool, arguments) = (tool.to_string(), arguments.clone());
        Some(cx.spawn(async move |this, cx| {
            // Every one waits for Roslyn to say the projects are loaded.
            let mut waited = Duration::ZERO;
            let server = loop {
                let state =
                    this.update(cx, |w, _| (w.server_ready(), w.server.clone(), w.server_failed().map(str::to_string)));
                match state.map_err(|e| e.to_string())? {
                    (_, _, Some(why)) => return Err(format!("Roslyn is not running: {why}")),
                    (true, Some(server), _) => break server,
                    _ if waited >= LOAD_WAIT => return Err("Roslyn has not loaded the projects: try again".into()),
                    _ => {
                        cx.background_executor().timer(Duration::from_millis(250)).await;
                        waited += Duration::from_millis(250);
                    }
                }
            };
            let said_it: Said = match tool.as_str() {
                "diagnostics" => diagnostics(&this, &server, &arguments, cx).await,
                "find_symbol" => find_symbol(&this, &server, &arguments, cx).await,
                "outline" => outline(&this, &server, &arguments, cx).await,
                "usages" => usages(&this, &server, &arguments, cx).await,
                "definition" => definition(&this, &server, &arguments, cx).await,
                "implementations" => implementations(&this, &server, &arguments, cx).await,
                "symbol_info" => symbol_info(&this, &server, &arguments, cx).await,
                _ => rename(&this, &server, &arguments, cx).await,
            };
            said_it.map(said)
        }))
    }

    /// A file's buffer, open in Roslyn as the editors have it, and its URI.
    fn agent_language_buffer(
        &self,
        file: &str,
        server: &Arc<LanguageServer>,
        cx: &mut Context<Self>,
    ) -> Result<(Entity<Buffer>, String), String> {
        let buffer = self.agent_buffer(file, cx)?;
        if buffer.read(cx).server().is_none() {
            buffer.update(cx, |b, cx| b.attach_server(server.clone(), cx));
        }
        let uri = buffer.read(cx).server().map(|(_, uri)| uri).ok_or("The file could not be opened in Roslyn")?;
        Ok((buffer, uri))
    }
}

type Window_ = WeakEntity<MainWindow>;

fn failed(e: impl std::fmt::Display) -> String {
    e.to_string()
}

async fn ask(server: &LanguageServer, method: &str, params: Value) -> Result<Value, String> {
    // A request Roslyn cancels because a file changed under it is asked again a few times.
    for attempt in 0..10 {
        match server.ask(method, params.clone()).await {
            Err(e) if e.0.contains("cancel") && attempt < 9 => {
                futures_timer(Duration::from_millis(200)).await;
            }
            answer => return answer.map_err(|e| format!("Roslyn refused {method}: {e}")),
        }
    }
    unreachable!("the loop returns by its tenth attempt")
}

async fn futures_timer(duration: Duration) {
    let (sender, receiver) = futures::channel::oneshot::channel();
    std::thread::spawn(move || {
        std::thread::sleep(duration);
        let _ = sender.send(());
    });
    let _ = receiver.await;
}

/// A place: the file, line and column from 1 named, or the one symbol a name finds.
async fn place(
    this: &Window_,
    server: &Arc<LanguageServer>,
    arguments: &Value,
    cx: &mut AsyncApp,
) -> Result<(PathBuf, Point, Entity<Buffer>, String), String> {
    let (file, at) = if let Some(file) = arguments["file"].as_str() {
        let line = arguments["line"].as_u64().filter(|l| *l >= 1).ok_or("a place needs line, from 1")? as usize - 1;
        let column =
            arguments["column"].as_u64().filter(|c| *c >= 1).ok_or("a place needs column, from 1")? as usize - 1;
        let (path, line_text) = this
            .update(cx, |w, cx| {
                let buffer = w.agent_buffer(file, cx)?;
                let b = buffer.read(cx);
                let text = b.document.text();
                if line >= text.len_lines() {
                    return Err(format!("{file} has no line {}", line + 1));
                }
                Ok((b.path.clone(), text.line(line).chars().take(column).map(char::len_utf16).sum::<usize>()))
            })
            .map_err(failed)??;
        (path.display().to_string(), Point::new(line, line_text))
    } else {
        let name = arguments["symbol"].as_str().ok_or("name a place: file, line and column, or symbol")?;
        let found = lsp::symbols(&ask(server, "workspace/symbol", json!({ "query": name })).await?);
        let bare = |s: &str| s.split(['(', '<']).next().unwrap_or(s).to_string();
        let named: Vec<_> = found.iter().filter(|s| bare(&s.name) == name).collect();
        match named[..] {
            [symbol] => (symbol.location.path.display().to_string(), symbol.location.span.start),
            [] if found.is_empty() => return Err(format!("no symbol named {name}")),
            _ => {
                let candidates = if named.is_empty() { found.iter().collect() } else { named };
                let listed: Vec<String> = candidates
                    .iter()
                    .take(20)
                    .map(|s| {
                        format!(
                            "{} at {}:{}:{}",
                            s.name,
                            s.location.path.display(),
                            s.location.span.start.line + 1,
                            s.location.span.start.character + 1
                        )
                    })
                    .collect();
                return Err(format!(
                    "{name} names no one symbol; give the file, line and column of one of:\n{}",
                    listed.join("\n")
                ));
            }
        }
    };
    let (buffer, uri) = this.update(cx, |w, cx| w.agent_language_buffer(&file, server, cx)).map_err(failed)??;
    let path = buffer.read_with(cx, |b, _| b.path.clone()).map_err(failed)?;
    Ok((path, at, buffer, uri))
}

fn shown(this: &Window_, path: &Path, at: Point, cx: &mut AsyncApp) -> String {
    let name = this.update(cx, |w, _| w.relative(path)).unwrap_or_else(|_| path.display().to_string());
    format!("{name}:{}:{}", at.line + 1, at.character + 1)
}

async fn diagnostics(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let Some(file) = arguments["file"].as_str() else {
        return this
            .update(cx, |w, cx| {
                if !w.solution_wide {
                    return Err("Solution-wide analysis is off: turn it on in Problems' Solution tab".to_string());
                }
                let listed = w.problems_listed(ProblemsTab::Solution, cx);
                Ok(if listed.is_empty() { "No problems in the solution.".into() } else { listed.join("\n") })
            })
            .map_err(failed)?;
    };
    let (path, _, _, uri) = {
        let args = json!({ "file": file, "line": 1, "column": 1 });
        place(this, server, &args, cx).await?
    };
    let report = ask(server, "textDocument/diagnostic", json!({ "textDocument": { "uri": uri } })).await?;
    let problems = lsp::problems(&report["items"]);
    if problems.is_empty() {
        return Ok(format!("No problems in {file}."));
    }
    Ok(problems
        .iter()
        .map(|p| {
            format!(
                "{} {}: {}",
                shown(this, &path, p.span.start, cx),
                if p.error { "error" } else { "warning" },
                p.message
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn find_symbol(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let query = arguments["query"].as_str().ok_or("find_symbol needs query")?;
    let found = lsp::symbols(&ask(server, "workspace/symbol", json!({ "query": query })).await?);
    if found.is_empty() {
        return Ok(format!("No symbol matches {query}."));
    }
    Ok(found
        .iter()
        .take(100)
        .map(|s| {
            let within = if s.container.is_empty() { String::new() } else { format!(" in {}", s.container) };
            format!(
                "{} {}{within}: {}",
                lsp::kind_name(s.kind),
                s.name,
                shown(this, &s.location.path, s.location.span.start, cx)
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn outline(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let file = arguments["file"].as_str().ok_or("outline needs file")?;
    let (_, _, _, uri) = place(this, server, &json!({ "file": file, "line": 1, "column": 1 }), cx).await?;
    let shape =
        lsp::outline(&ask(server, "textDocument/documentSymbol", json!({ "textDocument": { "uri": uri } })).await?);
    Ok(shape
        .iter()
        .map(|(depth, name, kind, at)| {
            format!("{}{} {name} (line {})", "  ".repeat(*depth), lsp::kind_name(*kind), at.start.line + 1)
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The text of a line of a file, as the editor has it or the disk.
fn line_of(this: &Window_, path: &Path, line: usize, cx: &mut AsyncApp) -> String {
    this.update(cx, |w, cx| {
        w.agent_buffer(&path.display().to_string(), cx)
            .ok()
            .and_then(|b| b.read(cx).document.text().lines().nth(line).map(|l| l.to_string().trim().to_string()))
    })
    .ok()
    .flatten()
    .unwrap_or_default()
}

async fn usages(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let (_, at, _, uri) = place(this, server, arguments, cx).await?;
    let mut params = lsp::at(&uri, at);
    params["context"] = json!({ "includeDeclaration": true });
    let found = lsp::locations(&ask(server, "textDocument/references", params).await?);
    if found.is_empty() {
        return Ok("No usages found.".into());
    }
    Ok(found
        .iter()
        .map(|l| {
            format!("{}: {}", shown(this, &l.path, l.span.start, cx), line_of(this, &l.path, l.span.start.line, cx))
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn definition(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let (_, at, _, uri) = place(this, server, arguments, cx).await?;
    let found = lsp::locations(&ask(server, "textDocument/definition", lsp::at(&uri, at)).await?);
    if found.is_empty() {
        return Ok("No declaration found.".into());
    }
    Ok(found.iter().map(|l| shown(this, &l.path, l.span.start, cx)).collect::<Vec<_>>().join("\n"))
}

async fn implementations(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let (_, at, _, uri) = place(this, server, arguments, cx).await?;
    let found = lsp::locations(&ask(server, "textDocument/implementation", lsp::at(&uri, at)).await?);
    if found.is_empty() {
        return Ok("No implementations found.".into());
    }
    Ok(found
        .iter()
        .map(|l| {
            format!("{}: {}", shown(this, &l.path, l.span.start, cx), line_of(this, &l.path, l.span.start.line, cx))
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

async fn symbol_info(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let (_, at, _, uri) = place(this, server, arguments, cx).await?;
    Ok(lsp::hover(&ask(server, "textDocument/hover", lsp::at(&uri, at)).await?)
        .unwrap_or_else(|| "Nothing is known of what is there.".into()))
}

async fn rename(this: &Window_, server: &Arc<LanguageServer>, arguments: &Value, cx: &mut AsyncApp) -> Said {
    let new_name = arguments["newName"].as_str().filter(|n| !n.is_empty()).ok_or("rename needs newName")?;
    let (_, at, buffer, uri) = place(this, server, arguments, cx).await?;
    let old = buffer
        .read_with(cx, |b, _| {
            let text = b.document.text();
            let offset = rsharp_text::offset_of_utf16(text, at.line, at.character);
            text.byte_slice(rsharp_text::word_at(text, offset).range()).to_string()
        })
        .map_err(failed)?;
    let here = lsp::at(&uri, at);
    let types = lsp::hierarchy_items(&ask(server, "textDocument/prepareTypeHierarchy", here.clone()).await?);
    let declared: Vec<PathBuf> = lsp::locations(&ask(server, "textDocument/definition", here.clone()).await?)
        .into_iter()
        .map(|l| l.path)
        .collect();
    let mut params = here;
    params["newName"] = json!(new_name);
    let mut edit = lsp::workspace_edit(&ask(server, "textDocument/rename", params).await?);
    if edit.is_empty() {
        return Err(format!("{old} cannot be renamed to {new_name}"));
    }
    // A type's files named after it are renamed with it, in the same change.
    if types.first().is_some_and(|t| t.name == old) {
        edit.moves.extend(lsp::files_named_after(&declared, &old, new_name));
    }
    let files = edit.files.len();
    let server = server.clone();
    this.update(cx, |_, cx| crate::language::apply_workspace_edit(&edit, Some(&server), cx)).map_err(failed)??;
    Ok(format!("Renamed {old} to {new_name} in {files} file{}", if files == 1 { "" } else { "s" }))
}
