# Git — Phase 7

Status: approved by the user's “ok” on 2026-10-25.

Builder check: an independent reviewer found no important unanswered planning questions.

## Intent

A person reviews changes, commits chosen files, reads history, switches and rebases branches, resolves
conflicts and pushes without leaving RSharp. This covers Phase 7 of ROADMAP.md. Passenger supplies the
behavior and Rider supplies the presentation and platform shortcuts. The acceptance scenario is a branch
rebased into a conflict, resolved in the merge view, continued and pushed to a local test remote.

## References and approaches

Use the installed `git` executable, with porcelain v2 for status, through `rsharp_core::process::spawn`.
Keep Git commands and their protocol models in `rsharp-core::git`, text comparison in `rsharp-text`, and
the controller and views in `rsharp-app`. Reuse the existing `imara-diff` dependency, file encodings,
Local History, state store, watcher, buffer ownership, action registry and docking frame.

This is the recommended approach, selected provisionally by the builder from the repository's constraints.
A separate Git crate would add a package without an independent dependency boundary: the implementation
already needs core's process and path services. A Git library contradicts the roadmap and Passenger's
executable-based behavior. A terminal-only integration cannot provide the requested editor and merge views.
No new dependency is expected; any required dependency or carried asset needs its licence and notice.

Read these Passenger sources during implementation:

- `src/Passenger.App/Vcs/Git.cs`: discovery, porcelain v2, paths, index, revisions and commands.
- `src/Passenger.App/Vcs/VcsController.cs`: shared state, serialized operations, buffer tracking and refresh.
- `src/Passenger.App/MainWindow.Vcs.cs`, `MainWindow.Branches.cs`, `MainWindow.History.cs` and
  `MainWindow.Conflicts.cs`: actions, branch menus, history scopes and operation lifecycle.
- `src/Passenger.App/Vcs/FileDiff.cs`, `DiffView.cs`, `ChangePopup.cs`, `CommitGraph.cs`, `MergeState.cs`,
  `ThreeWayMerge.cs`, `MergeView.cs`, `MergeWindow.cs` and `ConflictsDialog.cs`.
- `src/Passenger.App/ToolWindows/CommitView.cs`, `GitView.cs`, `GitLogView.cs` and `BranchesPanel.cs`.
- `src/Passenger.Text/WordDiff.cs` and `LineDiff.cs`.
- `tests/Passenger.Tests/VcsTests.cs`, `BranchesTests.cs`, `HistoryTests.cs`, `ConflictsTests.cs` and
  `DiffWordsTests.cs`: observable behavior to translate for the included features.

Relevant RSharp integration points are `buffer.rs`, `editor.rs`, `editor_area.rs`, `solution_view.rs`,
`solution_menu.rs`, `actions.rs`, `keymap.rs`, `dock.rs`, `window.rs`, `window/file_moves.rs`,
`window/solution_actions.rs`, `history_window.rs`, `state.rs`, `theme.rs`, core's `watcher.rs` and
`process.rs`, and text's existing `diff.rs`. Local History already uses `imara-diff`; preserve that behavior
when exposing shared hunk information.

## What the person gets

### Repository, branch and changes

Discover the repository containing the opened workspace, including an opened subdirectory and linked Git
worktrees. A folder outside a repository has an explicit empty state rather than a failing status loop.
Missing Git is reported with an actionable explanation. One repository is active per workspace in this
phase; nested independent repositories and submodule operations are not introduced.

The toolbar shows the current branch, detached HEAD or an unborn branch. Its popup lists branches and
explicit operation submenus: selecting a branch name does not silently check it out. The Commit and Git
tool windows use Rider's home positions and the existing dock, menus, Find Action and platform keymap.
Old saved layouts gain the tools through `DockLayout::complete`.

Read tracked, staged, unstaged, untracked, renamed, deleted and conflicted paths. Keep both names of a
rename and the index/worktree distinction. Solution rows show status through theme tokens, with a textual
status in the change list so color is not the only signal. Refresh on saves, IDE file operations, Git
operations, relevant external file/metadata changes and explicit Refresh. Preserve checked files and
selection by path across refreshes. Closing or changing a workspace discards its old asynchronous replies.

Watch the discovered Git metadata directories explicitly where they are outside the opened folder,
including linked worktrees' per-worktree metadata and common refs. Observe HEAD, index, refs, packed refs
and merge/rebase/cherry-pick/revert state without processing Git's object database. RSharp's existing
folder watcher currently passes only `.git/HEAD`, `.git/index` and `.git/refs`; extend this deliberately.

### Editor changes and diff

