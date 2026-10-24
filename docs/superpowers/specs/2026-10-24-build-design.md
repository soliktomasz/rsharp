# Build and the Build window — Phase 5, slice 1

Status: approved by the user on 2026-10-24.

## Intent and scope

A person can build, rebuild and clean the open solution or selected projects, see output as it arrives,
stop the build, and open each reported error at its source. Builds must cooperate with Roslyn's
design-time builds, especially when both would write WPF or Avalonia generated files on Windows.
Typing must stay responsive and everything the build starts must end with its window and the IDE.

This follows the user's first suggested PR slice, on `codex/phase5-build` from `main` at `23f2876`.
PRs #11 and #12 are merged. Run configurations, Run, browser launch, Hot Reload and unit testing follow
in separate slices, each designed and approved first. Run in the terminal remains deferred to Phase 6,
with a `// shortcut:` at its eventual integration point; this slice adds no terminal substitute.

## Reference and approach

Read Passenger's `Run/Building.cs`, the build half of `Run/BuildRunController.cs`, `Run/ProcessRunner.cs`,
`Core/ChildProcesses.cs`, `Workspace/ProjectBuilds.cs`, `Workspace/WorkspaceModel.cs`,
`ToolWindows/OutputView.cs`, `AppActions.cs`, and `tests/Passenger.Tests/BuildRunTests.cs`.
The pinned Host's `Server.cs`, `Protocol.cs` and `DesignTimeBuilds.cs` define the actual wire contract.
Passenger's CLAUDE sections on Run/ToolWindows, Testing and the Host, and issues #155, #198, #203 and
#206 were also read. #203 affects this slice's Windows targeting; terminal debugger attachment,
test attachments and test sharding belong to their later work.

The proposed approach is the existing architecture: Rust owns commands, output parsing and UI;
Passenger's existing Host coordinates its generating design-time builds. Running a second Host would
not coordinate with Roslyn's Host. Shelling out to a wrapper or adding a .NET build sidecar would
duplicate behavior already available through `dotnet` and the Host. No new dependency is expected.
This approach follows the user's requested slice order and was approved with this design.

## What the person gets

- Build Solution, Rebuild Solution and Clean Solution in the Build menu and Find Action; the corresponding
  selected-project actions in the Solution view and Find Action. Use the existing selected row: a project
  means that project, a solution folder means its projects, run in order until the first failure. Adding
  multiple-row selection is outside this slice.
- Rider's platform keys: Cmd+Shift+B / Ctrl+Shift+B for Build, Cmd+0 / Alt+0 for the Build tool window,
  and Shift+F5 for Stop. Existing action availability rules disable a second build while one is pending
  or running, and explain why a build is unavailable without a workspace or target selection.
- A Build tool window, initially bottom-left, participating in the existing docking, floating and saved
  layout. Its left toolbar has Rerun, Stop and Clear. Rerun repeats the last operation and targets;
  it is disabled during a build. Clear clears console text, without discarding reported problems.
- Output follows the end while new text arrives, is selectable and copyable, and distinguishes standard
  error using theme tokens. Long lines can be read without changing the editor's wrap setting. The
  output buffer keeps Rider's 1024 KB limit, dropping oldest text at valid character boundaries.
- Problems gains a Build tab with errors and warnings, location, code and message. Clicking a located
  diagnostic opens its file at the reported line and column. A compiler location in the Build output
  opens the same place; general URL and stack-frame console links come with the Run slice.
- The status bar says what is building and how it ended. Its result opens Problems' Build tab when
  diagnostics exist, or the Build output for an unexplained failure. Missing dotnet, a failed save,
  a missing target or a failed process start gets a concrete message and leaves no running indicator.
- Explicit builds open Build when they begin by default. Settings adds the explicit-build opening
  checkbox and results choices Always / Warnings or errors / Errors / Never, with Errors the default.
  A stopped build counts as neither an error nor a warning. Implicit-build presentation is added when
  Run supplies that use case; this slice does not expose an inactive setting.

Use the existing islands, fonts, focus conventions, settings controls and theme tokens. Keep the toolbar
usable in the short bottom island. No alternate visual style or new general-purpose widget framework.

