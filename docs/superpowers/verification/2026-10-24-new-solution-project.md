# New Solution / New Project verification

Candidate status: implementation in final review. The compact native New Project capture shows a long Location value painting over Browse; that material visual finding remains to be fixed before handoff.

## Scope

Creates, restores and opens solutions; adds projects with the existing preserving solution writer. Automatic build remains Phase 5. NuGet management and MSBuild code help remain the next Phase 4 parts. Test-only builds prove generated artifacts are sound.

## Evidence

- macOS: formatting, workspace/all-targets build, diff check and 295 workspace tests passed. The two ignored entries are subprocess stand-ins explicitly started by their parent tests. Windows and Linux execution remains with CI.
- SDK 10.0.101: the isolated catalog matches CLI names, short names and languages in both directions. Source/project bytes match an independent same-SDK Console/Class Library invocation.
- Passenger's original fixtures: .sln BOM/CRLF and .slnx gain exactly the expected project/configuration/nesting lines, using only the newly captured .sln ID.
- Real UI: New Solution grows to Console + Class Library + xUnit; Add Reference bytes match Passenger; the solution builds and Roslyn finds a symbol in the newly added library.
- Avalonia.Templates 12.1.4: local-feed Install, retained Type, real metadata, Accelerate removal before restore, and a successful artifact build. A local solution template adds all projects once and is excluded from New Project.
- Failures: invalid/duplicate names and destinations, process pipe pressure/timeout/descendants, cancellation, output ownership changes, unrelated files, late-result cleanup refusal, owner/context changes, deleted logical folders, unsaved solution buffers, batch attachment refusal and Local History.
- Native screenshots are development evidence under `.impeccable/review/new-project/` (ignored as repository policy requires). Captured default New Solution and New Project, compact New Project, missing pack, installation output, invalid name and a solution template's hidden project controls. The shell capture lacked Screen Recording access; CUA captured a temporary bundle of the same binary. Exact 720x480 is tested through gpui input; macOS's native quarter arrangement is 756x481 content.

## Performance

Release measurements are in `bench/results.csv`. The first post-change measurement overlapped tests; the second ran after they settled. No run stalled.

| Projects | Baseline first paint ms | Quiet after ms | Baseline key median ms | Quiet after ms | IDE RSS before / after MB |
|---|---:|---:|---:|---:|---:|
| 1 | 1270.8 | 746.9 | 3.934 | 6.822 | 102.9 / 103.0 |
| 3 | 308.6 | 282.8 | 3.919 | 3.944 | 101.3 / 103.0 |
| 10 | 338.1 | 293.4 | 3.861 | 3.900 | 112.7 / 103.8 |

The one-project quiet key P95 was 7.631 ms, maximum 7.931 ms; 3/10-project medians stayed around 3.9 ms. These are frame-timing measurements, not an added editor code path. A separate normal launch with preload enabled typed 200 keys during startup: first paint 820.527 ms, median 3.345 ms, P95 4.344 ms, maximum 6.564 ms, zero stalls. `RSHARP_PROFILE_STARTUP=1` uses the existing timing hooks without --bench's preload exclusion or settling delay.

## Rulings I made

