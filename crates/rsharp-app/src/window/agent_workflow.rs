//! The build, run and test tools an agent calls in a window (Passenger's `build`, `run`, `run_output`, `stop`, `tests_run` and
//! `tests_list`), and Git's stash: each starts what the window's own commands start, waits for it to end, and answers with
//! its result, never with what the status bar says by then.

use super::agent::said;
use super::*;
use crate::git_view::GitRequest;
use rsharp_core::build::Severity;
use rsharp_core::testing::TestState;
use serde_json::{Value, json};
use std::time::Duration;

type Reply = Task<Result<Value, String>>;

fn text(arguments: &Value, key: &str) -> Option<String> {
    arguments[key].as_str().map(str::to_string)
}

fn tail(all: &str, lines: usize) -> String {
    let lines: Vec<&str> = all.lines().rev().take(lines).collect();
    lines.into_iter().rev().collect::<Vec<_>>().join("\n")
}

impl MainWindow {
    pub(super) fn agent_workflow_tool(
        &mut self,
        tool: &str,
        arguments: &Value,
        cx: &mut Context<Self>,
    ) -> Option<Reply> {
        let done = |result: Result<Value, String>| Some(Task::ready(result));
        match tool {
            "build" => {
                if self.is_building() {
                    return done(Err("A build is already running".into()));
                }
                let selected = text(arguments, "project").map(|p| vec![self.agent_path(&p)]);
                self.begin_build(rsharp_core::build::Kind::Build, selected, cx);
                Some(cx.spawn(async move |this, cx| {
                    for _ in 0..6000 {
                        if !this.update(cx, |w, _| w.is_building()).map_err(|e| e.to_string())? {
                            break;
                        }
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                    }
                    this.update(cx, |w, cx| {
                        let Some(outcome) = w.build_outcome().cloned() else { return Err("The build did not start".to_string()) };
                        let problems: Vec<Value> = outcome
                            .problems
                            .iter()
                            .take(100)
                            .map(|p| {
                                let at = p.path.as_ref().map(|f| format!("{}:{}", w.relative(f), p.line + 1)).unwrap_or_default();
                                json!({ "severity": if p.severity == Severity::Error { "error" } else { "warning" },
                                        "code": p.code, "at": at, "message": p.message })
                            })
                            .collect();
                        let failed = !matches!(outcome.status, super::build::BuildStatus::Succeeded);
                        let output = if failed { tail(&w.build_output(cx), 60) } else { String::new() };
                        Ok(said(json!({ "status": format!("{:?}", outcome.status), "message": outcome.message, "problems": problems, "output": output }).to_string()))
                    })
                    .map_err(|e| e.to_string())?
                }))
            }
            "run" => {
                if self.is_running() {
                    return done(Err("A run is already under way: stop it first".into()));
                }
                let wait = Duration::from_millis(arguments["waitMs"].as_u64().unwrap_or(5000).min(600_000));
                match text(arguments, "configuration") {
                    Some(name) => match self.run_configs.iter().find(|c| c.name == name).cloned() {
                        Some(configuration) => self.launch(configuration, cx),
                        None => return done(Err(format!("No run configuration is named {name}"))),
                    },
                    None => self.run(cx),
                }
                Some(cx.spawn(async move |this, cx| {
                    let mut waited = Duration::ZERO;
                    loop {
                        let (running, outcome) = this.update(cx, |w, _| (w.is_running(), w.run_outcome().cloned())).map_err(|e| e.to_string())?;
                        // A run that has ended, or never began (a failed build, no configuration), answers at once.
                        if (!running && outcome.is_some() && waited > Duration::from_millis(300)) || waited >= wait {
                            break;
                        }
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                        waited += Duration::from_millis(100);
                    }
                    this.update(cx, |w, cx| {
                        let outcome = w.run_outcome().cloned();
                        said(json!({ "running": w.is_running(), "status": outcome.as_ref().map(|o| format!("{:?}", o.status)),
                                     "message": outcome.map(|o| o.message), "output": tail(&w.run_output(cx), 100) }).to_string())
                    })
                    .map_err(|e| e.to_string())
                }))
            }
            "run_output" => {
                let lines = arguments["lines"].as_u64().unwrap_or(200).max(1) as usize;
                done(Ok(said(tail(&self.run_output(cx), lines))))
            }
            "stop" => {
                let was = (self.is_running(), self.is_building(), self.is_testing());
                self.stop_everything(cx);
                done(Ok(said(format!("Stopped: run {}, build {}, tests {}", was.0, was.1, was.2))))
            }
            "tests_list" => {
                let filter = text(arguments, "filter").map(|f| f.to_lowercase());
                let state = text(arguments, "state").map(|s| s.to_lowercase());
                let limit = arguments["limit"].as_u64().unwrap_or(200).clamp(1, 2000) as usize;
                let listed = self.tests.view.read(cx).listing();
                let rows: Vec<Value> = listed
                    .iter()
                    .filter(|(t, _)| filter.as_ref().is_none_or(|f| t.fully_qualified_name.to_lowercase().contains(f)))
                    .map(|(t, r)| {
                        let state = r.as_ref().map_or("notrun", |r| state_name(r.state));
                        (t, r, state)
                    })
                    .filter(|(_, _, s)| state.as_ref().is_none_or(|wanted| wanted == s))
                    .take(limit)
                    .map(|(t, r, s)| {
                        json!({ "test": t.display_name, "name": t.fully_qualified_name, "project": self.relative(&t.project),
                                "state": s, "message": r.as_ref().and_then(|r| r.message.clone()),
                                "at": t.path.as_ref().map(|p| format!("{}:{}", self.relative(p), t.line)) })
                    })
                    .collect();
                done(Ok(said(json!({ "count": rows.len(), "tests": rows }).to_string())))
            }
            "tests_run" => {
                if self.is_testing() {
                    return done(Err("Tests are already running: stop them first".into()));
                }
                let filter = text(arguments, "filter").map(|f| f.to_lowercase());
                let failed_only = arguments["failed"] == true;
                Some(cx.spawn(async move |this, cx| {
                    // The test projects are found as the folder loads: wait for them, up to a minute.
                    for _ in 0..600 {
                        if this.update(cx, |w, _| w.tests_available()).map_err(|e| e.to_string())? {
                            break;
                        }
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                    }
                    let started = this
                        .update(cx, |w, cx| {
                            let chosen = if failed_only {
                                Some(w.tests.view.read(cx).failed_tests())
                            } else {
                                filter.as_ref().map(|f| {
                                    w.tests.view.read(cx).tests().into_iter().filter(|t| t.fully_qualified_name.to_lowercase().contains(f)).collect()
                                })
                            };
                            w.run_tests(chosen, cx);
                            w.is_testing()
                        })
                        .map_err(|e| e.to_string())?;
                    if !started {
                        return this.update(cx, |w, cx| Err(w.tests.view.read(cx).status().to_string())).map_err(|e| e.to_string())?;
                    }
                    for _ in 0..36_000 {
                        if !this.update(cx, |w, _| w.is_testing()).map_err(|e| e.to_string())? {
                            break;
                        }
                        cx.background_executor().timer(Duration::from_millis(100)).await;
                    }
                    this.update(cx, |w, cx| {
                        let view = w.tests.view.read(cx);
                        let listed = view.listing();
                        let count = |s: TestState| listed.iter().filter(|(_, r)| r.as_ref().is_some_and(|r| r.state == s)).count();
                        let failures: Vec<Value> = listed
                            .iter()
                            .filter(|(_, r)| r.as_ref().is_some_and(|r| r.state == TestState::Failed))
                            .take(20)
                            .map(|(t, r)| json!({ "test": t.fully_qualified_name, "message": r.as_ref().and_then(|r| r.message.clone()),
                                                  "stack": r.as_ref().and_then(|r| r.stack_trace.clone()) }))
                            .collect();
                        said(json!({ "summary": view.status(), "passed": count(TestState::Passed), "failed": count(TestState::Failed),
                                     "skipped": count(TestState::Skipped), "failures": failures }).to_string())
                    })
                    .map_err(|e| e.to_string())
                }))
            }
            "vcs_stash" => {
                let Some(folder) = self.folder().map(Path::to_path_buf) else {
                    return done(Err("No folder is open".into()));
                };
                match text(arguments, "action").as_deref().unwrap_or("push") {
                    "list" => Some(cx.background_executor().spawn(async move {
                        let repo = rsharp_core::git::Repository::open(&folder)?
                            .ok_or("The folder is not in a Git repository")?;
                        Ok(said(json!(repo.stash_list()?).to_string()))
                    })),
                    action @ ("push" | "pop") => {
                        let request = GitRequest::Stash {
                            pop: action == "pop",
                            message: text(arguments, "message"),
                            untracked: arguments["includeUntracked"] == true,
                        };
                        Some(self.agent_git(cx, move |_| request))
                    }
                    other => done(Err(format!("action is push, pop or list, not {other}"))),
                }
            }
            _ => None,
        }
    }
}

fn state_name(state: TestState) -> &'static str {
    match state {
        TestState::NotRun => "notrun",
        TestState::Running => "running",
        TestState::Passed => "passed",
        TestState::Failed => "failed",
        TestState::Skipped => "skipped",
    }
}
