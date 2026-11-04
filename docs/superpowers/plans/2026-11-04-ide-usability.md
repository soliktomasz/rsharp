# IDE Usability Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Close the gaps between RSharp and what a Rider user reaches for every day: debugging a test, File
Structure, Code Vision, Call Hierarchy, Bookmarks, TODO, a richer Problems view, coverage, spelling, a keymap editor,
a light theme, a kept editor layout, the HTTP client, Docker, databases, shipping and first run.

**Architecture:** Each feature is a work package on RSharp's existing seams: Roslyn's LSP through
`rsharp-lsp`/`language::ask`, tool windows as `dock::Tool`s, popups as `GoToPopup`-style entities, per-folder state in
`state::FolderState`, and every program through `rsharp_core::process::spawn`. Non-UI logic (parsers, protocol
messages, stores) lands in `rsharp-core` with unit tests; the window side gets a window test on gpui's test platform.

**Tech Stack:** Rust 2024, gpui 0.2.2, Roslyn LSP, VSTest and Microsoft.Testing.Platform, Passenger's debug adapter,
quick-xml, ureq, tree-sitter. New crates only where named below, each with its THIRD_PARTY_NOTICES entry.

**Spec:** the recommendation of 2026-11-04 (this session) and, per feature, Passenger's behaviour where it has it
(File Structure, Bookmarks, Call Hierarchy: `vendor/passenger/src/Passenger.App/Navigation/Navigator.cs`,
`MainWindow.Bookmarks.cs`, `MainWindow.Hierarchy.cs`), else Rider 2026.2's.

## Global Constraints

- CLAUDE.md's Rules hold for every task: every program through `process::spawn`; colours only as `theme.rs` tokens
  with their Rider source; non-UI crates never depend on gpui; a malformed flag or value is fatal; a key costs what it
  changed; shortcuts follow Rider's macOS keymap on macOS and its Visual Studio keymap elsewhere (`keymap.rs`).
- Every new dependency, icon or dictionary: an open-source licence and a THIRD_PARTY_NOTICES.md entry in the same
  change. Icons only from `lib/intellij.platform.ide.jar` (`expui/…_dark.svg`), never `intellij.rider.*`.
- New UI follows DESIGN.md: 24px rows, 28px icon buttons, 40px island headers, Inter 13px, no shadows except popups.
- Each task: `just build` clean (warnings are errors), its crates' tests and the window tests of the area it touched
  pass, `just screenshot` looked at for anything visible, CLAUDE.md's Layout and DESIGN.md updated where they describe
  it, a commit through `git commit -F`. The whole suite (`just test`) runs once at the end of each tier.
- Tests say what a person would see or get; their failure messages name the consequence.

## Review Focus

- A test run debugged while a normal run is in flight, or Stop pressed while the debugger attaches: the run ends and
  no testhost outlives it (Task 1: `debugging_a_test_then_stop_leaves_no_testhost`).
- File Structure, Code Vision and Call Hierarchy asked before Roslyn has loaded: the status bar says Roslyn is
  loading, nothing hangs or panics (Tasks 2–4 each: `…_before_roslyn_loads_says_so`).
- A bookmarked line deleted or moved by an edit, a reformat or an external change: the bookmark follows the text or
  goes with it, never points past the file's end (Task 5: `bookmark_follows_edits_and_dies_with_its_line`).
- A malformed `keymap.json`, `.http` file or `http-client.env.json`: refused with the file and line named, the
  defaults kept (Tasks 10 and 13).
- A light theme switched while popups, the terminal and the debugger are on screen: every view repaints in the new
  palette at once, no literal colour left (Task 11: `no_view_writes_a_literal_colour` and the switch test).

## Order and parallelism

| Tier | Tasks | Why this order |
|---|---|---|
| 1. Daily use | 1 Debug a test · 2 File Structure · 3 Code Vision · 4 Call Hierarchy · 5 Bookmarks · 6 TODO | Highest use, each built on seams that exist |
| 2. Quality loop | 7 Problems · 8 Coverage · 9 Spelling | Build on Tier 1's test and Problems work |
| 3. Comfort | 10 Keymap editor · 11 Light theme · 12 Kept editor layout | Independent of each other |
| 4. Beyond C# | 13 HTTP client · 14 Docker · 15 Databases | 13 and 14 share the Services window; 15 needs its own spec |
| 5. Reach | 16 Shipping · 17 First run | 16 is ROADMAP Phase 13 |

Independent within a tier (no shared files beyond `actions.rs`, `keymap.rs`, `dock.rs`, which merge trivially):
2, 3, 4, 5, 6; 10, 11, 12; 13 before 14. Task 1 and Task 8 both touch `rsharp-core/src/testing/`: run them in order.
Add each tier to ROADMAP.md as a phase (14–18) in its first commit.

