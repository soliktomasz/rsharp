# Edit Configurations Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [x]`) syntax for tracking.

**Goal:** The folder keeps run configurations of its own, edited in Rider's dialog, and runs them as it runs the generated ones.

**Architecture:** `rsharp-core`'s `run` module owns the document, its folder-relative form and its refusal reasons. The window keeps the list (own then generated), reads and writes the document with the folder, and shows an in-window dialog over the editor — the shape `refactor_dialog.rs` already has — whose Apply/OK hand back the list and the chosen name.

**Tech Stack:** Rust 2024, serde, gpui 0.2.2, the existing `field::TextField`, `theme` tokens and `StateStore`. No new dependency.

**Spec:** `docs/superpowers/specs/2026-10-24-run-configurations-dialog-design.md`.

## Global Constraints

- Non-UI crates have no gpui dependency; the window never blocks on a process while a field is typed.
- Colours are `theme.rs` tokens; every visible change is looked at with `just screenshot`.
- Projects are stored relative to the folder and resolved back absolute; every other value as typed.
- A configuration that cannot be kept says why in the person's words and is not written.
- The generated list keeps its current order and behaviour; the person's follow it.

## Review Focus

1. A stored document from another folder, or one hand-edited with a path outside the folder, must not be run or lose the rest of the file (Tasks 1, 2).
2. Cancel must leave the folder's state byte-identical, and Apply must keep the open dialog editable (Tasks 4, 5).
3. The chosen configuration must survive a rename, a removal and a reopen (Tasks 3, 5).
4. Environment variables must keep their order and allow duplicates' removal without losing the rest (Task 4).
5. An edit that changes the project must clear a framework and launch profile the new project does not have (Tasks 3, 4).

### Task 1: The document, its stored form and its refusals

**Files:** `crates/rsharp-core/src/run/configuration.rs`, `crates/rsharp-core/src/lib.rs` if needed.

**Interfaces:** `RunConfigurations { configurations: Vec<RunConfiguration>, selected: Option<String> }` and `impl crate::state::StateDocument` with `FILE = "runConfigurations.json"`; `for_store(&self, folder: &Path) -> Self` and `resolved(&self, folder: &Path) -> Self`; `RunConfiguration::problem(&self, taken: &[String], projects: &[PathBuf], folder: &Path) -> Option<String>`; `RunConfiguration::unique_name` as it is.

- [x] Acceptance tests: a project under the folder is stored relative and resolved back absolute; a project outside it is kept as written; a path with `..` resolves to the folder's own; the document's own file name is `runConfigurations.json`; a configuration with no name, a name already taken (any case), no project, or a project outside the folder each says why.
- [x] Implement and run `cargo test -p rsharp-core run::`.

### Task 2: The window keeps and reads the folder's own

**Files:** `crates/rsharp-app/src/window.rs`, `window/run.rs`, `state.rs`.

**Interfaces:** `MainWindow::run_own(&self) -> &[RunConfiguration]`; `refresh_run_configurations` lists own then generated; the chosen name comes from the document's `selected`; `FolderState.run_configuration` is removed.

- [x] Window tests: a folder with a stored configuration lists it before the generated ones and shows it in the toolbar; a stored path is resolved against the folder; the chosen name comes back after reopening.
- [x] Implement and run `cargo test -p rsharp-app --test run`.

### Task 3: The dialog

**Files:** create `crates/rsharp-app/src/run_dialog.rs`; export in `lib.rs`; `theme.rs` if a token is missing.

**Interfaces:** `RunDialog::new(own, generated, projects, chosen, cx)`; events `Ended::Applied(Vec<RunConfiguration>, Option<String>)` and `Ended::Dismissed`; `rows()`, `chosen()`, `problem()` for tests; actions `SelectNext`/`SelectPrevious`/`Choose`/`Dismiss` reused from `find_action`.

- [x] Window tests: a configuration with no name cannot be applied and the dialog says so; two with one name are refused; Apply hands back the list without closing; Cancel hands back nothing; a project change clears a framework the new project lacks.
- [x] Implement and test.

### Task 4: The action and the wiring

**Files:** `window/run.rs`, `actions.rs`, `keymap.rs`, `window.rs`.

**Interfaces:** `EditRunConfigurations` action, in the Run menu, Find Action and the toolbar menu's last row; `MainWindow::edit_run_configurations(&mut self, window, cx)`; the dialog applies through `store.write_for::<RunConfigurations>`.

- [x] Window tests: Find Action runs it by name; the toolbar's menu ends with it; Apply keeps the dialog open and writes the document; OK closes it and the toolbar shows the new configuration.
- [x] Implement and test.

### Task 5: Running what the dialog made

**Files:** `crates/rsharp-app/tests/run.rs`.

- [x] Window tests: a configuration made in the dialog runs with its arguments and environment (a real project); a copied generated one runs its launch profile's values under its own name; removing the chosen one leaves the first generated one chosen.
- [x] Implement and test.

### Task 6: Documentation and the whole suite

**Files:** `ROADMAP.md`, `CLAUDE.md`, this plan.

- [x] `cargo fmt`, `cargo clippy`, `just test`, and a screenshot of the dialog.
- [x] `ROADMAP.md`'s Phase 5 Run line says the folder's own configurations and Edit Configurations are in;
  `CLAUDE.md`'s window section names `run_dialog.rs` and the state document.