## Command, targets and lifetime

Add a core `DotnetCommand` equivalent that returns a literal `std::process::Command`: use
`programs::dotnet`, the workspace working directory and separate arguments; start only through
`rsharp_core::process::spawn`. Set English CLI output, no logo, forced UTF-8, no CLI/MSBuild server,
and an IDE-specific `MSBUILDNODEHANDSHAKESALT`, following Passenger. Remove inherited
`MSBUILDENSURESTDOUTFORTASKPROCESSES`, `MSBuildSDKsPath`, `MSBuildExtensionsPath` and `MSBUILD_EXE_PATH`
so the workspace's `global.json` controls SDK selection and child tasks cannot keep the console open.
Use this preparation for the Host as well; preserve Roslyn's existing node isolation when applying it
there if needed. Do not broadly rewrite unrelated template or NuGet execution in this PR.

Build runs `dotnet build TARGET`; Rebuild adds `--no-incremental`; Clean runs `dotnet clean TARGET`.
All use `-nologo`, `-tl:off`, `-clp:NoSummary;ForceNoAlign`. Where evaluated dependencies indicate that
the target or a reachable project requires Windows targeting, add `-p:EnableWindowsTargeting=true`,
following Passenger; a project-reference walk must tolerate cycles. This permits building Windows
projects off Windows, subject to installed targeting packs; running them is the next slice's concern.

Build Solution targets the opened `.sln` or `.slnx`; an explicitly opened project targets that project.
For a folder without a solution, use discovered project paths, including projects below the top folder,
in stable order. If there are no projects, give dotnet the folder and display its explanation. Project
discovery happens off the UI thread. Do not add project unloading as part of this change.

Before acquiring the Host hold or starting a process, save dirty buffers belonging to the workspace
through their existing save/Local History path, deduplicated by buffer identity across split editors
and detached windows. A save failure aborts the build. A build of unrelated external files is unnecessary.
After a successful New Solution creation, build through the same controller once the new workspace is
attached, following Passenger's `MainWindow.NewProject.cs::OpenNewSolution`. Add New Project and opening
an existing workspace do not automatically build, matching Passenger's creation handoff.

The app owns one build session per workspace window: preparing, waiting for the Host, running, finishing,
then idle, with an outcome Succeeded / Failed / Stopped / Refused / NotStarted. Preserve the last
operation and target list for Rerun and an immutable outcome with its diagnostics and last 20 output
lines. Future Run and Tests can consume that outcome rather than inspecting mutable window text.
Stop, Close Solution, switching workspace and closing the window cancel the pending wait, kill the
whole build process tree and release its hold. Late updates from the previous workspace cannot land
in the current one. Drain both streams and the last unterminated line before reporting completion.

## Host coordination

The current `SharedHost` has one mutex over synchronous request/read pairs. Simply adding `hold` to
that path would deadlock: a hold's final reply arrives only after cancellation, while a design-time
build can be waiting for that hold to be released. Replies from other requests are currently discarded.

Give the existing Host client one reader thread that routes partial and terminal replies by request id,
a short lock for complete writes, and pending-request channels. Preserve `build`, `evaluate` and
`inspect` callers' blocking interfaces off the UI thread. The SharedHost lock protects connection
creation/replacement, never waiting for a reply. Register a request before writing it. Host EOF,
invalid protocol or transport failure wakes every affected waiter; a normal request error does not
silently discard another request's answer or kill a valid connection.

Send `hold` with empty parameters. Display its `running` project list while waiting, and start dotnet
only after a partial reply has `held: true`. The hold remains registered after that partial; release
sends a separate `cancel` with `{request: HOLD_ID}`. A cancel acknowledgement alone does not mean the
hold has ended: consume its eventual cancellation reply and remove pending state. Release is safe
before acknowledgement and idempotent. Keep the hold across the complete selected-project queue.

Without carried sidecars there are no Host design-time builds to coordinate; normal dotnet builds
remain available. When a configured Host fails while acquiring a hold, report that failure and abort
this build. If it dies after the build starts, stop the build and release remaining state; reconnect
on the next request after this build is finished. Do not introduce an unheld replacement Host during
the external build. Restarting Roslyn against the same live SharedHost must leave the hold in force.

