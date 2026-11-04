//! The editor and file tools an agent calls in a window (Passenger's `PassengerTools.Ide`): the text as the editor has it,
//! `edit`, Find and Replace in Files, the settings, Problems, the Build console and Local History. Each answers with what
//! its operation did, on the UI thread as the window's own commands run.

use super::agent::said;
use super::*;
use crate::buffer::{self, Buffer};
use crate::settings::settings;
use crate::workspace::{all_files, find_in_files, fits_mask};
use rsharp_text::Edit;
use serde_json::{Map, Value, json};
use std::collections::HashMap;

/// Find in Files' most matches, and the files it looks among, as the popup's.
const MOST: usize = 1000;
const MOST_FILES: usize = 100_000;

/// A tool's reply, now.
type Reply = Task<Result<Value, String>>;

impl MainWindow {
    /// The path an agent named: from the folder when relative.
    fn agent_path(&self, file: &str) -> PathBuf {
        let path = PathBuf::from(file);
        match (&self.workspace, path.is_absolute()) {
            (Some(workspace), false) => workspace.folder.join(path),
            _ => path,
        }
    }

    /// A file's buffer as the editors share it, read from disk when no editor has it.
    pub(super) fn agent_buffer(&self, file: &str, cx: &mut Context<Self>) -> Result<Entity<Buffer>, String> {
        let path = self.agent_path(file);
        if let Some(open) = buffer::find(&path, cx) {
            return Ok(open);
        }
        if !path.is_file() {
            return Err(format!("{} is not a file", path.display()));
        }
        buffer::open(path.clone(), cx).map_err(|e| format!("{} could not be read: {e}", path.display()))
    }

    pub(super) fn relative(&self, path: &Path) -> String {
        let folder = self.folder();
        folder.and_then(|f| path.strip_prefix(f).ok()).unwrap_or(path).display().to_string()
    }

