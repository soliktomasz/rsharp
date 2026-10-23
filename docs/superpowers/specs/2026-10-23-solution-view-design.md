# The Solution view (Phase 4, first part)

## Why

RSharp's Solution tool window today shows the folder as it is on disk (`workspace.rs`, `MainWindow::rebuild_rows`).
A C# developer works in a solution: its projects, what each depends on and the files each one holds. Phase 4 starts
here because everything after it (New Project, NuGet, MSBuild code help, then Phase 5's builds and tests) sits on the
same project model. Passenger does all of this already, and its behaviour is the reference. **Where Passenger and this
document leave something open, Rider decides.**

It is built in two pieces, in this order: seeing the solution first, then changing it. Changing it means nothing until
the view shows it correctly.

## Goals

### Piece 1: seeing the solution

- With a `.sln` or `.slnx` open, the window shows the solution at the root. Under it are its solution folders (with
  the files they list) and its projects, in the solution's order, nested as the solution nests them.
- A project opens to **Dependencies** first: one node per target framework when it targets several, holding its
  packages (with versions), project references and frameworks (FrameworkReference, the SDK's implicit ones included).
  A project with one framework shows those groups directly, as Rider does.
- Then the project's files, **as MSBuild sees them**: every file item it holds (except `Visible="false"`) in its
  folder under the project, a file from outside the project at its `Link` (else at the project's top), and its `Folder`
  items. A file the project removes or excludes is not shown. `bin`/`obj` are not shown. An empty folder on disk is
  shown, since one just made would otherwise vanish.
- Files nest as Rider nests them: the project's `DependentUpon`, Rider's own rules (Passenger's `FileNesting.Defaults`,
  taken from Rider 2026.2's `ProjectViewNestingRulesProvider`s), and `appsettings.*.json` under `appsettings.json`.
  Nesting is one level deep.
- A project MSBuild could not evaluate shows on its row as failed, with MSBuild's message (file and line) on hover.
  Until its evaluation arrives, a project lists the files in its folder, as Passenger does.
- On opening, the tree shows at once from the folder's cache of its last evaluations. Only projects whose inputs
  changed since (the project file, its imports, the folders its wildcards walked) are evaluated again, the one holding
  the active file first.
- A project file saved in RSharp is evaluated again and its rows follow.
- Opening a file, the tree's keys and the selection keep working as they do now. Rider's Select Opened File is not in
  RSharp yet and is not part of this piece.

### Piece 2: changing the solution

- **Add** (the context menu's Add, and Rider's Cmd+N / Alt+Insert on a row): File, Directory, a C# file from Rider's
  built-in file templates (Class, Interface, Record, Struct, Enum), Solution Folder, Existing Project, Existing Item.
- **Add Reference**: a dialog of the solution's other projects with checkboxes, as Rider's.
- **Rename** (Shift+F6 / F2), **Delete** (to the system trash, asked first) and **Remove** (a project or solution
  item out of the solution, its files kept). Renaming a `.cs` file whose type has its name offers to rename the type
  through the Rename RSharp already has, and the reverse is already true.
- **Cut, Copy, Paste and drag** of files and folders, within a project and between projects. A file's nested
  dependents go with it. Open tabs follow their files, as `EditorArea`'s move already must for renames.
- **Every edit keeps every other byte**: a `.sln` changed line by line, a `.slnx` and a `.csproj` changed as text at
  the elements concerned, their whitespace, comments, order and line separators kept. Never `dotnet sln` or
  `dotnet add`. Passenger's `SolutionEdits` and `ProjectEdits` are the rules to follow, and its tests' expected files
  are the check.
- An open solution or project file is saved before RSharp writes to it, and the tree reads what was written.
- **Folder watcher** (the `notify` crate): files made, changed, moved or deleted outside RSharp reach the tree, the
  open buffers (`buffer.rs` already sees an External change when it next looks; the watcher makes it look) and Local
  History.

## Non-goals

- A folder with no solution keeps today's plain view of the folder. (Passenger treats a folder of projects without a
  solution the same way.)
- Rider's File System view, version-control colours, the Cut dimming before Paste, and File Nesting's settings dialog
  come later.
- New Solution, New Project, NuGet and code help in `.csproj`/`.props`/`.targets` are Phase 4's later parts, each with
  a design of its own.
- A person's own file templates.

## Anti-goals

- A `.sln`, `.slnx` or `.csproj` that comes back reformatted, reordered or with its line separators changed. That is
  a failure even if it still builds.
- The UI thread waiting on MSBuild. Evaluations take seconds; the tree shows the cache, or the folder's files, until
  they land.
- Opening a large solution costing more than it did before this change, in start time, memory or key-to-frame
  (`tools/bench.sh` before and after, as ROADMAP's "The edge, kept honest" asks).
- Rows built for projects nobody opened. A node's children are made the first time it opens, as Passenger's
  `ExplorerTree` does, and the list draws only the rows on screen.

## Constraints

- CLAUDE.md's rules hold: non-UI crates never depend on gpui; every program through `rsharp_core::process::spawn`;
  every new dependency (`notify`, any XML reader) with an open-source licence and its THIRD_PARTY_NOTICES.md entry in
  the same change; colours as `theme.rs` tokens; Rider's keys per keymap (`keymap.rs`); icons only the IntelliJ
  platform's Apache-2.0 ones.
- Project contents come from Passenger's Host's `evaluate` request (`vendor/passenger/src/Passenger.Host/Protocol.cs`:
  `EvaluateParams` → `EvaluatedProject`s, then `EvaluateDone`). RSharp already starts the Host (`rsharp-lsp/src/host.rs`,
  which only asks `build` today). One Host per folder serves both Roslyn's design-time builds and the Solution view;
  the view must not start a second one, and an evaluation must not hold up Roslyn's `passenger/projectInfo` answers
  longer than Passenger's sharing does.
- Evaluations are cached with the folder through the `StateStore` (`workspaces/<name>-<hash>/`), as Passenger's
  `projectModels.json`.
- Paths are `rsharp_core::paths::canonical`, as the solution's lines may use another letter case or separators.

## Decided

- Two pieces, seeing before changing (the person's call).
- Rider is the tie-breaker on any behaviour in doubt (the person's call).
- Passenger's structure is followed where it fits Rust (my call): a solution model read straight from the file; each
  project's evaluation in its place; files placed by `ProjectFiles`'s rules; nesting by `FileNesting`'s rules; edits
  as `SolutionEdits`/`ProjectEdits` do them.
- Where the code goes (my call; the builder may move it with reason):
  - reading `.sln`/`.slnx` and the byte-keeping edits: a non-UI crate (pure functions over text, so they are tested
    without a window). `rsharp-core` is the default home.
  - `evaluate`: `rsharp-lsp/src/host.rs`, beside `build`.
  - the tree: its own module in `rsharp-app` (`solution_view.rs`) rather than more of `window.rs`, which is already
    3,000 lines.

- Piece 2 needs what RSharp lacks today (my calls): a context menu on the tree's rows, built as the app's existing
  popups are (`Popup`), with Rider's items and order; Delete through the system trash (the `trash` crate, or the
  platform's own call; Passenger's tests set `Trash.Folder`, and the translated tests need the same seam); file moves
  through the existing `language::move_files`, so Roslyn and open buffers follow.

## Left to the builder

- The tree's node types and how rows flatten (Passenger's `NodeKind` is the list to cover).
- The XML approach for `.slnx`/`.csproj`, provided edits keep every other byte.
- How the Host is shared between Roslyn and the view, within the constraint above.
- How Piece 2 splits into commits.

## Done when

- **Piece 1:** Passenger's `SolutionViewTests` and `FileNestingTests` for what is shown, translated into
  `crates/rsharp-app/tests/`, pass. A `just screenshot` of RSharp on its own test solution and on Passenger's
  `Passenger.slnx` shows the tree as Rider would. Bench numbers on 1, 3 and 10 projects are no worse.
- **Piece 2:** Passenger's `SolutionFoldersTests`, `ExplorerFilesTests` and the edit tests of `SolutionViewTests`,
  translated, pass, with the files RSharp writes byte-identical to the ones Passenger's tests expect.
