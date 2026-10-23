# Solution View, Piece 1 (Seeing the Solution) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** With a `.sln` or `.slnx` open, the Solution tool window shows the solution as Rider does: solution folders,
projects, each project's Dependencies, and its files as MSBuild evaluates them, nested by Rider's rules.

**Architecture:** Pure, gpui-free model code in `rsharp-core`: the solution read from its file, the Host's evaluation
types, where an evaluated project's files show, and nesting. `rsharp-lsp` gains the Host's `evaluate` request and a
`SharedHost` that Roslyn's design-time builds and the Solution view both use. `rsharp-app` gets `solution_view.rs`: a
lazily opened tree flattened into rows, fed from the folder's cache of evaluations first and the Host after.

**Tech Stack:** Rust 2024, gpui, serde/serde_json, Passenger's Host (`vendor/passenger/src/Passenger.Host`, JSON lines).

**Spec:** `docs/superpowers/specs/2026-10-23-solution-view-design.md` (Piece 1). The Passenger code each task ports is
named in it; read that file before the task. Where Passenger and the spec are both silent, Rider decides.

## Global Constraints

- The non-UI crates never depend on gpui. Their callbacks may come on any thread; the app crosses to the UI through
  gpui's `spawn`/`background_executor` and never blocks the UI thread on the Host.
- Every program goes through `rsharp_core::process::spawn` (the Host already does).
- Every new dependency, font or icon has an open-source licence and an entry in THIRD_PARTY_NOTICES.md in the same
  change. Icons come only from the IntelliJ platform's Apache-2.0 set, never `intellij.rider.*`.
- Colours are `theme.rs` tokens. Keys follow Rider's (`keymap.rs`).
- Paths go through `rsharp_core::paths::canonical`.
- A known limit taken on purpose is a comment `// shortcut: <the limit>, <when to upgrade>`.
- `just build` treats warnings as errors; `cargo fmt` at 120 columns. Comments say why. Tests say what a person sees,
  and their failure message names the consequence.
- Commit messages go through a file: `git commit -F msg.txt`, ending with
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- A folder with no solution keeps today's view (`Workspace::entries`, `MainWindow::rebuild_rows`) unchanged.

## Review Focus

1. **A project the solution names that is not on disk.** Expect its row, dimmed, with no children, and no panic. Test
   in Task 1 (`Project.exists == false`) and Task 5 (the row).
2. **A `.sln` with CRLF, a BOM, Windows separators (`src\App\App.csproj`) or a GUID in another letter case in
   `NestedProjects`.** Expect the same tree as a clean file. Test in Task 1.
3. **A cache from an older or broken `projectModels.json`.** Expect it set aside by `StateStore` (it already does
   this) and the tree to fall back to folder files. Test in Task 3.
4. **A project whose evaluation fails.** Expect "(load failed)" beside its name, the folder's files under it, and the
   other projects still evaluated. Test in Task 6.
5. **A file both linked (`Link`) and in the project folder, or listed twice by two item types.** Expect one row. Test
   in Task 2.

---

### Task 1: Read a solution file

**Files:**
- Create: `crates/rsharp-core/src/solution.rs` (port of Passenger's `Workspace/SolutionModel.cs` `Load`, `Find`,
  `LoadSlnx`, `LoadSln`, `Placed`)
