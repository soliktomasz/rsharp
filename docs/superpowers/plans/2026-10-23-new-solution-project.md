# New Solution and New Project Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Create solutions and add projects entirely through RSharp, using Passenger's installed-template discovery, Rider's families and a shared native dialog, preserving the files Passenger would write.

**Architecture:** Port template metadata, discovery and creation into `rsharp-core`, keeping all CLI/filesystem work off the UI thread. A shared gpui dialog feeds the existing `solution_edits`/`write_edited` and `projects::open_project` paths. Catalogs are shared in memory by SDK and template home/hive; no new sidecar or persistent template database.

**Tech Stack:** Rust 2024 (workspace floor 1.95), gpui 0.2.2, std process/filesystem/threading, existing regex/notify/serde facilities, `dotnet new`. Reuse the locked `async_zip` 0.0.17, `futures-lite` 2.6.1 and `serde_json_lenient` 0.2.4 for package metadata rather than introducing another ZIP/JSON implementation.

**Spec:** [Approved design](../specs/2026-10-23-new-solution-project-design.md), approved by the user on 2026-10-23; initial design commit `daf4a31`. Read the entire spec before execution, plus `CLAUDE.md`, `ROADMAP.md` Phase 4 and the Passenger references below.

## Global Constraints

- **Create, restore and open now; automatic build belongs to Phase 5.** NuGet and MSBuild help remain separate parts.
- Every command uses `rsharp_core::process::spawn`; no child or descendant outlives its owning operation/IDE.
- The non-UI crates never depend on gpui. Command waits, package reads, discovery, default-name filesystem checks and cleanup run off the UI thread.
- Add projects through `rsharp_core::solution_edits`, with existing-file writes through `write_edited`; never `dotnet sln add`, `dotnet add` or XML reserialization.
- Preserve encoding, BOM, line endings, comments, order and every unrelated byte. Do not use `--force`.
- Refuse invalid names, duplicate project names and non-empty destinations before writing; do not recursively remove an output path merely because it was empty earlier.
- Take Avalonia Accelerate's developer-tool references/calls out before restoring. Never use paid/editor-restricted components, C# Dev Kit or vsdbg.
- UI inherits Rider Islands Dark and existing Inter/field/dialog controls. Every color is a token in `theme.rs`; no literal colors in a view.
- New Solution has no default key. New Project: Cmd+Shift+N on macOS, Ctrl+Shift+N elsewhere. The row's existing Cmd+N / Alt+Insert Add menu also reaches it.
- Dialog starts at **920×640**, minimum **720×480**, left pane initially **260px**; footer actions remain visible while content scrolls.
- One creation dialog per originating window; refresh cannot alter an active creation plan. Capture the owner, target solution and logical folder.
- SDK version: **1 minute**; template list: **3 minutes**; create/restore: **10 minutes**. Template-source changes settle for **500ms**; remember at most **four** requested contexts.
- Tests, screenshots and MCP do not preload. Normal preload starts only after the first normal window paints and never holds up editor input.
- Every new direct dependency has an open-source license and a notice in the same change. No Passenger/submodule/Host/Roslyn-extension edits.
- All shell commands start with `rtk`. Each commit message is written with a file tool and committed using `rtk git commit -F /private/tmp/rsharp-commit.txt`.

## Review Focus

1. Paths with spaces, Unicode and shell metacharacters must remain single literal arguments; CLI-home/hive settings must be per command, not mutations of process-wide environment. Tests: Tasks 3 and 4.
2. Two dialogs using different SDKs/hives, or a canceled dialog while a shared refresh finishes, must not use stale templates or reset user input. Tests: Tasks 5 and 7.
3. An existing empty destination, a symlink/replaced root, or an unrelated file arriving before cleanup must not lead to data loss; incomplete cleanup must name the remaining path. Tests: Task 4.
4. Multiple generated projects, an unsaved solution buffer, a deleted logical folder or a changed/closed owner must not produce partial silent attachment or changes to the wrong solution. Tests: Task 8.
5. Host JSON with comments/trailing commas and ordered nonalphabetical choices, old SDK format metadata, F#/VB variants and OS constraints must preserve what Passenger offers. Tests: Tasks 1 and 2.

---

## Execution context and file map

At execution start use `superpowers:using-git-worktrees` to inspect/reuse a suitable isolated checkout. Read
the task's attached worktrees first; do not assume this planning checkout should be moved. Record the baseline
with `rtk git status --short`, `rtk cargo test --workspace` and `rtk tools/bench.sh` before product edits.
Keep benchmark output by commit; if the machine cannot capture/benchmark, record the actual blocker.

This is one connected feature, implemented in the order below. Do not create separate new subsystems for
generic forms, jobs, terminals or command runners. Existing files to reuse:

- `crates/rsharp-core/src/process.rs`: `spawn`, `Child::kill`, `Child::wait`, Lifeline; keep generic behavior intact.
- `crates/rsharp-core/src/programs.rs`: `dotnet`, `home`; `paths.rs`, `files.rs`, `solution_edits.rs` for normal IO.
- `crates/rsharp-app/src/field.rs`: `TextField`, `Changed`, selected text and input handling.
- `settings_dialog.rs`, `reference_dialog.rs`, `refactor_dialog.rs`: native dialog, focus, controls and events.
- `window/solution_actions.rs`: `MainWindow::add_existing`, `write_edited`, tree reveal and Roslyn reload.
- `projects.rs`: `open_project`; `state.rs`: recents; `window.rs`: Welcome, project widget and action handlers.
- `tests/support/mod.rs`: `Run`, isolated store, `servers = false`; `tests/settings.rs`: second-window testing.

Create these focused units (do not put their implementations into `window.rs`):

