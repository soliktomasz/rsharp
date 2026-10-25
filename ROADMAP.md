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
remains Phase 5. NuGet is in: the folder's sources as restore reads them (credentials, credential providers and
the audit sources included), v3, v2 and folder feeds, search, known vulnerabilities, the NuGet window with Install,
Update and Remove through `dotnet add package`, and the Sources tab editing a nuget.config in place. Code help in
MSBuild files is in: completion from MSBuild's schemas and the Host's `inspect` (package ids and versions from the
feeds), the hover's evaluated values, Go to Declaration, and the evaluation's errors as problems; installed MSBuild
files opening read-only, CPM's PackageVersion on completion and advisories on versions are still to come. The
acceptance below holds: `crates/rsharp-app/tests/phase4.rs` takes New Solution, two Add New Projects, two Add
References and a NuGet install through the window, and its files are byte for byte those Passenger's window makes for
the same steps (`fixtures/phase4`, written by `tools/passenger-parity.sh` from the pinned submodule; the .sln's random
project GUIDs aside). Next: Phase 5.
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
Built: the first slice, explicit Build/Rebuild/Clean of the solution or selected project/folder, through
literal dotnet commands with SDK overrides removed and MSBuild workers isolated. Passenger's Host now routes
concurrent requests and holds its generating design-time builds off until the explicit build ends. Build has
a dockable console with bounded output, Rerun/Stop/Clear and compiler links; Problems has a separate Build tab.
Dirty workspace buffers, including detached editors, save first. Stop and leaving the workspace end the build's
process tree; New Solution builds once after opening. Presentation settings control opening Build and showing
its results. The second slice, Run: the folder's run configurations from its executable projects and their
launch profiles, chosen in the toolbar and kept with the folder; Run, Rerun and Stop on Rider's keys, the plan
resolved from MSBuild's properties (the project's assembly under `dotnet`, or a profile's program, with its
arguments, working folder and environment), the build before the launch, the program in a Run console with
bounded output and its exit code in the status bar, URL and stack-frame links in that console, and the browser
opened once the program listens at its profile's URL. The third slice, the folder's own run configurations and
Rider's Edit Configurations dialog: stored in `runConfigurations.json` with each project relative to the folder,
added, copied, removed and edited over the window, listed before the projects' generated ones, and run as they
are. The fourth slice, Hot Reload: Settings' Hot Reload runs a project under `dotnet watch` (no launch profile, so
the plan's arguments, environment and browser still hold), a save applies itself or restarts the program, Apply
Hot Reload Changes (Alt+F10) saves for it, and what watch made of a save is said in the person's words. Unit
Tests now find test methods from tree-sitter before a build, follow unsaved shared buffers and watched files,
and run through native Rust clients for Microsoft.Testing.Platform and VSTest. The docked tree merges runner
rows and target frameworks, keeps Rider's Group By and outcome filters with the folder, and shows results,
output, attachments and source links. Run All, context/selected tests and failed tests save and build first;
Stop and leaving the workspace end the hosts. Run in the terminal arrived in Phase 6.
- Builds through `DotnetCommand` with the host's hold, read for errors, and the Build window.
- Run configurations: from launchSettings, the folder's own, and Edit Configurations; `LaunchPlan`; Run in the
  terminal; console links; the browser opened once the port answers. (In; Run in the terminal came with Phase 6.)
- Hot Reload under dotnet watch. (In.)
- **Unit tests:**
  - `TestFinder` reads tests from code before any build, with tree-sitter, so no parse runs in .NET. (In.)
  - Microsoft.Testing.Platform's JSON-RPC, in Rust. (In.)
  - VSTest through its JSON protocol to `vstest.console`, without another sidecar. (In.)
  - The Unit Tests window, with Rider's Group By. (In.)

Passenger: `Run/`, `Testing/`, `ToolWindows/`; issues #155, #198, #203, #206.
Done when: Run, Build, Run with Hot Reload and the test tree work on a web app, a console app and an xunit v3
project, on macOS and Windows.

