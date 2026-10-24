# Roadmap: RSharp on par with Passenger

Passenger (`../passenger`) is the reference: 827 C# files, about 179k lines, and its MVP milestone of 100 issues,
all closed. "On par" means a Rider user can do in RSharp everything Passenger's CLAUDE.md lists, with RSharp's
edge from PRODUCT.md: a faster start, faster typing and less memory with several projects open.

Phase 0, the foundation, is done (commit 1): the window as Rider's Islands, the Solution view as a file tree,
tabs, a C# editor with tree-sitter colours, Roslyn kept in step with diagnostics, and saves.

## The strategy: rewrite the IDE, keep Passenger's sidecars

Passenger is two kinds of code. Its IDE process (the UI, editor, language clients, Git, the terminal and
the tool windows) is what RSharp rewrites in Rust, because that is where speed and memory are won. Everything
that has to run inside .NET already runs **in a process of its own, speaking a protocol**, and RSharp drives
those processes as they are:

| Passenger part | Lines | What it is | How RSharp uses it |
|---|---|---|---|
| Debugger, Debugger.Interop, Dap, DebugAdapter | 34.5k | Passenger's own ICorDebug engine behind DAP | runs `debugger/` as is; RSharp writes a DAP client |
| Xaml | 9.2k | Avalonia/WPF XAML core: completion, diagnostics, formatting | needs a thin host process (new, about 1k lines of C#) |
| Inspector, Inspector.Wpf | 7.7k | startup hooks inside the app being debugged; socket protocol v5 | carried as is; RSharp speaks `Protocol.cs` over the socket |
| Profiler | 3.2k | EventPipe and ClrMD into call trees, frames and heaps | needs a host process that writes snapshots as JSON (new) |
| Roslyn.Extension | 2.8k | loaded into Roslyn: refactorings, Hot Reload, project loader | carried as is with `--extension` |
| Host | 1.6k | MSBuild in its own process, JSON lines | carried as is |
| WpfPreview, TestFrames | 1.6k | the WPF previewer, and frames from failing headless tests | carried as is |

That leaves about 118k lines to write in Rust (App 93k, Editor 8k, Languages 5k, Text 4k, Lsp 3.7k, Terminal
3.4k, Core 1.2k), and about 59k that RSharp never rewrites.

**Licence:** Passenger is MIT, © 2026 Ted L (Zaneris/passenger). RSharp may carry its sidecars, with that notice
in THIRD_PARTY_NOTICES.md. **How to bring them in** is decision 1 below.

## Phases

Each phase ends in something a person can use, held by tests that say what a person sees. Each phase's
"Passenger" line names the issues and folders to read before starting it. Effort is relative: S is about one
focused week, M two to three, L a month or more.

### Phase 1: Platform (L) — in place (commit after 633ad72)
Built: window tests on gpui's test platform (crates.io gpui's `test-support` serves, so decision 2 is crates.io);
the state store, recent solutions, each folder's tabs, carets and layout; the log; the action registry, the
macOS menu bar and Find Action; Settings with its first three settings; the dock (six places, stripes, drag to
move, gaps that resize, Solution and Problems); several windows and the session kept at quit; `--key`/`--type`
for screenshots; the Lifeline; `tools/bench.sh`. Since: Windows' job-object Lifeline, Windows paths and URIs, the toolbar as Windows'
title bar, the main-menu button on Windows and Linux, and CI on all three systems (decisions 3 and 4: from
now, on GitHub); floating tool windows (a stripe button dropped on the editor, or Float Active Tool Window),
moved by the header, sized by the corner, docked by dropping the button on a stripe, drawn inside the main
window. Not taken: the comparison with Passenger on the same machine (Passenger has no `--bench`).

What every later phase builds on. Most of it is invisible, but without it each feature reinvents state, keys
and tests.
- **Tests that drive the window:** gpui's `TestAppContext` (the `test-support` feature), an `EditorRig` that
  presses Rider's keys, a quiet-wait (`Ui.Settle`) built on a `BackgroundWork` counter, and a screenshot from the
  test platform where gpui allows it.