| File | Responsibility |
| --- | --- |
| `rsharp-core/src/templates/mod.rs` | Public metadata and environment types; module exports |
| `templates/catalog.rs` | Cache/host metadata parsing, including package ZIP lookup |
| `templates/families.rs` | Rider grouping, sort, search and variant lookup |
| `templates/options.rs` | Option enable-condition grammar |
| `templates/packs.rs` | Passenger's missing-pack offers and provenance |
| `templates/command.rs` | Private template-command execution, output, timeout and cancellation |
| `templates/create.rs` | Validation, argument construction, creation, output ownership and results |
| `templates/developer_tools.rs` | Passenger's targeted Avalonia take-out |
| `templates/cache.rs` | Shared catalogs, source invalidation and bounded recent contexts |
| `rsharp-app/src/templates.rs` | gpui bridge for catalog load/refresh/preload; fixture seam |
| `new_project_dialog.rs` | Native dialog window, rendering, focus and worker result handling |
| `new_project_dialog/state.rs` | Dialog choices, names, search and retained option values |
| `window/new_project.rs` | Owner/context capture, open/add handoff and project-widget menu |

Paths in that table are beneath `crates/`. Core tests: `crates/rsharp-core/tests/templates.rs`,
`template_families.rs`, `template_creation.rs`, `template_catalog_live.rs`; module-local tests cover private
command/cache internals. Test data lives under `crates/rsharp-core/tests/fixtures/templates/`.
App tests: `crates/rsharp-app/tests/new_project.rs` and `new_project_live.rs`, with small reusable fixture
helpers in `tests/support/templates.rs`. No production executable is added for testing.

For each task's final commit, stage only its listed files plus necessary exports/manifests/notices/tests.
Write the stated commit title into `/private/tmp/rsharp-commit.txt` using a file tool, then run
`rtk git add <explicit paths>` and `rtk git commit -F /private/tmp/rsharp-commit.txt`.

## Task 1: Read the actual template metadata

**Files:** Create `crates/rsharp-core/src/templates/{mod,catalog}.rs`,
`crates/rsharp-core/tests/templates.rs` and `tests/fixtures/templates/` data. Modify
`crates/rsharp-core/src/lib.rs`, `Cargo.toml`, `crates/rsharp-core/Cargo.toml`, `Cargo.lock`,
`THIRD_PARTY_NOTICES.md`.

**Interfaces:**
- Produce `TemplateCatalog { sdk: String, families: Vec<TemplateFamily>, solution_formats: Vec<String>, solution_format: String }`.
- Produce `TemplateCatalog::templates(&self) -> Vec<&ProjectTemplate>` for flattened catalog checks.
- Produce Passenger's `TemplateFamily`, `ProjectTemplate`, `TemplateVariant`, `TemplateOption`,
  `TemplateChoice`, `TemplateOptionKind` and `TemplatePack`/`PackKind` records with snake_case versions of
  Passenger's fields. `pack` is optional; `is_solution`, variant precedence and `skip_restore_option` are explicit.
- Produce metadata helpers `ProjectTemplate::languages(&self) -> Vec<String>`,
  `frameworks(&self, language: Option<&str>) -> Vec<String>`,
  `variant(&self, language: Option<&str>, framework: Option<&str>) -> &TemplateVariant` and
  `short_name(&self) -> &str`, using Passenger's language/precedence rules and first short name.
- Produce `TemplatePlatform::{MacOS, Windows, Linux}` with `current() -> Self`, so OS filtering is testable on any host.
- Produce `catalog::parse_cache(path: &Path, sdk: &str, platform: TemplatePlatform) -> Result<TemplateCatalog, String>`.
  Until Task 2 groups families, parsed groups can appear as single-Type families; this temporary structure
  must remain private to catalog assembly, not leak into the UI.