### Phase 6: Terminal (S) — in place (`codex/phase6-terminal`)
Built: a `rsharp-terminal` crate with no gpui, `alacritty_terminal` for the emulator and `portable-pty` for the
shell. `rsharp_core::process::spawn_pty` is the only way a shell starts, and it keeps the Lifeline's promise: a
sidecar `pty-guard` ends the child's whole *session* rather than one process group, because an interactive
shell's jobs lead groups of their own, and it does so on tab close, workspace change and an IDE killed with
SIGKILL. A session reads in itself, writes from its own thread with a 1 MB pending-input limit that refuses a
paste rather than blocking the UI, drains the last output with a bounded wait, and publishes the exit once. The
Terminal tool window is at Bottom Left, opened lazily, with Local tabs and New Session, the person's login shell
in the folder (home on Welcome), an isolated `HISTFILE` in tests and screenshots, and a failed start left
visible with its reason. Only the visible grid is shaped and painted; scrollback is bounded at 10,000 lines.
Selection, Select All, copy and paste, scrollback, mouse reporting, alternate screen, Unicode and wide cells,
query replies and resize all come from the emulator; Cmd (Ctrl elsewhere) keeps the IDE's shortcuts, Cmd+K
clears the buffer, and Run/Rerun/Stop stay the IDE's in a program tab. URLs open the browser, stack frames open
their file at their line, and a plain path with Cmd/Ctrl opens against the session's folder, soft wraps joined.
The Solution view's Open In > Terminal starts a shell at the row's folder or a file's parent. Run configurations
have Use external console, off by default and persisted, which starts the same resolved plan — build first,
arguments, environment, working folder, browser, Stop and Rerun — in a tab named for the configuration, and
Hot Reload follows the same path. `--agent CMD` runs an explicitly supplied command in an Agent tab and refuses a
program it cannot find, as every malformed flag is refused.
- The `alacritty_terminal` crate (Apache-2.0) for the emulator and `portable-pty` for the shell, in place of
  porting `TerminalScreen`. (In.)
- Tabs, the IDE keys the terminal leaves to the IDE, links (files appear with Cmd/Ctrl), history kept apart in
  tests, and the Agent tab. (In.)
- Run in the terminal, deferred from Phase 5. (In.)

Passenger: `Passenger.Terminal`, `MainWindow.Terminal.cs`; issue #138.
Done when: vim, htop and a 10 MB `cat` run without freezing the window. A 10 MB `cat` is a test
(`crates/rsharp-terminal/tests/terminal.rs`) that holds scrollback at its bound and keeps the program's last
output; vim and htop are not exercised by a test, and Windows' ConPTY and Linux are left to CI.

### Phase 7: Git (M) — in place (`codex/phase-7-git`)
Built: `rsharp_core::git::Repository` drives the installed `git` through `process::spawn` and nothing else.
Discovery is `rev-parse --show-toplevel` with `--absolute-git-dir`, so a linked worktree's own metadata and its
common directory are known and an ordinary absence is told apart from a corrupt repository or a refused one;
status is porcelain v2 with both names of a rename and index/worktree kept apart; refs, log, file history
(`--follow`) and line history (`-L`), blobs through `cat-file --filters`, a commit's files, checkout, branch
create/rename/delete, merge, rebase, push (its upstream, or `-u` when one remote is unambiguous), fetch and pull,
the index's stages, whole-side acceptance, mark-resolved, continue, abort and index moves all live there.
Arguments are arrays — `--`, `:(literal)` pathspecs and `--end-of-options` — with inherited `GIT_DIR`,
`GIT_INDEX_FILE` and pathspec overrides cleared, `GIT_TERMINAL_PROMPT=0`, and both pipes drained concurrently
under 64 MiB and 1 MiB bounds. Every command runs under the Lifeline, so a cancelled operation or a closed
workspace ends its process tree, hooks and credential helpers included; a handle's cancellation is captured
before work is queued. An ordinary commit stages into a temporary index and installs it only when the real one is
byte for byte as it was, so it takes its chosen paths alone and leaves unrelated staged content alone, and its
message reaches `git commit -F` through a file.
`rsharp-text` has the line diff and word diff over `imara-diff` (byte ranges, UTF-8, a missing final newline, a
limit on word marks in a large rewrite), the three-way merge (agreeing edits, disjoint changes applied, the rest
explicit chunks with a choice each, simple resolution of disjoint words) and `hold_newlines`.
In the app, `window/git.rs` is the folder's controller: Git and comparisons on the background executor, one
mutation at a time, a refresh coalesced behind it, an epoch that drops a closed workspace's replies, baselines
cached by HEAD and path so a keystroke runs no Git, and a watcher on `.git`'s own directories — refs, the
operation state, no object database — which also watches a linked worktree's common directory. `git_view.rs` is
the Commit window and the Git window (branches by full ref with favorites, the Log with lanes, a commit's files
and diffs, scoped history tabs, Compare With, the branch menus), `git_view/diff.rs` one side-by-side view for
every comparison, and `git_view/merge.rs` Rider's merge window: Yours, editable Result and Theirs, the base,
synchronized scrolling, and an unresolved count that typing does not clear. The editor paints its change bars from
the buffer's own baseline, shows both texts when a bar is clicked, rolls back one hunk as one undoable edit, and
Next/Previous Change walks them; the toolbar's branch widget opens the Log; Commit and Git are tools in Rider's
places; the Solution view colours a changed file and its menu shows history and Compare With HEAD; Local History
keeps the bytes a Git operation replaces, a shared buffer is reloaded after it, and a file moved in the tree keeps
its staged blob under its new name.
- The `git` executable (porcelain v2), as Passenger uses it, never a library. (In.)
- Gutter change bars from a line diff (`imara-diff`), and word diff. (In: bars, both texts, Rollback Lines as one
  undoable edit, Next/Previous Change, and the word marks inside the comparison view.)
