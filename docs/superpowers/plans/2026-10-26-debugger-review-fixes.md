# Debugger review fixes

> **For agentic workers:** Use executing-plans for the connected Hot Reload changes and dispatching-parallel-agents for the independent DAP fix and window regressions.

**Goal:** Fix review findings 1–6 on PR #22.

**Architecture:** Keep the existing window controller and DAP client. Use the subscription's emitter, preserve explicit empty exception rules, queue saves behind an apply, collect solution buffers consistently, materialize decompiled frame sources, and end discarded Hot Reload sessions.

**Tech Stack:** Rust, gpui, Passenger DAP and Roslyn.

**Spec:** The six findings in this chat, authorized by “fix 1-6”. Finding 7 is outside this change.

## Constraints

- No new dependencies or sidecars. Every process uses the existing process owner.
- Keep the PR branch and leave changes reviewable in the working tree.
- Tests use private workspaces and real debugger behavior where available.

## Review focus

- Multiple saves while an update is pending: apply the latest text before a waiting resume.
- Failed or refused updates: do not retry indefinitely.
- Projects outside the solution folder: use the same scope for baseline and apply.
- Later stops in an already decompiled module: open its cached source again.
- Stop or window closure before the first Hot Reload result: end the returned Roslyn session.

## Tasks

- [x] 1. Add a last-tab buffer-release regression; remove the subscription's captured buffer. Track edit identity by buffer entity so reopening does not lose breakpoint moves.
- [x] 2. Preserve explicit empty exception rules; verify exception-off and DAP filter behavior.
- [x] 3. Add deterministic pending-save regression; coalesce a pending apply and drain it before resume, including after the saved buffer closes.
- [x] 4. Exercise a sibling project through the window; share solution buffer collection between baseline and apply.
- [x] 5. Extend decompiled navigation through another stop; materialize resolved decompiled frames before opening.
- [x] 6. Exercise discarded Hot Reload results; end their session when the generation or window is gone.
- [x] Run formatting, workspace check, debugger regressions and relevant DAP tests.
- [x] Review the combined change with a fresh agent and record verification.
- [ ] Inspect a real screenshot: capture attempted twice through tools/screenshot.sh, but macOS returned “could not create image from window”; the UI connector could not address the unbundled process either.

## Verification ledger

- Review established RED for 1, 2, 4 and 5 using real window tests; retain regressions in the repository.
- Execute the already-authorized fixes without an extra plan-approval pause.
- Final window suite: `CARGO_INCREMENTAL=0 cargo test -p rsharp-app --test debugger -- --test-threads=2 --nocapture`: 25 passed, no skips, 183.40 s.
- Final Hot Reload unit suite: `CARGO_INCREMENTAL=0 cargo test -p rsharp-app --lib window::debug::tests -- --nocapture`: 4 passed. Observed RED for dropped saves, discarded sessions, and queued saves after buffer closure before their fixes.
- DAP parity: `cargo test -p rsharp-dap --test passenger exception_filter -- --nocapture` and `... user_unhandled ...`: 6 + 3 passed.
- `cargo check --workspace`, `cargo fmt --all -- --check`, and `git diff --check`: passed. Check retains the existing optional-sidecar build warning; the live window tests used the successfully rebuilt adapter.
- `cargo test --workspace` was attempted but its build exhausted disk space before tests ran. Removed only regenerable incremental artifacts and this review's temporary probe binaries; used nonincremental builds for subsequent checks.
- Fresh review found a queued-save/closed-buffer interaction. Fixed it with one forced pending emission; its regression failed before the change and passed after it. No other concrete issue was reported.
- Finding 7 and unrelated stale-response changes remain outside the user's requested scope.
