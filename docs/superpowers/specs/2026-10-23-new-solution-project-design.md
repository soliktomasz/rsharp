# New Solution and New Project — Phase 4

## Intent and authority

A Rider user can start a solution, then add projects to it without leaving RSharp. This is Phase 4's next
part after the Solution view and its edits. NuGet and MSBuild code help follow as separate parts. The final
Phase 4 acceptance remains: create a solution, grow it to three projects, add references and packages,
and obtain the same files Passenger produces for the same steps.

Passenger's `src/Passenger.App/Templates/` supplies the behavior, especially `ProjectTemplates.cs`,
`TemplateFamilies.cs`, `NewProjectDialog.cs`, `ProjectTemplates.Ahead.cs`, `TemplatePacks.cs` and
`DeveloperTools.cs`. `MainWindow.NewProject.cs` supplies integration; `NewSolutionTests.cs` supplies
observable acceptance cases. `CLAUDE.md`, `PRODUCT.md` and `ROADMAP.md` Phase 4 constrain the work.
Where those leave a behavior or visual detail open, installed Rider decides. Rider 2026.2.3.1's live
New Solution dialog was inspected during design on 2026-10-23.

The user chose a direct Rust implementation with one shared gpui dialog. The user also explicitly chose
**create, restore and open now; automatic build belongs to Phase 5**. No browser mockup is required;
review stays in text, with Rider as the visual reference.

## Goals

- New Solution from Welcome, File, Find Action and the project widget.
- New Project from the solution or a solution folder's Add menu, Find Action and Rider's shortcut.
- Discover what the effective SDK's `dotnet new` actually has installed, including third-party templates;
  present Rider's families, languages, frameworks, Types and the options intended for an IDE.
- Offer Passenger's known template packs when missing; Install uses `dotnet new install` and shows its output.
- Create through `dotnet new`; add projects with `rsharp_core::solution_edits`, preserving existing bytes.
- Open a new solution through the existing project-window path, with Recent Solutions updated. An added
  project appears selected in its solution folder, evaluated and available to Roslyn.
- Keep the window responsive during discovery, installation, creation and restore. Cancellation stops
  child processes and cleans up only the creation's owned output.

## Non-goals and anti-goals

Build/run commands and automatic build are Phase 5. NuGet's package manager, MSBuild editing help, arbitrary
template-pack browsing/uninstall, workload installation, an SDK installer/selector, custom file templates,
Git initialization and Docker generation are outside this part. Rider's unrelated generators (CMake,
Godot, databases and its synthetic Empty Solution entry) are not added to Passenger's catalog here.
Languages supplied by templates may be created even where RSharp has no language-specific editor help yet.

Failure includes a fixed list replacing installed template discovery; loss of options on switching Types;
overwriting user files; a whole solution rewritten by `dotnet sln add`; proprietary tools surviving in
generated projects; background work blocking the UI or delaying ordinary editor keys; and a new solution
replacing an unrelated open workspace instead of following RSharp's existing window policy.

## Approaches

1. **Direct Rust implementation — selected by the user.** Catalog parsing, family rules and creation live
   in `rsharp-core`; the gpui dialog and window integration live in `rsharp-app`. Reuse the existing process,
   program discovery, solution edit, watcher and project-window facilities. This matches the roadmap's
   Rust IDE and avoids another runtime/protocol for local metadata and CLI orchestration.
2. **A new .NET template sidecar — rejected.** It could reuse .NET JSON/ZIP facilities, but adds a process,
   packaging and a protocol for logic Passenger currently keeps in its IDE. It offers no required template
   engine capability that `dotnet new` does not already provide.
3. **A curated template list — rejected.** Smaller initially, but fails installed-template parity, SDK
   changes, third-party families and metadata-driven options.

This is a project-sized change. The design fixes behavior and ownership; the plan will split catalog,
creation, dialog and integration into independently verifiable steps without introducing a template framework.

## Entry points and context

