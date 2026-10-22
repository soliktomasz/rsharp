---
version: 1
slug: "crates-rsharp-app-src-main-rs"
primary_target: "crates/rsharp-app/src/main.rs"
related_targets: []
---

## Scope

The main window of RSharp (crates/rsharp-app). Mode: Operate. Audience: Rider users opening a .NET solution.
Task: browse the solution, open files in tabs, read and edit C#, see where they are and what the IDE is doing.

## Direction contract

THESIS: Rider's Islands Dark, rendered natively: work areas are rounded islands floating on a darker-than-chrome frame, told apart by shade and gaps rather than rules. Refuses the flat VS Code grid of borders.
OWN-WORLD: Rider 2026.2 Islands Dark taken from its own theme files: frame #26282C, islands #191A1C at 10px radius with 6px gaps, text #D1D3D9 / secondary #73767C, accent #3871E1, selection #2A4371, active tab a #233558 fill with a #2E4D89 border at 6px radius, as Islands draws it; Inter for chrome at 13px, JetBrains Mono 13px for code in Rider Dark's C# colours.
STORY: a Rider user sees their solution tree, the file they opened and its place in the status bar, and can type at once with Rider's keys.
FIRST VIEWPORT: 40px main toolbar on the frame (project widget, branch widget; run config and run/debug removed until they run, ponytail scope); 40px stripe with the Project tool window button; Solution island 280px; editor island filling the rest with its 40px tab strip on top; 30px status bar on the frame (Rider's New UI sizes) with breadcrumbs left and Roslyn's state, caret, line separator and encoding right (indent dropped until .editorconfig and indent detection exist).
FORM: pinned by the user (Rider's own Dark); no concept-seed roll, position 1 of 1.
FINISH: unreviewed and undocumented is unfinished; this build ends with the finish review, the verdict, DESIGN.md, and every shipping raster carrying its provenance
SIGNATURE: the island ground itself: #191A1C islands at 10px radius on the #26282C frame, 6px gaps, no rules.
