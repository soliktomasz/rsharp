//! Passenger's project Host (vendor/passenger's Passenger.Host): MSBuild in a process of its own, loading the SDK the
//! folder's global.json picks, answering JSON lines: a `build` request is answered with a design-time build per project
//! as each ends (Roslyn's ProjectFileInfo, as its own build host writes them), then the round's end.

use crate::ServerFailed;
use rsharp_core::msbuild::insight::ProjectInsight;
use rsharp_core::process::{self, Child};
use rsharp_core::project::EvaluatedProject;
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Lines, Write};
use std::path::{Path, PathBuf};
use std::process::{ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

/// A project's design-time builds, or why there are none, as Roslyn's server takes them from the extension.
pub struct Built {
    pub infos: Option<String>,
    pub problem: Option<String>,
    pub diagnostics: Value,
}

pub struct ProjectHost {
    input: ChildStdin,
    output: Lines<BufReader<ChildStdout>>,
    next_id: i64,
    _child: Child,
}

impl ProjectHost {
    /// Starts the Host on `folder` and reads its first line, the SDK it loaded or why it loaded none.
    pub fn start(dotnet: &Path, host: &Path, folder: &Path) -> Result<Self, ServerFailed> {
        let mut command = Command::new(dotnet);
        command
            .arg(host)
            .arg("--folder")
            .arg(folder)
            .current_dir(folder)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        let mut child =
            process::spawn(command).map_err(|e| ServerFailed(format!("the project host did not start: {e}")))?;
        let input = child.process.stdin.take().expect("stdin was piped");
        let output = BufReader::new(child.process.stdout.take().expect("stdout was piped")).lines();
        let mut host = Self { input, output, next_id: 1, _child: child };
        let ready = host.reply()?;
        if let Some(problem) = ready["error"].as_str() {
            return Err(ServerFailed(format!("the project host loaded no SDK: {problem}")));
        }
        Ok(host)
    }

    fn reply(&mut self) -> Result<Value, ServerFailed> {
        let line = self
            .output
            .next()
            .ok_or_else(|| ServerFailed("the project host stopped".into()))?
            .map_err(|e| ServerFailed(format!("the project host could not be read: {e}")))?;
        serde_json::from_str(&line).map_err(|e| ServerFailed(format!("the project host said what is not JSON: {e}")))
    }

    /// Asks one request and passes each of its partial results on, until its result or its error.
    fn ask(&mut self, method: &str, params: Value, mut partial: impl FnMut(&Value)) -> Result<(), String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "id": id, "method": method, "params": params });
        writeln!(self.input, "{request}")
            .and_then(|_| self.input.flush())
            .map_err(|e| format!("the project host could not be asked: {e}"))?;
        loop {
            let reply = self.reply().map_err(|e| e.0)?;
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply["error"].as_str() {
                return Err(error.to_string());
            }
            if let Some(value) = reply.get("partial") {
                partial(value);
            }
            if reply.get("result").is_some() {
                return Ok(());
            }
        }
    }

    /// One project's design-time builds, with the global properties a build of its solution gives it.
    pub fn build(&mut self, project: &Path, properties: &Value) -> Result<Built, ServerFailed> {
        let mut built = None;
        self.ask("build", json!({ "projects": [project], "properties": properties }), |partial| {
            built = Some(Built {
                infos: partial["infos"].as_str().map(str::to_string),
                problem: partial["problem"].as_str().map(str::to_string),
                diagnostics: partial.get("diagnostics").cloned().unwrap_or(Value::Null),
            });
        })
        .map_err(|e| ServerFailed(format!("the project host could not build {}: {e}", project.display())))?;
        built.ok_or_else(|| ServerFailed(format!("the project host built nothing of {}", project.display())))
    }

    /// One project as MSBuild evaluates it, every framework of it, with the solution's global properties.
    pub fn evaluate(&mut self, project: &Path, properties: &Value) -> Result<EvaluatedProject, ServerFailed> {
        let mut evaluated: Option<EvaluatedProject> = None;
        let mut unread = None;
        self.ask("evaluate", json!({ "projects": [project], "properties": properties }), |partial| {
            // A multi-targeted project may come first by its outer evaluation alone; the complete one follows it.
            match serde_json::from_value(partial.clone()) {
                Ok(project) => evaluated = Some(project),
                Err(error) => unread = Some(error),
            }
        })
        .map_err(|e| ServerFailed(format!("the project host could not evaluate {}: {e}", project.display())))?;
        evaluated.ok_or_else(|| match unread {
            Some(error) => {
                ServerFailed(format!("the project host's evaluation of {} did not read: {error}", project.display()))
            }
            None => ServerFailed(format!("the project host evaluated nothing of {}", project.display())),
        })
    }
}

