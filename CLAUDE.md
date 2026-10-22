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
just run [PATH] [--file F] [--caret L:C]   # the IDE on a folder, solution or project
just test [filter]         # every test
just screenshot OUT.png [args]   # the real window captured on macOS (tools/screenshot.sh; WAIT=seconds)
```

`just` isn't required: each recipe is one cargo command. A Debug build optimises dependencies (gpui and
tree-sitter are slow unoptimised). gpui compiles its Metal shaders at start (`runtime_shaders`), so no Metal
toolchain is needed to build.

**Look at UI changes.** After changing anything visible, run `just screenshot` and look at the PNG before calling
it done. It captures the real window (gpui 0.2 has no off-screen capture), so it needs Screen Recording for the
terminal and takes focus for a moment.

## Layout

- `crates/rsharp-core`: no UI. `process` starts every program (`spawn`): each leads a process group that ends
  with it, and `end_all` runs as the IDE quits or panics. `paths` (canonical paths, `file://` URIs), `programs`
  (finds `dotnet` and tools as a login shell would), `files::write` (every save: temporary file and rename).
- `crates/rsharp-text`: no UI. `Document` (Ropey's rope, whose clone is a snapshot another thread may read; undo;
  `Change`s in application order with byte, line and UTF-16 places), `Selection`s, and every editing command as
  a pure function to an `EditPlan`. Offsets are UTF-8 bytes. A file's CRLF is held as `\n` and written back.
- `crates/rsharp-languages`: colour before any server answers: tree-sitter-c-sharp parsed incrementally (`edit`
  then `parse` with the old tree) and coloured by the grammar's own highlights query over the visible bytes
  only, its captures mapped to Rider's kinds (`Kind`).
- `crates/rsharp-lsp`: the LSP client (JSON-RPC over stdio, a reader thread, `ServerFailed` the one failure) and
  `start_roslyn`, which runs the person's `roslyn-language-server` dotnet tool with `--clientProcessId`, so it
  ends with the IDE even after a crash, and opens the solution (`solution/open`) or lets it find the projects.
- `crates/rsharp-app`: the shell. `window.rs` is the main window as Rider's Islands (toolbar, stripe and status
  bar on the frame; Solution and editor as islands), `editor.rs` the code editor (a gpui `Element` that shapes
  only the lines on screen; the parse runs on the background executor; Roslyn told every change and asked for
  diagnostics once typing pauses), `theme.rs` every colour, `assets.rs` the carried fonts and icons,
  `keymap.rs` Rider's keys, `launch.rs` the command line, `workspace.rs` the folder and its solution.

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

Tests say what a person would see or get, and their failure message names the consequence. The long random
editing session (`rsharp-text/tests/editing_session.rs`) checks 40,000 keys, undo and redo against a plain
string: keep it passing rather than narrowing it. The Roslyn test runs the real server on a fresh project and
skips, saying why, when the tool is not installed.

## Code style

Rust 2024, `cargo fmt`'s defaults at 120 columns (`rustfmt.toml`). Comments say why, never what the next lines
do; a `///` summary is at most five lines.
