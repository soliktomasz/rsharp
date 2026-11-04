---
name: RSharp
description: Rider's own Islands Dark, drawn natively in gpui
colors:
  frame: "#26282C"
  island: "#191A1C"
  text: "#D1D3D9"
  text-muted: "#9FA2A8"
  text-secondary: "#73767C"
  accent: "#3871E1"
  on-accent: "#FFFFFF"
  selection: "#2A4371"
  selection-inactive: "#33353B"
  row-hover: "#FFFFFF10"
  button-hover: "#FFFFFF17"
  button-pressed: "#FFFFFF29"
  tab-selected: "#233558"
  tab-selected-border: "#2E4D89"
  modified: "#548AF7"
  error: "#F75464"
  warning: "#E5A336"
  debug-breakpoint: "#DB5C5C"
  execution-point: "#2F3E54"
  editor-text: "#BCBEC4"
  editor-line-number: "#4B5059"
  editor-line-number-current: "#A1A3AB"
  editor-current-line: "#1F2024"
  editor-selection: "#214283"
  editor-caret: "#CED0D6"
  syntax-keyword: "#6C95EB"
  syntax-string: "#C9A26D"
  syntax-string-escape: "#D688D4"
  syntax-number: "#ED94C0"
  syntax-comment: "#85C46C"
  syntax-type: "#C191FF"
  syntax-method: "#39CC9B"
  syntax-property: "#66C3CC"
typography:
  title:
    fontFamily: "Inter"
    fontSize: "20px"
    fontWeight: 600
  body:
    fontFamily: "Inter"
    fontSize: "13px"
    fontWeight: 400
  body-strong:
    fontFamily: "Inter"
    fontSize: "13px"
    fontWeight: 600
  badge:
    fontFamily: "Inter"
    fontSize: "10px"
    fontWeight: 600
  code:
    fontFamily: "JetBrains Mono"
    fontSize: "13px"
    fontWeight: 400
    lineHeight: "20px"
    fontFeature: "\"liga\" 0, \"calt\" 0"
rounded:
  xs: "3px"
  sm: "4px"
  md: "6px"
  island: "10px"
spacing:
  gap: "6px"
  row-indent: "18px"
  toolbar: "40px"
  stripe: "40px"
  status-bar: "30px"
  row: "24px"
  tab-strip: "40px"
  tab: "28px"
  solution-width: "280px"
components:
  island:
    backgroundColor: "{colors.island}"
    rounded: "{rounded.island}"
  island-header:
    textColor: "{colors.text}"
    typography: "{typography.body-strong}"
    height: "40px"
    padding: "0 14px"
  toolbar-widget:
    textColor: "{colors.text}"
    rounded: "{rounded.md}"
    height: "30px"
    padding: "0 8px"
  toolbar-widget-hover:
    backgroundColor: "{colors.button-hover}"
  toolbar-widget-pressed:
    backgroundColor: "{colors.button-pressed}"
  project-badge:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    typography: "{typography.badge}"
    rounded: "{rounded.sm}"
    size: "20px"
  stripe-button:
    rounded: "{rounded.md}"
    size: "32px"
  stripe-button-shown:
    backgroundColor: "{colors.accent}"
  tree-row:
    textColor: "{colors.text}"
    rounded: "{rounded.sm}"
    height: "24px"
  tree-row-hover:
    backgroundColor: "{colors.row-hover}"
  tree-row-selected:
    backgroundColor: "{colors.selection}"
  tree-row-selected-unfocused:
    backgroundColor: "{colors.selection-inactive}"
  tab:
    textColor: "{colors.text-muted}"
    rounded: "{rounded.md}"
    height: "28px"
    padding: "0 4px 0 8px"
  tab-hover:
    backgroundColor: "{colors.button-hover}"
  tab-selected:
    backgroundColor: "{colors.tab-selected}"
    textColor: "{colors.text}"
  status-widget:
    textColor: "{colors.text-muted}"
    rounded: "{rounded.sm}"
    height: "22px"
    padding: "0 6px"
  button-primary:
    backgroundColor: "{colors.accent}"
    textColor: "{colors.on-accent}"
    rounded: "{rounded.sm}"
    height: "28px"
    padding: "0 14px"