impl ProjectHost {
    /// One project evaluated for the code help in its MSBuild files: the solution's global properties, and the open
    /// files' unsaved text, by path, read in place of the disk's.
    pub fn inspect(
        &mut self,
        project: &Path,
        properties: &Value,
        texts: &std::collections::HashMap<PathBuf, String>,
    ) -> Result<ProjectInsight, ServerFailed> {
        let texts: serde_json::Map<String, Value> =
            texts.iter().map(|(p, t)| (p.to_string_lossy().into_owned(), Value::String(t.clone()))).collect();
        let result = self
            .ask_result("inspect", json!({ "project": project, "properties": properties, "texts": texts }))
            .map_err(|e| ServerFailed(format!("the project host could not inspect {}: {e}", project.display())))?;
        serde_json::from_value(result).map_err(|e| {
            ServerFailed(format!("the project host's inspection of {} did not read: {e}", project.display()))
        })
    }

    /// Asks one request answered by a result alone.
    fn ask_result(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id;
        self.next_id += 1;
        let request = json!({ "id": id, "method": method, "params": params });
        writeln!(self.input, "{request}")
            .and_then(|_| self.input.flush())
            .map_err(|e| format!("the project host could not be asked: {e}"))?;
        loop {
            let reply = self.reply().map_err(|e| e.0)?;
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply["error"].as_str() {
                return Err(error.to_string());
            }
            if let Some(result) = reply.get("result") {
                return Ok(result.clone());
            }
        }
    }
}

/// The one Host a folder has, which Roslyn's design-time builds and the Solution view's evaluations take turns on:
/// started on first use, and again after it fails.
#[derive(Clone)]
pub struct SharedHost {
    running: Arc<Mutex<Option<ProjectHost>>>,
    dotnet: PathBuf,
    host: PathBuf,
    folder: PathBuf,
}

impl SharedHost {
    /// None without Passenger's sidecars or dotnet: then nothing is evaluated and Roslyn finds the projects itself.
    pub fn carried(folder: &Path) -> Option<Self> {
        let host = crate::roslyn::carried_host()?;
        let dotnet = rsharp_core::programs::dotnet()?;
        Some(Self { running: Arc::default(), dotnet, host, folder: folder.to_path_buf() })
    }

    fn with<T>(&self, ask: impl FnOnce(&mut ProjectHost) -> Result<T, ServerFailed>) -> Result<T, ServerFailed> {
        let mut running = self.running.lock().unwrap();
        if running.is_none() {
            *running = Some(ProjectHost::start(&self.dotnet, &self.host, &self.folder)?);
        }
        let answer = ask(running.as_mut().expect("started above"));
        if answer.is_err() {
            *running = None;
        }
        answer
    }

    /// Blocks while a build or another evaluation has the Host, so call it off the UI thread.
    pub fn build(&self, project: &Path, properties: &Value) -> Result<Built, ServerFailed> {
        self.with(|host| host.build(project, properties))
    }

    /// Blocks while a build or another evaluation has the Host, so call it off the UI thread.
    // shortcut: one project per evaluate request so Roslyn's builds wait at most one evaluation, until the Host is
    // read by a reader thread that routes replies by id.
    pub fn evaluate(&self, project: &Path, solution: Option<&Path>) -> Result<EvaluatedProject, ServerFailed> {
        let properties = crate::roslyn::solution_properties(solution);
        self.with(|host| host.evaluate(project, &properties))
    }

    /// Blocks while a build or another request has the Host, so call it off the UI thread.
    pub fn inspect(
        &self,
        project: &Path,
        solution: Option<&Path>,
        texts: &std::collections::HashMap<PathBuf, String>,
    ) -> Result<ProjectInsight, ServerFailed> {
        let properties = crate::roslyn::solution_properties(solution);
        self.with(|host| host.inspect(project, &properties, texts))
    }
}