New Solution has no default shortcut, as in Passenger/Rider. New Project uses Cmd+Shift+N on macOS and
Ctrl+Shift+N elsewhere, through the existing keymap/action registry. It is disabled with a reason when no
solution is open. Add → New Project is offered only on a solution or solution folder. The existing Add
popup and its Cmd+N / Alt+Insert keys also reach it.

The shared dialog has two modes:

| Context | Names and destination | Completion |
| --- | --- | --- |
| New Solution | Solution name, project name, parent directory, same-directory checkbox, supported solution formats | Open the new solution through `projects::open_project` |
| New Project | Project name, parent directory; capture the target solution and optional logical solution folder | Add all generated projects to that captured solution |

For New Solution, offer the parent of the most recent solution's directory, falling back to the home
directory as Passenger's `BesideLast` does. For New Project, offer the physical directory corresponding
to the selected solution folder's complete logical path if it exists; otherwise the solution's directory.
A selected project or file does not imply a logical solution folder for the global New Project action.

Capture the originating window and target solution identity. Only one creation dialog per originating
window is active (my implementation call); invoking it again brings it forward. Do not attach an async
result to another solution if the owner closes or changes workspace. Cancel restores focus to the owner.

## Catalog and SDK discovery

Follow Passenger's data model: catalog → families → Types (template groups) → language/framework variants
→ options. Store SDK version, supported solution formats and the SDK's preferred format in the catalog.
Group templates by `GroupIdentity`, falling back to `Identity`; precedence is newest first, languages C#,
F#, VB, then other names. A group's representative metadata comes from its newest usable variant.

Resolve `dotnet` with `programs::dotnet`. Resolve the SDK with `dotnet --version` in the catalog context:
the offered directory for New Solution, the target solution's directory for New Project, as Passenger
does. `dotnet new list --type project` refreshes the SDK's cache; parse `templatecache.json` from the
custom hive or `.templateengine` beneath `DOTNET_CLI_HOME`, otherwise the platform home. Support both
`dotnetcli/<sdk>/` and `dotnetcli/v<sdk>/`. Cache reads tolerate a concurrent writer with Passenger's
bounded retry; failures are reported and retriable, never cached permanently as an empty catalog.

Read project and solution templates from the cache. New Project excludes solution templates; New Solution
includes both. Honor OS constraints, omit WPF/Windows Forms off Windows, and filter .NET Framework-only
and Windows-only frameworks as Passenger does. Do not synthesize workloads that the SDK does not list.
Discover `.sln` / `.slnx` choices and default from the `sln` template; old SDKs with no format parameter
offer only `.sln` and receive no `--format` argument.

The CLI host metadata supplies long option names and hidden flags. Read each template's adjacent
`ide.host.json` from its folder or mounted package ZIP, opening a package once per read. Accept the same
JSON comments/trailing commas and case-insensitive host properties as Passenger. ZIP support is the one
likely direct dependency addition; use an already available suitable crate if practical, with minimal
features, its open-source license and a notices entry. Do not call an OS ZIP utility or start a sidecar.

IDE metadata determines visible options, order, labels, inverted booleans and HTTPS handling. Without it,
fall back to visible parameter metadata. Omit implicit/internal parameters and the name, language,
framework, skip-restore and other parameters Passenger's `NotShown` excludes. Preserve defaults, choice
labels/descriptions, requiredness and enable conditions. Support boolean, choice, text, integer and float
options. Evaluate Passenger's condition grammar (`!`, parentheses, `&&`, `||`, `==`, `!=`, literals and
option values); an unreadable condition leaves the option enabled as Passenger does.

## Families, search and retained choices

Port `TemplateFamilies`' predicates, precedence and ordering directly. Product classifications win first
(AI, Aspire, Azure Functions), then Unit Test, MAUI, Desktop, Web, Class Library, Services, Console by
their predicates. Display families in Rider's order: Console, Class Library, Desktop, Web, AI Extensions,
Services, Unit Test, Aspire, Azure Functions, MAUI; the remaining families follow alphabetically.