---

# Design System: RSharp

## Overview

**Creative North Star: "Rider's Own Dark"**

RSharp wears JetBrains Rider 2026.2's Islands Dark, read value for value from Rider's theme files (ManyIslandsDark over expUI_dark) and Rider Dark's C# colour scheme. Nothing here is invented: when a design question comes up, Rider's answer is the answer. The world is dense, quiet and keyboard-first; chrome recedes so code carries the colour.

The signature is the island ground: each work area (the Solution tool window, the editor) is a darker rounded island floating on a lighter frame, separated by 6px gaps of frame rather than by drawn lines. The toolbar, the tool window stripe and the status bar sit directly on the frame, not inside islands.

The platform is native (Rust, gpui). Tokens are functions in `crates/rsharp-app/src/theme.rs`, each commented with the Rider key it comes from; layout sizes are constants in `window.rs` and `editor.rs`.

**Key Characteristics:**
- Islands (#191A1C, 10px radius) on a frame (#26282C), 6px gaps, no rules between them.
- One accent, Rider's blue, used for selection-adjacent states, the shown stripe button, the project badge, the primary button and key hints.
- Inter 13px for all chrome; JetBrains Mono 13px on a 20px line for code, ligatures off.
- Rider's New UI sizes: 40px toolbar, stripe and tab strip; 30px status bar; 24px rows; 28px tabs.
- IntelliJ Platform expui icons, drawn in their own colours, never recoloured.

## Colors

A near-black, cool-neutral chrome where colour belongs to code and to one restrained blue.

### Primary
- **Rider Blue** (accent): the shown tool window stripe button, the project widget's initials badge, the welcome screen's primary button, and the keys in the empty editor's hints.
- **Modified Blue** (modified): the 7px dot an editor tab shows in place of its close button while the file has unsaved changes.

### Neutral
- **Frame Graphite** (frame): the window ground under toolbar, stripe and status bar; also the 1px line under the editor's tab strip.
- **Island Black** (island): the ground of every island, the editor included.
- **Rider Text** (text): primary chrome text.
- **Widget Grey** (text-muted): status bar widgets, unselected tabs, the welcome subtitle.
- **Secondary Grey** (text-secondary): the empty editor's instructions.
- **Focused Selection** (selection) and **Unfocused Selection** (selection-inactive): a tree row's selection with and without focus.
- **Hover veils** (row-hover, button-hover, button-pressed): translucent white over whatever ground lies beneath, so one token works on frame and island alike.
- **Selected Tab Navy** (tab-selected) with **Tab Edge** (tab-selected-border): the selected editor tab's fill and 1px border.

### Signal
- **Error Rose** (error): failed language server state, error diagnostics' wavy underline.
- **Warning Amber** (warning): warning diagnostics and status messages.

### Editor
- The editor scheme (editor-*) is Islands Dark's over Rider Dark's: soft text, dim line numbers with the caret line's number brighter, a barely lifted current-line band, a navy selection and a light caret 2px wide.
- Syntax (syntax-*) is Rider Dark's C# colours, one per kind: keywords blue, strings tan, escapes orchid, numbers pink, comments and doc comments green, types and namespaces violet, methods green-teal, properties cyan.

### Named Rules
**The Theme File Rule.** Every colour is a token in theme.rs, named with the Rider key it was read from; a view never writes a literal colour, and a new colour is read from Rider's theme files, not chosen.

**The Code Carries Colour Rule.** Chrome stays neutral; saturated hues belong to syntax, signals and the single accent.

## Typography

**UI Font:** Inter (Regular and SemiBold carried in the executable)
**Code Font:** JetBrains Mono (Regular and Bold carried in the executable)

**Character:** Rider's own pairing: a neutral grotesque for chrome at one size, and a monospace built for code.

### Hierarchy
- **Title** (600, 20px): the product name on the welcome screen only.
- **Body** (400, 13px): every piece of chrome text: tree rows, tabs, status bar, toolbar.
- **Body Strong** (600, 13px): island headers ("Solution"), the project widget's name, the tree's root row.
- **Badge** (600, 10px): the project's two initials in its 20px badge.
- **Code** (400, 13px on 20px): the editor, ligatures and contextual alternates off.

### Named Rules
**The One Size Rule.** Chrome is 13px throughout; hierarchy comes from weight (600) and colour, not size.

**The Plain Code Rule.** Code draws without ligatures, as Rider does until a person turns them on.

## Layout

A fixed IDE shell, top to bottom: a 40px toolbar on the frame, Rider New UI's order: project widget and branch widget on the left (on macOS from 84px in, clearing the traffic lights), then on the right Build, the run configuration widget, Run (Rerun while running), Debug, Stop while running, Search Everywhere and Settings, then a row holding the 40px tool window stripe and the islands, then a 30px status bar on the frame (breadcrumbs left, Roslyn's state, caret, line separator and encoding right).

