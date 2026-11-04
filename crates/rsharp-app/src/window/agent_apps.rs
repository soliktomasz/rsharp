//! The terminal, profile and app tools an agent calls in a window (Passenger's `terminal_*`, `profile` and `app_*`): a
//! command run in a Terminal tab and its screen read back, the program profiled for a few seconds or until it ends, and the
//! debugged Avalonia app's windows, tree, details, picture and input asked of the inspector in it.

use super::agent::said;
use super::*;
use serde_json::{Value, json};
use std::time::Duration;

type Reply = Task<Result<Value, String>>;

fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments[key].as_str().map(str::to_string)
}

impl MainWindow {
    pub(super) fn agent_apps_tool(&mut self, tool: &str, arguments: &Value, cx: &mut Context<Self>) -> Option<Reply> {
        let done = |result: Result<Value, String>| Some(Task::ready(result));
        match tool {
            "terminal_run" => {
                let Some(command) = text(arguments, "command").filter(|c| !c.trim().is_empty()) else {
                    return done(Err("terminal_run needs a command".into()));
                };
                let wait = Duration::from_millis(arguments["waitMs"].as_u64().unwrap_or(3000).min(120_000));
                let name = text(arguments, "name").unwrap_or_else(|| "Agent".into());
                let folder = self.folder().map(Path::to_path_buf).unwrap_or_else(rsharp_core::programs::home);
                let mut builder = if cfg!(windows) {
                    let mut b = rsharp_terminal::CommandBuilder::new("powershell");
                    b.args(["-NoProfile", "-Command", &command]);
                    b
                } else {
                    let mut b = rsharp_terminal::CommandBuilder::new("/bin/sh");
                    b.args(["-c", &command]);
                    b
                };
                builder.cwd(&folder);
                let id = self.terminal.update(cx, |p, cx| p.start(name, builder, folder, true, cx));
                self.dock.show(Tool::Terminal);
                cx.notify();
                Some(self.agent_terminal_reply(id, wait, cx))
            }
            "terminal_screen" => {
                let Some(id) = arguments["tab"].as_u64() else {
                    return done(Err("terminal_screen needs the tab terminal_run gave".into()));
                };
                Some(self.agent_terminal_reply(id as usize, Duration::ZERO, cx))
            }
            "profile" => {
                if self.profiler.is_some() || self.is_running() {
                    return done(Err("A run or a profile is under way: stop it first".into()));
                }
                let seconds = arguments["seconds"].as_f64().unwrap_or(5.).clamp(1., 300.);
                self.profile = None;
                self.profile(cx);
                Some(cx.spawn(async move |this, cx| {
                    let mut waited = Duration::ZERO;
                    let limit = Duration::from_secs_f64(seconds);
                    let mut stopped = false;
                    loop {
                        let state = this.update(cx, |w, cx| {
                            if w.profile_snapshot_ready() {
                                return Some(Ok(()));
                            }
                            if waited >= limit && !stopped {
                                stopped = true;
                                w.stop_profile(cx);
                            }
                            // A run that never started (a build failed) leaves nothing to wait for.
                            (waited > Duration::from_secs(5) && !w.is_running() && w.profile.is_none() && !w.profiling)
                                .then(|| {
                                    Err(w
                                        .run_outcome()
                                        .map_or("The profile did not start".to_string(), |o| o.message.clone()))
                                })
                        });
                        match state.map_err(|e| e.to_string())? {
                            Some(result) => {
                                result?;
                                break;
                            }
                            None if waited > limit + Duration::from_secs(300) => {
                                return Err("The profile did not finish".into());
                            }
                            None => {}
                        }
                        cx.background_executor().timer(Duration::from_millis(250)).await;
                        waited += Duration::from_millis(250);
                    }
                    this.update(cx, |w, _| said(w.profile_summary(25).to_string())).map_err(|e| e.to_string())
                }))
            }
            "hot_reload" => {
                self.apply_hot_reload(cx);
                done(Ok(said(self.said.as_ref().map(|s| s.to_string()).unwrap_or_else(|| "Applied".into()))))
            }
            "app_windows" | "app_tree" | "app_details" | "app_capture" | "app_input" => {
                let Some(app) = self.live_app() else {
                    return done(Err(
                        "No Avalonia app shows its UI: start one with debug_start, which runs it with the inspector"
                            .into(),
                    ));
                };
                let (tool, arguments) = (tool.to_string(), arguments.clone());
                let failures: Vec<String> = self
                    .live_failures()
                    .iter()
                    .map(|f| {
                        let (message, place, _) = super::live_tree::failure_row(f);
                        format!("{place} {message}")
                    })
                    .collect();
                Some(cx.background_executor().spawn(async move {
                    let mut content = rsharp_mcp::drive_app(&app, &tool, &arguments)?;
                    if tool == "app_input" && !failures.is_empty() {
                        if let Some(all) = content.as_array_mut() {
                            all.push(json!({ "type": "text", "text": format!("Bindings that have failed:\n{}", failures.join("\n")) }));
                        }
                    }
                    Ok(content)
                }))
            }
            "app_binding_errors" => {
                let failures: Vec<String> = self
                    .live_failures()
                    .iter()
                    .map(|f| {
                        let (message, place, count) = super::live_tree::failure_row(f);
                        format!("{place} {message} (×{count})")
                    })
                    .collect();
                done(Ok(said(if failures.is_empty() {
                    "No binding has failed.".to_string()
                } else {
                    failures.join("\n")
                })))
            }
            _ => None,
        }
    }

    /// A Terminal tab as it stands, once it has ended or `wait` has passed: its screen and its exit code.
    fn agent_terminal_reply(&mut self, id: usize, wait: Duration, cx: &mut Context<Self>) -> Reply {
        let terminal = self.terminal.clone();
        cx.spawn(async move |_, cx| {
            let mut waited = Duration::ZERO;
            loop {
                let state = terminal
                    .read_with(cx, |panel, _| {
                        if !panel.has_tab(id) {
                            return Err(format!("There is no terminal tab {id}"));
                        }
                        if let Some(problem) = panel.problem(id) {
                            return Err(problem.to_string());
                        }
                        Ok(panel.session(id).map(|s| (s.exit_code(), s.text())))
                    })
                    .map_err(|e| e.to_string())??;
                let over = state.as_ref().is_some_and(|(code, _)| code.is_some()) || waited >= wait;
                if over {
                    let (code, screen) = state.unwrap_or((None, String::new()));
                    // Trailing blank rows of the grid are not output.
                    let screen =
                        screen.lines().map(str::trim_end).collect::<Vec<_>>().join("\n").trim_end().to_string();
                    return Ok(said(
                        json!({ "tab": id, "exited": code.is_some(), "exitCode": code, "screen": screen }).to_string(),
                    ));
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
                waited += Duration::from_millis(100);
            }
        })
    }
}
