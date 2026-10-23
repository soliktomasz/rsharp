# Solution View, Piece 2 (Changing the Solution) Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or
> superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** The Solution view changes the solution as Rider's does: Add (files, folders, C# types, solution folders,
existing projects and items), Add Reference, Rename, Delete and Remove, Cut, Copy, Paste and drags. Every edit to a
`.sln`, `.slnx` or `.csproj` keeps every other byte, and a folder watcher brings in what other programs change.

**Architecture:** The text edits are pure functions in `rsharp-core` (`solution_edits`, `project_edits`): text in,
text out, tested without a window. The app reads and writes the files around them through one helper, which saves an
open buffer first, keeps the bytes as they were in Local History, writes in the file's own encoding, and reads back
what it wrote. The tree gets a right-click menu and a name popup built the way the tab menu and Go To popups are.
File moves go through `language::move_files`, so open tabs and Roslyn follow. `rsharp-core::watcher` wraps `notify`
and hands the window batches of changed paths.

**Tech Stack:** Rust 2024, gpui, quick-xml (already in), `notify` and `trash` (new).

**Spec:** `docs/superpowers/specs/2026-10-23-solution-view-design.md` (Piece 2). Piece 1's plan
(`docs/superpowers/plans/2026-10-23-solution-view-piece-1.md`) built what this extends: `rsharp_core::solution`,
`project`, `nesting`, `rsharp_lsp::SharedHost`, `rsharp_app::solution_view::SolutionTree`. **Passenger is the
reference for every behaviour below, and Rider where Passenger is silent.** Each task names the Passenger file to read
first, and its tests are the expected bytes.

## Global Constraints

- Every Piece 1 constraint holds: no gpui in non-UI crates and no blocking the UI thread on a server or the Host;
  every program through `rsharp_core::process::spawn`; every new dependency, icon or font with an open-source licence
  and its THIRD_PARTY_NOTICES.md entry in the same change; colours as `theme.rs` tokens; paths through
  `rsharp_core::paths::canonical`; a deliberate limit as `// shortcut: <the limit>, <when to upgrade>`; warnings are
  errors; `cargo fmt` at 120 columns; tests say what a person sees; commits through `git commit -F msg.txt`, ending
  `Co-Authored-By: Claude Opus 5.5 <noreply@anthropic.com>`.
- **An edit keeps every other byte**: its encoding, BOM, line separator, indentation, comments and element order.
  Never `dotnet sln`, `dotnet add` or a whole-document XML rewrite.
- **Anything RSharp writes to a solution or project file saves that file first if it is open, keeps the bytes it
  replaces in Local History, and then reads back what it wrote** (Passenger's `SaveBeforeWriting` and
  `FollowWritten`).
- **Keys follow Rider's** (`keymap.rs`), macOS keymap on macOS and the Visual Studio keymap elsewhere:

  | Action | macOS | Elsewhere |
  |---|---|---|
  | New… (the Add menu at the row) | `cmd-n` | `alt-insert` |
  | Rename | `shift-f6` | `f2` |
  | Delete / Remove | `cmd-backspace` and `backspace` | `delete` |
  | Cut, Copy, Paste | `cmd-x`, `cmd-c`, `cmd-v` | `ctrl-x`, `ctrl-c`, `ctrl-v` |

  All of these are bound in the `Explorer` key context only, so the editor keeps its own.
- Tests never touch the person's Trash: the Trash goes through `rsharp_core::trash`, which a test points at a folder
  of its own.
- Out of scope, named so no task grows into them: New Project, NuGet, Unload/Reload Project, Open in Terminal,
  Compare, Build Selected, Find/Replace in Files from the menu, undo of a file operation (Passenger's
  `CommandHistory`), and a person's own file templates.

## Review Focus