The Solution island is 280px wide; the editor island takes the rest. Islands are separated from each other and from the window edge and status bar by the 6px gap. Inside an island, a 40px header (14px side padding) or the 40px tab strip comes first. Tree rows are 24px, indented 18px per depth from a 6px start, inside 6px of island padding. The editor's gutter is the digit width times the line count's digits plus 32px, with 16px before text.

**The Rider Sizes Rule.** Shell measures are Rider New UI's: 40 / 40 / 30 / 24 / 28 / 6 / 10. A new part takes Rider's size for it rather than a nearby value.

## Elevation & Depth

Flat. There are no shadows. Depth is tonal: the island is darker than the frame it floats on, and states are translucent white veils (about 6%, 9% and 16%) laid over the ground.

**The Gap Not Line Rule.** Islands are told apart from the frame and from each other by shade and the 6px gap, never by a border or divider.

## Shapes

Soft rounded rectangles at four radii: 10px for islands, 6px for tabs, toolbar widgets and stripe buttons, 4px for tree rows, status widgets, the project badge and the primary button, 3px for a tab's close button. The modified dot is a full circle. Borders are rare: the selected tab's 1px edge and the tab strip's 1px bottom line in frame colour, both Rider's own.

## Components

### Islands
- **Corner Style:** 10px.
- **Background:** island.
- **Border / Shadow:** none.
- **Header:** 40px, 13px SemiBold, 14px side padding.

### Toolbar Widgets
- 30px high, 8px side padding, 6px radius, 6px between icon and label; transparent at rest, button-hover on hover, button-pressed while pressed. The project widget leads with a 20px accent badge of two initials. The run configuration widget leads with the project icon and opens its menu under itself.
- Toolbar icon buttons are 30px squares with a 16px icon and a tooltip naming the action, dimmed to 35% while the action has nothing to act on.

### Tool Window Stripe
- 32px square buttons with a 20px icon and 6px radius, centred in the 40px stripe. The shown window's button fills with the accent; others take the hover and pressed veils. Each tool window has its own IntelliJ expui tool window icon (Commit, Git, Debug, Profiler, Hierarchy and Live Visual Tree take `commit`, `vcs`, `debug`, `profiler`, `hierarchy` and `toolWindowComponents`); no two windows share one.

### Tree Rows
- 24px, 4px radius, 4px between chevron, icon and name; a 16px blank stands in for a leaf's chevron. Hover veil; selection is selection with focus and selection-inactive without.

- The Solution island's header carries Show Hidden Files (a 24px eye button before Hide, button-pressed while on):
  dot folders, bin and obj, and a project's dot files it does not hold, listed dimmed; kept with the folder.

### Editor Tabs
- 28px tabs in a 40px strip, 2px apart, 6px radius, a 1px border transparent unless selected. Unselected: text-muted, hover veil. Selected: tab-selected fill, tab-selected-border edge, text colour. A 16px close button (3px radius) at the end, replaced by the modified dot while unsaved.

### Commit Window
- Top down, as Rider's: the branch and Refresh, the toolbar (Show Diff, Rollback…, History), the changes (or "No
  changes" in their place), Compare With, the Amend checkbox, the message box (no line numbers, wrapped), then Commit
  as the primary button beside Commit and Push.

