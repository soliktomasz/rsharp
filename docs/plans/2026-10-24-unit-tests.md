# Phase 5 Unit Tests

Goal: complete ROADMAP.md's Unit Tests slice, following Passenger's Testing/ behavior.

Use tree-sitter for source discovery, Rust for Microsoft.Testing.Platform JSON-RPC and VSTest's socket
protocol, and a docked gpui Unit Tests window. Keep test identity stable across code and runner discovery;
the project, method and optional data row identify a test, with target framework distinguishing results.
Build and save before running, keep work off the UI thread, cancel process trees on Stop/workspace close,
and retain the folder's grouping and outcome filter. No new dependencies unless indispensable.

## Tasks

- [x] Discovery: tests first for framework attributes, namespaces, nesting, abstract types and traits;
  implement syntax extraction over existing trees, cached closed-file scanning, and open-file overrides.
- [x] Runners: shared test model, evaluated assembly resolution, MTP and VSTest discovery/run protocols,
  streaming outcomes, errors, and cancellation, with protocol and real SDK tests.
- [x] Window: source tree before build, Group By presets and levels, filtering, selected/all/failed runs,
  result details and source navigation; wire actions, persistence, build coordination and lifecycle.
- [x] Verify: targeted tests, workspace tests, formatting/build, real xUnit project discovery/run,
  review the combined change, and document delivered behavior and limits.
- [ ] Actual-window screenshot: attempted with tools/screenshot.sh; macOS Screen Recording permission is
  unavailable (`CGPreflightScreenCaptureAccess()` returned false), so visual QA remains pending.

Review focus: unsaved source edits; nested/linked projects; framework and data-row identity; runner failure
or cancellation; stale results after leaving a workspace. Use existing tokens, controls and process spawn.

## Verification

Final macOS run: `cargo test --workspace` passed all 467 tests across 70 test binaries/doc-test runs.
`cargo build --workspace --all-targets`, `cargo fmt --all --check`, and `git diff --check` passed.
Real SDK tests cover both MTP/xUnit v3 and VSTest/xUnit v2, selected data rows, runtime-generated rows,
traits/output/attachments, custom MSBuild target paths, and bounded cancellation with host exit. Window
tests include direct .csproj discovery without a solution or build, unsaved/closed/linked sources,
context selection, grouping persistence, failed builds, Stop, actual execution and rerunning failures.
Windows/Linux execution remains for CI.
