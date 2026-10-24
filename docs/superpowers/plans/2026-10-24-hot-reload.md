# Hot Reload Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** Run under `dotnet watch` when Hot Reload is on, so a save reaches the running program, and say what watch made of it.

**Architecture:** `rsharp-core`'s `run` owns the watch command, the folder it starts in and the words watch answers with; the window decides watch at Run, skips the build, streams watch's output into the same console and reads its verdict from those lines.

**Tech Stack:** Rust 2024, std processes through `process::spawn`/`build::run_while`, the existing `dotnet::command` isolation, gpui 0.2.2. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-24-hot-reload-design.md`.

## Global Constraints

- Every program starts through `rsharp_core::process::spawn`; Stop ends watch's whole tree.
- The UI never blocks on a process; a run's events cross on the bounded channel as they do now.
- The words a person reads are Rider's and Passenger's, never dotnet watch's own.
- Colours are `theme.rs` tokens; tests that need the SDK skip saying why when it is missing.

## Review Focus

1. Watch must not double the plan's environment (it passes no launch profile) nor its arguments (they follow `--` without the assembly) (Task 1).
2. A working folder that cannot be made must refuse the run, not start watch somewhere else (Tasks 1, 3).
3. Watch's verdict must be read only while a run under watch is going, and never from another run's output (Task 4).
4. Stop, leaving the folder and a second Run must not leave `dotnet watch` running (Task 3).

### Task 1: The watch command, its folder and its words

**Files:** `crates/rsharp-core/src/run/plan.rs`, create `crates/rsharp-core/src/run/watch.rs`, `run/mod.rs`, `lib.rs` if needed.

**Interfaces:** `LaunchPlan::why_not(&self, verb: &str, built: bool)`; `LaunchPlan::watching(&self, restart_on_rude_edit: bool) -> Result<std::process::Command, String>`; `LaunchPlan::make_working_folder(&self) -> Result<(), String>`; `run::watch::Verdict { Applied, NothingToApply, Restarting, Errors }` with `said(self) -> &'static str`, and `run::watch::verdict(line: &str) -> Option<Verdict>`.

- [x] Acceptance tests: the command is `watch --non-interactive run --project … --no-launch-profile`, with the framework when the plan has one and the plan's own arguments after `--` and the assembly left out; its folder is the plan's working folder; its environment holds the plan's and watch's two variables only when the setting asks; `why_not(built: false)` accepts an unbuilt plan but still refuses a missing folder; a folder that cannot be made says why; every phrase watch answers with maps to its sentence and a line with none maps to none.
- [x] Implement and run `cargo test -p rsharp-core run::`.

### Task 2: The setting and its page

**Files:** `crates/rsharp-app/src/settings.rs`, `settings_dialog.rs`.

**Interfaces:** `AppSettings::hot_reload: bool` (off) and `hot_reload_restart_on_rude_edit: bool` (on); a `Hot Reload` page under Build, Execution, Deployment with both checks.

- [x] Window tests: the page lists both checks; ticking one and pressing OK keeps it in `settings.json`; the search finds the page by "hot reload".
- [x] Implement and test.

### Task 3: Run under watch

**Files:** `crates/rsharp-app/src/window/run.rs`, `window.rs`.

**Interfaces:** `MainWindow::is_watching(&self) -> bool`; the launch decides `hot_reload && configuration.program.is_none()`; `start_plan` uses `plan.watching(...)`, says "Running X with Hot Reload: a save applies its changes", and makes the folder first; `begin_launch` skips the build; `stop_run`/`leave_run` end it.

- [x] Window tests: with the setting on, the Run console's first line names `dotnet watch` and the status bar says Hot Reload; no build runs first; with the setting off, the command line is the plain program; Stop ends it; leaving the folder ends it.
- [x] Implement and test.

### Task 4: Apply Hot Reload Changes and watch's verdict

**Files:** `window/run.rs`, `actions.rs`, `keymap.rs`.

**Interfaces:** `EditRunConfigurations`'s neighbour `ApplyHotReloadChanges` (Run menu, Find Action, Alt+F10 both platforms), available only while `is_watching()`; the console's lines are read for `run::watch::verdict` and its sentence put in the status bar.

- [x] Window tests: the action is unavailable without a run under watch and says why; pressed while one runs, it saves a dirty buffer and says watch was given the file; a line watch answers with is said in the status bar, and no line of a run without watch is.
- [x] Implement and test.

### Task 5: A save reaches the running program

**Files:** `crates/rsharp-app/tests/run.rs`.

- [x] Window test, live: a console project printing a value from its own code in a loop is run with Hot Reload on; the file is changed and saved from the window; the program prints the new value without a second Run.
- [x] Implement and test; skip saying why without the SDK.

### Task 6: Documentation and the whole suite

**Files:** `ROADMAP.md`, `CLAUDE.md`, this plan.

- [x] `cargo fmt`, `cargo clippy`, `just test`, and a screenshot of the Run window under watch.
- [x] `ROADMAP.md`'s Phase 5 says Hot Reload is in and unit tests remain; `CLAUDE.md` names the watch command and `run/watch.rs`.
