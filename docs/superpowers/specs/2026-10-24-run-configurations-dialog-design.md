# The folder's own run configurations and Edit Configurations — Phase 5, slice 3

Status: written from the user's choice of this slice on 2026-10-24, following
`2026-10-24-run-design.md` (whose "Out of scope" names exactly this work).

## Intent and scope

A person edits, copies, adds and removes run configurations of their own: a program other than the project's
own, arguments, environment variables, a working directory, whether the project builds first, and the browser
a web project opens. The folder's own are kept with the folder, listed before the projects' generated ones in
the toolbar, and chosen, run and rerun as any other.

This closes the Run bullet of Phase 5 that the previous slice left open. The generated configurations stay what
the projects and their launch profiles say; the folder's own are the person's, and Edit Configurations is where
they are made.

**Not in this slice:** Debug configurations (Rider's dialog has two tabs; RSharp has one until Phase 8),
"Use external console" (a terminal tab, Phase 6), Rider's "Before launch" task list beyond building the
project, and sharing configurations through `.idea/runConfigurations/*.xml` — Passenger keeps them in its own
state, and RSharp follows Passenger so a folder carries no IDE files it did not have.

## Reference

Read Passenger's `Run/RunConfigurationsDialog.cs`, `Run/RunConfigurations.cs` (its `RunConfigurationsState`,
`ForStore`/`FromStore`, `Set`), `Run/RunConfiguration.cs` (its `UniqueName`, `WhyNotHere`), and
`tests/Passenger.Tests/BuildRunTests.cs`'s dialog tests; issue #155.

## What the person gets

- **Run | Edit Configurations…** in the Run menu, Find Action (`edit configurations`) and as the last row of the
  toolbar's run menu, below a rule. It opens Rider's dialog over the window: the configurations down the left
  under Add, Copy and Remove, the chosen one's fields on the right, OK, Cancel and Apply below.
- **The list** holds the folder's own first, then the generated ones marked "(generated)", which change only by
  copying them, as Rider shows launch profiles. A row that cannot run here says why on hover.
- **The fields**: Name; Project (a menu of the folder's projects); Target framework (a menu when the project has
  several); Launch profile (a menu of the project's profiles, which fills the fields below as Rider's does);
  Exe path; Program arguments; Working directory; Environment variables (a row of name and value each, with Add
  variable and Remove); Build the project first; Open browser, and its URL.
- **Add** makes a configuration named "Unnamed" of the first project a person can run, numbered if that name is
  taken. **Copy** makes the chosen row — a generated one too — one of the folder's own. **Remove** takes away a
  own one; a generated one stays, its project's to change.
- **OK and Apply** keep the list and the chosen configuration with the folder and refresh the toolbar; OK closes
  the dialog. A list that cannot be kept — a configuration with no name, two with one name, one with no project,
  a project outside the folder — is refused with the reason at the dialog's foot, the row it is about chosen.
  Cancel changes nothing.
- **Running one** needs nothing new: the toolbar lists it, Run, Rerun and Stop act on it, its arguments,
  environment and working folder go to the program as a generated configuration's do, and its browser opens once
  it listens.

## The model and where it is kept

`rsharp-core/src/run/configuration.rs` gains:

- `RunConfigurations { configurations: Vec<RunConfiguration>, selected: Option<String> }`, a `StateDocument` at
  `runConfigurations.json` in the folder's state — its own file, as `projectModels.json` is the evaluations'.
- Stored projects relative to the folder (`for_store(folder)`), read back absolute (`resolved(folder)`), as
  Passenger stores them: the file reads plainly and a folder moved keeps its configurations.
- `problem(&self, taken: &[String], folder: &Path) -> Option<String>`: why a configuration cannot be kept, in
  the person's words — no name, a name another has, no project, a project outside the folder.

The chosen configuration moves from `FolderState.run_configuration` (added by the previous slice, never read
elsewhere) into this document's `selected`, so there is one place that says what the toolbar shows.

## Out of scope, and why

- **Debug configurations, Use external console, Before-launch tasks, .idea sharing**: Phases 8 and 6, and
  Passenger's own choices, as above.
- **Editing a generated configuration in place**: Rider copies it first, and so does RSharp.
- **Configurations for a folder's projects not in its solution**: the list still asks only the open solution's
  projects, as the previous slice does.