Other templates group by short-name prefix only when their display names have a shared word prefix;
otherwise each stands alone. Preserve Passenger's Type-name cuts, installed-first ordering, leading
short names and project-before-solution tie-break. In particular, Avalonia is its own family and test
templates from other authors can join Unit Test. A family of one has no Type chooser.

Search is case-insensitive and requires every word to match the family or a Type's name, shortened name
or short names. A query naming a Type selects that Type inside its family. Remember the last Type per
family and last chosen template across no-match queries. Clearing search recovers the choice. Remember
the person's option values by symbol name; reuse them on another Type/framework/language where valid,
discarding a saved choice only when that choice is unavailable. Keep a compatible language and framework
on switching Types, otherwise use the template's defaults.

Names start with the template's default base plus the first free positive number. For New Solution,
project name follows solution name until the person edits it independently. Changing Type retains names
the person typed; untouched names follow the new template. The initially focused name remains selected
even when asynchronous discovery supplies the initial default, so typing replaces it.

## Dialog and visual behavior

Mode: **Operate**. Audience: a Rider user creating a .NET project, usually by keyboard. Inherit RSharp's
Rider Islands Dark, Inter chrome, existing field/button/menu patterns and `theme.rs` tokens. No new visual
world or general design-system rewrite. Keep the native window chrome and platform button ordering.

Use Passenger's 920×640 starting size and 720×480 minimum as the initial geometry, checked against the
installed Rider reference and the real RSharp window. A searchable family list occupies the left pane
(Passenger's 260px starting width); the scrollable right pane contains names/location, the path preview,
Type, target framework, language and template options. Footer actions and errors/progress stay visible
while the right pane scrolls. Long paths, descriptions and labels wrap or truncate with access to their
full value; they cannot push Create or Cancel out of view. Reuse settings/dialog controls, not editor views
for ordinary input fields. Test the minimum size and the default size.

New Solution initially chooses Console/C# where available; otherwise the first available family/Type.
New Project follows Passenger's same initial catalog order. Show only relevant fields: a solution template
hides project name, same-directory and format, since it creates its own solution. A single language does
not need a chooser; a single framework remains readable but does not imply a choice. Show Passenger's
command preview from actual argument construction, without output/internal switches. Displayed quoting
is presentation only: execution always passes an argument array directly to `Command`.

Focus the solution/project name on open and select its default. Tab/Shift+Tab traverse available controls
in reading order; space toggles checkboxes, arrow keys operate lists/choices, Enter invokes Create when
available. In search, Up/Down select a family and Escape first clears a non-empty query. Otherwise Escape,
Cancel and closing the window cancel the operation. Busy disables mutating inputs/Create/Install but
keeps cancellation usable. A failed validation keeps the dialog and all entered values with a concrete
reason; errors are textual, not color-only. All interactive controls expose names and visible focus.

States: loading templates, ready, no matches, catalog failure with retry, missing pack, installing with
scrollable output, installation failure, creating/restoring, validation failure, cancellation/cleanup
failure, and successful handoff. A preliminary cached catalog can render immediately, but Create cannot
use another SDK's stale read while the dialog's authoritative read is unfinished or has failed.

## Template packs and catalog lifetime

Offer Passenger's recorded kinds for Avalonia, Aspire, Azure Functions and AI Extensions only where their
short names are missing. Preserve their package identities, version provenance, descriptions and project/
solution distinctions. These are offers, not installed entries; no network fetch is needed to show them.
MAUI remains workload-discovered. Choosing a missing Type shows its package offer and Install, with Create
disabled. Installation is an explicit user action, using `dotnet new install <id>` in the chosen existing
directory (otherwise the original catalog context), so its NuGet configuration applies. Stream stdout and
stderr to a bounded plain output area; no terminal emulator is needed.

After successful installation, invalidate and reload the catalog, retain the chosen short name and Type,
and show its real metadata/options. An installation that yields no matching template is an error. Cancel
stops the install but does not pretend to roll back a package `dotnet` already installed.

Share immutable catalogs per SDK and hive/home context within the process; share an in-flight read and
retry failed reads. As Passenger does, preload once after the first normal window is painted, never on
the input/draw path. Tests, screenshots and MCP do not preload; dialog tests request their own catalog.
Keep the most recent successful catalog available during refresh. Watch the template home's `packages.json`
and the .NET installation's SDK/template roots, debounce changes for 500ms, invalidate and reread recently
requested contexts (at most four), then notify open dialogs on the UI executor. Do not watch or repeatedly
scan the user's entire home. Refresh preserves choices and cannot alter an active Create's captured plan.

## Creation, validation and ownership

Validate before writing: selected installed template, non-empty valid names, absolute parent path,
case-insensitive project-name collisions in the target solution, output path that is not a file or a
non-empty directory, and required/numeric enabled options. Refuse path separators, `.`/`..`, NUL and
Passenger's cross-platform invalid-name characters. Recheck the captured destination and target solution
at Create time; never use `--force`. Surface filesystem errors without replacing them with empty state.

| Mode | Root owned by creation | Project output |
| --- | --- | --- |
| New Solution, project template | `<parent>/<solution>` | `<root>/<project>`, or root when same-directory is checked |
| New Solution, solution template | `<parent>/<solution>` | root; the template supplies its projects |
| New Project | `<parent>/<project>` | root |

For a project-template New Solution, run `dotnet new sln --name … --output …` with the supported selected
format, then the project template. For solution templates, let the template create its solution; choose
the top-level `.sln`/`.slnx` matching the requested name first, then ordinal path order. If it creates no
solution, create one with `dotnet new sln`. Enumerate generated `.csproj`, `.fsproj` and `.vbproj` files
outside bin/obj; requested project name first, then ordinal path. No projects is a failed creation.

Arguments match Passenger: template short name, name, output, `--no-update-check`, optional skip-restore,
language/framework, then only enabled options whose values differ from defaults (case-insensitive
comparison). Skip restore where the template exposes that option. Apply `DeveloperTools.TakeOut`'s exact
targeted removals of Avalonia Accelerate package/call lines, retaining BOM and every other byte, then
restore each generated project once where the template's restore was skipped. A restore failure leaves
the created project available as Passenger does, but its failure is shown/logged; it does not erase a
sound generated project. No package manager implementation is introduced by this restore.

All commands use `rsharp_core::process::spawn`, with concurrent stdout/stderr draining, captured exit
status, cancellation and Passenger's bounded timeouts (version 1 minute, list 3 minutes, create/restore
10 minutes). Keep filesystem work and waiting on the background executor. Dropping UI tasks must not
detach processes or abandon cleanup; cancellation first stops and waits for the child tree, then cleans.

