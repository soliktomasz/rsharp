//! The debug tools an agent calls in a window (Passenger's `PassengerTools.Ide` debug group): breakpoints, starting and
//! attaching, going on and stepping, and reading where the program stopped, answered from the window's own session as
//! its Debug window shows it. A tool that moves the program answers once it has stopped again, ended, or run on past
//! `waitMs` (five seconds by default), so an agent sees where it is.

use super::*;
use crate::window::agent::said;
use serde_json::{Value, json};
use std::time::Duration;

type Reply = Task<Result<Value, String>>;

fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments[key].as_str().map(str::to_string)
}

impl MainWindow {
    pub(in crate::window) fn agent_debug_tool(
        &mut self,
        tool: &str,
        arguments: &Value,
        cx: &mut Context<Self>,
    ) -> Option<Reply> {
        let done = |result: Result<Value, String>| Some(Task::ready(result));
        let wait = Duration::from_millis(arguments["waitMs"].as_u64().unwrap_or(5000).min(120_000));
        match tool {
            "breakpoints" => done(Ok(said(self.agent_breakpoints()))),
            "breakpoint_set" | "breakpoint_remove" => {
                done(self.agent_breakpoint(tool == "breakpoint_set", arguments, cx))
            }
            "debug_state" | "debug_variables" => done(Ok(said(self.agent_debug_state(cx)))),
            "debug_output" => {
                let lines = arguments["lines"].as_u64().unwrap_or(200).max(1) as usize;
                let all = self.debug_console.read(cx).text();
                let tail: Vec<&str> = all.lines().rev().take(lines).collect();
                done(Ok(said(tail.into_iter().rev().collect::<Vec<_>>().join("\n"))))
            }
            "debug_input" => {
                let Some(input) = text(arguments, "text") else { return done(Err("debug_input needs text".into())) };
                self.debug_input(&input, cx);
                done(Ok(said("Sent")))
            }
            "debug_processes" => Some(cx.spawn(async move |_, cx| {
                let listed = cx.background_executor().spawn(async move { rsharp_core::debug::processes() }).await;
                let rows: Vec<Value> = listed
                    .iter()
                    .map(|p| json!({ "pid": p.id, "name": p.program(), "commandLine": p.command_line }))
                    .collect();
                Ok(said(json!(rows).to_string()))
            })),
            "debug_start" => {
                let chosen = text(arguments, "configuration");
                let configuration = match &chosen {
                    Some(name) => self.run_configs.iter().find(|c| &c.name == name).cloned(),
                    None => self.chosen_run_configuration().cloned(),
                };
                let Some(configuration) = configuration else {
                    return done(Err(match chosen {
                        Some(name) => format!("No run configuration is named {name}"),
                        None => "Nothing to debug: this folder has no run configuration".into(),
                    }));
                };
                if self.is_debugging() {
                    return done(Err("A debugging session is already running: debug_stop it first".into()));
                }
                self.debug_configuration(configuration, cx);
                Some(self.agent_settled(wait, true, cx))
            }
            "debug_attach" => {
                let Some(pid) = arguments["pid"].as_u64() else { return done(Err("debug_attach needs pid".into())) };
                if self.is_debugging() {
                    return done(Err("A debugging session is already running: debug_stop it first".into()));
                }
                let handle = self.handle;
                Some(cx.spawn(async move |this, cx| {
                    let listed = cx.background_executor().spawn(async move { rsharp_core::debug::processes() }).await;
                    let process = listed
                        .into_iter()
                        .find(|p| u64::from(p.id) == pid)
                        .ok_or_else(|| format!("No .NET program has process id {pid}: debug_processes lists them"))?;
                    handle.update(cx, |m, _, cx| m.attach_to(process, cx)).map_err(|e| e.to_string())?;
                    let settled =
                        this.update(cx, |w, cx| w.agent_settled(wait, true, cx)).map_err(|e| e.to_string())?;
                    settled.await
                }))
            }
            "debug_detach" => {
                if self.debug.attached.is_none() {
                    return done(Err("No program is attached to".into()));
                }
                self.detach_debug(cx);
                done(Ok(said("Detached")))
            }
            "debug_stop" => {
                self.stop_debug(cx);
                done(Ok(said("Stopped")))
            }
            "debug_continue" | "debug_step" | "debug_pause" | "debug_set_next_statement" => {
                let paused = self.debug.phase == Phase::Paused;
                match tool {
                    "debug_pause" if self.debug.phase != Phase::Running => {
                        return done(Err("The program is not running".into()));
                    }
                    "debug_pause" => self.pause_debug(cx),
                    _ if !paused => {
                        return done(Err("The program is not stopped: debug_state says where it is".into()));
                    }
                    "debug_continue" => self.resume_debug(cx),
                    "debug_step" => {
                        let how = match arguments["how"].as_str().unwrap_or("over") {
                            "over" => Step::Over,
                            "into" => Step::Into,
                            "out" => Step::Out,
                            other => return done(Err(format!("how is over, into or out, not {other}"))),
                        };
                        self.step_debug(how, cx);
                    }
                    _ => {
                        let (Some(file), Some(line)) = (text(arguments, "file"), arguments["line"].as_u64()) else {
                            return done(Err("debug_set_next_statement needs file and line".into()));
                        };
                        let path = self.agent_path(&file);
                        self.set_next_statement_at(path, (line as usize).saturating_sub(1), cx);
                    }
                }
                Some(self.agent_settled(wait, false, cx))
            }
            "debug_evaluate" => {
                let Some(expression) = text(arguments, "expression") else {
                    return done(Err("debug_evaluate needs expression".into()));
                };
                let (Some(frame), Some(session)) =
                    (self.debug.frames.first().map(|f| f.id), self.debug.session.clone())
                else {
                    return done(Err("The program is not stopped: an expression is evaluated where it is".into()));
                };
                Some(cx.spawn(async move |_, cx| {
                    let read = cx
                        .background_executor()
                        .spawn(async move {
                            let value = session.evaluate(&expression, frame)?;
                            let members: Vec<Value> = if value.variables_reference == 0 {
                                Vec::new()
                            } else {
                                session
                                    .variables(value.variables_reference)
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(|v| json!({ "name": v.name, "value": v.value, "type": v.r#type }))
                                    .collect()
                            };
                            Ok::<_, String>(json!({ "result": value.result, "type": value.r#type, "members": members }))
                        })
                        .await?;
                    Ok(said(read.to_string()))
                }))
            }
            "debug_set_value" => {
                let (Some(expression), Some(value)) = (text(arguments, "expression"), text(arguments, "value")) else {
                    return done(Err("debug_set_value needs expression and value".into()));
                };
                if self.debug.phase != Phase::Paused {
                    return done(Err("The program is not stopped".into()));
                }
                self.set_value(expression.clone(), value.clone(), cx);
                Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(500)).await;
                    this.update(cx, |w, cx| said(format!("Set {expression} to {value}\n{}", w.agent_debug_state(cx))))
                        .map_err(|e| e.to_string())
                }))
            }
            _ => None,
        }
    }

    fn agent_breakpoints(&self) -> String {
        let rows: Vec<Value> = self
            .debug
            .breakpoints
            .files()
            .into_iter()
            .flat_map(|path| {
                let shown = self.relative(&path);
                self.debug.breakpoints.of(&path).into_iter().map(move |b| {
                    json!({ "file": shown, "line": b.line + 1, "enabled": b.enabled, "condition": b.condition,
                            "hitCount": b.hit_count, "logMessage": b.log_message })
                })
            })
            .collect();
        json!(rows).to_string()
    }

    fn agent_breakpoint(&mut self, set: bool, arguments: &Value, cx: &mut Context<Self>) -> Result<Value, String> {
        let (Some(file), Some(line)) = (text(arguments, "file"), arguments["line"].as_u64().filter(|l| *l >= 1)) else {
            return Err("A breakpoint needs file and line, from 1".into());
        };
        let path = self.agent_path(&file);
        let line = line as usize - 1;
        let has = self.debug.breakpoints.has(&path, line);
        if has != set {
            self.toggle_breakpoint_at(&path, line, cx);
        }
        if set {
            let (condition, hits, log) =
                (text(arguments, "condition"), text(arguments, "hitCount"), text(arguments, "logMessage"));
            if condition.is_some() || hits.is_some() || log.is_some() {
                self.debug.breakpoints.edit(&path, line, |b| {
                    b.condition = condition.clone().unwrap_or_else(|| b.condition.clone());
                    b.hit_count = hits.clone().unwrap_or_else(|| b.hit_count.clone());
                    b.log_message = log.clone().unwrap_or_else(|| b.log_message.clone());
                });
                self.save_breakpoints(cx);
                self.push_breakpoints(path.clone(), cx);
            }
        }
        Ok(said(format!(
            "{} {}:{}",
            if set { "Breakpoint set at" } else { "No breakpoint at" },
            self.relative(&path),
            line + 1
        )))
    }

    /// Where the debugger stands: its phase, the stopped thread's frames and the chosen frame's variables.
    fn agent_debug_state(&self, cx: &App) -> String {
        let phase = match self.debug.phase {
            Phase::Idle => "idle",
            Phase::Starting => "starting",
            Phase::Running => "running",
            Phase::Paused => "paused",
        };
        let view = self.debug_view.read(cx);
        let frames: Vec<Value> = view.frames().iter().map(|f| json!({ "name": f.name, "at": f.place })).collect();
        let variables: Vec<Value> =
            view.variable_names().into_iter().map(|n| json!({ "name": n, "value": view.variable_value(&n) })).collect();
        json!({ "phase": phase, "status": view.status().to_string(), "frames": frames, "variables": variables })
            .to_string()
    }

    /// Waits for the program to stop again or end, at most `wait` once it has started, then says where it stands.
    fn agent_settled(&mut self, wait: Duration, starting: bool, cx: &mut Context<Self>) -> Reply {
        cx.spawn(async move |this, cx| {
            let mut waited = Duration::ZERO;
            let mut begun = !starting;
            // A start first leaves Starting (its build and its launch take as long as they take).
            loop {
                let phase = this.update(cx, |w, _| w.debug.phase).map_err(|e| e.to_string())?;
                begun |= phase != Phase::Starting && phase != Phase::Idle;
                let over = match phase {
                    Phase::Paused => true,
                    Phase::Idle => begun || waited > Duration::from_secs(5),
                    Phase::Running => begun && waited >= wait,
                    Phase::Starting => false,
                };
                if over || waited > Duration::from_secs(600) {
                    break;
                }
                cx.background_executor().timer(Duration::from_millis(100)).await;
                if begun || !starting {
                    waited += Duration::from_millis(100);
                }
            }
            // The stop's frames and variables are read a moment after the phase turns.
            cx.background_executor().timer(Duration::from_millis(300)).await;
            this.update(cx, |w, cx| said(w.agent_debug_state(cx))).map_err(|e| e.to_string())
        })
    }
}
