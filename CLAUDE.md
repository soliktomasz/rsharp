# CLAUDE.md

Guidance for Claude Code working in this repository.

## What this is

**RSharp** is a Rider-inspired IDE for C# and modern .NET, written in Rust on Zed's gpui, the sibling of
Passenger (`../passenger`, the same product in C# on Avalonia). Passenger's behaviour is the reference: when
RSharp needs a feature, read how Passenger does it (its CLAUDE.md maps every area), then Rider
(`/Applications/Rider.app`). ROADMAP.md is the plan to reach Passenger's features, phase by phase; read the
phase's Passenger pointers before starting its work. PRODUCT.md holds who it is for and why; its edge is speed and memory, so a change
that makes a key cost more than what it changed needs a reason.

## Commands

```sh
git submodule update --init   # vendor/passenger, Passenger's sidecars, which the build makes with dotnet
just build                 # cargo build; warnings are errors (.cargo/config.toml)
just run [PATH] [--file F] [--caret L:C] [--key KEYS] [--type TEXT] [--agent CMD]   # the IDE on a folder, solution or project
just test [filter]         # every test
just screenshot OUT.png [args]   # the real window captured on macOS (tools/screenshot.sh; WAIT=seconds,
                           # SHOT_WINDOW=Settings for another window by title; a state folder of its own)
target/debug/rsharp --mcp [FOLDER]   # RSharp as an MCP server on standard input and output (no window)
tools/passenger-parity.sh  # Phase 4's acceptance, Passenger's side: the steps of tests/phase4.rs through Passenger's
                           # window, written to tests/fixtures/phase4 (needs the SDK vendor/passenger pins and 10.0.1xx)
tools/bench.sh             # the release build on 1, 3 and 10 projects: start, load, memory, key to frame;
                           # appends bench/results.csv by commit (keep its window in front)
tools/clean.sh             # what builds and test runs leave behind: a dry run unless --yes; --deep is cargo
                           # clean, --temp the stale test folders, --worktrees the agent's git worktrees
```

`--key` and `--type` are steps through the window's real input once it has opened, in order, each waiting for
what the one before opened, and all of them for Roslyn to load the projects (at most a minute): `--key cmd-shift-a
--type clo` shows Find Action with "clo" typed. `--agent CMD` runs an explicitly named program in a Terminal tab
and waits for it to end, logging what it made of a failure while the tab keeps its output and exit status; it is
the only way an agent runs. A malformed key or command is fatal, as every malformed flag is.

`just` isn't required: each recipe is one cargo command. A Debug build optimises dependencies (gpui and
tree-sitter are slow unoptimised). gpui compiles its Metal shaders at start (`runtime_shaders`), so no Metal
toolchain is needed to build.

**Look at UI changes.** After changing anything visible, run `just screenshot` and look at the PNG before calling
it done. It captures the real window (gpui 0.2 has no off-screen capture), so it needs Screen Recording for the
terminal and takes focus for a moment.

## Layout

