//! The tools that drive the Avalonia app being built (Passenger's `app_*`): it is built and started with Passenger's
//! inspector (`rsharp_dap::inspector`), which answers for its windows, its visual tree with each element's place and
//! text, an element's properties, a picture the app draws of a window, and input given as a person gives it.

use rsharp_dap::inspector::{LiveApp, Told};
use serde_json::{Value, json};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{Receiver, channel};
use std::time::Duration;

/// How long `app_run` waits for the app's UI once it has started.
const SHOWN: Duration = Duration::from_secs(60);

/// The app running, its inspector, and what it said of itself.
pub struct App {
    live: LiveApp,
    _child: rsharp_core::process::Child,
    told: Receiver<Told>,
    failed: Vec<String>,
}

/// The tools' schemas, added to the server's list.
pub fn tools() -> Vec<Value> {
    let element = json!({ "type": "string", "description": "The element: steps joined by /, each an @id from app_tree, or a type, #name, .class and \"text\" in any mix, with [n] for the nth match (Button#Save, \"Save\", #Panel/Button[2])" });
    vec![
        json!({ "name": "app_run", "annotations": { "readOnlyHint": false }, "description": "Builds an Avalonia project and runs it with RSharp's inspector, answering once its UI is up; the app tools then drive it.",
          "inputSchema": { "type": "object", "properties": { "project": { "type": "string", "description": "The project file, absolute or from the folder; the folder's one executable project by default" } } } }),
        json!({ "name": "app_windows", "annotations": { "readOnlyHint": true }, "description": "The running app's windows and popups: id, type, title, size.",
          "inputSchema": { "type": "object", "properties": {} } }),
        json!({ "name": "app_tree", "annotations": { "readOnlyHint": true }, "description": "The running app's visual tree, an element a line, indented: its id, type, #name, .classes, its box in its window, its text and state.",
          "inputSchema": { "type": "object", "properties": {
              "filter": { "type": "string", "description": "Only what this query finds, with what is inside it (see control)" },
              "properties": { "type": "array", "items": { "type": "string" }, "description": "Avalonia properties to read on each element (Owner.Name when attached)" } } } }),
        json!({ "name": "app_details", "annotations": { "readOnlyHint": true }, "description": "An element's properties with each value's source, and where its XAML is.",
          "inputSchema": { "type": "object", "properties": { "control": element }, "required": ["control"] } }),
        json!({ "name": "app_capture", "annotations": { "readOnlyHint": true }, "description": "A picture the app draws of a window, or of an element's part of it, as a PNG.",
          "inputSchema": { "type": "object", "properties": { "control": element, "scale": { "type": "number" } }, "required": ["control"] } }),
        json!({ "name": "app_input", "annotations": { "readOnlyHint": false }, "description": "Input as a person gives it: kind click (at control's centre, or x,y from its top left), type (text, replace), key (keys: [{key, modifiers}]) or scroll (deltaX, deltaY). Answers what it hit and what that shows after.",
          "inputSchema": { "type": "object", "properties": {
              "kind": { "type": "string", "enum": ["click", "type", "key", "scroll"] }, "control": element,
              "x": { "type": "number" }, "y": { "type": "number" }, "button": { "type": "string" }, "count": { "type": "integer" },
              "text": { "type": "string" }, "replace": { "type": "boolean" }, "keys": { "type": "array" },
              "deltaX": { "type": "number" }, "deltaY": { "type": "number" } }, "required": ["kind"] } }),
        json!({ "name": "app_stop", "annotations": { "readOnlyHint": false }, "description": "Ends the app app_run started.",
          "inputSchema": { "type": "object", "properties": {} } }),
    ]
}

/// The project's built assembly, as MSBuild names its target, once `dotnet build` has made it.
fn build(project: &Path) -> Result<PathBuf, String> {
    let dotnet = rsharp_core::programs::dotnet().ok_or("No .NET command line found")?;
    let folder = project.parent().unwrap_or(Path::new("."));
    let built = rsharp_core::dotnet::command(&dotnet, ["build", "-nologo", "-v", "q"], folder)
        .arg(project)
        .output()
        .map_err(|e| format!("dotnet build could not run: {e}"))?;
    if !built.status.success() {
        let said = String::from_utf8_lossy(&built.stdout);
        let errors: Vec<&str> = said.lines().filter(|l| l.contains(": error ")).take(20).collect();
        return Err(format!("The build failed:\n{}", errors.join("\n")));
    }
    let asked = rsharp_core::dotnet::command(&dotnet, ["msbuild", "-nologo", "-getProperty:TargetPath"], folder)
        .arg(project)
        .output()
        .map_err(|e| format!("dotnet msbuild could not run: {e}"))?;
    let target = String::from_utf8_lossy(&asked.stdout)
        .lines()
        .map(str::trim)
        .rfind(|l| !l.is_empty())
        .unwrap_or_default()
        .to_string();
    let target = PathBuf::from(target);
    target.is_file().then_some(target).ok_or_else(|| "MSBuild named no built assembly".into())
}