## Output and diagnostics

Pump stdout and stderr concurrently off the UI thread, keeping stream identity. Strip ANSI sequences
across read boundaries, frame complete lines and keep an incomplete final line. Parse every diagnostic
before console eviction. Accumulate UI updates in bounded batches, applying them per frame rather than
one notification per line; both the producer queue and retained console text must be bounded.

Translate Passenger's MSBuild diagnostic reader: errors and warnings with or without file locations,
optional columns and ranges, paths containing parentheses, Windows drive letters, compiler subcategories,
and trailing `[project]`. Join continuation lines and deduplicate repeated diagnostics within this build.
Resolve relative paths against the emitting project when known, otherwise the build working directory;
canonicalize with the existing path helpers. Convert one-based reported positions to the editor's
zero-based positions. Messages without a source location remain visible without a bogus navigation link.

Keep build diagnostics separate from Roslyn's File/Solution diagnostics and project-load Toolset errors.
A new build replaces its predecessor's diagnostics, not another source's. Counts and failure status use
the process exit as well as parsed errors, so an unknown SDK message or unparsed failure cannot become
success. Console trimming never changes the stored diagnostic set.

Likely homes: `rsharp-core` for dotnet preparation and build parsing; `rsharp-lsp::host` for request routing
and the hold; `rsharp-app` for the output entity and `window/build.rs` controller; existing actions,
keymap, Solution menu, dock, Problems, settings and creation handoff for integration. Exact types and
file splits are the implementer's call. Reuse existing editor text/selection and visible-line rendering
where practical; do not start language services for the console. Non-UI crates retain no gpui dependency.

## Acceptance and verification

- Pure tests cover exact command arguments/environment, target choice and reference traversal; MSBuild
  paths, locationless errors, continuations and deduplication; split UTF-8/ANSI reads and the final partial
  line; console eviction without losing errors. Include platform-specific Windows paths.
- A controllable Host protocol fixture interleaves build/evaluate/inspect and hold replies: cancellation
  while a design-time build is waiting must complete, no reply is lost, and EOF wakes all waiters.
- Live Host tests prove a generating design-time build holds the external build back, release permits
  another design-time build, and Stop while waiting releases the hold. Use a local custom MSBuild target
  with file gates for deterministic coordination, avoiding downloads of WPF or Avalonia packs.
- Window tests use `support::Run`, real keys and debug-selector clicks: build an edited console project,
  see a compilation error in Build/Problems, click to its source, fix and rerun, select projects for
  Build/Rebuild/Clean, stop a gated build, and close/switch workspace without surviving children. Cover
  creation's automatic build and presentation settings. Use `run_until_parked` for UI quiescence and
  bounded explicit waits for real processes; never call it inside an update borrowing the window.
- Offline integration fixtures include a console app and an SDK Web app without package references,
  plus a solution with two projects. Pin an installed SDK as Phase 4 does; isolate CLI home, package
  cache and nuget.config with cleared sources. Live tests skip with a reason without dotnet, the SDK or
  required carried Host. Protocol and UI fixture tests still run without them. Prove a new test fails
  when its build action, hold ordering or diagnostic location is deliberately broken, then restore it.
- Before the PR: check disk space without deleting anything; run `cargo fmt`, `cargo build`, then
  `cargo test --workspace` in the background with output captured to a file. Inspect real screenshots
  of successful and failing builds with the short bottom island, including output/source navigation.
  Run relevant before/after startup, typing and memory measurements from the roadmap where feasible,
  reporting any unavailable evidence. Windows behavior is verified by the existing Windows CI job;
  macOS-only local evidence is never labeled Windows verification.
- Update CLAUDE.md's layout, Phase 5's progress in ROADMAP.md, and notices for any new dependency or
  carried asset. Commit through a message file, push this branch and open a PR against `main` with a
  plain summary and Tests section. Later slices branch from `main` after this slice merges.

This document adds no implementation, build results or claim that Phase 5 is complete. After approval,
write the implementation plan in `docs/superpowers/plans/` before changing product code.