- `crates/rsharp-core`: no UI. `process` starts every program (`spawn`) with a Lifeline: on macOS and Linux the
  child leads a process group and a watchdog shell kills that group once the IDE's pipe to it closes; on
  Windows the child is in a job of its own, killed when the IDE's handle closes. Either way it holds however
  the IDE ended, SIGKILL included (Roslyn's BuildHost ignores SIGTERM and outlived Roslyn before it).
  `process::pty` is the other way in — `spawn_pty` for a terminal — under the same contract: `Lifeline::watch_pid`
  runs `sidecars/pty-guard` (built by the crate's `build.rs` from `src/process/pty_guard.rs`), which ends every
  process of the child's *session* and not only its original group, since an interactive shell's jobs lead groups
  of their own; Windows reuses the job. `PtyChild` owns the master, the pid, `try_wait` and the tree cleanup.
  `paths::canonical` drops Windows' `\\?\` and URIs are `file:///C:/…`, as Roslyn reports them. `paths` (canonical
  paths, `file://` URIs), `programs` (finds `dotnet` as a login shell would), `dotnet` (literal CLI commands,
  English UTF-8 output, isolated MSBuild workers and no inherited SDK overrides), `build` (Build/Rebuild/Clean
  arguments, MSBuild diagnostics with continuations and deduplication, bounded streaming of both pipes with
  ANSI/UTF-8 decoding and stoppable process trees), `files::write` (every save:
  temporary file and rename, a read-only file refused), `encodings` (a file's bytes as its text: BOM, else UTF-8,
  else its .editorconfig's 8-bit charset, else windows-1252; a character the encoding cannot hold refused at save),
  `solution` (a .sln or .slnx as its file says: folders, items, projects), `project` (the Host's `EvaluatedProject`, its
  `Stamp`s as the Host takes them, `ProjectFiles` placing its files as Rider's Solution view does, `Evaluations` the
  folder's `projectModels.json`), `nesting` (Rider's file nesting rules and DependentUpon, one level deep),
  `solution_edits` and `project_edits` (Rider's Solution view edits to a .sln, a .slnx and a project file, text in and
  text out, every other byte kept: never `dotnet sln` or `dotnet add`), `trash` (the system's Trash, or a test's
  folder), `watcher` (the folder watched through `notify`, a burst gathered for 150 ms, build output and Git's objects
  passed over),
  `templates` (Passenger's effective-SDK catalog and IDE metadata from mounted folders or ZIPs, Rider's families,
  options and missing-pack offers; `cache::Catalogs` shares reads, watches packages.json and SDK/template roots,
  and retries failed reads; `create` runs literal dotnet arguments, takes Avalonia Accelerate out before restore,
  and owns generated output until `CreatedProjects::keep`, with guarded cancellation cleanup),
  `nuget` (NuGet as restore sees it: `sources` reads the nuget.config files as NuGet.Configuration does, closer
  keys winning, `<clear />`, disabled feeds, packageSourceMapping, auditSources, each feed's credentials (a
  `NuGetPackageSourceCredentials_` variable, else the closest file's; `secret` keeps them unprinted and reads and
  writes NuGet's DPAPI-encrypted password on Windows), nuget.org assumed without a user file and never written;
  `feed` searches v3 and v2 feeds over `ureq` and folders of .nupkg files, All Feeds at once and merged, a feed's
  credentials sent to its own host alone, HTTP refused without allowInsecureConnections, and `providers` (NuGet's
  credential providers through its plugin protocol) asked once a feed without credentials answers 401;
  `vulnerabilities` the audit's VulnerabilityInfo advisories; `config_edit` changes one nuget.config as `dotnet
  nuget` does, every other byte kept; `change` runs `dotnet add package` and `dotnet remove package`, as Passenger
  does, so the bytes are dotnet's; `version` NuGet's order), `msbuild` (code help in project, .props and .targets
  files: `insight` the Host's `inspect` answer, `schema` dotnet/msbuild's XSDs carried in `assets/msbuild/` and read
  once, `context` where the caret is in the text as typed, `help` the completion, hover and declaration there),
  `editorconfig` (the .editorconfig files over a file, its own parser and globs), `git` (the installed `git` and
  nothing else, as Passenger drives it: discovery with linked worktrees told apart from an ordinary absence,
  porcelain v2 status keeping both names of a rename and the index apart from the worktree, refs, log, file and
  line history, blobs through `cat-file --filters`, selected-path commits staged in a temporary index that is
  installed only when the real one is byte for byte as it was, messages through `git commit -F`,
  branch/merge/rebase/checkout/delete/push/fetch/pull, the index's stages, whole-side acceptance with
  rebase's reversed Yours/Theirs in one place, mark-resolved, continue, abort, index moves and rollback; argument
  arrays with `:(literal)` and `--end-of-options`, inherited `GIT_*` overrides cleared, both pipes drained under
  bounds, and cancellation that ends the process tree), `history` (Local History: each version
  of a file under `localHistory/`, content once by hash in `blobs/`, a JSON line per version in `changes.log`), `state` (the one `StateStore`: `RSHARP_STATE` or the local app data; app documents
  at its root, each folder's under `workspaces/<name>-<hash>/`; writes debounced on its own thread, merged over
  the file key by key, read back from memory until written; an unreadable file set aside as `.bad` and said
  once), `logs` (`logs/rsharp.log` under the state root, rotated; gpui's warnings land there too).
- `crates/rsharp-text`: no UI. `Document` (Ropey's rope, whose clone is a snapshot another thread may read; undo;
  `Change`s in application order with byte, line and UTF-16 places), `Selection`s, and every editing command as
  a pure function to an `EditPlan`. Offsets are UTF-8 bytes. A file's CRLF is held as `\n` and written back. Undo
  joins a run of one `EditKind` (typing a word at a time); `carets` is what changes only the carets (occurrences,
  clones, columns; the last caret is the primary, the one added last); `search` is the find bar's matching; `rows` is
  lines as shown, folded lines left out and long ones wrapped, built only while something folds or wraps, and after
  one change only that change's lines made again. `diff` is the line hunks Local History and the gutter share
  (`diff_hunks`, `rollback_hunk`) and the word marks inside changed lines, byte ranges throughout;
  `hold_newlines` is a text held as a buffer holds it, without a copy when it is already `\n`; `merge` is the
  three-way merge — agreeing edits, disjoint changes applied, the rest explicit chunks with a choice each, and
  simple resolution of disjoint words.
- `crates/rsharp-languages`: colour before any server answers: tree-sitter-c-sharp parsed incrementally (`edit`
  then `parse` with the old tree) and coloured by the grammar's own highlights query over the visible bytes
  only, its captures mapped to Rider's kinds (`Kind`); the regions that fold (`folds_in`, only those near a range,
  through a tree cursor: a node's own sibling steps find its parent from the root) and the bracket pairs come from
  the same tree. tree-sitter's incremental parse grows with the members of one class, so a file over 4 MB is parsed
  once typing pauses; `tests/budget.rs` holds the UI thread's work per key flat on a file ten times larger.
  `testing` reads test declarations, traits and caret context from that same tree, without visiting method bodies.
- Unit tests: `rsharp-core::testing` owns test identities, MSBuild's actual target paths, and native Rust socket
  clients for Microsoft.Testing.Platform and VSTest. `rsharp-app::test_finder` caches closed files by stamp and
  overlays shared-buffer syntax reads; typed updates use cached listings without disk walks. `test_view` owns
  Rider's Group By, filters, merged source/runner tree and results; `window/tests.rs` saves and builds first,
  crosses worker events to the UI, and cancels hosts on Stop or workspace close. Grouping/filter choices live in
  the folder's `unitTests.json`.
- `crates/rsharp-terminal`: no UI. `Screen` is `alacritty_terminal`'s emulator with RSharp's bounds: 10,000 lines of
  scrollback, the visible cells, cursor, modes, title and selection, and answers to a program's queries (cursor
  position, size, colours) through an `EventListener`; `Session` owns one PTY — `rsharp_core::process::spawn_pty` —
  with a reader thread feeding the screen in bounded batches, a writer thread draining `Outbox`, whose
  `INPUT_LIMIT` is 1 MB and whose `paste` refuses rather than blocks, and a thread that waits for the child,
  drains the last output with a bounded wait and publishes its exit once. `input` is xterm's keys, mouse reports
  and a paste that cannot escape a program's bracketed paste. `shell` is the person's login shell (or `sh` in a
  test's own `HISTFILE`), PowerShell on Windows.
- `crates/rsharp-lsp`: the LSP client (JSON-RPC over stdio, a reader thread, `ServerFailed` the one failure; a
  server's request the client answers itself, as `passenger/projectInfo`, on a thread of its own) and `start_roslyn`,
  which runs Roslyn with `--clientProcessId`, so it ends with the IDE even after a crash. Its `build.rs` publishes
  Passenger's sidecars from `vendor/passenger` (a pinned submodule) into `target/<profile>/sidecars`: Roslyn then runs
  as Passenger runs it, the server version Passenger pins with its extension, the projects named in
  `passenger/loadProjects` and each one's design-time build asked of Passenger's Host (`host.rs`, JSON lines; `SharedHost` the folder's one Host, which the Solution view's
  `evaluate` shares; its reader routes replies by request id, its writer queues complete lines, and a cancellable
  `hold` keeps generating design-time builds away from an explicit build until it ends), a
  build's problem passed on as the project's error. Without dotnet or the submodule at build time it warns and runs
  the person's `roslyn-language-server` tool, which opens the solution (`solution/open`) or finds the projects.
- `crates/rsharp-lsp/xaml-host`: RSharp's own C#, the one sidecar not Passenger's: Passenger.Xaml (from the submodule)
  behind JSON lines in Passenger.Host's shape, published by `rsharp-lsp`'s build.rs into `sidecars/xaml` with
  Passenger's build props imported (its output under `target/xaml-host`). `rsharp_lsp::xaml` is its client, over the
  same `host::Connection` as the Host: `check`, `complete` and `symbol` take a XAML file's text as typed and its project,
  `changed` the files written on disk. `window/xaml.rs` starts it with the first XAML file and gives each XAML buffer
  its project (`msbuild_project`'s search); `editor_xaml.rs` is completion, the card and Go to Declaration there.
- `crates/rsharp-dap`: the Debug Adapter Protocol client and Passenger's debug adapter it runs. `build.rs`
  publishes Passenger's `Passenger.DebugAdapter` from the same submodule into `target/<profile>/sidecars/debugger`
  (its engine, ICorDebug behind DAP, with ILSpy's decompiler). `Session` starts it through `process::spawn` — so it
  and the program under it end with the IDE — and opens the conversation in the protocol's order: initialize, launch
  or attach, the adapter's `initialized`, each file's breakpoints, the exception filters, then `configurationDone`,
  whose answer waits for the program's runtime. Threads, a thread's frames, a frame's scopes and values, an
  expression (Evaluate and a watch), continue/next/stepIn/stepOut/pause, and disconnect/terminate are typed
  requests, as are Set Next Statement's `gotoTargets` and `goto`, Set Value's `setExpression`, Smart Step Into's
  `stepInTargets` (a step into one call of a statement), the function breakpoints' `setFunctionBreakpoints` and the
  exception a stop is at (`exceptionInfo`), and Hot Reload's ground: a launch's `hotReload` flag (the runtime's Edit and
  Continue on) and `hotReloadState`, the runtime's capabilities and each loaded module's editability. A
  source breakpoint carries its line or its column too, which binds one call of a line several share (a lambda's body). What the adapter says unasked (stops, output, exit,
  termination) comes as `Event`s on its own threads; its own request, a program started in the client's terminal,
  is answered when the app gives it a terminal runner (announced as `supportsRunInTerminalRequest`) and refused
  otherwise, so the adapter goes on rather than waiting.
- `crates/rsharp-mcp`: no UI. RSharp as an MCP server (`rsharp --mcp [FOLDER]`, JSON-RPC a line at a time on
  standard input and output), Roslyn on the folder behind its tools: `workspace`, `diagnostics` (a file's, or the
  solution's with solution-wide analysis), `find_symbol`, `outline`, `usages`, `rename` (a type's file renamed with it)
  and `edit` (answers with the file's problems after). A place is a file, line and column from 1, or a symbol's name;
  each tool waits for Roslyn's load, files are written in their own encoding and line separator, and Roslyn is told
  each write (a change names the range it replaces: Roslyn crashes on one that does not). An agent adds it as a
  stdio server: `claude mcp add rsharp -- /path/to/rsharp --mcp /path/to/folder`.
- `crates/rsharp-app`: the shell, a library (`lib.rs`, whose `init` the executable and the tests both call) and
  `main.rs`. `window.rs` is a project window as Rider's Islands: the toolbar and status bar on the frame, a stripe
  down each side, the editor and the open tool windows as islands, the gaps between them resizing what they
  divide; each action registers only while it has something to act on, so menus and Find Action grey it
  otherwise; the root has a focus of its own, so window-wide keys work on the Welcome screen.
  `window/build.rs` owns the workspace's Build/Rebuild/Clean: saves shared buffers, waits for the Host's hold,
  builds targets in order, and stops on Stop, close or workspace change. `console.rs` is its bounded plain
  console over the editor's selection and visible-line drawing, with Rerun/Stop/Clear and compiler source links:
  Build's and Run's are the same view in two windows. Build diagnostics have their own Problems tab;
  presentation settings decide when Build comes forward. `window/run.rs` owns the folder's run configurations
  (its own from `runConfigurations.json`, then its executable projects' launch profiles, kept in the toolbar and
  with the folder), the plan a launch resolves from MSBuild's properties off the UI thread, the build before it,
  the program in the Run console, and the browser opened once it listens; with Settings' Hot Reload on, the
  project runs under `dotnet watch` instead (its own build, the plan's arguments after `--`, no launch profile),
  Apply Hot Reload Changes saves for it, and `run::watch` turns watch's own lines into the person's sentences.
  A configuration with Use external console starts the same resolved plan in a Terminal tab instead, through
  `window/terminal.rs`, which also owns the toggle, Open In Terminal and `run_agent`'s Agent tab; the tab is
  named for the configuration, keeps its output and exit when it ends, and Rerun replaces it.
  `terminal/panel.rs` is the Terminal tool window (tabs, New Session, the program-or-shell view) and
  `terminal/view.rs` its gpui element: only the visible grid is shaped and painted, with selection, mouse
  reporting, scrolling, the context menu and links (URLs through the console's handler, stack frames, and
  plain paths with Cmd/Ctrl resolved against the session's folder). `terminal.rs` is the module root.
  `window/debug.rs` owns debugging: the folder's breakpoints (line, function and exception, loaded with it, set
  from the gutter or F9 or the Breakpoints dialog and sent to a running session at once), the plan a Debug resolves and the build before it, the `rsharp-dap` session on
  the background executor with a generation that drops a stopped session's replies, the stopped thread's frames
  and the chosen frame's values, watches answered as they are typed, and Stop/leave ending the session. A stop
  opens its file with the statement marked. `debug_view.rs` is Rider's Debug window: its step and run keys, Frames,
  Variables (members opened a level at a time), Watches and the debug console. The editor's gutter shows a
  breakpoint dot per line and toggles one on a click in its column, and the buffer carries the stopped line's band.
  `breakpoint_popup.rs` is Rider's breakpoint popup over the editor (Alt+F9, a right-click on a breakpoint): its
  condition, hit count and log message, each change applied at once. `run_dialog.rs` is Rider's Run/Debug
  Configurations dialog over the window: Add, Copy and Remove of the folder's own, the chosen one's fields, OK,
  Cancel and Apply, a generated one changing only by copying it.
  New Solution builds once after opening. `dock.rs` is the frame's layout: IntelliJ's six places, which window is where and open, the islands' sizes,
  kept with the folder (`state.rs`'s `FolderState`) and as `layout.json`, the default for a folder with none.
  A new tool window is a `Tool` with its title, icon and home place, and its content in `window.rs`.
  `projects.rs` is the process's windows: every open (command line, Open, the recent list) goes through
  `open_project`, which brings forward the window that has the folder, else fills a Welcome window, else opens
  one; quit keeps them all (`session.json`). `actions.rs` is every action by name for the main menu and
  `find_action.rs`.
  `new_project_dialog.rs` is the owner-scoped New Solution / New Project window: `state.rs` keeps choices and
  name-following, `render.rs` its controls; `templates.rs` shares the service and preloads after first paint.
  `window/new_project.rs` attaches through the preserving solution writer and opens through `projects`;
  creation restores and opens, and a new solution builds through the same controller. Live template tests prepare an isolated local feed
  with `python tools/prepare-template-tests.py` (Avalonia.Templates 12.1.4, matching Passenger), then use private
  CLI homes, hives and package caches. `RSHARP_PROFILE_STARTUP=1` uses the bench timing hooks on a normal launch,
  typing during preload instead of waiting for Roslyn to settle.
  `field.rs` is the single-line text field, its editing `rsharp-text`'s commands under the
  editor's own actions; `fuzzy.rs` how a query matches a name. `settings.rs` is `AppSettings` (`settings.json`),
  a gpui global that editors observe; `settings_dialog.rs` Rider's searchable Settings window (a new setting is
  a field, an `Opt` on a page, and what follows it). `nuget_view.rs` is Rider's NuGet window (Packages, Sources
  and Log tabs; the search field takes the keys when it opens; a vulnerable installed version marked, Update offering
  its fix), its projects fed by `window/nuget.rs` after each evaluation; a change saves open copies first, keeps the
  old bytes in Local History and tells the window what it wrote. `editor_msbuild.rs` is an MSBuild file's completion, hover
  card and Go to Declaration, its buffer given the Host by `window/msbuild.rs` and inspected once typing pauses, the
  evaluation's errors its problems. `nuget_view/sources.rs` is the Sources tab: the
  files in effect and the chosen one's feeds, added, edited (a masked password field), enabled and deleted in place. `solution_view.rs` is the Solution view's tree for a folder with a solution (only open nodes walked, each kept
  open by its key); a folder without one shows its files. `git_view.rs` is the Commit window and the Git window
  (Branches by full ref with favorites, the Log with its lanes, a commit's files and diffs, scoped History tabs,
  Compare With, the branch menus), `git_view/diff.rs` the one side-by-side comparison with word marks and
  synchronized scrolling, `git_view/merge.rs` Rider's merge window (Yours, editable Result, Theirs and the base;
  an unresolved count typing does not clear), and `window/git.rs` the folder's controller: every Git command and
  comparison on the background executor, one mutation at a time, a refresh coalesced behind it, an epoch that
  drops a closed workspace's replies, baselines cached by HEAD and path so a keystroke runs no Git, and
  `watcher::watch_git` on `.git`'s own directories, a linked worktree's common one included. The toolbar's branch
  widget opens the Log. `solution_menu.rs` is a row's menu (Rider's items and keys)
  and the name popup; `reference_dialog.rs` Add Reference; the window's side of changing the solution is in
  `window/`: `solution_actions.rs` (the menu's actions, solution folders, Add Existing, Remove, Add Reference, the
  watcher's bursts, and `write_edited`, the one way RSharp writes a solution or project file: saved first if open, the
  old bytes kept in Local History, written in its own encoding, read back), `file_ops.rs` (New, Rename, Delete) and
  `file_moves.rs` (Cut, Copy, Paste, drags). `find_bar.rs` is find and replace over the editor, one
  per window, following the active tab. `buffer.rs` is an open file's text, undo, tree, problems and server, shared by
  every editor of it, and how it is stored (encoding, line separator, indent from .editorconfig, the file's own lines
  or C#'s, read-only, the .editorconfig's save-time rules; the committed text the gutter compares with, kept as one
  shared string and diffed off the UI thread once typing pauses); `editor_area.rs` the groups of tabs, split right and down, pinned tabs and the tabs closed;
  `editor_window.rs` a tab dragged out into a window of its own; `history_window.rs` Local History's window and what
  it records (a file as opened, an External change seen then, each save). Every buffer in the process is found by its file
  (`buffer::find`), so a file in two windows is one text; the last editor of a file gone, the server is told. `editor.rs` is the code editor (a gpui `Element` that shapes
  only the lines on screen; the parse runs on the background executor; Roslyn told every change and asked for
  diagnostics once typing pauses; its gutter's Git bars come from the buffer's baseline, their popup shows both
  texts and rolls back one hunk as one undoable edit), `theme.rs` every colour, `assets.rs` the carried fonts and icons,
  `keymap.rs` Rider's keys, `launch.rs` the command line, `workspace.rs` the folder and its solution, `bench.rs`
  what `--bench` measures. Roslyn's progress is followed by token, and a Restore counts only once it names a
  project: Roslyn also restores its stand-in for files no project holds (`Canonical.csproj`), which never ends
  while such a file is open and is never shown or waited for.

## Systems

macOS, Windows and Linux, each built and tested by `.github/workflows/ci.yml` on every push to main and every
pull request. The toolbar is the title bar on macOS (the traffic lights drawn by the system over it) and on
Windows (marked `WindowControlArea::Drag`, so Windows moves, snaps and maximizes the window, with caption
buttons in Segoe's glyphs); Linux keeps its desktop's title bar. Windows and Linux have no menu bar, so the
toolbar's main-menu button opens every menu from `actions.rs` as one dropdown (`tests/main_menu.rs`, which runs
there only). `tools/screenshot.sh` and `tools/bench.sh` are macOS's.

## Rules

- **Never Microsoft's vsdbg or anything from C# Dev Kit**, and nothing under a paid or editor-restricted
  licence: no Rider plugin icons (`intellij.rider.*` jars), only the IntelliJ platform's Apache-2.0 ones.
- **Every program goes through `rsharp_core::process::spawn`**, so none outlives the IDE; a terminal's shell
  through `process::spawn_pty`, which carries the same Lifeline. Nothing calls `SlavePty::spawn_command` itself.
- **Every dependency, font or icon** has an open-source licence and an entry in THIRD_PARTY_NOTICES.md, added
  in the same change.
- **Colours are tokens** in `theme.rs`, each Rider's own value with where it came from; a view never writes a
  literal colour.
- **The non-UI crates never depend on gpui.** Their callbacks come on any thread; the app crosses to the UI
  through gpui's `spawn`/`background_executor`, never by blocking the UI thread on a server.
- **A malformed flag or value is fatal** (`launch.rs`), never a silent default.
- **A key costs what it changed**: edit the tree and reparse incrementally, shape and colour only visible lines.
- **Shortcuts follow Rider's**: its macOS keymap on macOS, its Visual Studio keymap elsewhere (`keymap.rs`).
- A known limit taken on purpose is a comment `// shortcut: <the limit>, <when to upgrade>`.
- Commit messages go through a file (`git commit -F msg.txt`).

## Tests

Window tests (`crates/rsharp-app/tests`) drive a real `MainWindow` on gpui's test platform: `support::Run` gives
each a folder and a state store of its own (never the person's) and turns language servers off unless a test
asks for one; keys go through `simulate_keystrokes`/`simulate_input`, clicks and drags to the element a view
tags with `debug_selector` (`debug_bounds`). `run_until_parked` is the quiet wait: the test platform runs every
background task deterministically. A test that reads a window's view does it through the handle, not inside an
update of that window, which gpui has leased out then.

Tests say what a person would see or get, and their failure message names the consequence. The long random
editing session (`rsharp-text/tests/editing_session.rs`) checks 40,000 keys, undo and redo against a plain
string: keep it passing rather than narrowing it. The Roslyn test runs the real server on a fresh project and
skips, saying why, when the tool is not installed.

Debug tests use real dotnet programs: `crates/rsharp-dap/tests/passenger.rs` translates Passenger's own debugger
tests against the same `tests/Debuggees` — `PauseTests`' three first, with `LoopingConsoleApp`: a pause stops its loop
and a second pause after continuing does too, a step from a pause inside its sleep comes back to the loop, and a
TimeSpan paused there shows its fields where its ToString cannot run, an evaluation that waits on the paused program's
other thread is stopped and says why, and a breakpoint condition calling a method stops a parallel loop on the
iteration it names; then `SteppingConsoleApp`'s cases: a step into
`Inner()` and out of it comes back to `Outer(Inner())`, a step over goes to the next statement, a step over the await
goes on after it, and the values `Describe(Count())` returned are among the locals; and Smart Step Into's two: the
calls of `Outer(Inner())` listed in the order they run and placed on their names, the one chosen stepped into past
the other, and of two `Add` calls the second chosen; and the synthetic variables: a step over lists what each call
returned in the order they returned, named `$ReturnValue1` and `$ReturnValue`, readable in expressions, replaced by
the next step's (and not given back for a struct), a step out lists what the method returned, and a stop at a throw
lists it as `$exception`, whose `Message` an expression reads; and a lambda on the line is stepped into when what it is
handed to runs it, from a breakpoint on the statement's column; and an evaluation in a paused frame answers for
fields, properties, static members and method calls on `DebuggableConsoleApp` (`myInt + 10`, `_instanceField + 4`,
`ClassProperty.IntField + 4`, `this.Get14() + 4`, `GetObject().IntProperty`), and a paused frame lists its eleven
locals with their types, a struct's `Id` and `Name`, the object's properties and fields, and an enum's value and
`Static members`, an exception thrown in a parallel loop stops where it was thrown, its `exceptionInfo` giving the
message, a stack lists the program's frames and the library's between them (the selector's line and column, the
library's frames with no source and a subtle hint, the call's frame and `Main`), and a stop inside a lambda lists the
ten values it closes over and `this`, a step through an async method goes over its sync statements and awaits,
into the awaited method and out again, and a launched program says what Hot Reload can change (the runtime's
capabilities and its own module editable); `crates/rsharp-dap/tests/session.rs` stops one
at a breakpoint through the client alone (frames, locals, evaluate, a step, its output) and `crates/rsharp-app/tests/debugger.rs` the same
through the window (F9, F5, the Debug window's buttons and lists, the marked line, resume to the end), along with
the Breakpoints dialog's condition and mute, a false condition and a logpoint as the adapter runs them, Any
exception turned off before an unhandled exception, attaching to a program the test started (its breakpoint is
hit, then Stop detaches and the program runs on), and Evaluate Expression answering in the paused frame before
Set Next Statement moves the thread back to the caret's line, and a local set with Set Value making the program
print its new total, a program's Main and worker threads listed before the other is chosen and its frames read, and
Smart Step Into choosing Sum of a statement's two calls and stopping in its body, a configuration with Use external
console starting its program in a Terminal tab whose text carries what it wrote, a step over showing what Double(21)
returned, Evaluate Expression showing a paused List's members, a step over an await stopping after it, a stop
setting its frame's values in at the end of its line (cleared when the program runs on), and a function breakpoint
added as Program.Double in the dialog stopping the program in that method, and the rest of Passenger's
`DebuggerTests` (ROADMAP's Phase 8 lists them and the few gated); `tests/debugger_crash.rs` is a binary of its own,
since the `RSHARP_ENGINE_LOG` it sets to crash the adapter would reach every session beside it. All skip, saying why,
when dotnet or the adapter is not there.

Git tests build real repositories and, when a remote is needed, a bare one beside them — never the person's and
never a network service: `crates/rsharp-core/tests/git.rs` the repository alone (porcelain records, odd paths,
worktrees, a rebase conflict resolved and pushed, an index changed underneath a commit, a failing hook ended with
its process tree), `crates/rsharp-core/tests/git_watch.rs` the metadata watcher, `crates/rsharp-text/tests/diff_merge.rs`
hunks, word marks and the three-way merge, and the window's `tests/git.rs`, `tests/git_views.rs` and
`tests/git_gutter.rs` the Commit window, the merge window, the branch list and the gutter, waited for with a
deadline because Git is a process of its own.

Terminal tests are the one place a test runs real programs against the clock: `crates/rsharp-core/tests/pty.rs`
a real PTY (size, final output, exit code, foreground and background jobs, the IDE killed outright),
`crates/rsharp-terminal/tests/terminal.rs` the emulator and a real session (query replies, 10 MB of output,
a blocked paste), and `crates/rsharp-app/tests/terminal.rs` the window, which types into `/bin/sh` and waits
with a deadline rather than `run_until_parked` alone. A test's shells keep their history under its own state
folder; a person's keep theirs.

## Code style

Rust 2024, `cargo fmt`'s defaults at 120 columns (`rustfmt.toml`). Comments say why, never what the next lines
do; a `///` summary is at most five lines.