- Ruling: Native worktree creation succeeded but writes were sandbox-denied; archived the unused worktree and use codex/new-solution-project in the writable checkout — skill sandbox fallback, main remains at the approved plan — cost if wrong: checkout branch changes are visible to the user.
- Ruling: cargo is installed under ~/.cargo/bin but absent from tool PATH; commands use a per-process PATH prefix — no machine configuration changes — cost if wrong: none beyond a failed command.
- Task 1: Ruling: use futures::io::AllowStdIo and futures::executor::block_on rather than futures-lite::io::AllowStdIo (which does not exist) — reuses the existing workspace futures dependency, avoids an extra direct dependency — cost if wrong: package metadata tests would fail.
- Task 2: Ruling: carry pack metadata as included JSON mechanically transcribed from Passenger rather than many Rust constructor calls — preserves every field/version and reduces handwritten duplication — cost if wrong: malformed carried data would panic, pinned by the pack-offer tests.
- Task 3: Ruling: verify and commit tasks 3/4 together because the private executor intentionally has no production caller until creation is connected, and warnings are errors — avoids exposing a generic public runner or suppressing dead-code warnings just for an intermediate commit — cost if wrong: a larger commit to review. RED command/request APIs observed; command unit tests already pass.
- Task 4: Ruling: split execution/ownership into create/job.rs, keeping validation and literal argument construction in create.rs — focused files without moving unrelated code — cost if wrong: one extra internal module.
- Task 4: Ruling: CreatedProjects owns a shared output lease until keep(), with explicit discard() and cleanup on an unaccepted result's drop — closes the cancellation race after generation but before window handoff — cost if wrong: callers must explicitly keep useful completed output; pinned by dropped-result/keep tests.
- Task 5: Ruling: preload accepts the last recent entry, resolving BesideLast on its worker, rather than resolving a directory on the UI thread — preserves the spec's directory rules without synchronous filesystem checks — cost if wrong: prefetch uses an unsuitable SDK, but the dialog's authoritative read still gates Create.
- Task 6: Ruling: retain disabled option values in the request snapshot for enable-condition evaluation; the argument builder still emits only enabled options — conditions can reference retained disabled fields, as Passenger does — cost if wrong: CLI argument tests would catch an extra emitted option.
- Task 7: Ruling: keep rendering in new_project_dialog/render.rs while state and async operation ownership stay in their own files — the shared dialog remains one view, with no form/job framework — cost if wrong: one extra internal file.
- Task 7: Ruling: gpui's test debug-selector map retains removed selectors, so hidden project inputs are asserted through the rendering predicate plus visible Install rather than asserting a stale selector disappears — live screenshots verify the hidden row — cost if wrong: a rendering-only visibility regression needs visual verification.
- Task 8: Ruling: expose a small attach_generated paths method, shared by real Created handoff and fixture integration tests, instead of manufacturing an output-ownership lease in tests — keeps lease internals private and routes existing projects through add_existing — cost if wrong: one extra application method.
- Task 8: Ruling: validate all picked project files before batching Add Existing Project's edits — a project removed between generation and attachment must not leave a partial solution write — cost if wrong: previously accepted missing or non-.NET project paths are now refused.
- Task 9: Ruling: prepare the pinned Avalonia 12.1.4 package and its build dependencies in a separate developer/CI script; live tests then install/restore only from a local feed into private homes/hives/caches — keeps ordinary fixture tests offline and leaves the person's installs alone — cost if wrong: live prerequisites could need maintenance when the SDK's xUnit template changes.
- Task 9: Ruling: add RSHARP_PROFILE_STARTUP to the existing bench hooks for a normal launch, without --bench's preload exclusion or Roslyn settling delay — measures keys while preload runs, no public screenshot/test CLI flag — cost if wrong: an extra internal profiling environment path to maintain.
- Task 9: Ruling: repeat the release benchmark once after the workspace suite settled because the first run overlapped test load — retain both CSV runs rather than hide the noisy figures — cost if wrong: one additional measurement run; no product behavior change.
- Task 9: Ruling: use CUA screenshots of a temporary bundle of the same compiled binary because the shell screenshot caller has no Screen Recording access and just is absent — preserves native rendering and the repo's ignore policy — cost if wrong: test-bundle launch environment may differ from the ordinary executable; live CLI/window tests cover creation separately.
- Task 9: Ruling: exact 720x480 geometry is verified by gpui real-input tests; native compact captures use macOS's supported 756x481 quarter layout because the CUA app-bound drag API cannot grab the window's outside resize border — no screenshot-only product switches — cost if wrong: a wrap specific to the remaining 36px was not visually captured.
- Task 9: Ruling: final whole-branch and Impeccable reviews run inside Task 9's final step, before its completion line, as the plan explicitly requests review there — avoids claiming Task 9 complete while review remains — cost if wrong: review precedes the ledger completion record rather than follows it.

## Review

Fresh whole-branch code and native Impeccable finish review pending. DESIGN.md and its pre-existing sidecar remain unchanged; no web detector ran for native gpui code.