- Modify: `crates/rsharp-core/src/lib.rs` (add `pub mod solution;`)
- Modify: `crates/rsharp-app/src/workspace.rs:180-189` (`find_solution` calls `solution::find`)
- Modify: `crates/rsharp-lsp/src/roslyn.rs:42-57` (`projects_of` takes the solution's projects from `solution::read`)
- Test: `crates/rsharp-core/tests/solution.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Solution { pub path: PathBuf, pub name: String, pub entries: Vec<SolutionEntry> }
  pub enum SolutionEntry {
      Folder { name: String, path: String /* "/src/tools/" */, children: Vec<SolutionEntry> },
      Project(Project),
      Item { name: String, path: PathBuf, exists: bool },
  }
  pub struct Project { pub path: PathBuf, pub name: String /* file stem */, pub exists: bool }
  pub fn read(path: &Path) -> Option<Solution>   // None when unreadable or malformed XML
  pub fn find(folder: &Path) -> Option<PathBuf>
  impl Solution { pub fn projects(&self) -> Vec<&Project> }   // depth-first, file order
  ```

- [ ] **Step 1: Write the failing tests**
  - `a_slnx_reads_its_folders_files_and_projects_nested_by_their_paths`: `<Folder Name="/src/"><Project Path="src/App/App.csproj"/></Folder><Folder Name="/src/tools/"><File Path="README.md"/></Folder>`
    gives one top `Folder "src"` (path `/src/`) holding `Project App` and `Folder "tools"` (path `/src/tools/`) holding
    `Item README.md`.
  - `a_sln_nests_projects_and_solution_items_by_nested_projects`: a CRLF `.sln` with a BOM, folder type
    `2150E333-8FDC-42A3-9474-1A3956D46DE8`, `ProjectSection(SolutionItems)` holding `README.md = README.md`, a project
    at `src\App\App.csproj`, and its `NestedProjects` line with the GUIDs in lower case. Expect the same tree as the
    `.slnx` case.
  - `a_missing_project_is_kept_and_marked`: `exists == false`.
  - `find_prefers_the_solution_named_for_the_folder_then_slnx`: a folder `Shop` with `A.slnx`, `Shop.sln` and
    `Shop.slnx` gives `Shop.slnx`.

- [ ] **Step 2: Run them and see them fail**: `rtk cargo test -p rsharp-core --test solution` fails to compile
  (no `solution` module).
- [ ] **Step 3: Implement** `solution.rs` as Passenger's `SolutionModel` reads. For `.slnx`, use an XML reader that
  gives element names and attributes. `quick-xml` (MIT) is the default choice and Piece 2 will reuse it for byte
  spans; add it with its THIRD_PARTY_NOTICES entry. For `.sln`, match lines as Passenger's three regexes do, by hand
  or with `regex` if it is already a dependency. Strip a BOM first. Project paths go through `paths::canonical`.
- [ ] **Step 4: Point `find_solution` and `projects_of` at it**, keeping `projects_of`'s fallback walk for a folder
  without a solution.
- [ ] **Step 5: Run** `rtk cargo test -p rsharp-core --test solution` (PASS), then `just test` (no regressions,
  `workspace.rs`'s own tests included).
- [ ] **Step 6: Commit** "Read a .sln or .slnx as its file says: solution folders, items and projects".

### Task 2: An evaluated project, and where its files show

**Files:**
- Create: `crates/rsharp-core/src/project.rs` (the Host's `EvaluatedProject` family from
  `vendor/passenger/src/Passenger.Host/Protocol.cs:72-152`, and a port of Passenger's `Explorer/ProjectFiles.cs`)
- Test: `crates/rsharp-core/tests/project.rs`

**Interfaces:**
- Produces (serde, `#[serde(rename_all = "camelCase")]`, unknown fields ignored, every `IReadOnlyList` field `#[serde(default)]`):
  ```rust
  pub struct EvaluatedProject { pub path: PathBuf, pub complete: bool /* default true */, pub problem: Option<String>,
      pub assembly_name: Option<String>, pub output_type: Option<String>, pub target_frameworks: Vec<String>,
      pub is_test_project: bool, pub dependencies: Vec<FrameworkDependencies>, pub items: Vec<EvaluatedItem>,
      pub inputs: Vec<Stamp>, pub folders: Vec<Stamp> }
  pub struct FrameworkDependencies { pub framework: Option<String>, pub packages: Vec<PackageDependency>,
      pub projects: Vec<PathBuf>, pub framework_references: Vec<String>, pub windows_targeting: bool }
  pub struct PackageDependency { pub name: String, pub version: Option<String>, pub implicit: bool }
  pub struct EvaluatedItem { pub r#type: String, pub path: PathBuf, pub link: Option<String>, pub dependent_upon: Option<String> }
  pub struct Stamp { pub path: PathBuf, pub ticks: i64, pub length: i64, pub folder: bool }
  pub struct ProjectFiles { .. }
  impl ProjectFiles {
      pub fn new(project: &EvaluatedProject) -> Self;
      pub fn folders_in(&self, relative: &str) -> Vec<String>;            // names, case-insensitively sorted
      pub fn files_in(&self, relative: &str) -> Vec<(String, PathBuf)>;   // (shown name, path), sorted by name
  }
  ```
  `relative` is `""` for the project's top and `"A/B"` below it.

- [ ] **Step 1: Write the failing tests**
  - `an_evaluated_project_reads_from_the_hosts_json`: a camelCase JSON line as the Host writes it deserializes, with
    `complete` true when absent.
  - `a_projects_files_show_in_their_folders_and_a_linked_one_at_its_link`: items `Compile Program.cs`,
    `Compile Models/Order.cs`, `Compile ../Shared/Version.cs` with `Link="Properties/Version.cs"`,
    `None ../README.md` with no link, `Folder Empty/`. Expect `folders_in("") == ["Empty", "Models", "Properties"]`,
    `files_in("")` to hold `Program.cs` and `README.md`, and `files_in("Properties")` to hold `Version.cs`.
  - `a_file_listed_twice_shows_once`: the same path as `Compile` and `None` gives one entry (Review Focus 5).
  - `an_empty_folder_on_disk_shows_and_bin_obj_and_dot_folders_do_not`.
  - `ds_store_and_thumbs_db_never_show`.
- [ ] **Step 2: Run** `rtk cargo test -p rsharp-core --test project` and see it fail.
- [ ] **Step 3: Implement** the types and `ProjectFiles` as Passenger's `ProjectFiles` does (`Logical`, `AddFolder`,
  `FoldersIn` with its empty-folder rule, `FilesIn`), skipping files `Visible="false"`. The Host already leaves those
  out of `items`, so nothing extra is needed; check this against `Protocol.cs` and note it in a comment.
- [ ] **Step 4: Run** the tests and see them pass.
- [ ] **Step 5: Commit** "An evaluated project's files where Rider shows them".

### Task 3: Rider's file nesting, and the folder's cache of evaluations

**Files:**
- Create: `crates/rsharp-core/src/nesting.rs` (port of Passenger's `Explorer/FileNesting.cs`: `RiderRules`, `Combine`,
  `Parents`, `Top`, the `appsettings.*.json` rule; not `DependentsOf` or `Renamed`, which belong to Piece 2)
- Modify: `crates/rsharp-core/src/project.rs` (add the cache and staleness)
- Test: `crates/rsharp-core/tests/nesting.rs`, `crates/rsharp-core/tests/project.rs`

**Interfaces:**
- Produces:
  ```rust
  // nesting.rs: each nested file's path to the one it shows under, one level deep.
  pub fn parents(files: &[PathBuf], dependent_upon: impl Fn(&Path) -> Option<PathBuf>, by_name: bool) -> HashMap<PathBuf, PathBuf>;
  // project.rs
  #[derive(Default)] pub struct Evaluations { pub projects: Vec<EvaluatedProject> }
  impl StateDocument for Evaluations { const FILE: &'static str = "projectModels.json"; }
  impl EvaluatedProject {
      pub fn is_stale(&self) -> bool;                      // any input or walked folder differs on disk now
      pub fn dependent_upon(&self, file: &Path) -> Option<PathBuf>;
  }
  ```
  `Stamp.ticks` is .NET's UTC `DateTime.Ticks`: `unix_nanos / 100 + 621_355_968_000_000_000`. A folder stamp's
  `length` is Passenger's own; read `Passenger.Host`'s `Stamp` producer to see what it counts and compare the same
  thing.

- [ ] **Step 1: Write the failing tests**
  - `nesting.rs`: port Passenger's `FileNestingTests.The_solution_view_nests_each_file_under_the_one_it_belongs_to…`
    cases as unit tests over `parents`. They cover `Form.cs` > `Form.designer.cs` and `Form.resx`; `.cs` to
    `.designer.cs` combined through `.resx`; `Index.razor` > `Index.razor.cs` and `Index.razor.css`;
    `appsettings.json` > `appsettings.Development.json`; `DependentUpon` winning over a name rule; `by_name == false`
    keeping only `DependentUpon`; and a cycle nesting nothing.
  - `project.rs`: `a_project_is_stale_once_its_project_file_changes`, `a_project_is_stale_once_a_walked_folder_gains_a_file`,
    `a_fresh_evaluation_is_not_stale`, and `a_broken_cache_reads_as_none`. For the last, write `{"projects": 7}` as
    `projectModels.json` under a `StateStore` folder: `read_for::<Evaluations>` gives an empty `Evaluations` and a
    `.bad` file appears (Review Focus 3).
- [ ] **Step 2: Run** `rtk cargo test -p rsharp-core --test nesting --test project` and see it fail.
- [ ] **Step 3: Implement** both as Passenger does.
- [ ] **Step 4: Run** them and see them pass.
- [ ] **Step 5: Commit** "Rider's file nesting, and the folder's evaluations kept".

### Task 4: The Host's `evaluate`, one Host per folder

**Files:**
- Modify: `crates/rsharp-lsp/src/host.rs` (add `evaluate`, and `SharedHost`)
- Modify: `crates/rsharp-lsp/src/roslyn.rs:104-205` (`start_roslyn` takes a `SharedHost`, and `projectInfo` builds
  through it)
- Modify: the callers of `start_roslyn`: `crates/rsharp-app/src/window.rs:348`, `crates/rsharp-mcp/src/lib.rs:148`,
  `crates/rsharp-lsp/src/lib.rs:259`, `crates/rsharp-lsp/tests/roslyn_features.rs:64`
- Test: `crates/rsharp-lsp/tests/host.rs`

**Interfaces:**
- Consumes: `rsharp_core::project::EvaluatedProject` (Task 2).
- Produces:
  ```rust
  #[derive(Clone)] pub struct SharedHost { .. }   // Arc<Mutex<Option<ProjectHost>>> + dotnet, host dll, folder
  impl SharedHost {
      /// None without Passenger's sidecars or dotnet: then nothing evaluates and Roslyn runs as it does today.
      pub fn carried(folder: &Path) -> Option<SharedHost>;
      /// Blocks; started on first use, started again after a failure.
      pub fn evaluate(&self, project: &Path, solution: Option<&Path>) -> Result<EvaluatedProject, ServerFailed>;
  }
  pub fn start_roslyn(folder: &Path, solution: Option<&Path>, log_folder: &Path, host: Option<SharedHost>,
      events: impl Fn(ServerEvent) + Send + Sync + 'static) -> Result<LanguageServer, ServerFailed>;
  ```
  The `evaluate` request is `{"id", "method": "evaluate", "params": {"projects": [project], "properties": {..}}}`.
  The properties are `build_properties`' solution properties without `RazorDesignTimeTargets`, so split that function
  into `solution_properties(solution)` plus the Razor addition. The reply is one `partial` per project (an
  `EvaluatedProject`; a multi-targeted one may come first with `complete: false`; keep the last), then `result`.
  `start_roslyn` with `None` makes its own `SharedHost::carried`, as now; with `Some`, it uses the one given.

- [ ] **Step 1: Write the failing test** `the_host_evaluates_a_project_into_its_files_and_packages`. Write a fresh
  `net10.0` console project with a `PackageReference Include="Humanizer" Version="2.14.1"` and a
  `Compile Include="../Shared/Version.cs" Link="Properties/Version.cs"`, then `evaluate` it. Expect
  `target_frameworks == ["net10.0"]`, the package in `dependencies[0].packages`, and an item with that `link`. Skip
  with `eprintln!("skipped: …")` when `SharedHost::carried` is `None`, as the Roslyn tests skip.
- [ ] **Step 2: Run** `rtk cargo test -p rsharp-lsp --test host` and see it fail.
- [ ] **Step 3: Implement** `evaluate` beside `build`, sharing its reply loop. Move the `Mutex<Option<ProjectHost>>`
  out of `start_roslyn`'s closure into `SharedHost`, so a build and an evaluation take turns on one Host. The view
  asks for one project per request so a waiting build waits for at most one evaluation; mark this
  `// shortcut: one project per evaluate request so Roslyn's builds wait at most one evaluation, until the Host is read by a reader thread that routes replies by id`.
- [ ] **Step 4: Update every caller** (the MCP server and the tests pass `None`), then run `rtk cargo test -p rsharp-lsp`.
  Run the Roslyn tests too, which exercise `projectInfo` through `SharedHost`.
- [ ] **Step 5: Commit** "The Host's evaluate, one Host shared by Roslyn and the Solution view".

### Task 5: The solution tree in the window, from the cache

**Files:**
- Create: `crates/rsharp-app/src/solution_view.rs` (port of Passenger's `Explorer/ExplorerTree.cs`'s Solution half:
  `Entries`, `Entry`, `Project`, `DetailOf`, `Dependencies`, `Referenced`, `HeldIn`, `Held`)
- Modify: `crates/rsharp-app/src/lib.rs` (`mod solution_view;`)
- Modify: `crates/rsharp-app/src/window.rs` (`Row`, `rebuild_rows`, `toggle`, `activate_row`, `solution_row`; a
  public `solution_rows`)
- Modify: `crates/rsharp-app/assets/icons/` and `crates/rsharp-app/src/assets.rs` (solution, solution folder,
  dependencies, package, project reference and framework icons), THIRD_PARTY_NOTICES.md
- Test: `crates/rsharp-app/tests/solution_view.rs`

**Interfaces:**
- Consumes: `solution::{read, Solution, SolutionEntry}` (Task 1), `project::{EvaluatedProject, ProjectFiles,
  Evaluations}` (Tasks 2–3), `nesting::parents` (Task 3).
- Produces:
  ```rust
  pub enum NodeKind { Folder, File, Solution, SolutionFolder, Project, Dependencies, Framework, Package, ProjectReference }
  pub struct SolutionTree { .. }
  impl SolutionTree {
      pub fn new(solution: Solution, evaluated: HashMap<PathBuf, EvaluatedProject>) -> Self;
      pub fn set_evaluated(&mut self, project: EvaluatedProject);   // replaces the project's children, keeps what is open
      pub fn rows(&self) -> Vec<TreeRow>;                           // visible rows, depth-first
      pub fn toggle(&mut self, row: usize, open: Option<bool>);
  }
  pub struct TreeRow { pub kind: NodeKind, pub name: String, pub detail: Option<String>, pub path: Option<PathBuf>,
      pub depth: usize, pub expandable: bool, pub expanded: bool, pub dimmed: bool, pub tip: Option<String> }
  // window.rs, for tests: each visible row as "  " per depth + name, then " (detail)" when it has one.
  pub fn solution_rows(&self) -> Vec<String>;
  ```
  Ordering follows Passenger: a solution level lists its folders, then its projects, then its items, each by name
  case-insensitively. A project lists Dependencies, then its folders, then its files. A project's detail is
  "load failed" when `problem` is set (its `tip` then being MSBuild's message), else its frameworks joined by ", ". Packages with `implicit` are left out. A
  node's children are made the first time it opens and kept until `set_evaluated` replaces that project's. What is
  open is remembered by a key per node (solution folder path, project path, and the logical path inside a project),
  so a re-evaluation keeps it open. Without an evaluation, a project lists its folder on disk, minus its project file
  and `bin`/`obj`, nested by name rules only.

- [ ] **Step 1: Write the failing window tests.** Each `Run` writes a solution and its projects, and seeds the
  evaluations by writing `Evaluations` with `StateStore::open(run.state).write_for(&run.folder, …)` and then `flush()`
  before opening the window. Servers are off, so the cache is all the tree has.
  - `a_solution_shows_its_folders_before_its_projects_each_by_name`: expect `["Shop", "  build", "  src", "  Api (net10.0)", "  Web (net10.0)"]`
    with the solution folders listed before the projects. Run it for `.sln` and `.slnx` (two tests, or one loop).
  - `a_project_opens_to_its_dependencies_then_its_folders_then_its_files`: opening `Api` gives `Dependencies`,
    `Models`, `Program.cs`; opening `Dependencies` gives `net10.0`, `Core`, `Humanizer (2.14.1)`, with no implicit
    package.
  - `a_multi_targeted_projects_dependencies_are_by_framework`.
  - `a_linked_file_shows_at_its_link_and_an_excluded_one_not_at_all`.
  - `a_file_nests_under_the_one_it_belongs_to`: `appsettings.json` opens to `appsettings.Development.json`.
  - `a_project_never_evaluated_shows_its_folders_files`: no cache entry. Expect the folder's files without
    `Api.csproj`, `bin` or `obj`.
  - `a_missing_project_is_dimmed_and_empty` (Review Focus 1).
  - `a_folder_without_a_solution_shows_as_before`: today's rows, unchanged.
  - `double_clicking_a_file_row_opens_it_in_the_editor`: `active_tab_name` is the file's name.
- [ ] **Step 2: Run** `rtk cargo test -p rsharp-app --test solution_view` and see it fail.
- [ ] **Step 3: Implement** `SolutionTree`. Keep the existing `Row` path for a folder without a solution: when
  `workspace.solution` is `Some`, `rebuild_rows` takes `SolutionTree::rows`, and the window's keys and clicks go
  through `SolutionTree::toggle` and open `path`. The folder state already remembers what is open; key that by the
  node key as well.
- [ ] **Step 4: Draw the rows**: each kind's icon, the detail after the name in `theme`'s secondary text token, a
  dimmed row in the dimmed token (add a token if none fits, with Rider's value and where it came from), and the tip on
  hover. Take the icons from the IntelliJ platform's Apache-2.0 expui set, as the existing ones were; add their
  notice entries.
- [ ] **Step 5: Run** the tests and see them pass, then `just test`.
- [ ] **Step 6: Look at it.** Run `just screenshot /tmp/solution.png ../passenger/Passenger.slnx` (WAIT=8), open a
  project and Dependencies with `--key`, and check against Rider's Solution view: icons, indent, detail colour.
- [ ] **Step 7: Commit** "The Solution view shows the solution: folders, projects, Dependencies, files as evaluated".

### Task 6: Evaluations landing in the open window

**Files:**
- Modify: `crates/rsharp-app/src/window.rs` (the Roslyn start at `:330-372`, the save path, and the tree)
- Test: `crates/rsharp-app/tests/solution_view.rs`

**Interfaces:**
- Consumes: `SharedHost` (Task 4), `SolutionTree::set_evaluated` and `Evaluations` (Tasks 3, 5).
- Produces: `MainWindow::evaluating(&self) -> bool`, true while a round runs, so tests and `--key` steps can wait.

Behaviour: when a window takes up a folder with a solution and `Ide.servers` is on, it makes one
`SharedHost::carried` and passes it to `start_roslyn`. On the background executor it then evaluates each project that
has no cached evaluation or `is_stale()`: the active file's project first (`rsharp_lsp::project_of`), then the rest in
solution order. Each result lands on the UI thread through `set_evaluated` and is written to `Evaluations` with
`write_for`. One round runs at a time; a request made during a round queues one more after it. Saving a `*.csproj`,
`*.props` or `*.targets` file in RSharp starts a round. Leaving the folder drops what lands for it.

- [ ] **Step 1: Write the failing tests.** Both skip, saying why, without sidecars, as `tests/language.rs` does.
  - `a_solution_opened_shows_each_projects_files_as_msbuild_evaluates_them`: a project with a `Compile Remove` of one
    file and a `Link`ed one. After `run_until_parked` and `evaluating() == false`, the removed file is absent and the
    linked one is at its link.
  - `a_project_file_saved_in_the_ide_shows_what_it_says_now`: add a `PackageReference` to the open `.csproj` and save.
    Dependencies shows the package.
  - `a_project_that_does_not_evaluate_says_so_and_the_rest_still_load` (Review Focus 4): a malformed `.csproj` beside
    a sound one. Expect `"(load failed)"` on its row, the folder's files under it, and the other's evaluated files.
- [ ] **Step 2: Run** them and see them fail, or skip where the machine has no sidecars. Say which in the task's report.
- [ ] **Step 3: Implement** the round as described.
- [ ] **Step 4: Run** `just test`, then `tools/bench.sh` (window in front) before and after, comparing the rows on 1,
  3 and 10 projects. Start, load, memory and key-to-frame must not get worse. If they do, find out why before going on.
- [ ] **Step 5: Look at it.** Run `just screenshot` on `Passenger.slnx` once evaluation ends, and compare with the
  cached first frame of a second start.
- [ ] **Step 6: Update the docs.** In ROADMAP.md, under Phase 4, add a "Built:" line for the Solution view's first
  piece. In CLAUDE.md's Layout, cover `solution.rs`, `project.rs` and `nesting.rs` in `rsharp-core`, `SharedHost` in
  `rsharp-lsp`, and `solution_view.rs` in `rsharp-app`.
- [ ] **Step 7: Commit** "Projects evaluated by the Host land in the Solution view, the cache first".