- [ ] **Step 1 — Add failing fixture tests.** In `templates.rs`, add `metadata_matches_passenger`,
  `host_metadata_reads_from_a_folder_and_a_zip`, `host_json_keeps_choice_order`,
  `old_sdks_offer_only_sln` and `platform_and_framework_constraints_match_passenger`.
  Fixtures contain a Console C#/F#/VB group with two precedence bands, a project and a solution template,
  CLI long names, invisible parameters, inverted booleans, HTTPS metadata, required numeric options,
  OS constraints and a framework-only template. Construct the test ZIP from the same host JSON fixture.
  Keep representative assertions literal:
  ```rust
  assert_eq!(console.default_name, "ConsoleApp");
  assert_eq!(console.languages(), ["C#", "F#", "VB"]);
  assert_eq!(variant.options[0].label, "Do not use top-level statements");
  assert_eq!(variant.options[0].option, "use-program-main");
  assert_eq!(choice_values, ["zeta", "alpha"]); // Source order, not map-key order.
  assert_eq!(old.solution_formats, ["sln"]);
  assert_eq!(old.solution_format, "sln");
  ```
  Test missing/unreadable IDE metadata falls back to parameter metadata, while corrupt cache data is an error.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-core --test templates`.** Expect failure because the template module/API does not exist.
- [ ] **Step 3 — Implement the metadata records and parser.** Read Passenger `ProjectTemplates.cs` in full.
  Keep grouping by GroupIdentity/Identity, newest representative selection, exact NotShown set, option
  labels/defaults/kinds and filtering. Use `serde_json_lenient` with `preserve_order`; do not strip JSON
  comments with regex. Use `async_zip`'s seek reader with `futures_lite::io::AllowStdIo<File>` behind a
  buffered reader, executing its async reads on the worker; read only matching host entries, not whole
  packages into RAM. Cache archive handles for the duration of one catalog read. Decode file URIs with
  existing path helpers, enforce package-relative entry paths, and never extract archives to disk.
  Add direct workspace deps using the already locked versions: async_zip with only deflate support,
  futures-lite with std, serde_json_lenient with preserve_order. Add their license/copyright notices.
- [ ] **Step 4 — Rerun `rtk cargo test -p rsharp-core --test templates`.** All fixture cases pass, including
  comments/trailing commas, source-order choices, missing hosts, package/folder parity and OS cases.
- [ ] **Step 5 — Commit:** `feat: read dotnet template metadata`.

## Task 2: Rider families, options and missing packs

**Files:** Create `templates/{families,options,packs}.rs`, `crates/rsharp-core/tests/template_families.rs`;
modify `templates/{mod,catalog}.rs` and relevant fixtures.

**Interfaces:**
- Consume Task 1's metadata records and `parse_cache`.
- Produce `families::of(templates: Vec<ProjectTemplate>) -> Vec<TemplateFamily>` and
  `packs::offered(installed: &[ProjectTemplate]) -> Vec<ProjectTemplate>`.
- Produce `options::is_enabled(condition: Option<&str>, values: &BTreeMap<String, String>) -> bool`.
- Consume Task 1's language/framework/variant/short-name helpers. Produce
  `TemplateFamily::matches(&self, words: &[&str]) -> Option<Vec<usize>>` (None = no match, empty = family-name match),
  `TemplateFamily::projects() -> Option<Self>`, `TemplateFamily::not_installed() -> bool`.

- [ ] **Step 1 — Add failing tests** `families_use_riders_order_and_type_names`,
  `product_classifications_win_over_test_and_web`, `search_names_a_type_and_requires_every_word`,
  `solution_types_are_absent_from_add_project`, `known_pack_offers_disappear_when_installed`,
  `enabled_conditions_match_passenger`:
  ```rust
  assert_eq!(names, ["Console", "Class Library", "Web", "Unit Test", "Aspire", "Avalonia"]);
  assert_eq!(unit_test_names, ["xUnit", "NUnit", "MSTest"]);
  assert!(!add_types.iter().any(|t| t.is_solution));
  assert!(!offered.iter().any(|t| t.short_names.contains(&"avalonia.app".into())));
  assert!(!is_enabled(Some("UseMSTestSdk && (TestRunner == MSTest)"), &values));
  assert!(is_enabled(Some("(!broken"), &values));
  ```
  Add table cases for quoted operands, !/&&/|| precedence, inequality, missing operands, all-frameworks
  filtered, compatible older-band selection, mixed installed/missing Types and ungroupable short prefixes.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-core --test template_families`.** Expect missing family/condition APIs.
- [ ] **Step 3 — Port `TemplateFamilies.cs`, `TemplatePacks.cs` and `IsEnabled` exactly.** Preserve predicate
  precedence separately from display order; installed-first, leading-short-name and project-before-solution
  sorting; shared-prefix cuts and case-insensitive search. Copy the pack-kind data/provenance from Passenger,
  including Avalonia 12.1.4, Aspire 13.6.1, Functions 4.0.5708 and AI 10.11.0-preview.3.26513.7. These are
  provenance versions, not forced install versions; install stays `dotnet new install <id>`. MAUI is not
  synthesized. Finish catalog assembly with real families and only missing offers.
- [ ] **Step 4 — Run `rtk cargo test -p rsharp-core --test template_families` and
  `rtk cargo test -p rsharp-core --test templates`.** Both pass; Task 1's metadata order stays intact.
- [ ] **Step 5 — Commit:** `feat: group project templates as Rider families`.

## Task 3: Literal, cancellable template commands and creation requests

**Files:** Create `templates/{command,create}.rs`; create
`crates/rsharp-core/tests/template_creation.rs`; modify `templates/mod.rs`.

**Interfaces:**
- Produce `TemplateEnvironment { dotnet: PathBuf, cli_home: PathBuf, hive: Option<PathBuf>,
  nuget_packages: Option<PathBuf> }` and
  `TemplateEnvironment::discover() -> Result<Self, String>`. Explicit environments keep tests isolated.
  `nuget_packages` is a per-child override for isolated runs; normal discovery leaves it unset.
- Produce private `command::run(command: Command, limit: Duration, cancel: &AtomicBool,
  output: &mut dyn FnMut(&str)) -> Result<CommandResult, String>`;
  `CommandResult { status: ExitStatus, stdout: String, stderr: String }` and `said() -> String`.
- Produce `ProjectTarget { solution: PathBuf, folder: Option<String>, projects: Vec<String> }`;
  `CreateRequest { target: Option<ProjectTarget>, template: ProjectTemplate, language: Option<String>,
  framework: Option<String>, solution_name: String, project_name: String, directory: PathBuf,
  same_directory: bool, solution_format: String, values: BTreeMap<String, String> }`.
- Produce `CreationPlan { root: PathBuf, solution: Option<PathBuf>, project_folder: PathBuf }`;
  `create::plan(request: &CreateRequest, catalog: &TemplateCatalog) -> Result<CreationPlan, String>` and
  `create::arguments(request: &CreateRequest, output: Option<&Path>) -> Result<Vec<OsString>, String>`.
  `output = None` supplies the visible command, excluding internal/output/skip-restore switches.

