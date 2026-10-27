# XAML review fixes implementation plan

> Execute inline in this chat. The user authorized fixes 1–5 from the PR review.

**Goal:** Keep completion and XAML project data correct through edits, closes, and solution switches.

**Architecture:** Preserve Passenger.Xaml's immutable project snapshots. Send project-change events through the existing Rust background-task/channel pattern, and remove file overlays and workspace hosts at their owning lifetimes.

**Tech stack:** Rust, gpui, Passenger.Xaml, JSON lines.

**Spec:** The five numbered findings in this chat's review of PR #25.

## Constraints

- Keep UI work off blocking server calls; use existing process lifelines.
- Add no dependencies, literal UI colours, or unrelated features.
- Keep the documented restore-watcher limitation.
- Capture and inspect the real window after visible behavior changes.

## Tasks

- [x] Add failing real-host/window tests for binding separators, sibling-project changes, dependency diagnostics, discarded resources, and reopening a solution after a closed-session change.
- [x] Preserve completion word characters in `rsharp-lsp/src/xaml.rs` and `editor_assist.rs`; request a fresh list for context separators in `editor_xaml.rs`.
- [x] In `xaml-host/Program.cs`, let builders decide C# ownership; publish summary changes from checks and restore closed files from disk. Keep snapshot mutations in protocol order.
- [x] In the Rust XAML client and `buffer.rs`, relay completed invalidations and recheck open buffers. The listener belongs to the shared buffer so splits and detached editors keep receiving changes. Release the window's host when leaving the workspace.
- [x] In `buffer.rs`, close the host's file overlay when the last buffer goes away, without blocking the UI.
- [x] Run the new regressions, relevant existing tests, formatting, and the workspace suite; attempt a real-window screenshot. Review the diff and record verification exceptions before reporting.

## Review focus

- Dots/colons start a new completion context unless the returned list treats them as name characters.
- C# referenced from a sibling directory must refresh even though it is outside the app directory.
- Unsaved resource edits must recheck other buffers without an endless loop of checks.
- Closing one split must retain the shared buffer's overlay; closing its final editor must drop it.
- Closed workspaces must not receive old events or retain cached projects on reopen.

## Verification notes

- The sibling-project regression failed before removing the folder restriction, then passed.
- The window regressions exercise dot completion, disk-source refresh, live resources, closing one split versus the last editor, and solution reopening.
- Final independent review found an empty-member completion case: a pending/empty result left the old popup active. Its regression failed before clearing the popup when requesting a new context; all six window tests then passed.
- `cargo fmt --all -- --check` and `git diff --check` pass.
- The first workspace run passed every app test, then failed `install_update_and_remove_write_the_project_as_dotnet_does` with a .NET dependency-graph error. That test passed in isolation and with the other core tests on retry.
- The remaining-crates run passed the core tests and 95 of 96 Passenger debugger tests. `a_user_unhandled_filter_stops_where_the_user_left_the_exception` hit a five-second implicit-evaluation timeout; its isolated rerun passed. The debugger session test and the language, LSP, MCP, terminal, and text suites then passed separately.
- Final XAML verification: all six window tests and both host tests pass. The full LSP crate also passes (19 unit tests, two Host tests, one Roslyn integration test).
- Real-window capture was attempted twice. macOS refused it; `CGPreflightScreenCaptureAccess()` returned `false`. The app automation tool also could not bind the unbundled executable. Visual capture remains unverified.
