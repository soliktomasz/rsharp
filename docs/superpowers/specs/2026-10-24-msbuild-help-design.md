# Code help in MSBuild files (Phase 4's last item)

Passenger's `MSBuild/` (`MSBuildService`, `MSBuildDocument`, `MSBuildCompletion`, `MSBuildNavigation`,
`MSBuildSchema`) and its Host's `inspect` (`Inspection.cs`) are the reference. A project file, `.props` or `.targets`
gets completion, a hover card and Go to Declaration, and its evaluation's errors as problems.

## Pieces

- **The Host's `inspect`** (`rsharp-lsp::host`): one project evaluated for code help, with the solution's global
  properties and the unsaved text of open MSBuild files. It returns the properties (value, origin, the places that
  set them), item types and the items each element made, targets and where they're defined, imports, the SDK's
  frameworks, the errors, and the file table its places index.
- **Schema** (`rsharp-core::msbuild::schema`): dotnet/msbuild's `Microsoft.Build.Core.xsd` and
  `Microsoft.Build.CommonTypes.xsd` (MIT), carried in `rsharp-core/assets/msbuild/` because the SDK doesn't ship
  them. Read once, on first use, off the UI thread. From CommonTypes come the properties, item types with their
  metadata, and tasks with their parameters, each with its documentation. Core.xsd's structural elements (Project,
  PropertyGroup, ItemGroup, Target, Import, Choose, and the rest) and their attributes are a small table written
  from it.
- **Where the caret is** (`msbuild::context`): a tolerant scan of the text as typed up to the caret, which tells:
  an element's name (and the path of elements it's inside), an attribute's name or value, element text, or a
  `$(`, `@(` or `%(` reference.
- **Help** (`msbuild::help`): for each place, the suggestions (each with the range it replaces and an
  explanation), the hover text, and the declaration.
  - **Completion:**
    - inside PropertyGroup: the schema's properties and the evaluation's own;
    - inside ItemGroup: item types;
    - inside Target: tasks, PropertyGroup and ItemGroup;
    - structural children of each structural element;
    - an item's metadata, as an attribute or an element;
    - `$(` gives properties, `@(` item types, `%(` metadata;
    - DependsOnTargets, BeforeTargets, AfterTargets and CallTarget's Targets give targets;
    - TargetFramework(s) text gives the SDK's frameworks;
    - PackageReference and PackageVersion's Include or Update give package ids from the folder's feeds, and
      Version gives that package's versions (the app asks NuGet for these).
  - **Hover:** a property shows its evaluated value, origin and where it's set; an item element shows how many
    items it made and the first of them; a target shows where it's defined; an import shows the file it brought in.
  - **Go to Declaration:** a property goes to where it's last set, a target to its definition, and an `Import` or
    `Sdk` to the file it brought in.
- **The app:** a buffer of an MSBuild file is given the window's Host and its project (itself; else the
  solution's project whose evaluation's inputs include it; else the nearest project above it). It's inspected
  once typing pauses (500 ms), and the evaluation's errors in this file become the buffer's problems. The editor's
  completion, hover and Go to Declaration use `msbuild::help` for such a buffer in place of Roslyn. Completion opens
  as Roslyn's does on a word's first letter, and also after `<`, `$(`, `@(`, `%(` and an attribute's opening quote.

## Not in this PR

- The SDK's and packages' own MSBuild files opening read-only.
- Central package management adding a `PackageVersion` when a new `PackageReference` is completed.
- Advisories on package versions.
- MCP's MSBuild tools.

Each is marked with a `shortcut:` comment where it would go.

## Tests

- Context and help are tested on text, with a made-up insight.
- The schema is tested against known names (TargetFramework's documentation, PackageReference's metadata, the Copy
  task's parameters).
- `inspect` gets a live test against the real Host on a fresh project, skipped without the sidecars.
- Window tests: completion inside PropertyGroup on an SDK project, the hover's value of `$(TargetFramework)`, Go to
  Declaration of `$(…)` to where it's set, and an unknown SDK shown as a problem.