- [ ] **Step 1 — Write failing command/validation tests.** Module-local tests use the current Rust test
  executable's ignored child-test entry to emit large stdout/stderr, echo received arguments, wait and
  spawn a descendant; no shell-based fake-dotnet script or shipping binary. Test nonzero status, a short
  injected timeout and cancel; check both pipes drain and the child/descendant end. Integration tests
  `invalid_requests_write_nothing`, `output_layout_matches_passenger`, `arguments_are_literal_and_only_changed_options`
  assert these exact refusals and unchanged destination contents:
  ```rust
  assert_eq!(why, "Solution name cannot be empty");
  assert_eq!(why, "Solution name contains invalid characters");
  assert_eq!(why, "Project Shop.Core already exists in the solution");
  assert_eq!(plan.project_folder, parent.join("Greeter/Greeter"));
  assert_eq!(same.project_folder, parent.join("Greeter"));
  assert!(args.windows(2).any(|a| a == ["--name", "Hello; $HOME Ω"]));
  assert!(!args.iter().any(|a| a == "--force"));
  ```
  The metacharacter argument test calls argument construction directly, independently of name validation.
  Check NUL/separators/dot names, relative paths, a destination file, existing empty vs non-empty roots,
  required/integer/float options, disabled changed options, case-insensitive default comparison and old SDK format omission.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-core templates::command` and
  `rtk cargo test -p rsharp-core --test template_creation`.** Expect missing executor/request APIs.
- [ ] **Step 3 — Implement the worker-only executor and pure request/argument rules.** Each call builds a
  `Command` with per-command env and argument arrays, appending the same hive arguments to every
  `dotnet new` invocation. Drain stdout/stderr concurrently into a channel; the owning worker forwards
  output, observes cancellation/deadline and kills/waits through Lifeline before returning. Keep bounded
  captured error tails, not an unbounded install transcript. Reuse this private executor for discovery,
  install and creation; do not add a generic runner to all of RSharp. Validate filesystem destinations
  again immediately before creation in Task 4. `said` uses stderr first, otherwise stdout, up to six nonempty lines.
- [ ] **Step 4 — Rerun both commands.** The pipe-pressure, timeout and descendant tests finish within their
  bounded test deadlines; argument round-trip proves metacharacters were never interpreted by a shell.
- [ ] **Step 5 — Commit:** `feat: validate and run template commands safely`.

## Task 4: Create, restore and clean up generated output

**Files:** Extend `templates/create.rs`; create `templates/developer_tools.rs`; extend
`tests/template_creation.rs` and template fixtures.

**Interfaces:**
- Consume Tasks 1–3, `solution_edits` only later at the window handoff.
- Produce `CreatedProjects { solution: Option<PathBuf>, projects: Vec<PathBuf>, warnings: Vec<String> }`;
  `CreationProgress::{Message(String), Output(String)}`.
- Produce `create::create(environment: &TemplateEnvironment, catalog: &TemplateCatalog,
  request: &CreateRequest, cancel: &AtomicBool, progress: &mut dyn FnMut(CreationProgress))
  -> Result<CreatedProjects, String>`.
- Produce `developer_tools::take_out(folder: &Path) -> Result<Vec<PathBuf>, String>`.
- Keep a private command-function seam in `create.rs` for deterministic failed/interrupted-flow tests;
  the public function always runs Task 3's executor. Use the same invocation data, not an alternate fake creation algorithm.

- [ ] **Step 1 — Write failing flow tests** `project_solution_runs_sln_then_template_then_restore`,
  `solution_template_keeps_its_solution_or_gets_a_fallback`, `projects_are_ordered_and_build_outputs_excluded`,
  `take_out_preserves_every_unrelated_byte`, `failed_creation_and_cancel_clean_only_owned_output`,
  `restore_failure_retains_generated_projects` and `uncertain_cleanup_reports_residue`.
  Stub command responses at the private function seam and record cwd/argv/env. Assert:
  ```rust
  assert_eq!(steps, ["new sln", "new console", "take out", "restore Greeter"]);
  assert_eq!(made.projects, [root.join("Greeter/Greeter.csproj")]);
  assert_eq!(std::fs::read(&untouched).unwrap(), before);
  assert!(std::fs::read(&changed).unwrap().starts_with(b"\xEF\xBB\xBF"));
  assert_eq!(std::fs::read(&interloper).unwrap(), b"keep me\r\n");
  assert!(existing_empty_root.is_dir());
  assert!(!made.warnings.is_empty()); // A failed restore still hands off usable source.
  ```
  Cover cancel during template and during restore, no project produced, multiple solutions with same-name
  preference, `.fsproj`/`.vbproj`, CRLF/BOM take-out, take-out failure, read-only leftovers, root replacement,
  symlinked child paths and unrelated files appearing before cleanup. Assert no project has been attached yet.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-core --test template_creation` and
  `rtk cargo test -p rsharp-core templates::create`.** New orchestration/take-out cases fail.
- [ ] **Step 3 — Implement creation in Passenger's order.** Create project-template solutions first; generate
  solution templates directly and find/fallback-create their solution. Use the selected supported format,
  no-update-check and the variant's skip-restore long name. Discover projects in the exact name-first/ordinal
  order, excluding bin/obj. Port `DeveloperTools.cs`'s targeted package, Debug-only and call removals; retain
  BOM/line separators and use atomic file writes. Restore each project only when the template's own
  restore was skipped. Nonzero restore becomes a warning; cancellation/timeouts remain distinguishable
  from ordinary nonzero exit and stop the operation. Return canonical paths and restore warnings.
- [ ] **Step 4 — Implement guarded cleanup and rerun the tests.** Keep an operation-owned output journal,
  including original empty-root identity and the files/versions established as generated at completed
  stages. Stop/wait for all children before cleanup; remove owned files only while their identity/content
  still matches, then empty owned directories bottom-up. Never follow symlinks or use broad recursive
  deletion across unknown entries. If interruption inside a command leaves files whose ownership cannot
  be established, preserve/report them as incomplete cleanup rather than guessing; this is the spec's
  cleanup-failure path, not a successful cancel. Preserve a pre-existing empty root. Rerun both Step 2
  commands; all flow/failure cases pass and residue errors identify an absolute path.
- [ ] **Step 5 — Commit:** `feat: create projects from dotnet templates`.

## Task 5: Shared SDK catalogs, install, preload and invalidation

