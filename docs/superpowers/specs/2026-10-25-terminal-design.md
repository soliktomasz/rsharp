# Terminal — Phase 6

Status: approved by the user's “ok” on 2026-10-25.

## Intent

A person opens Terminal and uses their shell inside RSharp: commands, vim, htop and interactive .NET
programs work as they do in a terminal. Output never prevents typing in the editor or stopping a program.
This completes Phase 6 and the Run-in-terminal item explicitly deferred from Phase 5.

## References and approach

The roadmap selects `alacritty_terminal` for emulation and `portable-pty` for the PTY. This is the proposed
approach: reuse their screen and platform support, and write only RSharp's session ownership, input and gpui
view. Porting Passenger's emulator would duplicate maintained terminal logic; embedding an external
terminal would not provide the dock, focus, links or shared Run lifecycle the phase requires.

Passenger references:

- `src/Passenger.Terminal/TerminalSession.cs`: login shell, isolated history, asynchronous reads and writes,
  bounded output and final-output draining.
- `src/Passenger.Terminal/TerminalView.cs` and `TerminalScreen.cs`: input, selection, scrolling, Unicode,
  resizing and program-requested terminal modes. The Rust emulator replaces the screen implementation.
- `src/Passenger.App/MainWindow.Terminal.cs`, `ToolWindows/TerminalPanel.cs` and `Run/TerminalRun.cs`: dock,
  tabs, IDE shortcuts and interactive runs.
- `src/Passenger.App/MainWindow.Agents.cs::RunAgent`: an explicitly supplied command in an Agent tab.
- `tests/Passenger.Tests/TerminalTests.cs`: observable behavior and responsiveness checks.