Compare open buffers, including unsaved text, with the last committed version. Show added, modified and
deleted lines in the gutter, aligned with folding and wrapping. Next/Previous Change navigates hunks;
clicking a bar shows the changed text and Rollback Lines. Rollback is one undoable buffer edit and
preserves all text outside the chosen hunk, including the final newline.

Provide a reusable side-by-side diff with line and word highlighting, synchronized vertical scrolling,
next/previous difference and revision/path titles. Use it from Commit, Log, Show History and Compare With.
Decode revision bytes using the file's supported encoding and normalize line separators as buffers do.
Binary files and unsupported non-text changes receive an explicit description; no lossy text rewrite.
Deletion, creation, rename and no-final-newline differences remain visible.

Compute comparisons off the UI thread from immutable snapshots. Cache the baseline by repository, HEAD
and path; a keystroke does not spawn Git or reload history. Debounce buffer comparisons, reject stale
results by buffer version, and shape/paint only visible diff/editor rows. Large files do not force a
whole-document comparison on the UI thread. Keep work queues and revision caches bounded.

### Commit, rollback and remote operations

The Commit window lists changed and unversioned files with checkboxes, a multiline message, Amend,
Commit, Commit and Push, Show Diff, Refresh and Rollback. Save affected shared buffers before commands
that consume or replace their disk content; a failed save prevents the command. Refuse an empty message,
an ordinary commit without chosen files, and a commit while conflicts remain. Preserve message and
selection after failure and report Git's actual error.

An ordinary commit includes only chosen paths (both names of a rename), and does not consume unrelated
staged changes. Stage only the chosen paths and use Git's selected-path commit semantics; never run a
pathless `add -A`. Write commit messages to a temporary file and use `git commit -F`. Amend handles a
message-only change without staging the whole tree. A merge/cherry-pick/revert completion follows Git's
whole-index contract and Passenger's explicit handling of excluded changes; test this separately from
ordinary selected-path commits.

Rollback of whole files asks for confirmation with affected paths, keeps recoverable prior bytes in
Local History, and reloads affected buffers and project models afterward. Specify untracked-file deletion
explicitly in the confirmation; never silently remove an unchosen file. Line rollback stays undoable.
File moves through the Solution view preserve the staged blob under its new name, rather than staging
unsaved or unstaged content incidentally.

Push uses the configured upstream or sets one for the first push when its remote is unambiguous. If
several remotes are possible, ask the person to choose. Update Project follows Git's configured pull
merge/rebase policy and surfaces conflicts. Include Fetch and progress/cancellation for long operations.
Use the person's normal Git credentials, SSH agent and configuration; do not store credentials or
implement a credential provider. Disable invisible terminal prompts and report a useful authentication
failure if existing helpers cannot answer. Failed pushes never claim the earlier commit failed.

### Log, branches and history

The Git window contains a Log with graph lanes, commit/author/date/subject/ref information, commit details
and changed files with diffs. List local and remote branches and tags; selecting a branch filters the
log. Render only visible rows and bound/paginate history (Passenger initially requests up to 2,000
commits). Preserve selections by commit hash and avoid Git calls from rendering.

Support checkout, new branch from a chosen revision, rename/delete local branch, merge and rebase onto a
chosen branch. Validate ref names with Git, protect the checked-out branch from deletion, and explain
dirty-tree or linked-worktree refusals. Destructive branch deletion needs the affected branch named in
confirmation. Do not introduce force push or destructive reset as an incidental convenience.

Show History opens scoped tabs for a file (following renames), folder, or selected lines. Compare With
Branch or Tag and Compare With Revision open the same diff view against the current buffer. History
paths are literal pathspecs, including brackets, leading dashes, spaces and Unicode. A missing historical
file is an absent side; an unrelated Git failure is an error, not an empty successful history.

### Conflicts and three-way merge

Detect merge, rebase, cherry-pick and revert state and list unmerged paths, including add/add and
modify/delete conflicts. Provide Accept Yours, Accept Theirs and Merge. Obtain base/ours/theirs blobs
from index stages with Git's filters and decode in the file's encoding. Do not parse conflict markers
as the source of truth. Refuse unsupported binary or symlink text merges with a clear explanation;
whole-side acceptance may still use Git's own file operation.

The merge window has Yours, editable Result and Theirs, a base comparison, synchronized scrolling,
hunk navigation, per-side acceptance, resolve-simple-conflicts and an unresolved count. Apply
nonoverlapping changes automatically; agreeing edits are not conflicts. Word-level simple resolution
handles disjoint word edits, while overlapping edits remain explicit conflicts. Editing Result does
not silently mark every conflict resolved: resolution state remains visible and must be acknowledged.
Cancel leaves disk and index untouched.

