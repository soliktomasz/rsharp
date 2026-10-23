//! Passenger's extension in Roslyn's server (vendor/passenger's Passenger.Roslyn.Extension): the refactorings that take
//! options, each a message to one of its handlers about a document, the handler named by its type. A Describe handler
//! says what is at the caret; the refactoring's handler answers with the files it changes, or why it changes none.

use crate::{Point, Span, TextEdit, WorkspaceEdit};
use rsharp_core::paths;
use serde_json::{Value, json};
use std::path::PathBuf;

pub const DESCRIBE_SIGNATURE: &str = "Passenger.Roslyn.Extension.DescribeSignature";
pub const CHANGE_SIGNATURE: &str = "Passenger.Roslyn.Extension.ChangeSignature";
pub const DESCRIBE_INTERFACE: &str = "Passenger.Roslyn.Extension.DescribeInterface";
pub const EXTRACT_INTERFACE: &str = "Passenger.Roslyn.Extension.ExtractInterface";
pub const DESCRIBE_TYPE: &str = "Passenger.Roslyn.Extension.DescribeType";
pub const MOVE_TYPE_TO_FILE: &str = "Passenger.Roslyn.Extension.MoveTypeToFile";
pub const MOVE_TYPE_TO_NAMESPACE: &str = "Passenger.Roslyn.Extension.MoveTypeToNamespace";
pub const DESCRIBE_HIERARCHY: &str = "Passenger.Roslyn.Extension.DescribeHierarchy";
pub const PULL_MEMBERS_UP: &str = "Passenger.Roslyn.Extension.PullMembersUp";
pub const PUSH_MEMBERS_DOWN: &str = "Passenger.Roslyn.Extension.PushMembersDown";

/// The request that sends `message` to `handler` about the document at `uri`: its method and params (the message as
/// the JSON string the server reads it from; the method's name is Roslyn's own, typo and all).
pub fn request(handler: &str, message: &Value, uri: &str) -> (&'static str, Value) {
    (
        "textDocument/_vs_dipatchExtensionMessage",
        json!({ "messageName": handler, "message": message.to_string(), "textDocument": { "uri": uri } }),
    )
}

/// What a handler answered, or why it did not: the exception it threw, or no answer at all.
pub fn answer(reply: &Value) -> Result<Value, String> {
    let thrown = &reply["extensionException"];
    if !thrown.is_null() {
        let said = thrown["message"]
            .as_str()
            .or(thrown["Message"].as_str())
            .map_or_else(|| thrown.to_string(), str::to_string);
        return Err(format!("Passenger's extension to Roslyn failed: {said}"));
    }
    let response = reply["response"]
        .as_str()
        .filter(|r| !r.is_empty())
        .ok_or("Roslyn gave no answer from Passenger's extension")?;
    serde_json::from_str(response).map_err(|e| format!("Passenger's extension answered what is not JSON: {e}"))
}

/// The message naming where the caret is, as a Describe handler and the refactorings take it.
pub fn at(point: Point) -> Value {
    json!({ "line": point.line, "character": point.character })
}

/// A refactoring's answer as a workspace edit: each file it makes, then every file's edits; its problem when it makes
/// none.
pub fn refactored(answer: &Value) -> Result<WorkspaceEdit, String> {
    if let Some(problem) = answer["problem"].as_str() {
        return Err(problem.to_string());
    }
    let mut edit = WorkspaceEdit::default();
    for change in answer["changes"].as_array().into_iter().flatten() {
        let Some(path) = change["path"].as_str().map(|p| paths::canonical(&PathBuf::from(p))) else { continue };
        if change["created"].as_bool().unwrap_or(false) {
            edit.created.push(path.clone());
        }
        let point = |v: &Value| {
            Point::new(v["line"].as_u64().unwrap_or(0) as usize, v["character"].as_u64().unwrap_or(0) as usize)
        };
        let edits = change["edits"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|e| TextEdit {
                span: Span { start: point(&e["start"]), end: point(&e["end"]) },
                text: e["text"].as_str().unwrap_or_default().into(),
            })
            .collect();
        edit.files.push((path, edits));
    }
    Ok(edit)
}