- **State:** one `StateStore` (settings, layout, recent solutions, per-folder state under `workspaces/<name>-<hash>/`),
  written debounced through a temporary file, merged over what is on disk, and set aside as `.bad` when it won't
  read.
- **Settings:** `AppSettings`, and Rider's searchable Settings dialog with its first pages.
- **Actions:** an `ActionRegistry` of `IdeAction`s, each with an id, title, keys, menu places and why it can't
  run. The main menu, context menus and Find Action all run through it. The keymap is Rider's, per platform.
- **The docking frame:** IntelliJ's six places, stripes, drag to move, floating windows, and the layout kept.
- **Logs:** `logs/` under the state root, rotated, with protocol traces on demand.
- **Windows:** Welcome and Recent Solutions, File > Open (a .sln, .slnx, .csproj or folder), several windows per
  process, and the Quit and reopen rules.
- **Benchmarks** that make the edge measurable: cold start to first paint, a key's latency in a 20,000-line file,
  and RSS with 1, 3 and 10 projects. Recorded per commit, compared with Passenger on the same machine.

Passenger: `State/`, `Settings/`, `Docking/`, `Welcome/`, `AppActions.cs`, `PassengerApp.cs`; issues #144,
#166, #167, #172, #237.
Done when: a window test opens a folder, presses keys and checks the text; settings and layout survive a restart;
every action is reachable from Find Action; and the benchmark numbers are written down.

### Phase 2: The editor (L)
- Multiple carets, column selection, and undo that merges a run of typing.
- The find and replace bar.
- Folding, from the tree-sitter syntax; brace matching; soft wrap; whitespace shown; zoom.
- Splits, pinned tabs, Reopen Closed Tab, closing by side, a tab dragged out into a window, and global undo across
  files (`EditorArea.Commands`).
- How a file is stored: encodings (8-bit charsets, BOM, CR), line separators from the status bar, .editorconfig
  and indent detection (`FileIndent`), and read-only files.
- Local History.
- Large files: shaped lines cached, a 20,000-line file under budget, and the long random session run with soft
  wrap on.

Passenger: `Passenger.Editor`, `Passenger.Text`, `Editors/`, `History/`, `Workspace/EditorConfig`; issues #174,
#179, #180, #182.
Done when: Passenger's editor tests, translated, pass, and the per-key allocation budget holds against a file
10 times larger.

### Phase 3: C# language service (L) — in place
Built: the protocol's answers made plain (`rsharp-lsp`'s `protocol`, checked against the real server); semantic
tokens for the lines on screen; completion, parameter info, Quick Documentation and the hover card, usage
highlights; context actions and quick fixes; Rename across files; Reformat Code; Go to Declaration (and
Cmd+click), Implementation and Type Declaration; Find Usages into a Find window; Back and Forward, Last Edit
Location, Recent Files, Go to File, Type and Symbol, and Find in Files; inlay hints; Problems' Solution tab (with
solution-wide analysis turned on) and Toolset tab, and project-load problems in the status bar; Base and Derived Symbols; a type's file renamed with it; and the MCP
server (`rsharp --mcp`) with `workspace`, `diagnostics`, `find_symbol`, `outline`, `usages`, `rename` and `edit`;
Search Everywhere (types, files, symbols), Recent Locations, and Find in Files' Solution and Directory scopes, file
mask and Replace in Files; Optimize Usings, Introduce Variable, Field and Parameter, Encapsulate Field, Extract
Method and Inline (Roslyn's code actions by their titles), Safe Delete at a declaration, Surround With and Complete
Current Statement; live templates (Rider's built-in ones on Tab, a variable's every use typed at once).
Roslyn runs as Passenger runs it: the server version
Passenger pins, its Roslyn.Extension, and each project's design-time build from its Host. Change Signature, Extract Interface, Move, Pull Members Up and Push Members Down
through dialogs the extension answers; Safe Delete from any use, a declaration's whole extent from the syntax, Delete
Anyway asked over its usages; the live templates in completion; and the target framework switcher in the status bar.
Not taken: a person's own live templates (Settings' Live Templates page); Undo of a type rename's file move.
- **Roslyn in full:** Passenger's `Host` for design-time builds and `Roslyn.Extension` loaded with `--extension`, so
  projects load the way Passenger loads them.