### Status Bar
- Widgets are 22px, 6px side padding, 4px radius, text-muted. While a build, a test run or Roslyn's load is under way, Rider's indeterminate progress bar
  follows its text: a 120×4px track in progress-track (gray-50) with a 40px segment, a gradient from progress-start
  (blue-110) to the accent, sweeping across every 1.2s; one bar at a time, the build's or tests' before Roslyn's. Breadcrumbs join with a 12px chevron at 60% opacity. The language server's failure and error messages take error, warnings take warning.

### Primary Button
- 28px, 14px side padding, 4px radius, accent fill with white text. Used on the welcome screen.

### Debug Window
- The toolbar is 32px: 28px square buttons with 16px expui icons in Rider's groups — Resume, Pause, Stop | Step Over,
  Step Into, Step Out | Mute Breakpoints — split by 1px separator rules, dimmed to 35% while they cannot act. The
  session's state is the island header's subtitle only.
- After a step over, a Return values group sits above Variables: 24px rows with the method's name, `= value` in
  text-muted and the type in text-secondary. Its members are not opened in place yet.
- Frames and Variables share the body: Frames is a 300px column of 24px rows (a paused program's threads above it
  when there is more than one, the chosen thread in selection; the chosen frame in selection) and
  Variables takes the rest, its rows indented 18px a level with a 12px chevron (the chosen variable in selection,
  with a 28px Set value row — a field and a Set button — under the tree while one is chosen); Watches sit under it
  around a 28px search-field-styled row. The console is a bordered-off share of the bottom under a 24px "Console" label.
- A stopped line's frame values are one inlay hint at the line's end — `a = 41 · b = 42` — in the inlay-hint colour
  on the inlay-hint chip, four values at most.
- The gutter draws a 10px breakpoint dot in breakpoint red, left of the line numbers; one the debugger will not hit
  is 8px in warning amber, and one turned off or muted is 8px in text-secondary. The stopped statement's band is
  execution-point over the full width, behind the text.
- Rider's Smart Step Into popup is 460 wide: a 40px title, then 26px rows of the statement's calls (the chosen one in
  selection) with their line:column in text-secondary.
- Rider's Evaluate Expression dialog is 560 wide: a 44px title with the frame's name, the expression field in a
  26px box, a Result box at least 48px high (warning when the debugger refused), the value's members as 22px rows in
  a 160px scroll (name, `= value` in text-muted, type in text-secondary), and Close beside Evaluate.
- Rider's Attach to Process dialog is 820×520: a 44px title, a 40px row with the search field and Refresh, the
  Executable / Process ID / User / Command Line headings over 24px rows (Recently attached first), and Cancel beside
  the primary "Attach with .NET Debugger" in a 56px footer.
- Rider's Breakpoints dialog is 760×540 over the window: a 44px title row with Mute Breakpoints, a 300px list
  (26px rows, the chosen one in selection, a 16px tick before each name) of Line breakpoints, Function breakpoints
  (whose form has a Function name field) and Exception breakpoints, and the chosen item's form on the right, with
  Add Function Breakpoint, Add Exception Breakpoint, Remove and Done in a 56px footer.

### Unit Tests Window
- A 32px toolbar: Run All (icon and label), then 28px icon buttons with tooltips — Run Selected, Debug Selected, Rerun Failed, Stop |
  Expand All, Collapse All — dimmed to 35% while they cannot act. At its right, one chip per outcome (Failed, Passed,
  Skipped, Not Run, and Running only during a run): the expui test icon and its count, 24px, 4px radius. A chip is a
  filter: pressed it takes tab-selected with its border, and pressed chips together show only their outcomes; a chip
  with no tests is at 45%.