1. **A solution or project file open with unsaved edits when RSharp writes to it.** Expect it saved first and the
   edit kept, with no question asked (Passenger's test of that name). Test in Task 3.
2. **A Windows-1252 `.sln` or `.csproj`, or a CRLF one with a BOM.** Expect every other byte the same, and Local
   History holding the file as it was. Tests in Tasks 1, 2 and 4.
3. **A name already taken, an empty name, or a name with `/`, `\`, `:` or `..`.** Expect the popup to say why and keep
   the name typed, with nothing on disk changed. Tests in Tasks 3 and 5.
4. **Pasting a folder into itself or one of its own folders, or a file onto its own folder.** Expect a refusal for
   the first and Rider's Duplicate name question for the second, never a recursive copy. Test in Task 7.
5. **The Trash refusing a delete** (a locked file, no Trash on the volume). Expect the file, its open tab and its
   unsaved edit all kept, with the reason said. Test in Task 5.

---

### Task 1: Solution file edits

**Files:**
- Create: `crates/rsharp-core/src/solution_edits.rs` (port of Passenger's `Workspace/SolutionEdits.cs`: `SlnText`
  line by line, `SlnxText` as text spans)
- Modify: `crates/rsharp-core/src/lib.rs`
- Test: `crates/rsharp-core/tests/solution_edits.rs`

**Interfaces:**
- Produces (each takes the file's decoded text and its path, returns the new text or why not, and changes nothing
  else):
  ```rust
  pub enum SolutionEdit<'a> {
      AddFolder { parent: Option<&'a str>, name: &'a str },   // parent "/src/" or none for the top
      RenameFolder { folder: &'a str, name: &'a str },
      RemoveFolder { folder: &'a str },                        // with every folder, project and item under it
      AddProject { folder: Option<&'a str>, project: &'a Path },
      RemoveProject { project: &'a Path },
      AddFile { folder: &'a str, path: &'a Path },
      RemoveFile { folder: &'a str, path: &'a Path },
  }
  pub fn edit(solution: &Path, text: &str, change: SolutionEdit) -> Result<String, String>;
  pub fn invalid_folder_name(name: &str) -> Option<String>;   // "A name is needed" / "{name} is not a solution folder name"
  ```
  Paths inside the file are written relative to the solution's folder, with `\` in a `.sln` and `/` in a `.slnx`, as
  Visual Studio writes them. A `.sln` keeps the line separator its text already uses. A project added to a `.sln`
  gets its `Project(...)` block with a new upper-case GUID, its `ProjectConfigurationPlatforms` lines for each
  configuration the solution lists, and a `NestedProjects` line when it goes in a folder, all as Passenger writes them.
  A `.slnx` element is inserted at the indentation of its siblings (or its parent's plus two spaces), and `<Folder />`
  turns into a start and end tag when it gains a child.

- [ ] **Step 1: Write the failing tests.** Port Passenger's `SolutionFoldersTests` expected texts as pure tests over
  `edit`, both for `.sln` and `.slnx` (its `[InlineData]` formats):
  - `a_solution_folder_added_renamed_given_a_project_and_a_file_and_removed_changes_only_its_own_lines` (from its
    lines 113–227): the text after each step equals Passenger's expected string, byte for byte.
  - `a_folder_added_to_a_crlf_sln_with_a_bom_keeps_its_separators`: every line of the output ends in `\r\n`.
  - `removing_a_folder_takes_every_project_folder_and_item_under_it`, including a `.sln`'s `NestedProjects` and
    configuration lines for the projects removed.
  - `a_folder_name_with_a_slash_or_dots_is_refused`: `invalid_folder_name("a/b")`, `("..")` and `("  ")` give reasons.
  - `renaming_a_folder_that_is_not_there_says_so`: `Err`, the text unchanged.
- [ ] **Step 2: Run** `cargo test -p rsharp-core --test solution_edits`. Expect: it fails to compile (no
  `solution_edits`).
- [ ] **Step 3: Implement** `solution_edits.rs` as Passenger's `SlnText` and `SlnxText` do, over the text. The
  `.slnx` side finds elements with `quick_xml::Reader`'s `buffer_position()` spans and splices the text, so nothing
  outside the element changes.
- [ ] **Step 4: Run** the tests. Expect: PASS.
- [ ] **Step 5: Commit** "Solution file edits that change only their own lines".

### Task 2: Project file edits

**Files:**
- Create: `crates/rsharp-core/src/project_edits.rs` (port of Passenger's `Workspace/ProjectEdits.cs`)
- Modify: `crates/rsharp-core/src/lib.rs`
- Test: `crates/rsharp-core/tests/project_edits.rs`

**Interfaces:**
- Produces:
  ```rust
  pub fn add_reference(project: &Path, text: &str, referenced: &Path) -> Result<String, String>;
  pub fn remove_reference(project: &Path, text: &str, referenced: &Path) -> Result<String, String>;
  /// Each moved file's `Include`/`Update`/`Remove`, `DependentUpon` and `LastGenOutput` made to follow it; None when
  /// the project names none of them, so a glob-held project file is left untouched.
  pub fn move_items(project: &Path, text: &str, moves: &[(PathBuf, PathBuf)]) -> Option<String>;
  ```
  A reference is written as `..\Core\Core.csproj`, relative and with `\`, as `dotnet add reference` and Rider write
  it. It goes into the first `ItemGroup` that already holds a `ProjectReference`, else into a new `ItemGroup` before
  `</Project>` at the file's own indentation. Removing the last one in an `ItemGroup` removes the group and the blank
  line before it, so the file comes back as it was before the add.

- [ ] **Step 1: Write the failing tests**, from Passenger's `SolutionViewTests` lines 205–272 and 468 onward:
  - `adding_and_removing_a_reference_leaves_the_project_file_as_it_was`: `remove_reference(add_reference(t)) == t`
    for a project with no `ItemGroup`, one with a `PackageReference` group, and one with a `ProjectReference` group.
  - `a_reference_is_written_as_visual_studio_writes_it`: equals Passenger's expected text byte for byte.
  - `adding_a_reference_already_there_says_so`.
  - `moving_an_item_named_in_the_project_follows_it`: a `<Compile Update="Old.cs"><DependentUpon>Form.cs</DependentUpon>`
    pair moved to `Sub/` updates both.
  - `moving_a_file_the_project_holds_by_glob_changes_nothing`: `None`.
- [ ] **Step 2: Run** `cargo test -p rsharp-core --test project_edits`. Expect: it fails to compile.
- [ ] **Step 3: Implement** as Passenger does, splicing text at element spans.
- [ ] **Step 4: Run** the tests. Expect: PASS.
- [ ] **Step 5: Commit** "Project file edits: references in and out, items following a move".

### Task 3: The menu, the name popup and writing a file RSharp edits

**Files:**
- Create: `crates/rsharp-app/src/solution_menu.rs` (the row's menu: which items a row offers, as Passenger's
  `ExplorerView.axaml.cs:200-310` builds them, minus the out-of-scope ones; and `NamePopup`, the name question)
- Modify: `crates/rsharp-app/src/window.rs` (open the menu on a row's right click and on its key; draw the menu and
  the popup as `tab_menu_view` and `goto.rs` draw theirs; `write_edited`; reload the tree from what was written)
- Modify: `crates/rsharp-app/src/solution_view.rs` (`set_solution`, `forget`)
- Modify: `crates/rsharp-app/src/keymap.rs`, `crates/rsharp-app/src/actions.rs` (the keys table above, for the
  `Explorer` context)
- Test: `crates/rsharp-app/tests/solution_edit.rs`

**Interfaces:**
- Consumes: `solution_edits::{edit, invalid_folder_name}` (Task 1).
- Produces:
  ```rust
  // solution_menu.rs
  pub enum MenuItem { Action(&'static str, Option<&'static str> /* key shown */), Submenu(&'static str, Vec<MenuItem>), Separator }
  pub fn menu_for(row: &TreeRow, can_paste: bool) -> Vec<MenuItem>;
  pub struct NamePopup { .. }   // a TextField, a title, an optional list of kinds (Class, Interface, …), the problem shown
  // solution_view.rs
  impl SolutionTree {
      pub fn set_solution(&mut self, solution: Solution);   // read again after a write, what was open kept
      pub fn forget(&mut self, path: &Path);                 // drop the cached levels at and under a folder
  }
  // window.rs, public for the tests as the menu's and popup's way in
  pub fn solution_menu(&self, row: usize) -> Vec<String>;        // "Add > New Solution Folder...", "Edit > Rename..."
  pub fn choose(&mut self, row: usize, item: &str, window: &mut Window, cx: &mut Context<Self>);
  pub fn name_popup(&self) -> Option<Entity<NamePopup>>;
  /// Writes a solution or project file through `change`: the open buffer saved first, the bytes replaced kept in
  /// Local History, the text written in its own encoding, the buffer and the tree reading back what was written.
  fn write_edited(&mut self, path: &Path, change: impl FnOnce(&str) -> Result<String, String>, cx: &mut Context<Self>) -> Result<(), String>;
  ```
  The name popup takes Enter to accept and Escape to dismiss, and shows a refused name's reason under the field,
  keeping the popup open. Wired actions in this task: New Solution Folder, Rename of a solution folder, and Remove of
  a solution folder. Remove asks first, naming what goes with the folder, as Passenger's
  `Removing_a_solution_folder_names_the_projects_folders_and_files_that_go_with_it` does, through `window.prompt`.

- [ ] **Step 1: Write the failing tests.**
  - `the_solution_and_project_menus_offer_riders_items_with_their_keys`: `solution_menu` on the solution row, a
    solution folder, a project, Dependencies, a project reference, a folder and a file each equal the list from
    Passenger's `The_solution_and_project_menus_show_the_keys_of_the_actions_their_items_run`, minus the out-of-scope
    items.
  - `a_solution_folder_added_renamed_and_removed_shows_and_changes_only_its_lines`: through `choose` and typing into
    the popup. The `.slnx` on disk equals Task 1's expected text after each step, and the tree shows the folder under
    its new name.
  - `a_name_taken_or_invalid_keeps_the_popup_open_saying_why` (Review Focus 3).
  - `a_solution_open_with_unsaved_edits_is_saved_before_a_folder_is_added_and_keeps_them` (Review Focus 1): the
    `.slnx` open in a tab with a typed comment. After New Solution Folder, the file on disk holds both the comment and
    the folder, and the tab shows the same.
  - `a_folder_added_to_a_windows_1252_sln_keeps_every_other_byte_and_local_history_keeps_it_as_it_was` (Review Focus
    2), from Passenger's test at `SolutionFoldersTests.cs:228`.
  - `the_new_key_opens_the_add_menu_and_rename_key_asks_the_name`: `cmd-n` (macOS) on a selected solution folder
    shows the Add submenu's items, and `shift-f6` shows the popup filled with the name.
- [ ] **Step 2: Run** `cargo test -p rsharp-app --test solution_edit`. Expect: it fails to compile.
- [ ] **Step 3: Implement** `solution_menu.rs`, the menu and popup drawing, the keys, and `write_edited` (decode and
  encode through `rsharp_core::encodings`, write through `rsharp_core::files::write`, keep the old bytes through
  `history_window::record`, refresh an open buffer with `refresh_from_disk`, re-read with `solution::read` into
  `set_solution`, and start `evaluate_projects` when a project file was written).
- [ ] **Step 4: Run** the tests, then `just test`.
- [ ] **Step 5: Look at it.** Take `tools/screenshot.sh` with the menu open on a solution folder (`--key` to select
  the row, then the menu key) and check it against Rider's menu: order, separators and key hints.
- [ ] **Step 6: Commit** "The Solution view's menu, name popup and solution folders".

### Task 4: Existing projects and items, and Remove of a project or item

**Files:**
- Create: `crates/rsharp-core/src/trash.rs`
- Modify: `crates/rsharp-app/src/window.rs`
- Modify: `Cargo.toml`, `crates/rsharp-core/Cargo.toml`, `THIRD_PARTY_NOTICES.md` (the `trash` crate, MIT)
- Test: `crates/rsharp-app/tests/solution_edit.rs`, `crates/rsharp-core/tests/trash.rs`

**Interfaces:**
- Consumes: `write_edited`, `choose` (Task 3); `SolutionEdit::{AddProject, RemoveProject, AddFile, RemoveFile}`
  (Task 1).
- Produces:
  ```rust
  // rsharp-core::trash
  pub fn move_to_trash(path: &Path) -> std::io::Result<()>;   // the system's Trash, or the folder set
  pub fn set_folder(folder: Option<PathBuf>);                  // tests: a folder of their own instead of the Trash
  /// After the file picker (`cx.prompt_for_paths`, which a test cannot answer): what Add Existing Project and Item do
  /// with the files picked.
  pub fn add_existing(&mut self, into: Option<String>, picked: Vec<PathBuf>, projects: bool, cx: &mut Context<Self>) -> Result<(), String>;
  ```
  Remove on a project takes it out of the solution and keeps its files. Then, only when the project is in a folder of
  its own (not the open folder, not one above it, not one holding the solution or another of its projects), a second
  question offers to move that folder to the Trash, as Passenger's `RemoveProjectAsync` asks. Remove on a solution
  item takes it out of its folder and keeps the file. Roslyn is told of a project added or removed
  (`rsharp_lsp::reload_projects_of`).

- [ ] **Step 1: Write the failing tests**: `an_existing_project_added_to_a_folder_shows_there_and_in_the_sln` (`.sln`
  and `.slnx`; the file on disk byte for byte as Task 1's expected text),
  `an_existing_item_added_to_a_solution_folder_shows_there`,
  `delete_on_a_project_beside_its_solution_takes_it_out_and_leaves_the_folder`, and
  `delete_on_a_project_in_its_own_folder_moves_that_folder_to_the_trash_only_once_asked_again`, from Passenger's
  `ExplorerFilesTests.cs:96-140`, the Trash pointed at the test's own folder with `trash::set_folder`; and in
  `trash.rs`, `a_file_moved_to_the_set_folder_leaves_its_place`.
- [ ] **Step 2: Run** them. Expect: FAIL.
- [ ] **Step 3: Implement.** The menu items call the picker, then `add_existing`.
- [ ] **Step 4: Run** them, then `just test`.
- [ ] **Step 5: Commit** "Existing projects and items, and Remove from the solution".

### Task 5: New file, folder and C# type; Rename; Delete to the Trash

**Files:**
- Modify: `crates/rsharp-app/src/window.rs`, `crates/rsharp-app/src/language.rs` (a type renamed with its file)
- Test: `crates/rsharp-app/tests/solution_files.rs`

**Interfaces:**
- Consumes: `trash::{move_to_trash, set_folder}` (Task 4), `write_edited` and `NamePopup` (Task 3).
- Produces:
  ```rust
  pub const TYPE_KINDS: [&str; 5] = ["Class", "Interface", "Record", "Struct", "Enum"];
  ```
  A new type's file is `namespace {Root}.{Folders};\n\n{declaration}` with the declarations exactly as Passenger's
  `Add_class_interface_writes_each_of_Riders_kinds_of_type_in_its_folders_namespace` expects. `Root` is the project
  file's stem, each folder appended with `.` and characters that cannot be in a C# identifier made `_`; add
  `// shortcut: the project's file name as its root namespace, not its RootNamespace property, until the Host's evaluation carries RootNamespace`.
  A name with a `/` (`Models/Invoice`) makes the folders and puts the type in their namespace (Passenger's
  `New_class_named_with_a_folder_goes_in_that_folder_in_its_namespace`). The new file opens in a tab.

  Renaming a `.cs` file whose type has its old name offers to rename the type as well. It asks through
  `window.prompt` ("Rename type Invoice to Bill too?", Rename / Don't Rename), finds the declaration with tree-sitter
  (`rsharp_languages::parse`, a `class`/`record`/`struct`/`interface`/`enum` declaration named after the old stem),
  and renames it through `textDocument/rename` and `language::apply_workspace_edit`, as `editor_assist.rs:700-740`
  does. Every file rename goes through `language::move_files`, so open tabs keep their text and undo.

  Delete asks first ("Move {name} to the Trash?", or for a folder "Move the folder {name} and everything in it to the
  Trash?", Delete / Cancel). It saves the open files under the path first, so the Trash and Local History hold what the
  editor showed, then closes their tabs once the path is gone, or keeps them when the Trash refused.

- [ ] **Step 1: Write the failing tests**, from Passenger's `ExplorerFilesTests` and `SolutionViewTests`:
  - `add_class_interface_writes_each_of_riders_kinds_of_type_in_its_folders_namespace`: the five files byte for byte.
  - `a_new_class_named_with_a_folder_goes_in_that_folder_in_its_namespace`.
  - `new_file_and_new_folder_show_in_the_tree_and_open_a_new_file`.
  - `a_name_taken_in_the_folder_is_refused_and_nothing_changes_on_disk` (Review Focus 3).
  - `a_file_renamed_keeps_its_tab_its_text_and_its_undo`, from `ExplorerFilesTests.cs:213`, minus bookmarks and
    breakpoints, which RSharp does not have yet.
  - `renaming_a_folder_with_two_open_files_keeps_the_one_in_front_in_front`.
  - `a_file_renamed_after_its_type_offers_to_rename_the_type`: skips, saying why, without Roslyn, as
    `tests/language.rs` does. Expect the class renamed in its file and in a file that uses it.
  - `a_file_deleted_with_unsaved_edits_goes_to_the_trash_with_them_and_local_history_keeps_them`.
  - `a_delete_the_trash_refuses_keeps_the_open_file_and_its_unsaved_edit` (Review Focus 5): `set_folder` at a path
    that cannot be written.
- [ ] **Step 2: Run** them. Expect: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** them, then `just test`.
- [ ] **Step 5: Commit** "New file, folder and type; Rename with the type; Delete to the Trash".

### Task 6: Add Reference and Remove of a reference

**Files:**
- Create: `crates/rsharp-app/src/reference_dialog.rs` (Passenger's `Explorer/AddReferenceDialog.cs`: the solution's
  other projects as checkboxes, those already referenced ticked; a project that references this one, directly or
  through others, shown greyed with why, as Passenger's `Dependents` decides)
- Modify: `crates/rsharp-app/src/window.rs`
- Test: `crates/rsharp-app/tests/solution_edit.rs`

**Interfaces:**
- Consumes: `project_edits::{add_reference, remove_reference}` (Task 2), `write_edited` (Task 3).
- Produces: `pub fn set_references(&mut self, project: &Path, wanted: Vec<PathBuf>, cx: &mut Context<Self>) -> Result<(), String>`,
  which is what OK does with the boxes ticked. It adds the new references and removes the unticked ones in one
  `write_edited`, then tells Roslyn (`reload_projects_of`) and evaluates the project again. Remove on a
  `ProjectReference` row asks first, then removes that one.

- [ ] **Step 1: Write the failing tests**:
  `add_reference_adds_one_and_remove_takes_it_out_leaving_the_project_file_as_it_was` (from
  `SolutionViewTests.cs:205`; the tree's Dependencies shows the project once evaluated, which skips without the Host
  as Piece 1's real-Host tests do, while the file assertions run everywhere),
  `a_project_that_depends_on_this_one_cannot_be_ticked`, and
  `add_reference_to_a_windows_1252_project_keeps_its_other_bytes_and_local_history_keeps_it_as_it_was`.
- [ ] **Step 2: Run** them. Expect: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** them, then `just test`, and take a screenshot of the dialog.
- [ ] **Step 5: Commit** "Add Reference and Remove of a reference".

### Task 7: Cut, Copy, Paste and drags

**Files:**
- Create: `crates/rsharp-app/src/file_moves.rs` (port of Passenger's `MainWindow.FileMoves.cs`: `PutAsync`,
  `PutOneAsync`, `DependentsOf`, `FollowInProjects`, `Taken`, `CopyOnDisk`, `CopyFolder`)
- Modify: `crates/rsharp-app/src/window.rs` (the actions, the clipboard, a drag from one row to another with
  `on_drag`/`on_drop` as `editor_area.rs` drags tabs, and the copy modifier: `alt` on macOS, `ctrl` elsewhere)
- Test: `crates/rsharp-app/tests/solution_files.rs`

**Interfaces:**
- Consumes: `nesting::parents` (a file's dependents go with it, renamed as its copy's name asks),
  `project_edits::move_items` (Task 2), `language::move_files`, `write_edited`.
- Produces:
  ```rust
  /// Puts files and folders into a folder, moved or copied, their dependents with them; a copy into its own folder
  /// asks its name as Rider's Duplicate does. The paths put, or why not.
  pub fn put(&mut self, paths: Vec<PathBuf>, into: PathBuf, copy: bool, window: &mut Window, cx: &mut Context<Self>) -> Result<Vec<PathBuf>, String>;
  ```
  Cut or Copy writes the paths to the clipboard as text, one a line, and remembers them. Paste works only while the
  clipboard still holds that text. A cut row is not dimmed (Piece 1's non-goal stays). A name taken in the target asks
  to replace it, as Passenger's `Taken` does.

- [ ] **Step 1: Write the failing tests**, from Passenger's `SolutionViewTests.cs:59-204`:
  - `a_file_cut_and_pasted_into_another_projects_folder_moves_there_with_its_open_tab_and_its_unsaved_edit`.
  - `a_folder_copied_and_pasted_copies_all_it_holds_and_a_copy_into_its_own_folder_asks_its_name`.
  - `a_file_dragged_onto_another_folder_moves_and_with_the_copy_key_held_is_copied`. Drive it through
    `debug_selector` bounds and `simulate_mouse_*`, as the tab drag tests do.
  - `a_parent_moved_takes_its_dependents_and_its_projects_dependent_upon_follows`, from
    `FileNestingTests.A_parent_renamed_or_moved…`.
  - `a_folder_pasted_into_itself_is_refused` (Review Focus 4).
- [ ] **Step 2: Run** them. Expect: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** them, then `just test`.
- [ ] **Step 5: Commit** "Cut, Copy, Paste and drags in the Solution view".

### Task 8: The folder watcher

**Files:**
- Create: `crates/rsharp-core/src/watcher.rs`
- Modify: `crates/rsharp-app/src/window.rs` (start it with the folder, end it as the folder is left), and
  `crates/rsharp-app/src/buffer.rs:205` (remove the `shortcut:` it closes)
- Modify: `Cargo.toml`, `crates/rsharp-core/Cargo.toml`, `THIRD_PARTY_NOTICES.md` (the `notify` crate, its licence as
  its Cargo.toml states it)
- Test: `crates/rsharp-core/tests/watcher.rs`, `crates/rsharp-app/tests/solution_files.rs`

**Interfaces:**
- Produces:
  ```rust
  pub struct Watcher { .. }   // dropped, it stops
  /// Watches `folder` recursively and calls `changed` on its own thread with the paths changed in each burst, gathered
  /// for 150 ms (Passenger's `Gather`). bin, obj, node_modules and .git are passed over, except .git's HEAD, index
  /// and refs.
  pub fn watch(folder: &Path, changed: impl Fn(Vec<PathBuf>) + Send + 'static) -> notify::Result<Watcher>;
  ```
  In the window, each burst crosses to the UI through a channel and `cx.spawn`, as Roslyn's events do. For each
  changed path, the tree forgets the levels at and under its folder (`SolutionTree::forget`). An open buffer of that
  path is refreshed (`refresh_from_disk`, which keeps an unsaved one as it is); a changed file that is not open is
  kept in Local History as an External change, as Passenger's `FolderWatching` feeds `LocalHistory`, off the UI
  thread. A changed `.csproj`, `.props` or
  `.targets` starts `evaluate_projects`, and a changed solution file is read again into `set_solution`. RSharp's own
  writes come back through the watcher too, and are harmless because `refresh_from_disk` sees its own stamp.

- [ ] **Step 1: Write the failing tests**:
  - `watcher.rs`: `a_file_made_in_the_folder_is_reported_once_its_burst_ends`, and `bin_obj_and_git_objects_are_passed_over`.
  - `solution_files.rs`: `a_file_made_by_another_program_shows_in_the_tree`,
    `an_open_file_changed_by_another_program_shows_its_new_text_and_local_history_keeps_it`, `a_file_not_open_changed_by_another_program_is_kept_in_local_history`, and
    `a_project_file_changed_by_another_program_is_evaluated_again` (skips without the Host). The window tests wait in
    real time, as `tests/language.rs` does, since notify's events come on its own thread.
- [ ] **Step 2: Run** them. Expect: FAIL.
- [ ] **Step 3: Implement.**
- [ ] **Step 4: Run** them, then `just test`, then `tools/bench.sh` before and after (the watcher's start and its
  idle cost count). Start, load, memory and key-to-frame must not get worse; if they do, find out why first.
- [ ] **Step 5: Update the docs.** In ROADMAP.md, add Piece 2 to Phase 4's "Built:" line. In CLAUDE.md's Layout, cover
  `solution_edits`, `project_edits`, `trash` and `watcher` in `rsharp-core`, and `solution_menu.rs`,
  `reference_dialog.rs` and `file_moves.rs` in `rsharp-app`.
- [ ] **Step 6: Commit** "The folder watcher: what other programs change reaches the tree, the open files and the projects".