impl App {
    /// Builds and starts the project with the inspector, and waits for its UI.
    pub fn run(project: &Path) -> Result<(Self, String), String> {
        let built = build(project)?;
        if !rsharp_dap::inspector::inspects(&built) {
            return Err(format!("{} is not an Avalonia app RSharp's inspector serves", built.display()));
        }
        let (send, told) = channel();
        let (live, variables) =
            LiveApp::listen(&Default::default(), move |said| drop(send.send(said))).map_err(|e| e.to_string())?;
        let dotnet = rsharp_core::programs::dotnet().ok_or("No .NET command line found")?;
        let mut command =
            rsharp_core::dotnet::command(&dotnet, [built.as_os_str()], built.parent().unwrap_or(Path::new(".")));
        command
            .envs(variables)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let child = rsharp_core::process::spawn(command).map_err(|e| format!("The app did not start: {e}"))?;
        let mut app = Self { live, _child: child, told, failed: Vec::new() };
        let deadline = std::time::Instant::now() + SHOWN;
        loop {
            let left = deadline.saturating_duration_since(std::time::Instant::now());
            match app.told.recv_timeout(left) {
                Ok(Told::Hello { problem: Some(problem), .. }) => return Err(problem),
                Ok(Told::Hello { avalonia, .. }) => {
                    let windows = windows(&app.live).unwrap_or_default();
                    let version = avalonia.unwrap_or_default();
                    return Ok((app, format!("Running on Avalonia {version}.\n{windows}")));
                }
                Ok(Told::Ended) => return Err("The app ended before it showed its UI".into()),
                Ok(Told::BindingFailed(failure)) => {
                    app.failed.push(failure["message"].as_str().unwrap_or_default().into())
                }
                Ok(_) => {}
                Err(_) => return Err("The app showed no UI within a minute".into()),
            }
        }
    }

    /// What the app said since it was last asked: its bindings that failed.
    fn drained(&mut self) -> Vec<String> {
        while let Ok(said) = self.told.try_recv() {
            if let Told::BindingFailed(failure) = said {
                let place = &failure["place"];
                let at = place["file"]
                    .as_str()
                    .map(|f| format!("{f}({},{}): ", place["line"], place["column"]))
                    .unwrap_or_default();
                self.failed.push(format!(
                    "{at}{}.{} to {}: {}",
                    failure["control"].as_str().unwrap_or_default(),
                    failure["property"].as_str().unwrap_or_default(),
                    failure["path"].as_str().unwrap_or_default(),
                    failure["message"].as_str().unwrap_or_default()
                ));
            }
        }
        std::mem::take(&mut self.failed)
    }

    /// A tool's answer, with the bindings that failed since the last one told after it.
    pub fn call(&mut self, tool: &str, args: &Value) -> Result<Value, String> {
        let answer = drive(&self.live, tool, args)?;
        let failed = self.drained();
        Ok(if failed.is_empty() {
            answer
        } else {
            let mut all = answer.as_array().cloned().unwrap_or_default();
            all.push(json!({ "type": "text", "text": format!("Bindings that failed:\n{}", failed.join("\n")) }));
            Value::Array(all)
        })
    }
}

