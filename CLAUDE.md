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
just build                 # cargo build; warnings are errors (.cargo/config.toml)
just run [PATH] [--file F] [--caret L:C] [--key KEYS] [--type TEXT]   # the IDE on a folder, solution or project
just test [filter]         # every test
just screenshot OUT.png [args]   # the real window captured on macOS (tools/screenshot.sh; WAIT=seconds,
                           # SHOT_WINDOW=Settings for another window by title; a state folder of its own)
tools/bench.sh             # the release build on 1, 3 and 10 projects: start, load, memory, key to frame;
                           # appends bench/results.csv by commit (keep its window in front)
```

`--key` and `--type` are steps through the window's real input once it has opened, in order, each waiting for
what the one before opened: `--key cmd-shift-a --type clo` shows Find Action with "clo" typed. A malformed key is
fatal, as every malformed flag is.

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
  `paths::canonical` drops Windows' `\\?\` and URIs are `file:///C:/…`, as Roslyn reports them. `paths` (canonical
  paths, `file://` URIs), `programs` (finds `dotnet` as a login shell would), `files::write` (every save:
  temporary file and rename, a read-only file refused), `encodings` (a file's bytes as its text: BOM, else UTF-8,
  else its .editorconfig's 8-bit charset, else windows-1252; a character the encoding cannot hold refused at save),
  `editorconfig` (the .editorconfig files over a file, its own parser and globs), `history` (Local History: each version
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
  one change only that change's lines made again.
- `crates/rsharp-languages`: colour before any server answers: tree-sitter-c-sharp parsed incrementally (`edit`
  then `parse` with the old tree) and coloured by the grammar's own highlights query over the visible bytes
  only, its captures mapped to Rider's kinds (`Kind`); the regions that fold (`folds_in`, only those near a range,
  through a tree cursor: a node's own sibling steps find its parent from the root) and the bracket pairs come from
  the same tree. tree-sitter's incremental parse grows with the members of one class, so a file over 4 MB is parsed
  once typing pauses; `tests/budget.rs` holds the UI thread's work per key flat on a file ten times larger.
- `crates/rsharp-lsp`: the LSP client (JSON-RPC over stdio, a reader thread, `ServerFailed` the one failure) and
  `start_roslyn`, which runs the person's `roslyn-language-server` dotnet tool with `--clientProcessId`, so it
  ends with the IDE even after a crash, and opens the solution (`solution/open`) or lets it find the projects.
- `crates/rsharp-app`: the shell, a library (`lib.rs`, whose `init` the executable and the tests both call) and
  `main.rs`. `window.rs` is a project window as Rider's Islands: the toolbar and status bar on the frame, a stripe
  down each side, the editor and the open tool windows as islands, the gaps between them resizing what they
  divide; each action registers only while it has something to act on, so menus and Find Action grey it
  otherwise; the root has a focus of its own, so window-wide keys work on the Welcome screen.
  `dock.rs` is the frame's layout: IntelliJ's six places, which window is where and open, the islands' sizes,
  kept with the folder (`state.rs`'s `FolderState`) and as `layout.json`, the default for a folder with none.
  A new tool window is a `Tool` with its title, icon and home place, and its content in `window.rs`.
  `projects.rs` is the process's windows: every open (command line, Open, the recent list) goes through
  `open_project`, which brings forward the window that has the folder, else fills a Welcome window, else opens
  one; quit keeps them all (`session.json`). `actions.rs` is every action by name for the main menu and
  `find_action.rs`; `field.rs` the single-line text field, its editing `rsharp-text`'s commands under the
  editor's own actions; `fuzzy.rs` how a query matches a name. `settings.rs` is `AppSettings` (`settings.json`),
  a gpui global that editors observe; `settings_dialog.rs` Rider's searchable Settings window (a new setting is
  a field, an `Opt` on a page, and what follows it). `find_bar.rs` is find and replace over the editor, one
  per window, following the active tab. `buffer.rs` is an open file's text, undo, tree, problems and server, shared by
  every editor of it, and how it is stored (encoding, line separator, indent from .editorconfig, the file's own lines
  or C#'s, read-only, the .editorconfig's save-time rules); `editor_area.rs` the groups of tabs, split right and down, pinned tabs and the tabs closed;
  `editor_window.rs` a tab dragged out into a window of its own; `history_window.rs` Local History's window and what
  it records (a file as opened, an External change seen then, each save). Every buffer in the process is found by its file
  (`buffer::find`), so a file in two windows is one text; the last editor of a file gone, the server is told. `editor.rs` is the code editor (a gpui `Element` that shapes
  only the lines on screen; the parse runs on the background executor; Roslyn told every change and asked for
  diagnostics once typing pauses), `theme.rs` every colour, `assets.rs` the carried fonts and icons,
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
- **Every program goes through `rsharp_core::process::spawn`**, so none outlives the IDE.
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

## Code style

Rust 2024, `cargo fmt`'s defaults at 120 columns (`rustfmt.toml`). Comments say why, never what the next lines
do; a `///` summary is at most five lines.