- The Commit window, the Log with lanes, branches and the branch widget, Show History (file, folder, lines),
  Compare With. (In.)
- Rollback, push and update. (In: a file's rollback asks first and names what an unversioned file loses; push
  uses the upstream or sets one; Update follows Git's own pull policy; both Fetch and Cancel are there.)
- Conflicts, and the three-way merge. (In.)

Passenger: `Vcs/`, `MainWindow.Vcs.cs`, `Branches.cs`, `Conflicts.cs`, `History.cs`.
Done when: a branch can be rebased with a conflict, resolved in the merge view and pushed, all in RSharp.
A window test does exactly that against two local clones and a bare local remote
(`crates/rsharp-app/tests/git.rs`), checking the remote's bytes and the cleared operation; core's
`tests/git.rs` covers the same ground through the repository alone, and `tests/git_watch.rs` the watcher.
macOS is where these ran; Windows and Linux are CI's. Not taken, as the design says: a Git library, hosting
integrations, pull requests, several repositories at once, clone and init onboarding, a stash window, blame,
interactive rebase editing, signing setup, force push, and MCP VCS tools. Limits taken on purpose: binary, symlink
and unsupported non-text changes are refused in the merge view rather than guessed at, gutter bars stop at 4 MiB
(a `// shortcut:` in `buffer.rs`), the Log asks for 2,000 commits, and a failed save stops a Git command rather
than running it on stale bytes.

### Phase 8: Debugger (L) — first slice in place
Built: the DAP client and the adapter. `crates/rsharp-dap` publishes Passenger's `Passenger.DebugAdapter` from
the submodule into `sidecars/debugger` and drives it as Passenger does, over the protocol's framed messages on
standard input and output: initialize, launch or attach, `initialized`, each file's breakpoints, the exception
filters, `configurationDone`, then threads, frames, scopes, values, evaluate, continue and the steps, pause, and
disconnect or terminate. Every request waits off the UI thread, the adapter runs under the Lifeline so it and its
program end with the IDE, and a request the client does not offer — a program started in the terminal — is
refused rather than left unanswered. A live test stops a real program at a real breakpoint and reads its frame,
locals and an expression, steps it, and sees its output.
In the app: a folder's breakpoints are its own `breakpoints.json` (`rsharp-core`'s `debug`), kept relative to the
folder: each line's enabled flag, condition, hit count and log message, Rider's "Any exception" beside typed CLR
exception breakpoints with their own conditions and five "Suspend if" cases, and Mute Breakpoints. Rider's
Breakpoints dialog (Cmd+Shift+F8) lists them, edits every field, adds and removes exception breakpoints and
mutes them all, applying each change at once: a running session is told again, and the adapter's answer (bound, or
why not) is the gutter dot's colour. Debug (F5)
resolves the chosen configuration's plan as Run does, builds it, and starts it under the adapter with the folder's
breakpoints; F9 sets or clears a breakpoint at the caret and in the gutter's dot column, F10/F11/Shift+F11 step,
F5 resumes, and Stop (Shift+F5) disconnects, ends a launched program and says where it stood. A stop opens its file
with the statement marked and shows the thread's frames, the chosen frame's locals (members a level at a time on
click), watches answered as they are typed, and the program's output in the Debug window's console. Rider's Debug
window is `debug_view.rs` (its keys, Frames, Variables, Watches and Console) at Bottom Left, and `window/debug.rs`
is its controller. Attach to Process lists the machine's .NET programs as CoreCLR's own debugger transports name
them — executable, process id, user and command line, read natively on macOS and Linux — with a search, Refresh and
"Recently attached" first; the chosen one is attached to with the folder's breakpoints, Stop lets it go on (Detach
does the same), and Reattach finds the last one in the folder by id, then by program. Windows lists no programs yet. Evaluate Expression (Alt+F8) answers an
expression in the frame the program is stopped in, Set Next Statement (Ctrl+Shift+F10) moves the paused thread to
the caret's line where the debugger allows it, the frames and the marked line following, and a variable chosen in
Variables takes a new value through Set Value, the tree read again and the program going on with it. The Debug
window lists a paused program's threads above Frames when there is more than one, and choosing one shows its frames
and values, the steps going to it. Smart Step Into (Shift+Alt+F11) offers the calls the stopped statement makes, in
the order they run, and steps into the one chosen. A configuration with Use external console debugs its program in
a Terminal tab: the adapter's runInTerminal request is answered with the tab's process, and Stop ends it. What a
step over's calls returned is shown in its own Return values group above the locals. Evaluate Expression lists the
value's members a level deep, and a step over an await stops after it (the adapter's own async stepper, proven by a
window test). The stopped line carries its frame's simple values at its end as inline values, four at most.
Function breakpoints (Add Function Breakpoint in the Breakpoints dialog, a method named as `Program.Double`) stop
the program in that method wherever it is, as does a line breakpoint. The window test takes a breakpoint through F9, F5, the Debug window's own Step Over and Resume
buttons and its frame list, and checks the marked line and the program's output; both debugger suites skip,
saying why, when dotnet or the adapter is not there. macOS is where they ran; Windows and Linux are CI's.
- Data breakpoints (Windows x64's to offer). (Not yet.)
- Decompiled symbols. (Not yet: the adapter's `resolveStackFrame` writes a PDB whose sources are embedded, and names
  them under a `decompiled/…` path that no file on disk has; the client would have to read the embedded source out of
  the PDB, or a decompiler of its own would. The client's side — `isResolved` on a frame and `resolveStackFrame` —
  was written and taken back out until then.)
- Hot Reload while debugging. (Not yet.)
- Attach to Process on Windows. (Not yet: its named pipes are not read.)

Passenger: `Debugging/`, `MainWindow.Attach.cs`, the Debugger notes in its CLAUDE.md; issues #145–#157, #214, #219.
Done when: Passenger's debugger window tests, translated, pass on macOS, Windows x64 and Linux, against the same
`tests/Debuggees`. The translation has begun: `crates/rsharp-dap/tests/passenger.rs` runs `PauseTests`' three through
the client against `LoopingConsoleApp` (a pause in its loop and a second after continuing, a step from a pause in its
sleep back to the loop, a TimeSpan's fields where its ToString cannot run, an evaluation that waits on the paused
program's other thread being stopped with its reason, and a parallel loop's condition stopping it on its iteration), and `SteppingConsoleApp`'s stepping,
await and returned-value cases (into `Inner()` and out, over to the next statement, over an await, and the values
`Describe(Count())` returned), its Smart Step Into cases (the calls of `Outer(Inner())` in the order they run, the one
chosen stepped into, and the second of two `Add` calls), and its synthetic variables (what each call a step ran
returned, named `$ReturnValue1` and `$ReturnValue` and readable in an expression, replaced by the next step's, what a
step out's method returned, `$exception` at a throw, whose `Message` an expression reads, and a lambda stepped into
from a breakpoint on its statement's column), an evaluation in a paused frame answering for fields, properties, static
members and calls, a paused frame's locals and their members (a struct's, the object's, an enum's values) on
`DebuggableConsoleApp`, an exception thrown in a parallel loop stopping where it was thrown, its message read
through `exceptionInfo`, and a stack naming the program's frames and the library's between them, each placed or
subtle as Passenger's test expects.

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