On failure/cancel before successful handoff, remove the newly created root, or the generated contents if
the root existed empty. Preserve the pre-existing empty root. Never delete parents or an existing
non-empty directory. Track ownership so a changed destination or unrelated file appearing during the
operation is not recursively deleted. A cleanup failure names what remains; the operation never claims
success. After generation succeeds, a failure to attach projects to a solution retains the generated
files and reports their paths rather than deleting useful output or silently returning success.

## Solution and window integration

For a new solution, ensure every generated project is included via the existing `solution_edits` rules;
an entry already supplied by a solution template is success, not duplication. Then defer opening through
`projects::open_project`, which reuses Welcome or opens another window and records the canonical solution
path in recents. Do not call `dotnet sln add` or add a competing solution writer. The first workspace
evaluation and Roslyn loading follow the ordinary open path. Automatic build is absent by agreement.

For New Project, revalidate that the owner still has the captured solution and logical destination.
Use `MainWindow::add_existing` (or extract only the small shared portion needed), retaining its
`write_edited` path: save modified open solution buffer first, preserve encoding/BOM, record Local
History, edit only project entries/configurations/nesting, reload the buffer/tree, evaluate and tell
Roslyn about the added projects. Use RSharp's existing reload behavior; do not require Passenger's
server-object restart when RSharp's protocol already loads additional projects. Reveal the destination
folder and select the last generated project, as Passenger does. Existing editors and references remain.