- Semantic tokens over the syntax colours (this closes the shortcut in `rsharp-languages`), refreshed for the
  screen first.
- Completion, signature help, hover cards, inlay hints, usage highlights.
- The Problems window (File, Solution and Toolset tabs), and project-load problems in the status bar.
- Code actions and quick fixes; rename (a type's file renamed with it); formatting.
- Go to Declaration, Implementation, Type Declaration, Base and Derived; Find Usages.
- Surround With, Complete Current Statement, Optimize Usings, Safe Delete, Introduce Field and Parameter,
  Encapsulate Field.
- The dialogs: Change Signature, Extract Interface, Move, Pull Members Up and Push Members Down.
- Live templates, and the target-framework switcher.
- **Navigation:** Go to File, Type, Symbol and Action (Search Everywhere); Recent Files and Locations; Back and
  Forward; Last Edit Location; Find in Files with Rider's scopes.
- **MCP server, first tools:** `workspace`, `diagnostics`, `find_symbol`, `outline`, `usages`, `rename`, `edit`.
  With those, agents working on RSharp can use RSharp on itself, as Passenger is used.

Passenger: `CSharp/`, `Navigation/`, `Mcp/`, `Passenger.Lsp`, `Passenger.Host`, `Passenger.Roslyn.Extension`;
issues #174, #175, #177, #178, #181, #210.
Done when: Passenger's C# feature tests pass against RSharp, and an agent can run `diagnostics` and `rename` on
RSharp's own repository.

Open: Roslyn 5.8 now and then ends its project load ("Loaded 1 project(s)") without loading the project or saying
why, a sound project and a broken one alike; the status bar then stays on "Loading solution…" and the code tools
find nothing. Starting it again in the same process stalls the same way, and turning off MSBuild's node reuse or
file-based programs changes nothing. The window keeps what Roslyn said (`MainWindow::roslyn_said`), which the
Roslyn window tests print when they fail; Passenger's Host (decision 1) loading the projects itself may be the way
round it. Since the Host and the extension load the projects, the stall has not been seen; watch for it.

### Phase 4: Solution and projects (M)
Built: the Solution view's first piece, seeing the solution: its folders, the files they list and its projects
(`rsharp-core`'s `solution`, a .sln and a .slnx); each project's Dependencies, by framework when it has several, and
its files as Passenger's Host evaluates them (`project`'s `ProjectFiles`, links and removes kept), nested by Rider's
rules (`nesting`); the folder's last evaluations shown first (`projectModels.json`), each project evaluated again once
its inputs change, the active file's first, and on a project file saved in the IDE. Its second piece, changing it:
the row's menu and Rider's keys; New Solution Folder, Existing Project and Item, Rename and Remove of solution folders,
projects and items (`solution_edits`, a .sln line by line and a .slnx spliced at its elements); New File, New Folder
and Rider's five C# types in their folder's namespace; Rename (a type named after its file renamed by Roslyn with it)
and Delete to the Trash; Add Reference and Remove of one (`project_edits`); Cut, Copy, Paste and drags, a file's
dependents and the project's items following; every write saving an open file first, keeping the old bytes in Local
History and every other byte of the file; and the folder watcher (`watcher`). New Solution and New Project now use
the effective SDK's dotnet new catalog, Rider's families and IDE options, and missing-pack Install. The shared native
dialog retains names and choices; creation removes Accelerate tools, restores, then opens or adds through the same
preserving solution writer. Live tests cover source bytes, both solution formats, local Avalonia installation,
multi-project templates and growing a solution to three projects with a reference and Roslyn loading. Automatic build
remains Phase 5. NuGet's first part is in: the folder's sources as restore reads them, v3 and folder feeds, search,
the NuGet window, and Install, Update and Remove through `dotnet add package`. Next: NuGet's second part
(credentials and credential providers, v2 feeds, vulnerabilities, editing the Sources tab), then code help in
MSBuild files; the whole Phase 4 acceptance below remains open.
- **The Solution view as Rider's:** the solution's projects, Dependencies, solution folders, file nesting.
  Cut, copy, paste and drag; every Add kind; Add Reference.
- **Edits that keep every other byte:** `SolutionEdits` (.sln line by line, .slnx with its whitespace kept) and
  `ProjectEdits`. Never `dotnet sln`.
- A folder watcher (the `notify` crate) feeding the Solution view, open files and Local History.
- New Solution and New Project from dotnet new's templates, grouped into Rider's families.
- **NuGet:** feeds from nuget.config, search, install and remove, credential providers, and vulnerabilities.
- **Code help in .csproj, .props and .targets:** completion, evaluated values and navigation (`Host`'s `inspect`).

Passenger: `Workspace/`, `Explorer/`, `Templates/`, `Packages/`, `MSBuild/`; issues #173, #176, #184, #185, #204,
#212.
Done when: a solution can be built up from New Solution to three projects with references and packages,
entirely in RSharp, its files byte-identical to Passenger's for the same steps.

### Phase 5: Build, run and test (M)
- Builds through `DotnetCommand` with the host's hold, read for errors, and the Build window.
- Run configurations: from launchSettings, the folder's own, and Edit Configurations; `LaunchPlan`; Run in the
  terminal; console links; the browser opened once the port answers.
- Hot Reload under dotnet watch.
- **Unit tests:**
  - `TestFinder` reads tests from code before any build, with tree-sitter, so no parse runs in .NET.
  - Microsoft.Testing.Platform's JSON-RPC, in Rust.
  - VSTest through its JSON protocol to `vstest.console` (or a small sidecar if the object model proves needed).
  - The Unit Tests window, with Rider's Group By.

Passenger: `Run/`, `Testing/`, `ToolWindows/`; issues #155, #198, #203, #206.
Done when: Run, Build, Run with Hot Reload and the test tree work on a web app, a console app and an xunit v3
project, on macOS and Windows.

### Phase 6: Terminal (S)
- The `alacritty_terminal` crate (Apache-2.0) for the emulator and `portable-pty` for the shell, in place of
  porting `TerminalScreen`.
- Tabs, the IDE keys the terminal leaves to the IDE, links (files appear with Cmd/Ctrl), history kept apart in
  tests, and the Agent tab.

Passenger: `Passenger.Terminal`, `MainWindow.Terminal.cs`; issue #138.
Done when: vim, htop and a 10 MB `cat` run without freezing the window.

### Phase 7: Git (M)
- The `git` executable (porcelain v2), as Passenger uses it, never a library.
- Gutter change bars from a line diff (`imara-diff`), and word diff.
- The Commit window, the Log with lanes, branches and the branch widget, Show History (file, folder, lines),
  Compare With.
- Rollback, push and update.
- Conflicts, and the three-way merge.

Passenger: `Vcs/`, `MainWindow.Vcs.cs`, `Branches.cs`, `Conflicts.cs`, `History.cs`.
Done when: a branch can be rebased with a conflict, resolved in the merge view and pushed, all in RSharp.

### Phase 8: Debugger (L)
- A DAP client in Rust over `debugger/` (Passenger.DebugAdapter, carried as is).
- Breakpoints (line, exception, conditional, hit counts, logpoints, data), stepping (async, Smart Step Into,
  return values), call stack, variables, watches, Evaluate, and inline values.
- Attach to Process, Run in Terminal, decompiled symbols, and Hot Reload while debugging (the extension's
  `HotReload` and `applyChanges`).
- The Debug window.

Passenger: `Debugging/`, `MainWindow.Attach.cs`, the Debugger notes in its CLAUDE.md; issues #145–#157, #214, #219.
Done when: Passenger's debugger window tests, translated, pass on macOS, Windows x64 and Linux, against the same
`tests/Debuggees`.

### Phase 9: Avalonia and WPF (L)
- A XAML host: `Passenger.Xaml` behind a thin JSON-lines (or LSP) process, giving completion, diagnostics,
  navigation, formatting, Extract Resource and Style, and create from usage.
- The previewers: Avalonia.Designer.HostApp, and Passenger's WpfPreview.
- From the inspector: the Live Visual Tree, binding errors in Run, Debug and Problems, XAML breakpoints, binding
  values, and Break When Value Changes.
- The MCP `app_*` tools that let an agent drive the app being built.

Passenger: `Passenger.Xaml`, `Xaml/`, `LiveTree/`, the Inspectors; issues #186, #187, #191–#202.
Done when: AvaloniaGallery and WpfGallery pass Passenger's XAML editing tests in RSharp.

### Phase 10: Web languages and other files (M)
- The web language servers bundle (TypeScript, HTML, CSS, JSON, ESLint), driven by the `rsharp-lsp` client on
  Node.js, plus Prettier.
- Colours for other files: tree-sitter grammars (XML, JSON, CSS, JS/TS, HTML, Markdown, YAML) in place of
  TextMate.
- Razor through Roslyn's cohosting, with the HTML server for its HTML.

Passenger: `Web/`, `Passenger.Languages/TextMate`; issues #189, #190.
Done when: a Blazor app's .razor, .ts and .css files have completion, diagnostics and formatting.

### Phase 11: Profiler (M)
- A profiler host: `Passenger.Profiler` behind a process that records, and writes call trees, hot spots, UI
  frames and heap graphs as JSON.
- RSharp draws the flame graph, Frames, Memory and comparisons, and supports Attach Profiler.

Passenger: `Passenger.Profiler`, `Profiling/`; issues #139, #199.
Done when: a CPU and a memory snapshot of the same app match what Passenger shows.

### Phase 12: Agents in full (M)
- All of Passenger's MCP tools (the `rmcp` crate), each answering with its operation's own result.
- An agent's socket, `--mcp`, `--mcp-attach`, admission and asking.
- Settings' MCP Server page.

Passenger: `Mcp/`, its CLAUDE.md's MCP notes; issues #143, #197, #211, #235, #236.
Done when: Passenger's `McpTests` scenarios pass against RSharp.

### Phase 13: Shipping (M)
- Packages: a signed and notarized macOS disk image; a Windows installer (Velopack, which has a Rust SDK); and
  Linux archives.
- Updates from GitHub releases, and Discord presence.
- Notices generated with `cargo about` for all ~550 crates, failing on a missing licence.
- CI on macOS, Windows and Linux, sharded as Passenger's is.

Passenger: `Updates/`, `Discord/`, `tools/package-*`, `.github/workflows`; issues #163, #164, #170, #171, #188,
#205, #215, #226.
Done when: a tagged release installs and updates itself on all three systems.

## Order and what can run at once

Phases 1–3 are sequential: each needs the one before. After phase 3, the work divides cleanly:
- Phases 4 and 5 (projects, then build and test) belong together.
- Phases 6 and 7 (terminal, Git) are independent of everything else.
- Phase 8 (debugger) needs phase 5's launch plan.
- Phase 9 needs phases 5 and 8.
- Phases 10–12 can go whenever after phase 3.
- Phase 13 can start as soon as there is something to ship, and should start no later than phase 5.

## Decisions to make before phase 1

1. **How Passenger's sidecars get here.** Decided: a pinned git submodule (`vendor/passenger`), built by RSharp's build
   (`crates/rsharp-lsp/build.rs`). The options were:
   - a pinned git submodule, built by RSharp's build;
   - downloaded from Passenger's GitHub releases by version;
   - copied into the repository.

   The submodule keeps one source of truth and builds reproducibly. It also makes `dotnet` a build requirement,
   which RSharp needs at run time anyway.
2. **Which gpui.** crates.io 0.2.2 (stable, no window-test support published) or a pinned Zed git revision (newer,
   with `test-support` and visual tests). Phase 1's test harness depends on this answer.
3. **When Windows and Linux.** Passenger ships all three. Building and testing them from phase 1 costs less than
   catching up later. gpui's Windows backend is younger than its macOS one.
4. **Where the work is tracked.** A GitHub repository with a milestone and one issue per phase item, written as
   Passenger's issues are (what Rider does, and when it is done).

## The edge, kept honest

Every phase reports the phase 1 benchmarks before and after. A phase that makes a key, a start or an idle
project cost more than it did says why in its commit.