**Files:** Create `templates/cache.rs` and `crates/rsharp-app/src/templates.rs`; modify core/app
`lib.rs` exports, `crates/rsharp-app/src/main.rs`, `tools/screenshot.sh`, app test support.
Add `crates/rsharp-core/tests/template_catalog_live.rs` and module-local cache tests.

**Interfaces:**
- Produce cloneable `Catalogs::new(environment: TemplateEnvironment) -> Self`,
  `load(&self, folder: &Path) -> Result<Arc<TemplateCatalog>, String>`,
  `ready(&self, folder: &Path) -> Option<Arc<TemplateCatalog>>`, `forget(&self)` and
  `install(&self, package: &str, folder: &Path, cancel: &AtomicBool,
  output: &mut dyn FnMut(&str)) -> Result<(), String>`. These are blocking **worker-only** APIs.
- In core, watch source changes with owned `notify::RecommendedWatcher` handles and a callback/channel;
  the private cache coordinator owns refresh work and deduplicates in-flight reads by SDK/home/hive.
  Produce `Catalogs::subscribe(&self, changed: impl Fn() + Send + Sync + 'static) -> CatalogSubscription`;
  dropping the subscription removes its callback. The app bridge forwards this worker-thread callback
  through `futures::channel::mpsc` to the UI executor; callbacks never access gpui directly.
- App `templates::Templates` is a gpui global holding the shared `Catalogs` and `enabled: bool`;
  `templates::init(cx: &mut App)` installs it. `enabled = false` in ordinary window-test support suppresses
  live discovery/preload, allowing Task 7 to feed an explicit fixture. `templates::preload(folder: PathBuf,
  cx: &mut App)` starts a single post-paint background read; app subscribers receive refreshed catalogs.

- [ ] **Step 1 — Add failing tests** `one_read_is_shared_per_sdk_home_and_hive`,
  `failed_reads_can_retry`, `a_preliminary_catalog_cannot_authorize_create`,
  `source_bursts_refresh_only_four_recent_contexts`, `install_invalidates_and_keeps_the_short_name`.
  Keep the cache's read-function/clock seam private for unit tests; assert:
  ```rust
  assert_eq!(reads_for_same_key, 1);
  assert!(!Arc::ptr_eq(&sdk_a, &sdk_b));
  assert_eq!(remembered_contexts.len(), 4);
  assert_eq!(refreshes_after_one_burst, 1);
  assert!(after_install.templates().iter().any(|t| t.short_name() == "avalonia.app" && t.pack.is_none()));
  ```
  Test a cache rewritten while read, missing/v-prefixed paths, different CLI homes with the same SDK,
  a missing global.json SDK, removed source directories, watcher failure and failed reinstall. Add a
  live smoke test comparing installed catalog groups/languages with `dotnet new list --columns-all`.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-core templates::cache` and
  `rtk cargo test -p rsharp-core --test template_catalog_live -- --nocapture`.** Expect missing cache APIs;
  absent dotnet must print a precise skip reason, not synthesize fixture success.
- [ ] **Step 3 — Implement discovery/cache/install.** Resolve SDK in the context set by the spec, list then
  parse the SDK cache with three 200ms retries after the first attempt. Scope memory by SDK/home/hive;
  do not hold the store mutex while running a command. Share successful/in-flight reads, evict failures,
  keep last successful catalogs and distinguish preliminary vs authoritative results at the app bridge.
  Watch packages.json and SDK/templates/template-packs roots nonrecursively using notify (the existing
  workspace watch recursively watches too much). Settle for 500ms and reload at most four live contexts.
  Install with Task 3's executor in the selected existing directory or original read context, then forget/reload.
  An already installed package is not rolled back by Cancel.
- [ ] **Step 4 — Wire preload and run both Step 2 commands plus `rtk cargo test -p rsharp-app --test settings`.**
  Initialize the global without launching dotnet. Start preload after the first normal window paints;
  skip for MCP, bench, scripted launch steps and screenshot mode. Set an internal screenshot environment
  marker in `tools/screenshot.sh` rather than adding a public CLI flag. Discovery is service-owned;
  closing one dialog removes its subscription and never kills another dialog's shared read. Service
  shutdown drops watches and stops its children. `Run::window` disables discovery explicitly alongside
  language servers. Existing window tests must remain deterministic and network-free.
- [ ] **Step 5 — Commit:** `feat: keep template catalogs ready and fresh`.

## Task 6: Dialog choices and names without rendering

**Files:** Create `crates/rsharp-app/src/new_project_dialog/state.rs` and the minimal
`new_project_dialog.rs` module/export needed for its state tests. Do not render UI yet.

**Interfaces:**
- Produce `Draft::new(directory: PathBuf, target: Option<ProjectTarget>) -> Self`,
  `show_catalog(&mut self, catalog: Arc<TemplateCatalog>, authoritative: bool)`,
  `search(&mut self, query: &str)`, `choose_family(&mut self, index: usize)`,
  `choose_type(&mut self, short_name: &str)`, `set_language(&mut self, language: &str)`,
  `set_framework(&mut self, framework: &str)`, `set_option(&mut self, symbol: &str, value: String)`,
  `set_solution_name(&mut self, value: String)`, `set_project_name(&mut self, value: String)` and
  `request(&self) -> Result<CreateRequest, String>`.
- Store directory, same-directory and selected solution format on Draft. Private fields retain generated
  names, per-family Type keys, by-person values, last choice and authoritative-load state. Task 7 reads
  current visible choices/options/validation from Draft; do not maintain a second copy in UI fields.
- Produce state accessors `solution_name(&self) -> &str`, `project_name(&self) -> &str`,
  `chosen(&self) -> Option<&ProjectTemplate>` and `option_value(&self, symbol: &str) -> Option<&str>`
  used in the assertions below. Internal tests may inspect private selection/visibility state directly.
- Resolve occupied destination names on a worker when context/directory changes; give Draft the resulting
  names with `set_occupied_names(&mut self, names: Vec<String>)`. Tag worker results with the current
  directory/revision so late default-name work cannot replace a typed name or apply to another directory.

- [ ] **Step 1 — Add module-local failing tests** `project_name_follows_until_the_person_types_it`,
  `switching_types_keeps_only_valid_person_values`, `searching_a_type_keeps_family_history`,
  `clearing_no_matches_recovers_the_choice`, `refresh_preserves_names_and_choices`,
  `solution_template_hides_project_inputs`, `stale_catalog_cannot_create`:
  ```rust
  assert_eq!(draft.project_name(), "Greeter");
  assert_eq!(after_own_project_name, "Greeter.Core");
  assert_eq!(draft.chosen().unwrap().short_name(), "nunit");
  assert_eq!(draft.option_value("useProgramMain"), Some("true"));
  assert!(draft.request().is_err()); // Preliminary data, or no matches.
  assert_eq!(default_after_collisions, "ConsoleApp3");
  ```
  Add fixture helpers producing the Task 1 catalog and its missing-pack/solution-template variants.
  Test invalid saved choices fall back, language/framework compatibility, a missing previous Type,
  case-insensitive existing project names and a late name-default result after the user typed.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-app new_project_dialog::state`.** Expect missing Draft APIs.
