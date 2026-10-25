//! The protocol's answers as RSharp reads them: a frame, a scope, a value, a breakpoint and what the adapter says it can
//! do. What is sent is built where it is sent; this is the shape of what arrives.

use serde::Deserialize;
use std::path::PathBuf;

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Capabilities {
    pub supports_configuration_done_request: bool,
    pub supports_conditional_breakpoints: bool,
    pub supports_hit_conditional_breakpoints: bool,
    pub supports_log_points: bool,
    pub supports_evaluate_for_hovers: bool,
    pub supports_terminate_request: bool,
    pub supports_restart_request: bool,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Source {
    pub name: Option<String>,
    pub path: Option<PathBuf>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Thread {
    pub id: i64,
    pub name: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Threads {
    #[serde(default)]
    pub threads: Vec<Thread>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Frame {
    pub id: i64,
    pub name: String,
    #[serde(default)]
    pub source: Option<Source>,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
    /// "subtle" for code outside the person's own.
    #[serde(default)]
    pub presentation_hint: Option<String>,
}

impl Frame {
    pub fn path(&self) -> Option<PathBuf> {
        self.source.as_ref().and_then(|source| source.path.clone())
    }

    /// The frame's name without its assembly: "Program.Main(string[] args)", as Rider lists it.
    pub fn short_name(&self) -> String {
        self.name.rsplit_once("! ").map_or(self.name.as_str(), |(_, name)| name).to_string()
    }
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StackTrace {
    #[serde(default)]
    pub stack_frames: Vec<Frame>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scope {
    pub name: String,
    pub variables_reference: i64,
    #[serde(default)]
    pub expensive: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Scopes {
    #[serde(default)]
    pub scopes: Vec<Scope>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variable {
    pub name: String,
    #[serde(default)]
    pub value: String,
    #[serde(default)]
    pub r#type: Option<String>,
    /// What a watch could ask for it again.
    #[serde(default)]
    pub evaluate_name: Option<String>,
    /// Non-zero when it has members of its own.
    #[serde(default)]
    pub variables_reference: i64,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Variables {
    #[serde(default)]
    pub variables: Vec<Variable>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Evaluated {
    pub result: String,
    pub r#type: Option<String>,
    pub variables_reference: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Breakpoint {
    pub id: Option<i64>,
    pub verified: bool,
    pub message: Option<String>,
    pub line: Option<usize>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Breakpoints {
    pub breakpoints: Vec<Breakpoint>,
}

/// What a debugged program can take of Hot Reload, as the adapter describes it: the runtime's capabilities and each
/// loaded module, editable or why not.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HotReloadState {
    pub capabilities: Option<Vec<String>>,
    pub capabilities_unknown: Option<String>,
    pub modules: Vec<HotReloadModule>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct HotReloadModule {
    pub mvid: String,
    pub name: String,
    pub path: Option<PathBuf>,
    pub editable: bool,
    pub why_not: Option<String>,
}

/// What became of each module's Hot Reload update, in the order given: applied, or why not.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ApplyChanges {
    pub modules: Vec<ModuleApplied>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ModuleApplied {
    pub module: String,
    pub name: String,
    pub applied: bool,
    pub message: Option<String>,
}

/// A stop at an exception, as the adapter describes it: what was thrown and where.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExceptionInfo {
    pub exception_id: String,
    pub description: Option<String>,
    pub break_mode: String,
    pub code: Option<i64>,
    pub details: Option<ExceptionDetails>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ExceptionDetails {
    pub message: Option<String>,
    pub type_name: Option<String>,
    pub full_type_name: Option<String>,
    pub evaluate_name: Option<String>,
    pub stack_trace: Option<String>,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct SetValue {
    pub value: String,
    pub r#type: Option<String>,
    pub variables_reference: i64,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Continued {
    pub all_threads_continued: Option<bool>,
}

/// The adapter's request to the client, to start its program in a terminal of the client's own.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RunInTerminal {
    /// "integrated" or "external".
    #[serde(default)]
    pub kind: String,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub cwd: Option<std::path::PathBuf>,
    /// The program and its arguments, as the terminal starts them.
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: Option<std::collections::BTreeMap<String, String>>,
}

/// A call a frame's statement can step into, in the order the calls run: Smart Step Into's choice.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StepInTarget {
    pub id: i64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct StepInTargets {
    pub targets: Vec<StepInTarget>,
}

/// A place a paused thread could go on from in a source at a line: Set Next Statement's choice.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GotoTarget {
    pub id: i64,
    #[serde(default)]
    pub label: String,
    #[serde(default)]
    pub line: usize,
    #[serde(default)]
    pub column: usize,
}

#[derive(Clone, Debug, Default, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct GotoTargets {
    pub targets: Vec<GotoTarget>,
}