Batch projects through the existing text edit function before the solution write when practical, so a
multi-project template cannot leave a partly attached set on a later validation failure (my call).
Whichever small integration is chosen must report attachment failure and retain the generated files.

## Scope of changes

- `rsharp-core`: template metadata/catalog/families/packs, condition evaluation, CLI creation/cleanup and
  Avalonia take-out; exports, focused fixtures/tests, and a ZIP dependency/notices only if needed.
- `rsharp-app`: shared `new_project_dialog` module and the window's small integration module; `lib.rs`
  exports, `window.rs` action wiring, `actions.rs`, `keymap.rs`, `solution_menu.rs`, Welcome and project
  widget entry points. Reuse `TextField`, dialog/theme helpers, state/recents, solution writes and loading.
- Window/core tests and fixtures; update CLAUDE/ROADMAP to describe only what is actually delivered.
- No changes to Passenger, the pinned submodule, its Host protocol or Roslyn extension for this feature.

The builder chooses module boundaries, Rust types and minimal dependency features. Do not introduce a
generic job system, form framework, terminal view or persistent second template database. Existing
template cache files are the disk source of truth; RSharp's catalog memory is disposable.

## Verification and acceptance

Translate the applicable `NewSolutionTests` through gpui's real-window test harness:

1. Welcome → New Solution, type over the focused default, toggle Console's top-level-statements option,
   Create; inspect generated Program.cs, solution entries, opened path and recents. Verify restore/open;
   leave Passenger's automatic build/run assertions to Phase 5. A test may invoke `dotnet build` solely
   to prove the generated artifact is valid, without adding build UI.
2. Solution folder → Add → New Project, type `Shop.Data`, choose Class Library; both `.sln` and `.slnx`
   gain only the expected project/configuration/nesting text. It appears in `src`, is selected, and the
   language service sees it. A physical matching `src` is preferred; a missing one falls back to root.
3. Empty/invalid names, duplicate names and non-empty destinations are refused before writes, with
   Passenger's concrete messages. Escape and Cancel leave the solution and existing files unchanged.
4. Installed catalog matches `dotnet new list` on the same SDK/hive, subject to the same OS/framework
   filters; tests do not assume the machine has a workload. Family, search, options, name-following,
   cached refresh and keyboard cases use deterministic fixture catalogs.
5. An isolated feed/hive offers and installs Avalonia, refreshes to the selected real Type, creates it,
   removes Accelerate references/calls byte-for-byte as Passenger does, and yields a buildable artifact.
   A solution-producing template is offered only by New Solution and contributes all projects once.
6. Inject failures at command/cleanup/handoff boundaries: nonzero exit, timeout, no project, cancel while
   creating/restoring, owner close/workspace change, attachment failure and cleanup refusal. Prove no
   process outlives its operation and no unrelated files are removed.

Byte parity uses the same SDK/template versions, hive, names, options and starting fixtures. For
deterministic output compare raw bytes, including BOM, whitespace, order and line endings. For `.sln`
additions, use Passenger's captured-generated-ID assertion: all old bytes must remain, and all new lines
must match Passenger with the same captured ID. A controlled-ID fixture can additionally compare whole
outputs. Independent template executions may generate GUIDs, ports or secrets; do not hide that by
normalizing arbitrary text and calling it byte parity. Do not compare bin/obj or machine-specific
restore output as template source files. Package parity remains the NuGet part's acceptance work.

Run appropriate core/window tests, formatting and the warnings-as-errors build. Real CLI integration
tests use their own state/hive/feed; missing SDKs are reported explicitly, not silently passed. Inspect
real `just screenshot` captures for New Solution, New Project and a representative option/error/install
state at default and minimum size; the PNG must be opened before UI completion. Benchmark normal startup
and editor latency with preload enabled to ensure it stays off the critical path. Only claim the
complete Phase 4 acceptance after the NuGet and MSBuild parts are also delivered.