    /// The tool `tool` if this module answers it.
    pub(super) fn agent_editor_tool(&mut self, tool: &str, arguments: &Value, cx: &mut Context<Self>) -> Option<Reply> {
        let text = |key: &str| arguments[key].as_str().map(str::to_string);
        let done = |result: Result<Value, String>| Some(Task::ready(result));
        match tool {
            "editor_text" => {
                let Some(file) = text("file") else { return done(Err("editor_text needs a file".into())) };
                let buffer = match self.agent_buffer(&file, cx) {
                    Ok(buffer) => buffer,
                    Err(why) => return done(Err(why)),
                };
                let body = buffer.read(cx).document.text().to_string();
                let lines: Vec<&str> = body.lines().collect();
                let from = arguments["from"].as_u64().unwrap_or(1).max(1) as usize;
                let to = arguments["to"].as_u64().map_or(lines.len(), |t| t as usize).min(lines.len());
                let numbered: Vec<String> =
                    (from..=to).filter_map(|n| lines.get(n - 1).map(|l| format!("{n}: {l}"))).collect();
                done(Ok(said(numbered.join("\n"))))
            }
            "edit" => done(self.agent_edit(arguments, cx)),
            "find_in_files" | "replace_in_files" => {
                let Some(query) = text("text").filter(|t| !t.is_empty()) else {
                    return done(Err(format!("{tool} needs the text to find")));
                };
                let mask = text("mask").unwrap_or_default();
                let replacement = text("replacement");
                if tool == "replace_in_files" && replacement.is_none() {
                    return done(Err("replace_in_files needs a replacement".into()));
                }
                let limit = arguments["limit"].as_u64().map_or(200, |l| (l as usize).clamp(1, MOST));
                let Some(folder) = self.folder().map(Path::to_path_buf) else {
                    return done(Err("No folder is open".into()));
                };
                let open: HashMap<PathBuf, String> = self
                    .workspace_buffers(cx)
                    .iter()
                    .map(|b| (b.read(cx).path.clone(), b.read(cx).document.text().to_string()))
                    .collect();
                // A replacement takes every match, a listing the first thousand.
                let take = if replacement.is_some() { usize::MAX } else { MOST };
                let search = {
                    let (folder, query, open) = (folder.clone(), query.clone(), open.clone());
                    cx.background_executor().spawn(async move {
                        let files: Vec<PathBuf> = all_files(&folder, MOST_FILES)
                            .into_iter()
                            .filter(|p| fits_mask(&p.file_name().unwrap_or_default().to_string_lossy(), &mask))
                            .collect();
                        find_in_files(&files, &open, &query, take)
                    })
                };
                let replacing = replacement;
                Some(cx.spawn(async move |this, cx| {
                    let hits = search.await;
                    this.update(cx, |w, cx| match replacing {
                        None => {
                            let capped = hits.len() >= MOST;
                            let matches: Vec<Value> = hits
                                .iter()
                                .take(limit)
                                .map(|h| json!({ "file": w.relative(&h.path), "line": h.line + 1, "column": h.column + 1, "text": h.text }))
                                .collect();
                            Ok(said(json!({ "count": hits.len(), "capped": capped, "matches": matches }).to_string()))
                        }
                        Some(replacement) => {
                            if hits.is_empty() {
                                return Ok(said("Nothing matches, so nothing was replaced"));
                            }
                            let mut files: Vec<PathBuf> = hits.into_iter().map(|h| h.path).collect();
                            files.dedup();
                            let server = w.server.clone();
                            let n = crate::language::replace_in_files(&files, &query, &replacement, server.as_deref(), cx)?;
                            Ok(said(format!("Replaced {n} matches in {} files", files.len())))
                        }
                    })
                    .map_err(|e| e.to_string())?
                }))
            }
            "settings" => {
                let all = serde_json::to_value(settings(cx)).unwrap_or(Value::Null);
                done(Ok(said(json!({ "kept": true, "settings": all }).to_string())))
            }
            "settings_set" => done(self.agent_settings_set(arguments, cx)),
            "problems" => {
                use super::ProblemsTab::{Build, Solution, Toolset};
                let tab = |t| self.problems_listed(t, cx);
                done(Ok(said(
                    json!({ "toolset": tab(Toolset), "build": tab(Build), "solution": tab(Solution) }).to_string(),
                )))
            }
            "build_output" => {
                let lines = arguments["lines"].as_u64().unwrap_or(200).max(1) as usize;
                let all = self.build.read(cx).text();
                let tail: Vec<&str> = all.lines().rev().take(lines).collect();
                done(Ok(said(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))))
            }
            "local_history" => {
                let Some(file) = text("file") else { return done(Err("local_history needs a file".into())) };
                let path = self.agent_path(&file);
                let history = crate::history_window::local_history(cx);
                let revisions = history.revisions(&path);
                let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
                let versions: Vec<Value> = revisions
                    .iter()
                    .enumerate()
                    .rev()
                    .map(|(n, r)| json!({ "version": n, "secondsAgo": now.saturating_sub(r.time), "label": r.label }))
                    .collect();
                done(Ok(said(json!({ "file": self.relative(&path), "versions": versions }).to_string())))
            }
            "local_history_revert" => {
                let (Some(file), Some(version)) = (text("file"), arguments["version"].as_u64()) else {
                    return done(Err("local_history_revert needs a file and a version".into()));
                };
                done(self.agent_revert(&file, version as usize, cx))
            }
            _ => None,
        }
    }

    fn agent_edit(&mut self, arguments: &Value, cx: &mut Context<Self>) -> Result<Value, String> {
        let (Some(file), Some(old), Some(new)) =
            (arguments["file"].as_str(), arguments["oldText"].as_str(), arguments["newText"].as_str())
        else {
            return Err("edit needs a file, oldText and newText".into());
        };
        if old.is_empty() {
            return Err("Give oldText, the text to replace as the file has it".into());
        }
        let (old, new) = (old.replace("\r\n", "\n"), new.replace("\r\n", "\n"));
        let buffer = self.agent_buffer(file, cx)?;
        let name = self.relative(&buffer.read(cx).path.clone());
        let body = buffer.read(cx).document.text().to_string();
        let found: Vec<usize> = body.match_indices(&old).map(|(at, _)| at).collect();
        let all = arguments["all"] == true;
        match found.len() {
            0 => return Err(format!("oldText is not in {name}: it must be exactly as the file has it")),
            n if n > 1 && !all => {
                let lines: Vec<String> =
                    found.iter().map(|at| (body[..*at].lines().count().max(1)).to_string()).collect();
                return Err(format!(
                    "oldText is in {name} {n} times (lines {}): make it longer to be one, or all",
                    lines.join(", ")
                ));
            }
            _ => {}
        }
        let edits: Vec<Edit> = found.iter().map(|at| Edit { range: *at..*at + old.len(), text: new.clone() }).collect();
        let first_line = body[..found[0]].lines().count().max(1);
        buffer.update(cx, |b, cx| {
            b.apply_edits(edits, cx);
            b.save(cx).map_err(|e| format!("{name} could not be written: {e}"))
        })?;
        let now = buffer.read(cx).document.text().to_string();
        let span = new.lines().count().max(1);
        let shown: Vec<String> = now
            .lines()
            .enumerate()
            .skip(first_line - 1)
            .take(span + 1)
            .map(|(i, l)| format!("{}: {l}", i + 1))
            .collect();
        Ok(said(format!(
            "Changed {name} ({} place{})\n{}",
            found.len(),
            if found.len() == 1 { "" } else { "s" },
            shown.join("\n")
        )))
    }

    fn agent_settings_set(&mut self, arguments: &Value, cx: &mut Context<Self>) -> Result<Value, String> {
        let Some(changes) = arguments["changes"].as_object().filter(|c| !c.is_empty()) else {
            return Err("Name at least one setting to change, with its new value; settings lists them".into());
        };
        let before = serde_json::to_value(settings(cx)).map_err(|e| e.to_string())?;
        let mut next = before.clone();
        for (name, value) in changes {
            if name == "agentsEnabled" {
                return Err("agentsEnabled is the person's to change: Settings > MCP Server".into());
            }
            let Some(current) = next.get_mut(name) else {
                return Err(format!("Settings has no setting {name}; the settings tool lists those it has"));
            };
            merge(current, value);
        }
        let parsed: crate::settings::AppSettings =
            serde_json::from_value(next).map_err(|e| format!("A setting is refused, nothing changed: {e}"))?;
        crate::settings::apply(parsed, cx);
        let after = serde_json::to_value(settings(cx)).map_err(|e| e.to_string())?;
        let reported: Map<String, Value> = changes
            .keys()
            .map(|name| (name.clone(), json!({ "before": before[name], "after": after[name] })))
            .collect();
        Ok(said(Value::Object(reported).to_string()))
    }

    fn agent_revert(&mut self, file: &str, version: usize, cx: &mut Context<Self>) -> Result<Value, String> {
        let path = self.agent_path(file);
        let history = crate::history_window::local_history(cx);
        let revision =
            history.revisions(&path).into_iter().nth(version).ok_or_else(|| {
                format!("{} has no version {version}; local_history lists them", self.relative(&path))
            })?;
        let bytes = history.content(&revision).map_err(|e| format!("The version could not be read: {e}"))?;
        let buffer = self.agent_buffer(file, cx)?;
        let (text, _) = rsharp_core::encodings::decode(&bytes, Some(buffer.read(cx).charset));
        let text = rsharp_text::Document::new(&text).text().to_string();
        buffer.update(cx, |b, cx| b.replace_all(text, cx));
        Ok(said(format!("{} is as version {version} had it", self.relative(&path))))
    }
}

/// An object's members merged into another, anything else replaced.
fn merge(into: &mut Value, from: &Value) {
    match (into.as_object_mut(), from.as_object()) {
        (Some(into), Some(from)) => {
            for (key, value) in from {
                match into.get_mut(key) {
                    Some(there) => merge(there, value),
                    None => drop(into.insert(key.clone(), value.clone())),
                }
            }
        }
        _ => *into = from.clone(),
    }
}