During a rebase the person's Yours is Git's theirs. Centralize this mapping so labels, previews and
Accept Yours/Theirs agree. Applying a resolution checks that the file/stages have not changed since
opening, saves any open buffer first, preserves encoding/BOM/line separators and Local History through
the normal writer, then stages the resolution. If staging fails, keep the saved result and report the
remaining conflict. A modify/delete resolution can explicitly choose deletion. Never overwrite an
externally changed file with a stale merge result.

Continue Rebase and the corresponding operation completion become available when allowed by Git's
state. Use noninteractive editor configuration for continuation. Abort names the operation and asks for
confirmation because it replaces working files. After apply, continue, abort, checkout or update,
refresh buffers, the tree, project evaluation, diagnostics, status and history through existing paths.

## Command and lifecycle contract

Build argument arrays directly, never shell command strings. Clear inherited repository/index override
variables as Passenger does; use literal pathspecs, `--` and `--end-of-options` where the command allows.
Resolve revision objects before combining them with paths, and preserve filesystem path bytes where
the platform supports non-UTF-8 names. Validate repository containment before writes. Discovery returns
normal absence only for the expected Git result, not for permission or corrupt-repository errors.

Every Git process runs under Lifeline. Drain stdout and stderr concurrently; never let a full error
pipe deadlock a command. Preserve exit status and useful stderr, bound displayed output, and bound
status/log/blob reads with explicit errors rather than truncated successful data. Large stdin writes
and output reads run concurrently where needed. Cancellation and workspace close end the process tree,
including hooks and helpers. No Git subprocess, disk walk or blocking pipe operation runs on gpui.

Serialize repository mutations and coalesce reads; another window of the same repository must not race
an IDE mutation. Git's own index lock remains authoritative for external programs. Use a process-wide
repository coordinator with per-window subscriptions; clear subscribers and cached buffers on close.
Only persist user choices (commit draft, branch favorites and appropriate panel choices) in the existing
folder state; refresh transient status and operation state from Git.

## Verification and completion

1. Core integration tests create isolated repositories and identities, with hooks, signing, configuration
   and credential prompts controlled by the fixture. Cover porcelain records and odd paths, missing Git,
   absent/corrupt repository distinction, unborn/detached HEAD, worktrees, selected-path commits,
   unrelated staged data, renames, amend, failed hooks, rollback, command failure and cancellation.
2. Text tests cover hunk locations and word boundaries, UTF-8 offsets, CRLF normalization, final newlines,
   empty sides, adjacent edits, three-way agreeing/disjoint/conflicting changes and line rollback.
3. Window tests use `support::Run` and real keys/selectors: dock/actions/focus, checked paths and message,
   diffs and gutter marks after unsaved edits, external refresh, graph/history/branch actions, rollback
   undo, stale replies, failed operations and updates of shared buffers.
4. The full acceptance test uses two local clones and a bare local remote: commit, push, divergent edits,
   rebase conflict, merge-window resolution, Continue Rebase and push. Check the final remote commit,
   resulting bytes and cleared operation state. Also cover modify/delete and rebase-side reversal.
   Tests never push to the person's repository or require a network service.
5. Verify typing responsiveness while status/diff/log work runs and against a file ten times larger;
   measure the existing benchmark before and after when available. No per-key Git commands or history
   refresh, and no worker or polling loop on Welcome without a workspace.
6. Run appropriate suites, formatting, build and repository-required checks. Capture representative
   Commit, Log/diff and conflict/merge states with `just screenshot` and inspect the real-window PNGs.
   Reuse existing UI controls and theme tokens; any new tokens cite Rider's source value.
7. Verify macOS locally and Windows/Linux in CI, distinguishing checks actually run from pending ones.
   Update ROADMAP.md and repository guidance with what shipped and any limits; do not mark Phase 7 done
   before the rebase/resolve/push scenario passes.

## Implementation order and builder decisions

Implement in reviewable layers within this phase: Git process/protocol and real-repository tests;
shared diff/word/merge models; repository controller and refresh ownership; Commit/diff/gutter integration;
Log/branches/scoped history; conflicts/merge and continuation; end-to-end acceptance and visual/performance
verification. Keep each layer usable by the next without speculative extension APIs.

The builder may choose Rust type names, module splits, debounce intervals, bounded cache sizes and exact
test fixture organization to fit existing conventions. Observable Passenger behavior, repository data
preservation, process cleanup and the final acceptance workflow are requirements, not optional defaults.

## Exclusions

This phase does not add Git hosting account integrations, PR review, multiple repository management,
clone/init onboarding, stash UI, annotations/blame, interactive rebase editing, signing setup, force push
or MCP VCS tools. Those are separate features beyond the Phase 7 bullets. Ordinary hooks, credentials
and signing already configured by the person remain Git's responsibility and failures remain visible.