---

## Tier 1: Daily use

### Task 1: Debug a test

Rider's Debug Selected Tests: the tests run under the debugger, breakpoints in test and product code stop.

**Files:**
- Modify: `crates/rsharp-core/src/testing/runners.rs` (`TestRunners::run` gains a debug hook)
- Modify: `crates/rsharp-core/src/testing/vstest.rs` (`DebuggingEnabled`, `TestExecution.EditorAttachDebugger2`)
- Modify: `crates/rsharp-core/src/testing/platform.rs` (`start` hands the host's command to the hook instead of spawning)
- Modify: `crates/rsharp-app/src/window/tests.rs`, `crates/rsharp-app/src/window/debug.rs`,
  `crates/rsharp-app/src/test_view.rs` (Debug Selected button and `TestAction::DebugSelected`), `actions.rs`, `keymap.rs`
- Test: `crates/rsharp-core/tests/testing_debug.rs`, `crates/rsharp-app/tests/debugger.rs`

**Interfaces:**
- Produces (rsharp-core, no gpui):
  ```rust
  pub enum DebugTarget {
      /// Microsoft.Testing.Platform: the debugger starts the test app itself.
      Launch { program: PathBuf, arguments: Vec<String>, working_directory: PathBuf },
      /// VSTest: the testhost VSTest started, waiting for a debugger.
      Attach { process_id: u32 },
  }
  pub type DebugHook<'a> = &'a mut dyn FnMut(DebugTarget) -> Result<(), String>;
  impl TestRunners {
      pub fn run_debugged(&mut self, tests: &[TestItem], cancel: &AtomicBool, event: Events<'_>, debug: DebugHook<'_>)
          -> Result<RunSummary, String>;
  }
  ```
  The hook returns once the debugger is attached or has launched the program; an `Err` ends the run with that reason.
- Produces (app): `TestAction::DebugSelected`; `MainWindow::debug_tests(&mut self, chosen: Vec<TestItem>, cx)`;
  `MainWindow::start_debug_target(&mut self, target: DebugTarget, cx) -> oneshot::Receiver<Result<(), String>>`, which
  builds `rsharp_dap::Start::Launch(Launch { program, arguments, working_directory, environment: default(),
  stop_at_entry: false, hot_reload: false, just_my_code: settings })` or `Start::Attach { process_id, just_my_code }`.

- [ ] **Step 1: Write the failing core tests** in `crates/rsharp-core/tests/testing_debug.rs`:
  `vstest_asks_for_a_debugger_with_the_testhost_pid` (a fake VSTest console over the existing `Host` socket replies
  to `TestExecution.RunAllWithDefaultHost` carrying `"DebuggingEnabled": true` with
  `TestExecution.EditorAttachDebugger2 { ProcessId: 4242 }`; assert the hook got `DebugTarget::Attach { process_id:
  4242 }` and the client answered `TestExecution.EditorAttachDebuggerCallback { Attached: true }`), and
  `platform_hands_its_server_command_to_the_debugger` (assert `Launch.arguments` ends with `["--server",
  "--client-host", "127.0.0.1", "--client-port", <port>]` and `program` is the test assembly). Message names: check
  them against microsoft/vstest's `MessageType.cs` before writing the fake; the test pins what is found there.
- [ ] **Step 2: Run** `just test testing_debug` — expect both to fail (no `run_debugged`).
- [ ] **Step 3: Implement** `run_debugged`: VSTest sets `DebuggingEnabled` in the run payload and answers each
  `EditorAttachDebugger2` through the hook; MTP's `start` takes an optional hook and, when given, passes the command
  it would have spawned instead of spawning, then waits for the host's connection as it does now (60 s).
- [ ] **Step 4: Run** `just test testing_debug` — PASS.
- [ ] **Step 5: Write the failing window test** `a_debugged_test_stops_at_its_breakpoint` in
  `crates/rsharp-app/tests/debugger.rs` (dotnet- and adapter-gated as the file's others): an xUnit project from
  `tests/Debuggees` or a generated one, F9 on the test's `Assert` line, `DebugSelected` on that test; assert the Debug
  window's frames start with the test method, then Resume and the Unit Tests row shows Passed. Plus
  `debugging_a_test_then_stop_leaves_no_testhost`: Stop while stopped; assert no `testhost` or test app process with
  the run's pid remains (`process::alive`).
- [ ] **Step 6: Implement** the app side: `window/tests.rs`' worker calls `run_debugged` with a hook that crosses to
  the UI (`cx.spawn` + a `futures::channel::oneshot`), `debug.rs`' new `start_debug_target` reuses the existing
  session start and resolves the oneshot when the session reaches Running; Stop on either window ends both. Unit
  Tests toolbar: a `debug` icon button "Debug Selected Tests" after Run Selected; context menu of a test row and the
  editor gutter's test icon gain Debug. Keys: Rider's `ctrl-u d` (both keymaps, beside `ctrl-u r`).
- [ ] **Step 7: Run** `just test debugger test_view` — PASS. `just screenshot` with a stopped test.
- [ ] **Step 8: Commit** "Debug Selected Tests: VSTest attach and MTP launch under the debugger".

**Done when:** a breakpoint in a test (VSTest and MTP projects) stops in the Debug window, results still stream into
Unit Tests, and Stop leaves no process behind.

### Task 2: File Structure popup

Passenger's `Navigator.ShowFileStructure`: the active file's types and members, indented, filtered as typed with the
containers of matches kept, opened on the member at the caret.

**Files:**
- Create: `crates/rsharp-app/src/file_structure.rs`
- Modify: `crates/rsharp-lsp/src/protocol.rs` (`outline` keeps the full range, not only the selection, so the caret's
  member can be found), `crates/rsharp-app/src/window.rs` (hosting, like `GoToPopup`), `actions.rs` (Navigate),
  `keymap.rs`
- Test: `crates/rsharp-app/tests/file_structure.rs`, a unit test in `file_structure.rs`

**Interfaces:**
- Consumes: `lsp::outline(&Value) -> Vec<(usize /*depth*/, String, u64 /*kind*/, Span)>`; `fuzzy::score`.
- Produces: `pub struct OutlineRow { depth: usize, name: String, kind: u64, range: Span, selection: Span }`;
  `pub fn filter(rows: &[OutlineRow], query: &str) -> Vec<usize>` (indices kept: matches and their containers, in
  order); `pub struct FileStructure` (gpui entity, emits `Ended::Go(Place)` / `Ended::Dismissed` as `goto::Ended`).

- [ ] **Step 1: Unit test** `filter_keeps_the_containers_of_a_match`: rows `Shop(0) > Cart(1) > Add(2), Remove(2) >
  Order(1) > Pay(2)`; `filter(&rows, "rem")` is `[0, 1, 3]`; `filter(&rows, "")` is every index.
- [ ] **Step 2: Implement** `filter` (case-insensitive contains or `fuzzy` match; containers by depth stack, as
  Passenger's `FilterOutline`). Run the unit test — PASS.
- [ ] **Step 3: Window test** `file_structure_lists_members_and_goes_to_one` (Roslyn-gated): a file with two classes;
  `alt-\` opens the popup titled "File Structure: Shop.cs", the selected row is the member the caret is in, typing
  "pay" leaves `Order` and `Pay`, Enter puts the caret on `Pay`'s name. And
  `file_structure_before_roslyn_loads_says_so`: servers off; the status bar says "File Structure needs Roslyn, which
  is still loading" and no popup opens.
- [ ] **Step 4: Implement** the popup: the `GoToPopup` look (field on top, 24px rows, depth indent 18px, a kind icon
  per LSP symbol kind from expui's `nodes/class`, `interface`, `enum`, `method`, `property`, `field` `_dark.svg`,
  added with their notice), asks `textDocument/documentSymbol` once on open. Key:
  `alt-\` on both keymaps (Passenger's), and `cmd-f12` on macOS (Rider's macOS keymap).
- [ ] **Step 5: Run** `just test file_structure goto` — PASS; screenshot; commit "File Structure popup".

### Task 3: Code Vision

Rider's Code Vision: "N usages" (and "N implementations" on interfaces and abstract members) above each member;
clicking opens Find Usages / the implementations.

**Files:**
- Modify: `crates/rsharp-lsp/src/roslyn.rs` (client capability `textDocument.codeLens`), `crates/rsharp-lsp/src/protocol.rs`
  (`code_lenses`, `resolved_lens`)
- Modify: `crates/rsharp-app/src/buffer.rs` (`lenses: Vec<Lens>`, asked as typing pauses with diagnostics),
  `crates/rsharp-app/src/editor.rs` (a lens row drawn above its line, outside the text's rows), `settings.rs` +
  `settings_dialog.rs` (Editor | Inlay Hints | Code Vision, on by default as Rider's)
- Modify: `crates/rsharp-text/src/rows.rs` only if the lens row must be a row of its own; prefer drawing it in the
  line's top padding first (see Step 4)
- Test: `crates/rsharp-app/tests/code_vision.rs`

**Interfaces:**
- Produces: `pub struct Lens { line: usize, title: String, command: Option<Value>, raw: Value }`;
  `lsp::code_lenses(&Value) -> Vec<Lens>`; `Buffer::lenses(&self) -> &[Lens]`.

- [ ] **Step 1: Window test** `code_vision_counts_usages_and_opens_them` (Roslyn-gated): `Cart.Add` used twice;
  assert a lens on `Add`'s line reads "2 usages"; a click on it opens the Find window with 2 rows. And
  `code_vision_before_roslyn_loads_says_so` (no lens, no request queued while the server is absent).
- [ ] **Step 2: Implement** the request (`textDocument/codeLens`, then `codeLens/resolve` only for lenses on visible
  lines — a key costs what it changed) and the drawing: text-secondary, 12px Inter, at the member's indent.
- [ ] **Step 3: Decide the row height**: if the editor's rows can carry a per-line extra height cheaply, draw the
  lens in it; else add a `Row::Lens` to `rsharp-text::rows` (built only while lenses exist, as folds are). Note the
  choice in a `// shortcut:` if the first is taken. `tests/budget.rs` must stay flat.
- [ ] **Step 4: Run** `just test code_vision editor budget` — PASS; screenshot; commit "Code Vision: usages above members".

### Task 4: Call Hierarchy

Passenger's `ShowCallHierarchyAsync`: a Hierarchy tool window with Callers and Callees, each level asked as it opens.
Type Hierarchy joins the same window (today it is only Go to Base/Derived popups).

**Files:**
- Create: `crates/rsharp-app/src/hierarchy_view.rs` (the tree: root, a Callers/Callees or Supertypes/Subtypes toggle,
  lazily opened nodes, double-click goes to source)
- Modify: `dock.rs` (`Tool::Hierarchy`, icon `tw-hierarchy` — today the Live Visual Tree's; give Live Visual Tree
  expui's `toolwindows/structure_dark.svg` or another unused one so no two windows share an icon),
  `crates/rsharp-lsp/src/roslyn.rs` (capability `callHierarchy`), `crates/rsharp-lsp/src/protocol.rs`
  (`calls(&Value) -> Vec<HierarchyCall>`), `window.rs`, `actions.rs`, `keymap.rs`
- Test: `crates/rsharp-app/tests/hierarchy.rs`

**Interfaces:**
- Consumes: `lsp::hierarchy_items` (exists, used by `editor_assist.rs`).
- Produces: `pub struct HierarchyCall { item: HierarchyItem, ranges: Vec<Span> }`;
  `pub enum Direction { Callers, Callees, Supertypes, Subtypes }`;
  `HierarchyView::show(&mut self, title: String, root: HierarchyItem, directions: [Direction; 2], cx)`.

- [ ] **Step 1: Window test** `call_hierarchy_lists_callers_then_callees` (Roslyn-gated): `A()` calls `B()`, `C()`
  calls `B()`; Call Hierarchy on `B` shows "Calls of B" with callers `A`, `C`; toggling Callees on `A` shows `B`;
  double-click `C` puts the caret on its call of `B`. `call_hierarchy_before_roslyn_loads_says_so`.
- [ ] **Step 2: Implement** the view and requests (`textDocument/prepareCallHierarchy`,
  `callHierarchy/incomingCalls`, `callHierarchy/outgoingCalls`; a node's children asked when it opens). Type
  Hierarchy action (Rider: `ctrl-h`) opens the same window with Supertypes/Subtypes.
- [ ] **Step 3: Keys**: `ctrl-alt-h` (Passenger's, both keymaps). Run `just test hierarchy editor_assist` — PASS;
  screenshot; commit "Call Hierarchy and Type Hierarchy window".

### Task 5: Bookmarks

Passenger's: Toggle Bookmark on the caret's line, Next/Previous across files, a Bookmarks window, a ribbon in the
gutter, kept with the folder, following edits.

**Files:**
- Create: `crates/rsharp-core/src/bookmarks.rs` (no gpui: the store and how a change moves it)
- Create: `crates/rsharp-app/src/bookmarks_view.rs`
- Modify: `state.rs` (`FolderState.bookmarks: Vec<Bookmark>`), `buffer.rs` (follow each `Change`),
  `editor.rs` (gutter ribbon, theme token `bookmark` read from Rider's expui `bookmark` icon colour), `dock.rs`
  (`Tool::Bookmarks`, icon expui `toolwindows/bookmarks_dark.svg`), `actions.rs` ("Navigate/Bookmarks"), `keymap.rs`
- Test: `crates/rsharp-core/tests/bookmarks.rs`, `crates/rsharp-app/tests/bookmarks.rs`

**Interfaces:**
- Produces: `#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)] pub struct Bookmark { path: PathBuf, line:
  usize }`; `pub struct Bookmarks`: `toggle(path, line) -> bool` (true when added), `follow(path, &Change)`,
  `lines_in(path) -> Vec<usize>`, `next(from: (&Path, usize)) -> Option<Bookmark>`, `previous(...)`, `all()`.

- [ ] **Step 1: Core tests** `bookmark_follows_edits_and_dies_with_its_line` (insert 2 lines above line 10 → 12;
  delete the range holding line 12 → gone; a change past the end never leaves a line ≥ the line count),
  `next_and_previous_wrap_across_files` (files ordered by path, lines ascending, wrapping at the ends).
- [ ] **Step 2: Implement** `Bookmarks` over `rsharp_text::Change`'s line places. Run — PASS.
- [ ] **Step 3: Window test** `toggle_next_and_the_window` with Passenger's keys (toggle `cmd-k k` and `cmd-f2` on
  macOS, `ctrl-k k` elsewhere; next `cmd-k n`/`ctrl-k n`; previous `cmd-k p`/`ctrl-k p`; window `cmd-k w`/`ctrl-k w`
  and `ctrl-\``): assert the ribbon's `debug_selector`, the window's row "Shop.cs:12  public void Pay()", and that
  reopening the folder keeps the bookmark.
- [ ] **Step 4: Implement**, run `just test bookmarks`, screenshot, commit "Bookmarks".

### Task 6: TODO window

Rider's TODO tool window: every `TODO`/`FIXME` comment in the solution, grouped by file, kept fresh as files change.

**Files:**
- Create: `crates/rsharp-core/src/todo.rs` (patterns and the scan of one text), `crates/rsharp-app/src/todo_view.rs`
- Modify: `crates/rsharp-languages/src/lib.rs` (`comments_in(&Tree, &str) -> Vec<Range<usize>>` from each grammar's
  `comment` nodes), `dock.rs` (`Tool::Todo`, icon expui `toolwindows/todo_dark.svg`), `window.rs` (the walk on the
  background executor with Find in Files' walker in `goto.rs`, refreshed by the folder watcher's bursts)
- Test: `crates/rsharp-core/tests/todo.rs`, `crates/rsharp-app/tests/todo.rs`

**Interfaces:**
- Produces: `pub struct TodoItem { path: PathBuf, line: usize, column: usize, kind: &'static str, text: String }`;
  `pub fn scan(text: &str, comments: &[Range<usize>]) -> Vec<(usize /*byte*/, &'static str, String)>`; patterns are
  Rider's defaults: `\btodo\b` and `\bfixme\b`, case-insensitive, matched inside comments only.

- [ ] **Step 1: Core test** `todos_are_found_in_comments_only`: `// TODO: pay` and `/* fixme later */` found with
  their text after the word; `"TODO"` in a string literal not found; `TodoList` not found (word boundary).
- [ ] **Step 2: Implement** `scan` and `comments_in`; run — PASS.
- [ ] **Step 3: Window test** `todo_window_lists_and_refreshes`: two files, the window shows 2 groups; saving a file
  that removes its TODO leaves 1 group (watcher burst); double-click goes to the line.
- [ ] **Step 4: Implement**, `just test todo`, screenshot, commit "TODO tool window". **End of Tier 1:** `just test`.

---

## Tier 2: Quality loop

### Task 7: Problems window upgrade

Problems already has a Solution tab (`window.rs` `solution_wide`, `workspace/diagnostic`). Add what Rider's has.

**Files:** Modify `crates/rsharp-app/src/window.rs` (split the Problems view out into `problems_view.rs` while here:
it is inside a 185 KB file), `state.rs` (`FolderState.problems: ProblemsState { severities, group_by_project }`).
Test: `crates/rsharp-app/tests/problems.rs`.

- [ ] Severity chips with counts (Errors, Warnings, Suggestions) in the Unit Tests chips' style (one click filters;
  test `severity_chips_filter_and_count`).
- [ ] Group by project then file, toggle kept with the folder (`grouping_is_kept_with_the_folder`).
- [ ] Alt+Enter on a row: the row's code actions; a fix with a "Fix all in solution / project / file" variant
  (Roslyn's `codeAction/resolve` with `FixAllScope`) applied through `language::apply_workspace_edit`
  (`fix_all_in_file_removes_every_instance`).
- [ ] Status bar widget "N errors, M warnings" opening the window (Rider's). Commit "Problems: filters, grouping,
  fix all".

### Task 8: Code coverage

**Files:**
- Create: `crates/rsharp-core/src/testing/coverage.rs` (Cobertura parse with `quick-xml`, merge of several reports),
  `crates/rsharp-app/src/coverage_view.rs` (`Tool::Coverage`, expui `toolwindows/coverage_dark.svg`)
- Modify: `testing/vstest.rs` (run settings with the `XPlat code coverage` data collector, attachment sets read from
  `TestExecution.Completed`), `testing/platform.rs` (`--coverage --coverage-output-format cobertura` when the project
  references `Microsoft.Testing.Extensions.CodeCoverage`), `editor.rs` (gutter stripe: covered `passed`, uncovered
  `failed`, partial `warning`), `test_view.rs` ("Cover Selected" button)
- Test: `crates/rsharp-core/tests/coverage.rs` (fixture Cobertura under `tests/fixtures/coverage/`),
  `crates/rsharp-app/tests/coverage.rs` (dotnet-gated)

**Interfaces:** `pub struct Coverage { files: BTreeMap<PathBuf, FileCoverage> }`, `pub struct FileCoverage { lines:
BTreeMap<usize, Hits>, covered: usize, total: usize }`, `pub enum Hits { Covered, Partial, Missed }`,
`pub fn read_cobertura(xml: &str, sources: &Path) -> Result<Coverage, String>`, `Coverage::merge(&mut self, other)`.

- [ ] Core: `cobertura_lines_and_branches_become_hits` (a `<line hits="0">` Missed, `hits="3"` Covered,
  `condition-coverage="50% (1/2)"` Partial; paths joined to `<source>`), `merge_takes_the_better_hit`.
- [ ] App: `covering_a_test_marks_its_lines` — the generated xUnit template (it carries coverlet.collector) covered,
  the gutter's covered and missed lines in the right files, the Coverage window lists the project at its percentage.
  A project with neither collector: the window says which package to add, no run fails.
- [ ] Commit "Code coverage: Cobertura from VSTest and MTP, gutter and window". **End of Tier 2** includes Task 9.

### Task 9: Spell checking

**Files:**
- Create: `crates/rsharp-core/src/spelling.rs` (words of comments, strings and identifiers split by camelCase and
  `_`; the checker; the person's dictionary), dictionary `crates/rsharp-core/assets/dictionaries/en_US.{aff,dic}`
- Modify: `buffer.rs` (typos found off the UI thread as typing pauses, visible lines first), `editor.rs` (a dotted
  underline, token `typo` from Rider's TYPO attribute), `editor_assist.rs` (Alt+Enter: suggestions, "Save to
  dictionary"), `settings.rs` (Editor | Natural Languages | Spelling, on)
- Dependencies: `spellbook` (MPL-2.0, Helix's Hunspell-compatible checker); dictionary from `wooorm/dictionaries`
  `dictionary-en` (MIT and BSD). Both in THIRD_PARTY_NOTICES.
- Test: `crates/rsharp-core/tests/spelling.rs`, `crates/rsharp-app/tests/spelling.rs`

**Interfaces:** `pub struct Speller` with `fn new(user_words: &[String]) -> Speller`, `fn typos(&self, text: &str,
spans: &[(Range<usize>, Part)]) -> Vec<Range<usize>>`, `fn suggest(&self, word: &str) -> Vec<String>`;
`pub enum Part { Comment, String, Identifier }`.

- [ ] Core: `identifiers_are_split_before_checking` (`GetCustmerName` flags `Custmer` only), `known_code_words_pass`
  (`async`, `bool`, `int`, `Json`, `Uri`, `Guid`, `Utf8` — a carried list of programming words), `user_words_pass`.
- [ ] App: `typo_is_underlined_and_fixed` (Alt+Enter offers "Customer", choosing it renames through Roslyn's rename
  when the word is an identifier, else edits the text). Kept dictionary in the state root `dictionary.txt`.
- [ ] Commit "Spell checking". **End of Tier 2:** `just test`.

---

## Tier 3: Comfort

### Task 10: Keymap editor

**Files:**
- Modify: `crates/rsharp-app/src/keymap.rs` (the defaults as data: `pub fn defaults(mac: bool) -> Vec<Binding>`,
  then `bind(cx)` applies defaults and the person's overrides), `settings_dialog.rs` (a Keymap page: search by action
  or by pressed keys, the action's keys, Add / Remove / Reset, conflicts named)
- Create: `crates/rsharp-app/src/keymap_store.rs` (`keymap.json` in the state root)
- Test: `crates/rsharp-app/tests/keymap.rs`

**Interfaces:** `pub struct Binding { action: &'static str, keys: String, context: Option<&'static str> }`;
`#[derive(Serialize, Deserialize)] pub struct KeymapOverrides { added: Vec<KeptBinding>, removed: Vec<KeptBinding> }`;
`pub fn effective(defaults: &[Binding], overrides: &KeymapOverrides) -> Result<Vec<Binding>, KeymapProblem>`;
`KeymapProblem { line: usize, why: String }`. Actions are found by their gpui name (`cx.build_action`); a name gpui
does not know is a problem, not a silent drop.

- [ ] `overrides_add_remove_and_reset` (removing `cmd-8` from ToggleUnitTests leaves it unbound; reset restores it),
  `a_conflict_is_named` (binding `cmd-7` to Run names NuGet as its other owner), `malformed_keymap_is_refused_with_its_line`
  (the defaults kept and the status bar names `keymap.json:3`), and a window test pressing a rebound key.
- [ ] Re-binding at run time: `cx.clear_key_bindings()` then `bind(cx)` (check gpui 0.2.2 has it; else keep the
  change for the next start and say so). Commit "Keymap editor".

### Task 11: Light theme

**Files:**
- Modify: `crates/rsharp-app/src/theme.rs`: each token becomes `pick(dark, light)` over a process-wide `AtomicBool`,
  so no call site changes; the light values read from `themes/islands/ManyIslandsLight.theme.json` over
  `themes/expUI/expUI_light.theme.json` (`lib/intellij.platform.ide.impl.jar`) and Rider Light's C# colours, each
  with its key as the dark ones are. Terminal palette and syntax included.
- Modify: `settings.rs` (Appearance | Theme: Dark, Light, Sync with OS — `window.appearance()` and its observer),
  `assets.rs` (icons: the jar's `…_light.svg` beside each `_dark.svg`, chosen by `icon()` from the theme)
- Test: `crates/rsharp-app/tests/theme.rs`

- [ ] `every_token_has_a_light_value` (iterate a `theme::TOKENS` table: dark ≠ light for the grounds and text),
  `no_view_writes_a_literal_colour` (grep test over `src/` for `rgb(0x`/`rgba(0x` outside `theme.rs`),
  `switching_repaints_open_windows` (switch with the editor, a popup and the terminal shown; their grounds read the
  light values). DESIGN.md gains the light table; screenshot both. Commit "Islands Light".

### Task 12: Kept editor layout

Today `FolderState.tabs` keeps one group (`// shortcut: the splits' tabs are kept as one group's`), and Recent
Locations are not kept.

**Files:** Modify `state.rs` (`FolderState.editors: Option<KeptArea>` — `enum KeptArea { Group { tabs: Vec<KeptTab>,
active: Option<usize> }, Split { axis: Axis, children: Vec<KeptArea> } }`; `recent_locations: Vec<KeptPlace>`, at most
50), `editor_area.rs` (save and restore the tree; tabs whose files are gone dropped), `goto.rs` (Recent Locations read
back). Remove the two `// shortcut:` comments this resolves. Test: `crates/rsharp-app/tests/editor_area.rs`.

- [ ] `splits_and_their_tabs_survive_a_reopen` (right split with 2 tabs, down split in it with 1; reopen; same tree,
  same active tabs and carets), `a_missing_file_drops_only_its_tab`, `older_editors_json_still_opens` (a file with
  only `tabs` restores as one group). Commit "Keep the editor's splits and recent locations". **End of Tier 3:** `just test`.

---

## Tier 4: Beyond C#

### Task 13: HTTP client (`.http` files)

The JetBrains / Visual Studio `.http` format: requests split by `###`, `@name = value` variables, `{{name}}`,
`http-client.env.json` (and `http-client.private.env.json`) environments, a ▶ in the gutter per request.

**Files:**
- Create: `crates/rsharp-core/src/http_file.rs` (parse, substitute, send with `ureq`, the response),
  `crates/rsharp-app/src/services_view.rs` (`Tool::Services`, expui `toolwindows/services_dark.svg`: a list of runs
  on the left, the chosen response — status, time, headers, body pretty-printed when JSON — on the right)
- Modify: `crates/rsharp-languages` (`Grammar::for_path` gives `.http` plain text with request lines coloured by a
  small line classifier — no new grammar crate), `editor.rs` (gutter ▶), `window.rs`
- Test: `crates/rsharp-core/tests/http_file.rs` (a loopback server as `nuget` tests use), `crates/rsharp-app/tests/http.rs`

**Interfaces:** `pub struct HttpRequest { line: usize, method: String, url: String, headers: Vec<(String, String)>,
body: Option<String> }`, `pub fn parse(text: &str) -> Result<Vec<HttpRequest>, HttpProblem>`,
`pub fn substitute(request: &HttpRequest, variables: &BTreeMap<String, String>) -> Result<HttpRequest, HttpProblem>`
(an unknown `{{name}}` is a problem naming it), `pub fn send(request: &HttpRequest, cancel: &AtomicBool) -> Result<HttpResponse, String>`.

- [ ] Core: `requests_split_on_triple_hash`, `file_variables_and_environment_substitute` (env file wins over none,
  private over public), `unknown_variable_is_named`, `sends_and_reads_the_response` (status, headers, body, elapsed).
- [ ] App: `gutter_run_shows_the_response_in_services`. Commit "HTTP client for .http files".

### Task 14: Docker

**Files:**
- Create: `crates/rsharp-core/src/docker.rs` (literal `docker` commands through `process::spawn`: `ps --format json`,
  `compose up -d`/`down`, `build`, `logs -f`, `stop`; JSON lines parsed), run configuration kinds `Dockerfile` and
  `DockerCompose` in `crates/rsharp-core/src/run/` (kept in `runConfigurations.json`)
- Modify: `services_view.rs` (a Docker node: containers with state, Start/Stop/Logs), `run_dialog.rs` (the two kinds'
  fields), `window/run.rs`
- Test: `crates/rsharp-core/tests/docker.rs` (parsing from fixtures; real `docker` gated and skipped saying why)

- [ ] `ps_json_lines_become_containers`, `compose_configuration_round_trips_in_run_configurations_json`, gated
  `compose_up_and_down_through_the_run_widget`. Commit "Docker run configurations and containers in Services".

### Task 15: Databases

Too large to plan task by task here: run `superpowers:brainstorming` for a spec first. Scope proposal for that spec:
a `crates/rsharp-db` crate (no gpui) with SQLite (`rusqlite`, bundled, MIT), PostgreSQL (`postgres`, MIT/Apache) and
SQL Server (`tiberius`, MIT/Apache); a Database tool window (connections from `appsettings*.json` connection strings
and by hand; schema tree read lazily); a SQL console as an editor buffer (tree-sitter-sequel if its licence holds,
else plain) with results in a grid under it; credentials through the system keychain, never in state files.
**Done when** the spec is approved and has its own plan.

---

## Tier 5: Reach

### Task 16: Shipping

ROADMAP Phase 13, already specified there; give it its own plan file. Work items: a signed and notarized DMG around
`tools/package-macos.sh`'s bundle (`hdiutil`, `notarytool`, `stapler`; the icon is in place); Windows through
Velopack's Rust SDK with an `.ico` made from `assets/icon/rsharp.svg`; Linux tarball and AppImage with the PNGs at
16–512; updates from GitHub releases (Velopack's update check on all three); THIRD_PARTY_NOTICES generated by `cargo
about`, failing on a missing licence; a tagged-release workflow in `.github/workflows`. **Done when** a tagged
release installs and updates itself on all three systems.

### Task 17: First run

**Files:** Modify `crates/rsharp-app/src/window.rs`' Welcome screen (a first-run panel shown until dismissed, kept as
`firstRun: false` in the app state), `crates/rsharp-core/src/programs.rs` (exists: `dotnet` discovery), create
`crates/rsharp-core/src/rider_import.rs` (reads Rider's `options/` folder under `~/Library/Application
Support/JetBrains/Rider*/`, `%APPDATA%\JetBrains\Rider*\`, `~/.config/JetBrains/Rider*/`: `recentSolutions.xml`, the
editor font size and line-number setting, the keymap's name). Test: `crates/rsharp-core/tests/rider_import.rs`
(fixture options folder), `crates/rsharp-app/tests/welcome.rs`.

- [ ] Panel: ".NET SDK 10.0.100 found" or "No .NET SDK: install it from dot.net" with the link; "Import from Rider"
  when a Rider config exists (recent solutions into RSharp's list, font size, line numbers), never writing Rider's
  files; a keymap choice when Task 10 has landed.
- [ ] `newest_rider_config_is_read`, `missing_rider_is_no_offer`, `import_fills_recent_and_settings`. Commit
  "First run: SDK check and Rider import".

---

## Self-review notes

- Coverage of the 2026-11-04 list: 1 debug a test, 2 File Structure, 3 Code Vision, 4 Call Hierarchy, 5 Bookmarks,
  6 TODO, 7 Problems, 8 coverage, 9 spelling, 10 keymap, 11 light theme, 12 kept layout, 13 HTTP, 14 Docker,
  15 databases (spec first), 16 shipping (ROADMAP Phase 13), 17 first run. None left out.
- To verify before the step that relies on it: VSTest's `EditorAttachDebugger2` / `EditorAttachDebuggerCallback`
  names (Task 1), Roslyn's LSP `codeLens` in the pinned server version (Task 3; Call Hierarchy needs no check, since
  Passenger asks the same server for it), gpui 0.2.2's `clear_key_bindings` (Task 10),
  `spellbook`'s current licence (Task 9).
