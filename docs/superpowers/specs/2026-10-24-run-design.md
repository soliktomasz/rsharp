# Run configurations and Run — Phase 5, slice 2

Status: written from the user's "GO with phase 5 slice 2" on 2026-10-24, following
`2026-10-24-build-design.md`.

## Intent and scope

A person picks a run configuration for the open solution or folder, presses Run, and the project builds
(if its configuration says so) and starts: its output streams into a Run window, its exit code is in the
status bar, Stop ends it, and a web project's browser opens once the program listens at its URL. Launch
profiles (`Properties/launchSettings.json`) supply a configuration's arguments, environment, working
directory and browser address.

This follows the slice order the build design fixed: Run configurations, then Run. Hot Reload under
`dotnet watch` and unit testing stay for later slices, and Run in the terminal stays with Phase 6, as the
roadmap says; a `// shortcut:` marks where the console would hand its program a terminal.

**Not in this slice:** the folder's own configurations and Rider's Edit Configurations dialog. Every
configuration this slice runs is generated from the folder's executable projects and their launch profiles,
which is what a person runs in a fresh solution; the dialog comes next, on the same `RunConfiguration`
data, which is already what the state store would keep.

## Reference and approach

Read Passenger's `Run/LaunchProfile.cs`, `Run/RunConfiguration.cs`, `Run/RunConfigurations.cs`,
`Run/LaunchPlan.cs`, `Run/BrowserLaunch.cs`, `Run/ProcessRunner.cs`, the run half of
`Run/BuildRunController.cs`, and `tests/Passenger.Tests/BuildRunTests.cs`; issues #155, #198 and #206.

Rust owns the model, the command line and the console; nothing new runs in .NET. What MSBuild must say
about a project (`TargetPath`, `ProjectDir`, and any property a profile's values name) is asked of
`dotnet msbuild -getProperty`, as Passenger asks it, once per launch and off the UI thread: it needs no
build and no Host, so a launch works whether or not Roslyn's Host is running. The build before a launch
goes through the existing build controller, so the Host's hold, output bounds, Problems and Stop behave
exactly as an explicit build's do.

A run's program starts through `rsharp_core::process::spawn` — its process group (macOS, Linux) or job
(Windows) — so nothing outlives the IDE, exactly as a build does. Its output crosses to the UI on a
bounded channel like a build's, and Stop ends the whole tree.

## What the person gets

- **Run configurations** for the open solution or folder: one per launch profile of every executable
  project, named "Project: profile" as Rider names them, and the project itself when none of its profiles
  runs here. A profile of a kind that cannot run here (IIS Express, Docker, WSL 2, a Roslyn component) is
  listed with the reason, and Run refuses it saying that reason.
- **A selector in the toolbar** beside the project widget: the chosen configuration's name, and a menu of
  every configuration, the ones that cannot run here greyed with their reason on hover. The choice is kept
  with the folder (`FolderState.runConfiguration`), and a configuration that disappears takes the first
  with it.
- **Run, Stop and Rerun** in the Run menu, Find Action and the Run window's toolbar, on Rider's keys as
  Passenger binds them: Ctrl+F5 (Run), Shift+F5 (Stop, shared with a build's Stop), and Alt+Shift+F5 on
  macOS, Ctrl+Shift+F5 elsewhere (Rerun). Run is unavailable with no configuration, says why when the chosen one
  cannot start, and greys a second Run while one is launching or running.
- **A Run window**, Rider's, docked bottom-left beside Build: the program's output as it arrives, standard
  error in the error colour, following the end, bounded at Rider's 1024 KB. Its toolbar has Rerun, Stop
  and Clear. `Cmd+4`/`Alt+4` shows it. The command line it started is its first line, as Rider's console
  shows it.
- **Links in the Run console**: Cmd/Ctrl+click on a URL opens it in the browser, and on a stack frame
  (` at Namespace.Type.Method() in /path/File.cs:line 42`) opens that file at that line. Compiler
  locations keep working as Build's do.
- **The browser** at a configuration's URL, once something listens at its host and port — the tries
  Passenger's `BrowserLaunch` uses — never on a refused connection: the status bar says
  "Opening http://localhost:5000/…" and, when the program never listens, "Unable to open … because the URL
  does not respond". `0.0.0.0` and `[::]` open at `localhost`. A URL with a wildcard host (`http://*:5000`)
  is read as Rider reads it.
- **Build before launch**, as the configuration says: the project builds first through the build
  controller, its output in the Build window, and a failed build starts nothing — the status bar says the
  build failed and the Run window says the run did not start. Dirty buffers are saved first, and a save
  that fails stops the launch, as an explicit build's does.
- **The status bar** says "Running Project: profile…", "Project: profile exited with code 0", or why the
  run did not start (no .NET SDK, a working folder gone, MSBuild's refusal, a profile this IDE does not
  run).
- Leaving the workspace, closing the window or Stop ends the program's whole tree; a second Run waits for
  the first to end.

## The model

`rsharp-core/src/run/`:

- `profile`: `LaunchProfile` — a `Properties/launchSettings.json` profile as `dotnet run`, Visual Studio and
  Rider read one (lenient JSON: comments and trailing commas allowed), with `Unsupported` naming why a
  profile's kind does not run here, the environment a run of it gets (the profile's own variables, and its
  `applicationUrl` as `ASPNETCORE_URLS` unless it sets that), and the browser URL its `launchUrl` makes
  against its first `applicationUrl`.
- `command_line`: `split` — a command line as the program's arguments, split as .NET splits one (spaces
  separate, double quotes group, backslashes escape only the quotes they precede).
- `configuration`: `RunConfiguration` — data alone: name, project, framework, launch profile, program,
  arguments, environment, working directory, whether it builds first, the browser URL. `generate` makes
  the folder's from its projects' profiles; `unique_name` names a copy as Rider does; `why_not_here` says
  why one cannot launch among the projects loaded now (its profile's kind, or a project that runs only on
  Windows, elsewhere).
- `plan`: `LaunchPlan` — what a configuration launches, worked out once: the program (the `dotnet` host and
  the project's built assembly, or another program), its arguments, working folder and environment, the
  build that comes first, and the file the build makes. `resolve` takes MSBuild's properties, so it is
  testable without a build; `properties` asks MSBuild for them.
- `browser`: `address` (a profile's URL as an address, wildcard hosts read as Rider reads them), `shown`
  (what the browser is given: a wildcard host is this machine's), `listening` (Passenger's tries, every
  half second then every second then every five, ending early when the run ends).
- `link`: the console's links — a URL at an offset, or a stack frame's file and line.

## Out of scope, and why

- **Edit Configurations** and the folder's own configurations: the next slice, on this data.
- **Hot Reload** (`dotnet watch`), **unit tests**: later slices of Phase 5.
- **A program's standard input**: the Run console is read-only, as Rider's is until its terminal mode;
  `// shortcut:` where the terminal will come in.
- **Debug** (Start Debugging) and **Profile**: Phases 8 and 11.
- **Run in the terminal**: Phase 6, as the roadmap says.
- **Multi-row selection** of projects, as in the build slice.
