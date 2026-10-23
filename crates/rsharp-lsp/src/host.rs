//! Passenger's project Host (vendor/passenger's Passenger.Host): MSBuild in a process of its own, loading the SDK the
//! folder's global.json picks, answering JSON lines: a `build` request is answered with a design-time build per project
//! as each ends (Roslyn's ProjectFileInfo, as its own build host writes them), then the round's end.

use crate::ServerFailed;
use rsharp_core::process::{self, Child};
use serde_json::{Value, json};
use std::io::{BufRead, BufReader, Lines, Write};
use std::path::Path;
use std::process::{ChildStdin, ChildStdout, Command, Stdio};

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

    /// One project's design-time builds, with the global properties a build of its solution gives it.
    pub fn build(&mut self, project: &Path, properties: &Value) -> Result<Built, ServerFailed> {
        let id = self.next_id;
        self.next_id += 1;
        let request =
            json!({ "id": id, "method": "build", "params": { "projects": [project], "properties": properties } });
        writeln!(self.input, "{request}")
            .and_then(|_| self.input.flush())
            .map_err(|e| ServerFailed(format!("the project host could not be asked: {e}")))?;
        let mut built = None;
        loop {
            let reply = self.reply()?;
            if reply["id"] != id {
                continue;
            }
            if let Some(error) = reply["error"].as_str() {
                return Err(ServerFailed(format!("the project host could not build {}: {error}", project.display())));
            }
            if let Some(partial) = reply.get("partial") {
                built = Some(Built {
                    infos: partial["infos"].as_str().map(str::to_string),
                    problem: partial["problem"].as_str().map(str::to_string),
                    diagnostics: partial.get("diagnostics").cloned().unwrap_or(Value::Null),
                });
            }
            if reply.get("result").is_some() {
                return built
                    .ok_or_else(|| ServerFailed(format!("the project host built nothing of {}", project.display())));
            }
        }
    }
}