pub fn windows(live: &LiveApp) -> Result<String, String> {
    let windows = live.ask("windows", json!({}))?;
    Ok(windows["windows"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|w| {
            format!(
                "{} {} \"{}\" {}x{}{}",
                w["id"],
                w["type"].as_str().unwrap_or_default(),
                w["title"].as_str().unwrap_or_default(),
                w["width"],
                w["height"],
                if w["active"] == true { " (active)" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n"))
}

/// The app's tree with each element's place, text and state, and the properties asked for.
fn tree(live: &LiveApp, properties: Value) -> Result<Value, String> {
    live.ask("tree", json!({ "layout": true, "properties": properties }))
}

/// The id of the one element a tool's `control` query names.
fn named(live: &LiveApp, args: &Value, tool: &str) -> Result<i64, String> {
    let query = args["control"].as_str().ok_or_else(|| format!("{tool} needs control"))?;
    let tree = tree(live, Value::Null)?;
    let roots = tree["tree"]["roots"].as_array().cloned().unwrap_or_default();
    let nodes = crate::query::flatten(&roots);
    let found = crate::query::one(&nodes, query)?;
    nodes[found].element["id"].as_i64().ok_or_else(|| "The app named the element no id".into())
}

/// A tool's answer as MCP content, asked of an app's inspector: text, or an image for a capture.
pub fn drive(live: &LiveApp, tool: &str, args: &Value) -> Result<Value, String> {
    let text = |t: String| json!([{ "type": "text", "text": t }]);
    let answer = match tool {
        "app_windows" => text(windows(live)?),
        "app_tree" => {
            let tree = tree(live, args["properties"].clone())?;
            let roots = tree["tree"]["roots"].as_array().cloned().unwrap_or_default();
            let nodes = crate::query::flatten(&roots);
            let shown: Vec<&Value> = match args["filter"].as_str() {
                Some(filter) => crate::query::find(&nodes, filter)?.into_iter().map(|n| nodes[n].element).collect(),
                None => roots.iter().collect(),
            };
            let mut lines = Vec::new();
            fn walk(node: &Value, depth: usize, lines: &mut Vec<String>) {
                let name = node["name"].as_str().map(|n| format!("#{n}")).unwrap_or_default();
                let classes: String = node["classes"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(Value::as_str)
                    .map(|c| format!(".{c}"))
                    .collect();
                let b = &node["bounds"];
                let bounds = if b.is_object() {
                    format!(" [{},{} {}x{}]", b["x"], b["y"], b["width"], b["height"])
                } else {
                    String::new()
                };
                let shows = node["text"].as_str().map(|t| format!(" \"{t}\"")).unwrap_or_default();
                let state = [("disabled", "disabled"), ("focused", "focused"), ("outOfView", "out of view")]
                    .iter()
                    .filter(|(key, _)| node[*key] == true)
                    .map(|(_, word)| format!(" {word}"))
                    .collect::<String>();
                let values =
                    node["values"].as_object().map(|v| format!(" {}", Value::Object(v.clone()))).unwrap_or_default();
                lines.push(format!(
                    "{}@{} {}{name}{classes}{bounds}{shows}{state}{values}",
                    "  ".repeat(depth),
                    node["id"],
                    node["type"].as_str().unwrap_or_default()
                ));
                for child in node["children"].as_array().into_iter().flatten() {
                    walk(child, depth + 1, lines);
                }
            }
            for node in shown {
                walk(node, 0, &mut lines);
            }
            text(lines.join("\n"))
        }
        "app_details" => {
            let element = named(live, args, "app_details")?;
            let details = live.ask("details", json!({ "element": element }))?;
            let d = &details["details"];
            let source = d["source"]["file"]
                .as_str()
                .map(|f| format!("{f}:{}", d["source"]["line"]))
                .unwrap_or_else(|| d["noSource"].as_str().unwrap_or("no XAML place").into());
            let properties: Vec<String> = d["properties"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|p| {
                    format!(
                        "{} = {} ({})",
                        p["name"].as_str().unwrap_or_default(),
                        p["value"].as_str().unwrap_or_default(),
                        p["source"].as_str().unwrap_or_default()
                    )
                })
                .collect();
            text(format!("{} at {source}\n{}", d["fullType"].as_str().unwrap_or_default(), properties.join("\n")))
        }
        "app_capture" => {
            let element = named(live, args, "app_capture")?;
            let picture = live.ask("capture", json!({ "element": element, "scale": args["scale"] }))?;
            // The protocol's PNG is a byte array, which System.Text.Json writes in base64.
            let png = picture["picture"]["png"].as_str().ok_or("The app drew nothing")?;
            json!([{ "type": "image", "data": png, "mimeType": "image/png" }])
        }
        "app_input" => {
            let mut input = args.clone();
            // Keys and typing go to whatever has the focus when no control is named.
            let element = if args["control"].is_string() { Some(named(live, args, "app_input")?) } else { None };
            input.as_object_mut().map(|o| o.remove("control"));
            input["element"] = json!(element);
            let done = live.ask("input", json!({ "input": input }))?;
            let d = &done["done"];
            text(format!(
                "{} at {},{}; it shows {}; focus on {}",
                d["targetName"].as_str().unwrap_or("nothing"),
                d["x"],
                d["y"],
                d["shows"].as_str().unwrap_or("nothing"),
                d["focused"].as_str().unwrap_or("nothing")
            ))
        }
        _ => return Err(format!("no tool {tool}")),
    };
    Ok(answer)
}