- [ ] **Step 3 — Implement the state transitions from Passenger's `ChooseFamily`, `ChooseType`, `Retarget`,
  `ShowOptions`, `Filter`, `NameChanged` and `Unique`.** Use short names as stable keys across reload/install;
  initially choose Console/C# where available, else the first Type. All state-only methods are synchronous
  and do no filesystem/CLI work. Keep name-following exactly as Passenger's generated-vs-person comparison,
  not a permanently latched edit flag. `request` snapshots an installed, authoritative selection and only
  includes enabled option values. Missing packs/search misses cannot produce a request.
- [ ] **Step 4 — Rerun the Step 2 command.** Each state assertion passes without opening windows or running dotnet.
- [ ] **Step 5 — Commit:** `feat: retain New Project choices and names`.

## Task 7: Render and drive the shared native dialog

**Files:** Extend `new_project_dialog.rs`; add `crates/rsharp-app/tests/new_project.rs` and
`tests/support/templates.rs`. Modify `keymap.rs` only for dialog-context navigation as needed; use
`theme.rs` only for missing Rider-derived tokens.

**Interfaces:**
- Consume Draft, Catalogs, create/plan/arguments and the existing `TextField`/Changed APIs.
- Produce `new_project_dialog::open(owner: WindowHandle<MainWindow>, directory: PathBuf,
  target: Option<ProjectTarget>, cx: &mut App) -> Result<WindowHandle<NewProjectDialog>, String>`.
- Produce dialog accessors `problem() -> Option<&str>`, `is_busy() -> bool`, `draft() -> &Draft`,
  `show_catalog(&mut self, catalog: Arc<TemplateCatalog>, authoritative: bool, cx: &mut Context<Self>)`;
  `EventEmitter<Ended>` with `Ended::{Created(CreatedProjects), Cancelled}`. Task 8 supplies the owner handoff.
- Use an `Arc<AtomicBool>` cancellation flag per active owned command. Keep worker completion/cleanup
  alive until settled even when the dialog window closes; callbacks use weak UI handles.