Library API references checked on 2026-10-25:
[alacritty_terminal 0.26.0](https://docs.rs/alacritty_terminal/0.26.0/alacritty_terminal/) and
[portable-pty 0.9.0](https://docs.rs/portable-pty/0.9.0/portable_pty/).
Confirm licences and Rust/platform compatibility before adding them, and add notices with the dependencies.

## What the person gets

- A Terminal tool window at Bottom Left, reachable from the menu, Find Action and Rider's platform keymap.
  Opening it starts the first shell lazily, in the workspace folder, or the home folder on Welcome.
  Toggling from the focused terminal hides it and returns to the editor.
- Local, Local (2), and subsequent tabs, with New Session, switching and Close Session. Hiding or docking
  keeps sessions alive; closing a shell tab ends its process tree. A shell's own exit closes its tab;
  the final shell tab closes the tool window. Workspace changes and window close end every session.
- The user's login shell on Unix and PowerShell on Windows, with a UTF-8 environment, `TERM=xterm-256color`,
  `COLORTERM=truecolor` and `TERM_PROGRAM=RSharp`. Failed starts remain visible with an explanation.
- Terminal emulation including alternate screen, cursor positioning, scroll regions, ANSI/indexed/true
  colours, Unicode and wide characters, scrollback, resize/reflow and replies to terminal queries.
  Paint only the viewport; never turn scrollback into editor documents or shape it all per frame.
- Text input, terminal navigation/function/control keys, application cursor mode, mouse reporting when
  requested, selection/copy, Select All, paste, scrolling and Clear Buffer. Modifier-assisted selection
  remains available when a full-screen program takes mouse input. Pasted controls cannot escape a
  bracketed paste; large pastes use a writer thread and never block gpui.
- Tool-window and frame shortcuts remain with the IDE; the terminal receives other keys before editor
  bindings. On macOS Cmd shortcuts remain with the IDE except Cmd+K for Clear Buffer. In a program tab,
  Run, Rerun and Stop remain IDE actions. Use a terminal key context rather than changing editor behavior.
- URLs and existing console source locations open through the IDE's existing browser/file handlers.
  Plain file paths appear as links with Cmd on macOS or Ctrl elsewhere, resolved against the session's
  working folder. Soft-wrapped output is treated as one logical line when detecting links.
- Solution view's Open In Terminal opens a new shell at the selected folder, or a file's parent.
- Run configurations offer Use external console, persisted with a backward-compatible default of false.
  It launches the resolved program directly in a tab named for the configuration, preserving build-first,
  arguments, environment, working folder, browser launch, Stop and Rerun. Ended program tabs retain their
  output and exit status; rerunning replaces the ended tab of that name. Hot Reload uses the same PTY
  path when this option is on, preserving the existing restart setting and status messages.
- An explicitly supplied `--agent` command runs directly in an Agent tab in the workspace folder and
  retains its output/status. Malformed flags and commands are fatal, as other launch steps are. No agent
  executable is assumed or launched merely by opening Terminal. Window attachment, agent sockets and
  admission remain Phase 12; this tab provides the terminal transport only.

## Boundaries and process ownership

Add a small `rsharp-terminal` crate without gpui for the emulator/session/input logic. The app owns its
gpui terminal view and panel, and `window/terminal.rs` connects them to the dock, actions and Run.
Reuse existing fonts, theme tokens, tool-window controls, console links and configuration storage.

All PTY spawning belongs inside `rsharp_core::process`, under the same Lifeline contract as `spawn`.
The terminal must not call `SlavePty::spawn_command` outside that boundary. Extend the process module with
a PTY spawn path which owns the child, its wait/kill operations and its crash-cleanup guard. Unix PTY
children need a controlling terminal/session, so do not blindly apply the ordinary `process_group(0)`
setup. Windows PTY children need the same kill-on-close job ownership as ordinary children. If the
library cannot expose the ownership needed, resolve that adapter first rather than bypassing Lifeline.
Verify both foreground jobs and background descendants: an interactive shell changes process groups,
so killing only the shell's original group does not satisfy the contract.

PTY reads, parsing and writes run off the UI thread. Output notifications coalesce; parsing releases its
screen lock in bounded batches so painting, resize and input can acquire it promptly. Pending output and
scrollback have explicit bounds. Pending input has an explicit byte limit and reports refusal rather
than blocking or silently truncating a paste. Shutdown wakes queued work and releases blocked PTY I/O.
Do not start worker threads or sessions for an unopened Terminal.

Drain the process's final output before reporting its exit, with a bounded wait when descendants keep the
PTY open. Publish exit once, including a command that exits before startup subscriptions finish. A failed
start, closed tab or workspace switch must not leave a child, waiter or writer running indefinitely.

Tests and screenshots get shell history under their own state folder, including zsh, bash and
PowerShell/PSReadLine. A test uses a known plain shell and controlled startup environment rather than the
person's shell configuration. Production shells retain the person's normal configuration and history.

## Verification

- Core/process integration tests: a real PTY, reported size and resize, UTF-8 input/output, quick nonzero
  exit with final output, failed startup, and cleanup of foreground/background descendants after Stop,
  tab close, workspace close and abrupt IDE death.
- Emulator/input tests: alternate-screen restoration, modes and query replies, wide/combining text,
  resize/reflow, safe bracketed paste and platform key encodings. Exercise the integration rather than
  duplicating the emulator's entire upstream test suite.
- Window tests through `support::Run`: lazy startup, cwd, tabs, focus and IDE shortcuts, copy/paste,
  scroll/selection stability, links and file navigation, Open In Terminal, isolated history, program
  configuration persistence, interactive .NET input, Stop/Rerun and Agent command status.
- Flood output with at least 10 MB while a window test types in the editor and interrupts the terminal;
  assert bounded retained output and completion. A blocked large paste must also leave keys responsive.
- Run the relevant suites, then the workspace's required checks. Capture the real window with
  `just screenshot` and inspect it. Manually exercise vim and htop where installed, stating unavailable
  checks explicitly. Verify Windows/ConPTY and Linux on those platforms or CI before claiming them tested.
- Record the existing benchmark before and after when the local capture environment permits it, and
  explain any material change to start, key latency or memory. An unopened terminal should have no PTY
  or worker cost.

## Implementation order

1. Prove PTY process ownership and crash/descendant cleanup; add dependencies and notices.
2. Implement the non-UI session, bounded I/O, emulator integration and input encoding.
3. Add the gpui view, dock integration, tabs, focus/shortcuts, selection and links.
4. Connect Open In Terminal, run configurations/Hot Reload and the explicit Agent command.
5. Complete acceptance tests, real-window inspection and available platform/performance verification;
   update ROADMAP.md and repository guidance to describe what actually shipped.

## Exclusions

Debugger DAP runInTerminal belongs to Phase 8. Agent sockets, MCP attachment/admission and the complete agent
workflow belong to Phase 12. Shell sessions and running processes are not restored across an IDE restart.
No shell customization UI, remote terminal, terminal splits or new font/icon system is introduced by this phase.
