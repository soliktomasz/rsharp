# NuGet (Phase 4, first part)

## Why

Phase 4 is done when a solution can be built up to three projects "with references and packages", its files
byte-identical to Passenger's for the same steps. References work; packages don't. This part covers finding a package
and putting it in a project, through the same `dotnet add package` that Passenger runs, so the bytes match.
Passenger's `Packages/` and `ToolWindows/NuGetView.cs` are the reference; where they leave something open, Rider
decides.

It is built in two PRs:
1. **This one:** the folder's package sources as restore reads them, v3 and folder feeds, search, versions, the
   NuGet tool window, Install, Update and Remove.
2. **Next:** credentials (`packageSourceCredentials`, `NuGetPackageSourceCredentials_*`), credential providers
   (NuGet's plugin protocol), v2 (OData) feeds, known vulnerabilities (`VulnerabilityInfo`, `auditSources`), and the
   Sources tab's editing of a nuget.config with every other byte kept.

## Goals (this PR)

### Sources: `rsharp-core::nuget::sources`

The feeds a folder's restore would use, read as NuGet.Configuration reads them, with no process:

- The nuget.config files in NuGet's order. From the solution's folder up to the root, the first of `nuget.config`,
  `NuGet.config` and `NuGet.Config` in each folder. Then the user's file (`~/.nuget/NuGet/NuGet.Config`, on Windows
  `%APPDATA%\NuGet\NuGet.Config`, and the extra `*.config` files in its `config/` folder). Last, the machine's
  (`/Library/Application Support/NuGet/Config` on macOS, `/etc/opt/NuGet/Config` on Linux,
  `%ProgramFiles(x86)%\NuGet\Config` on Windows).
- `packageSources`: a key in a closer file wins. A file's `<clear />` drops every feed from the files after it.
  `disabledPackageSources` from any file applies. `%VAR%` in a value is expanded. A relative folder is made absolute
  against its file's folder.
- With no user file, nuget.org is assumed, as NuGet would write it. RSharp never writes the person's home to make it.
- `packageSourceMapping`, as restore decides: a package may come from a feed when one of that feed's patterns is the
  package's longest match.
- Each file is listed with its place (Solution, Folder, User, Machine), its feeds and any problem reading it. One
  unreadable file is a problem on that file, not on the whole set.
- Read off the UI thread. Read again when the watcher sees a nuget.config change.

### Feeds: `rsharp-core::nuget::feed`

- **v3:** its service index names the search (`SearchQueryService/3.5.0`, then older names), the registrations
  (`RegistrationsBaseUrl/3.6.0` first, the hive that includes SemVer 2 versions) and the flat container
  (`PackageBaseAddress/3.0.0`). Search asks for 30 hits, `semVerLevel=2.0.0`, with prerelease as chosen. Versions come
  from the registrations, following paged ones and leaving out unlisted versions; a feed without registrations uses
  the flat container.
- **Folder:** a folder of `.nupkg` files, flat or in NuGet's `id/version/` layout. Each package's id, version,
  description and authors come from its `.nuspec`. A file that isn't a package is skipped.
- **All Feeds:** every enabled feed asked in parallel. A package found in several feeds is listed once, at its newest
  version, with the feeds that have it. One the mapping keeps out of a feed isn't offered from that feed, and the
  status line says how many were left out. With several feeds, the list is ordered by how well the id matches before
  downloads, so a private feed's packages don't sink. A feed that fails is named in the status line with why; the rest
  still answer.
- **Versions** in NuGet's order (SemVer 2 precedence: `1.0` is `1.0.0.0`, a prerelease sorts before its release,
  labels compare dot by dot, numbers by value and below words, case ignored, build metadata ignored).
- HTTP through `ureq` (rustls, the platform's certificates, gzip), with RSharp's user agent. A 401 or 403 says the
  feed needs credentials, which the next PR adds.

### Changing a project: `rsharp-core::nuget::change`

- Install or Update: `dotnet add <project> package <id> --version <version>`. Remove:
  `dotnet remove <project> package <id>`. Both run through `process::spawn`, bounded and cancellable as template
  creation's commands are. An id or version that is empty or starts with `-` is refused before anything runs.
- Before running: open copies of the project file and of any `Directory.Packages.props` above it are saved first, and
  the old bytes of each go into Local History.
- After running: open buffers of the files that changed take the new text from disk, and the project is evaluated
  again, so the Solution view's Dependencies follow.
- dotnet's output goes to the window's log, line by line.

### The NuGet tool window

- `Tool::NuGet`, home at bottom left, in the Tools menu, with Passenger's keys: `cmd-7` / `cmd-shift-7` on macOS,
  `alt-7` / `alt-shift-7` elsewhere. Only registered while a solution or project is open.
- **Packages tab:** a search field (300 ms pause before asking), the feed chooser (All Feeds, then each enabled feed)
  and a Prerelease checkbox.
  - With nothing typed, the list shows the packages the solution's projects already have. Otherwise it shows what the
    chosen feeds find, installed ones marked.
  - Choosing a package shows its id, description, authors and feeds; a version choice (newest first); and a row per
    project with the version it has, if any, and Install, Update or Remove.
  - A log below shows dotnet's output.
  - A status line says what was found, which feeds failed and why, and how many packages the mapping left out.
- **Sources tab (read-only here):** each nuget.config in effect, in order, with its place, its feeds (enabled or not,
  name and URL or folder) and any problem reading it.
- Colours are theme tokens; keys follow Rider's.

## Not in this PR

Everything in the second PR above. Also MCP `nuget_install`/`nuget_remove`, package icons, the readme pane, and
Rider's "Update all" across projects.

## Tests

- Offline only; no test reaches nuget.org.
- Sources: nuget.config fixtures for closer-wins, `<clear />`, disabled feeds, relative folders, `%VAR%`, the missing
  user file and the mapping's longest match. The user and machine paths come from a test override, never the
  person's.
- Feeds: a stand-in v3 feed on loopback (a small `std::net` server in the tests) serving a service index, a search, a
  paged registration and a flat container. A folder feed made of real `.nupkg` files written by the test.
- Versions: NuGet's ordering cases.
- Window tests drive the NuGet window against a folder feed: search, choose, Install into a project with `dotnet add`
  (skipped, saying why, without dotnet), the project file's bytes checked, and Remove.
- The two refusal cases (an id starting with `-`, an empty version) never start a process.

## Dependencies

`ureq` (MIT or Apache-2.0) with rustls and the platform verifier, sharing the `rustls` gpui already brings in. Added to
THIRD_PARTY_NOTICES.md in the same change. `quick-xml` and `async_zip` are already here.