- Below it the search field (search-field ground) and the borderless "Group by" chooser.
- The footer leads with Rider's results bar, 96×4px on progress-track: failed in `failed` (#DB5C5C), passed in
  `passed` (#57965C), skipped in text-secondary; then "N tests · n passed · n failed · n skipped · time".

### NuGet Window
- Package rows are 44px: the package icon, the id with its latest version right-aligned (warning when vulnerable),
  and the description's first line at 12px in text-secondary beneath. Feed is a borderless "Feed All Feeds" chooser.
- Details: the id and version chooser, then the projects (32px rows; Install is the accent button, Update and
  Remove bordered), advisories, "by … · from …" and the description last, so a short island still shows the projects.

### App Icon
- `assets/icon/rsharp.svg`: a graphite squircle (the frame) holding a darker inset island, a bold R in light grey and
  a blue sharp (#5A8DF0 to the accent) at its shoulder. `RSharp.icns` is made from it and carried by the bundle.

### Hierarchy Window
- On the right (RightTop), `tw-hierarchy`. A 32px row of two 24px segments (Callers | Callees, or Supertypes |
  Subtypes; the chosen one in tab-selected), then 24px rows indented 18px a level: chevron, a node-method or
  node-class icon, the symbol's short name (`B()`, not `Shop.Calls.B()`), and file:line in text-secondary. A
  double-click goes to a caller's call, else the symbol's name.

### Bookmarks
- A 16px gutter column between the numbers and the swatches, there only while the file has bookmarks, holding expui's
  `gutter/bookmark` in bookmark (#F2C55C). The Bookmarks window (LeftBottom, `tw-bookmarks`) lists 24px rows: the
  icon, file:line in text-secondary, then the line's text in the code font.

### Code Vision
- Rider's Code Vision at its "Right" position: each member's usage count ("2 usages", "1 usage", "no usages") three
  spaces after its line's end, in the inlay-hint colour with no chip; a click lists the usages in Find. Settings'
  Show Code Vision turns it off. Squiggles, bands and selections end at the text, never under a lens or hint there.

### Empty Editor Hints
- Two aligned columns 10px apart: instructions right-aligned in text-secondary, the keys left-aligned in the accent (an instruction that is not a key stays text-secondary).

## Alternate Design: Zed

Settings | Appearance & Behavior | Appearance | Design switches the whole shell to Zed's world (`theme::Design::Zed`), read
from Zed's One Dark theme (`assets/themes/one/one.json`) and zed.dev: every token in theme.rs holds both values.

- **Flat panes, not islands.** Radius 0, the 6px gap becomes a 1px line of the frame (#3B414D, One Dark's title and
  status bar ground); panels on #2F343E, the editor a step darker on #282C33. Splitters keep a 7px grab area.
- **Square tabs.** A 32px bar of full-height cells split by 1px border.variant (#363C46) lines; the shown tab takes
  the editor's ground and breaks the bar's bottom line, so it opens into the text.
- **Denser shell.** 34px toolbar, 36px stripe with 28px buttons and 16px icons, 28px status bar. A shown tool
  window's button is element.selected (#454A56), not the accent.
- **Colour.** Accent #74ADE8 with dark text on it; modified and warning #DEC184, error #D07277. Syntax is One Dark's:
  keywords violet #B477CF, strings green #A1C181, types teal #6EB4BF, functions blue #73ADE9, properties #D07277,
  numbers #BF956A, comments #5D636F. The terminal takes One Dark's ANSI 16.
- **Fonts.** Zed's own pair, carried in the executable: IBM Plex Sans (Regular, SemiBold) for chrome and Lilex
  (Regular, Bold, Italic) for code, ligatures still off. Sizes stay the person's.

## Do's and Don'ts

### Do:
- **Do** read a new colour from Rider's theme files and add it to theme.rs with its Rider key.
- **Do** float every work area as an island (#191A1C, 10px) on the frame with 6px gaps.
- **Do** keep chrome text at Inter 13px and mark hierarchy with weight 600 or a muted colour.
- **Do** use the translucent hover and pressed veils so one state token works on any ground.
- **Do** draw IntelliJ expui icons in their own colours at 16px (12px for breadcrumb chevrons, 20px in the stripe).

### Don't:
- **Don't** write a literal colour in a view.
- **Don't** draw borders or dividers between islands, or between islands and the frame.
- **Don't** add shadows; depth is tonal.
- **Don't** turn on code ligatures by default.
- **Don't** introduce sizes between Rider's (a 36px toolbar, a 26px row); take Rider's.