- [ ] **Step 1 — Add gpui failing tests** `the_default_name_has_the_keys_on_open`,
  `tab_and_arrows_reach_all_visible_fields`, `escape_clears_search_before_cancel`,
  `fields_follow_the_selected_type`, `errors_keep_inputs_and_actions_visible`,
  `refresh_does_not_replace_an_active_creation`, `closed_dialog_ignores_a_late_read`.
  Mirror `tests/settings.rs` for `VisualTestContext::from_window`, reading through the window handle.
  Feed fixture catalogs through `show_catalog`, with live discovery disabled by Run. Define stable
  debug selectors: `field: Solution name`, `field: Project name`, `field: Search templates`,
  `button: Create`, `button: Cancel`, `button: Install`, `button: Retry`, `template family: <name>`,
  `option: <symbol>`. Assert through real input and Draft state:
  ```rust
  dialog_cx.simulate_input("Greeter");
  assert_eq!(names, ("Greeter".into(), "Greeter".into()));
  assert_eq!(problem, Some("Solution name cannot be empty"));
  assert!(dialog_cx.debug_bounds("button: Create").is_some());
  assert!(dialog_cx.debug_bounds("button: Cancel").is_some());
  assert!(!solution_type_asks_project_name);
  ```
  Check fixture loading/failure/no-match/missing-pack/install-error states, disabled controls while busy,
  accessible names/focus, long localized-looking labels and paths, and both 920×640 and 720×480 bounds.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-app --test new_project`.** New window/rendering cases fail.
- [ ] **Step 3 — Read Impeccable's `reference/craft-floor.md` immediately before UI edits, then implement
  the dialog.** Inherit DESIGN.md/Rider rather than starting a new visual identity. Use native window
  sizing and platform footer order, left search/list, right scrollable names/location/preview/Type/
  framework/language/options and a persistent footer. Reuse TextField and existing checkbox/menu/button
  styling; keep any new choice renderer local. Hide the Type row for a single Type, language for a single
  language, and project/format/same-directory inputs for solution templates. Show template descriptions,
  labels and full values on hover/wrap as appropriate. Build command preview from Task 3's arguments,
  quoting only for display. Implement browse using the existing directory-picker API.
- [ ] **Step 4 — Connect async load/install/create and rerun Step 2.** Show ready data immediately, then
  authoritative data; Retry performs another read and failure keeps Create unavailable. Worker results
  carry revision/context checks. Disable input mutations while busy, retain Cancel/window-close, stream
  bounded installation output (last 256 KiB, with an explicit earlier-output-truncated marker), and show
  progress/errors as text. Install retains the chosen short name, uses the chosen existing directory's
  feeds, reloads options and refuses Create until installed. Expose cleanup/restore warnings without
  marking a failed operation successful. Focus/select the initial name on open and on its asynchronous
  default update; use Tab/Shift+Tab/Space/arrows/Enter/Escape behavior from the spec. Emit Created only after
  generation is complete; closure/cancel must not dispatch it later. Ordinary fixture tests run no CLI.
- [ ] **Step 5 — Commit:** `feat: add the New Solution and New Project dialog`.

## Task 8: Wire every entry point and preserve solution writes

**Files:** Create `window/new_project.rs`; modify `window.rs`, `actions.rs`, `keymap.rs`,
`solution_menu.rs`, `window/solution_actions.rs`, `tests/{new_project,solution_edit,actions,windows}.rs`.
Do not replace `projects::open_project` or the Solution view model.

**Interfaces:**
- Produce `MainWindow::show_new_solution(&mut self, window: &mut Window, cx: &mut Context<Self>)` and
  `show_new_project(&mut self, into: Option<String>, window: &mut Window, cx: &mut Context<Self>)`;
  `MainWindow::new_project_dialog(&self) -> Option<WindowHandle<NewProjectDialog>>` for ownership/tests.
- Define gpui actions `NewSolution`, `NewProject`; register them in `actions::ALL` and route through those methods.
- Preserve `MainWindow::add_existing(into: Option<String>, picked: Vec<PathBuf>, projects: bool,
  cx: &mut Context<Self>) -> Result<(), String>`. Batch its **project** edits into one `write_edited` call
  before tree/reload/reveal; keep Existing Item behavior and callers intact. New Project calls this same path.
- Capture target solution/folder at dialog open and compare them before attachment. A new solution's
  missing project entries are added with `solution_edits` before deferring `projects::open_project`.

- [ ] **Step 1 — Write failing integration tests** `welcome_file_find_action_and_widget_open_new_solution`,
  `new_project_shortcut_and_add_menu_capture_the_solution_folder`,
  `a_new_solution_uses_welcome_or_another_window`, `new_projects_save_the_open_solution_then_add_only_their_lines`,
  `a_multi_project_attachment_is_one_preserving_write`, `a_changed_or_closed_owner_keeps_output_without_attachment`.
  Extend existing exact menu assertions with New Project and its platform key. Set up Task 4's generated
  result fixture for the handoff (UI creation behavior already tested separately); assert:
  ```rust
  assert_eq!(selected.as_deref(), Some("    Shop.Data"));
  assert_eq!(new_recent.path, canonical_solution);
  assert_eq!(windows_after, windows_before + 1); // Unrelated workspace already open.
  assert_eq!(bytes_with_added_lines_removed, before);
  assert!(history_contains_the_saved_open_buffer);
  assert_eq!(other_solution_bytes, other_before);
  assert!(generated_project.exists()); // Attachment refusal preserves generated output.
  ```
  Cover `.sln` BOM+CRLF and `.slnx`, nested `/src/tools/` physical directories, missing physical folder
  fallback, unsaved open solution edits, preexisting template entries, one failed member of a multi-project
  batch, deleted logical folder, owner workspace switch/close and repeated action bringing forward one dialog.
- [ ] **Step 2 — Run `rtk cargo test -p rsharp-app --test new_project` and
  `rtk cargo test -p rsharp-app --test solution_edit`.** Entry points/new menu and handoff assertions fail.
- [ ] **Step 3 — Implement action/owner wiring and batch attachment.** Add File/Find Action entries,
  Welcome's New Solution button, project-widget dropdown and solution/folder Add items; keep existing Open
  and recents behavior. The widget menu initially needs only functional New Solution/Open entries, using
  existing popup machinery. Register New Project only with a solution, report the concrete no-solution
  reason through Find Action, and bind the specified platform keys. New Solution has no shortcut.
  Compute Passenger's BesideLast and physical-folder fallback off the UI thread, then open/activate the
  owner-scoped dialog. Store its handle and subscription, clear on close, and restore owner focus.
  On Created, validate the captured owner/context before committing; for existing solutions call the
  shared batch-add path, preserving save/history/encoding/reload/evaluation/reveal. For fresh solutions,
  skip already included projects by model membership, add missing ones with `solution_edits`, and defer
  open_project outside the owner's leased update. Attachment errors report generated paths; never
  delete completed generation. Surface restore warnings in the resulting window/status and log.
- [ ] **Step 4 — Run Step 2 plus `rtk cargo test -p rsharp-app --test actions`,
  `rtk cargo test -p rsharp-app --test windows` and `rtk cargo test -p rsharp-core --test solution_edits`.**
  New/existing project paths pass, and other byte-preserving edits and multiwindow behavior stay intact.
- [ ] **Step 5 — Commit:** `feat: create solutions and add projects through the IDE`.

## Task 9: Real Passenger parity, visual evidence and delivery checks

**Files:** Create `crates/rsharp-app/tests/new_project_live.rs`; extend `tests/support/templates.rs`,
`crates/rsharp-core/tests/template_catalog_live.rs` and relevant fixtures. Update `CLAUDE.md`,
`ROADMAP.md` Phase 4, notices if needed; store screenshot evidence under `.impeccable/review/new-project/`
and benchmark rows in the existing `bench/results.csv` workflow.

**Interfaces:** Consume the completed APIs above; add no shipping API unless a real behavioral test requires it.
Test support constructs per-run TemplateEnvironment/Catalogs with their own hive, CLI home, state and feed,
without changing global HOME/USERPROFILE or the person's template installs.

- [ ] **Step 1 — Add live acceptance tests corresponding to Passenger `NewSolutionTests`.** Use actual
  clicks/typing for Welcome Console creation with top-level-statements changed, class library addition
  under `src` for both solution formats, validation before writes and the installed-list comparison.
  Assert generated source bytes, recents, selected project, tree evaluation and a real Roslyn symbol in
  the added project (enable servers only for that case, following `tests/language.rs`). Build the generated
  solution through the test's core process runner to validate files; do not add build UI/run assertions.
  Grow one solution to Console + Class Library + Unit Test and use existing Add Reference UI. Inspect
  solution/project raw bytes; leave PackageReference UI/acceptance to the later NuGet part.
  ```rust
  assert!(program.contains("static void Main"));
  assert_eq!(actual_project_bytes, passenger_expected_project_bytes);
  assert_eq!(actual_slnx_bytes, passenger_expected_slnx_bytes);
  assert_eq!(added_sln_lines, passenger_expected_lines_with_captured_id);
  assert!(added_project_symbol_was_found);
  assert!(build_succeeded);
  ```
  Expected deterministic files come from the same SDK/template/version and names, not a hardcoded SDK
  assumed on all machines. Port Passenger's Added assertion for `.sln` generated IDs; do not normalize
  unrelated content, whitespace or existing IDs. Exclude bin/obj from source parity.
- [ ] **Step 2 — Add and run the isolated install scenario.** Follow Passenger's `AvaloniaApp.cs`:
  copy pinned Avalonia.Templates 12.1.4 from a prepared package cache to a local feed, with a config
  that clears other template sources. Start with an empty custom hive, click missing Avalonia → Install,
  assert output and refreshed Type/options, create an app, verify Accelerate is gone and test-build it.
  Also install a small local solution-producing template fixture to prove all its projects attach once
  and it is absent from New Project. Record reproducible prerequisite preparation for external packages;
  do not let ordinary fixture tests download or alter the person's packages. Missing dotnet/package
  fixtures print explicit skips locally; before claiming this task complete, prepare and run the live
  cases. Existing three-OS CI must run the live cases where prerequisites are prepared; if adding a feed
  preparation step is necessary, add it to `.github/workflows/ci.yml` in this task, with exact pinned versions.
  Run `rtk cargo test -p rsharp-core --test template_catalog_live -- --nocapture` and
  `rtk cargo test -p rsharp-app --test new_project_live -- --nocapture`.
- [ ] **Step 3 — Verify the native render in one batched inspection pass.** Use the real executable via
  `rtk just screenshot`, targeting `SHOT_WINDOW='New Solution'` or `'New Project'` with `rtk env`.
  For New Solution reach it through `--key cmd-shift-a --type 'New Solution' --key enter`; for New Project
  use a disposable solution fixture and `--key cmd-shift-n`. Do not add screenshot-only public CLI flags.
  Capture default/minimum geometry and representative option/error/install states; resize with the
  permitted computer-use API where needed. Open every PNG. Verify the footer, focus, Type/options,
  wrapping/scrolling, disabled/busy state, text errors and native styling against Rider. Fix material
  findings in one batch and confirm with at most one more render round. gpui is native: skip the
  HTML/CSS detector. Follow the invoked Impeccable finish-review/documentation handoff for this native
  extension; preserve DESIGN.md unless the user separately approves a system change. No mockup or
  metadata-drift repair is part of this feature.
- [ ] **Step 4 — Run the final checks once after fixes.** `rtk cargo fmt --all --check`,
  `rtk cargo build --workspace --all-targets`, `rtk cargo test --workspace`, `rtk git diff --check`.
  Run `rtk tools/bench.sh` for comparable 1/3/10-project figures and compare with the recorded baseline.
  That script uses `--bench` and therefore excludes preload; additionally record first paint/key latency
  for a normal launch while preload is enabled, using existing bench timing hooks rather than claiming
  the --bench result covers preload. A material regression requires explanation/fix, not repeated noisy
  runs. Retain actual failures/limits and OS checks left to CI in the delivery report.
- [ ] **Step 5 — Update docs and commit:** `feat: verify New Solution and New Project parity`.
  Document module/API locations, cache/source watches, install behavior and the Phase 5 build boundary in
  CLAUDE.md. ROADMAP marks only this part delivered; NuGet/MSBuild help remain next. The entire Phase 4
  completion criterion is still open. Commit source/tests/docs/evidence according to repository ignore
  policy; don't force-add transient screenshots or test caches. Request the final code review required
  by the selected execution method, resolve actionable findings, and hand off the verified branch using
  the finishing-development-branch workflow. Do not merge/push merely because the plan has ended.

## Self-review against the approved design

- Catalog/SDK/host metadata and legacy/OS cases: Tasks 1, 3, 5; Rider families/options/packs: Task 2.
- Search, names, defaults, retained values and stale reads: Tasks 5–7.
- Creation layouts, arguments, take-out, restore, cancellation and guarded cleanup: Tasks 3–4.
- Dialog hierarchy, native controls, keyboard/focus, busy/error states and visual proof: Tasks 6–7, 9.
- Every entry point, current-context checks, shared writes/history/reload/recents and multiwindow policy: Task 8.
- Passenger fixtures, raw-byte parity, live install/project loading/build validity and performance: Task 9.
- All five Review Focus conditions have named test coverage in their owning tasks. Shared interfaces
  use `TemplateEnvironment`, `Catalogs`, `CreateRequest`, `CreatedProjects` and Draft consistently.
- No product code is written by this plan. The user reviews it and chooses execution before implementation.
