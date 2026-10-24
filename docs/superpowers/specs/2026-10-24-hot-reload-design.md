# Hot Reload under dotnet watch — Phase 5, slice 4

Status: written from the user's "Implement the Hot reload" on 2026-10-24, after
`2026-10-24-run-design.md` and `2026-10-24-run-configurations-dialog-design.md`.

## Intent and scope

A person turns Hot Reload on, presses Run, and the program starts under `dotnet watch`: a save rebuilds
and applies the change to the running program, or restarts it when the runtime cannot take it, and what
`dotnet watch` made of the save is in the status bar. Stop ends the whole tree, and Rerun starts it again.

This is the roadmap's "Hot Reload under dotnet watch" for a run without the debugger. The debugger's own Hot
Reload (Edit and Continue, `Roslyn.Extension`'s `applyChanges`) belongs to Phase 8, where the debugger is.

**Not in this slice:** Hot Reload while debugging (Phase 8); a program in the Terminal window and its
external console (Phase 6); dotnet watch's restart question as a dialog — the run is `--non-interactive` because
the Run console takes no input until the terminal (Phase 6), so the setting decides instead: on, watch restarts
the program for a change the runtime cannot apply; off, it says a restart is needed and the program keeps its
old code; an agent's `hotReload` override of a run (Phase 12); and `dotnet watch` for a configuration that runs
another program, which Rider does not offer either.

## Reference

Passenger's `Run/BuildRunController.cs` (`RunAsync`'s `watched`, `MakeWorkingFolder`, `Watch`),
`MainWindow.HotReload.cs` (its `WatchSays` table, `SaveForWatchAsync`, `ApplyHotReloadChanges`),
`Settings/AppSettings.cs` (`HotReload`, `RestartOnRudeEdit`) and `Settings/SettingsDialog.cs`'s Hot Reload
page; issue #198. Rider's Hot Reload page and its Apply Hot Reload Changes.

## What the person gets

- **Settings | Build, Execution, Deployment | Hot Reload**: "Run with Hot Reload (dotnet watch)", off until
  turned on as Passenger's is, and "Always restart for a change it cannot apply", on by default.
- **A run with it on** starts the chosen configuration's project under
  `dotnet watch --non-interactive run --project <project> --no-launch-profile [--framework <tfm>] -- <its own
  arguments>`, in the plan's working folder, with the plan's environment — so a launch profile's arguments,
  variables and application URL reach the program exactly as they do without watch. `dotnet watch` builds the
  project itself, so the build before the launch is skipped and its output comes through the Run window.
- **The working folder** is made first when it is not there yet, since watch starts there and a project never
  built has no output folder: as Passenger does, and it says why if it cannot be made.
- **The status bar** says "Running Shop with Hot Reload: a save applies its changes"; the browser opens once
  the program listens, as without watch.
- **A save applies itself** — `dotnet watch` watches the folder — and what it makes of it is in the status bar,
  its words translated: the changes applied, nothing to apply, a restart to apply them (or one under way), or
  that the code does not compile. A run without Hot Reload never reads those words.
- **Run | Apply Hot Reload Changes**, Alt+F10 on both platforms' Rider keymaps (Passenger's key): saves every
  open buffer, since watch reads the files on disk, and says what watch makes of the save. It is available only
  while a program runs under watch, and says why otherwise.
- **Stop** ends `dotnet watch` and the program it started together, and **Rerun** starts it again — both as any
  other run, because the whole tree is the one RSharp started.

## The model

`rsharp-core`'s `run`:

- `plan::LaunchPlan::why_not(verb, built: bool)` — watch's plan is checked without the "has not been built"
  check, since its first build is watch's.
- `plan::LaunchPlan::watching(restart_on_rude_edit: bool) -> Result<Command, String>` — the `dotnet watch`
  command line: the literal arguments above, the plan's working folder, the plan's environment, and
  `DOTNET_WATCH_SUPPRESS_EMOJIS=1` with `DOTNET_WATCH_RESTART_ON_RUDE_EDIT=true` when the setting is on.
- `plan::LaunchPlan::make_working_folder() -> Result<(), String>` — the folder watch starts in, made when absent.
- `watch::verdict(line) -> Option<Verdict>` — what `dotnet watch` said of a save, from its own words, with
  `Verdict::said()` the person's sentence for each.

`crates/rsharp-app`: `settings::AppSettings::hot_reload` and `hot_reload_restart_on_rude_edit`; the Hot Reload
settings page; `window/run.rs` deciding watch at Run, skipping the build, the Apply Hot Reload Changes action,
and reading watch's verdict from the console's lines.

## Out of scope, and why

- **Debugging Hot Reload, the external console, the restart dialog, an agent's override**: Phases 8, 6, 12, as
  above.
- **`dotnet watch` for another program or a library**: nothing to run under it.
- **A per-configuration Hot Reload switch**: Rider has the setting, not the configuration; Passenger agrees.
