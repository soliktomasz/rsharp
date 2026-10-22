# Product

<!-- impeccable:product-schema 1 -->

## Platform

desktop (gpui; macOS first, then Windows and Linux)

## Stack

Rust 2024 workspace; UI in Zed's gpui (crates.io 0.2.2, runtime-compiled Metal shaders). The C#
intelligence is Roslyn's own language server (`roslyn-language-server`) over LSP, and MSBuild via `dotnet`.

## Users

.NET developers who use, or would use, JetBrains Rider: people writing C# and modern .NET (CoreCLR, no
.NET Framework, Mono or Unity) on macOS, Windows and Linux, often with several solutions open at once.

## Product Purpose

RSharp is a Rider-inspired IDE for C# and .NET, written in Rust. It is the sibling of Passenger (the
same product written in C# on Avalonia) and takes Passenger's behaviour as its reference. Success: a
Rider user opens a solution and works without missing Rider's workflow, keymaps or answers.

## Positioning

Native, GPU-drawn and free of any runtime in the IDE process: it starts instantly, never pauses to
collect garbage while typing, and stays small in memory even with several projects and their language
servers running. Open source, with no component under a paid or editor-restricted licence.

## Operating Context

Long sessions in large solutions; keyboard-first, Rider's keymaps (Visual Studio on Windows, Visual Studio
OSX on macOS); builds, runs, tests and Git alongside the editor; often several windows, one per solution.

## Capabilities and Constraints

- Every program the IDE starts goes through one child-process module and dies with the IDE.
- Never Microsoft's vsdbg or anything from C# Dev Kit, and no proprietary UI components.
- Every dependency carries an open-source licence and an entry in THIRD_PARTY_NOTICES.md.
- Undecided: the debugger (Passenger's own is C#; RSharp needs a DAP adapter that is not vsdbg), the
  profiler, the XAML tooling, and whether RSharp and Passenger share state on disk.

## Brand Commitments

Name: RSharp. Its look and behaviour follow Rider wherever design is in question, as Passenger's do.

## Evidence on Hand

None yet: no benchmarks, users or screenshots. Never claim speed or memory numbers until measured.

## Product Principles

1. A key costs what it changed, not the size of the file or solution.
2. Rider is the reference for behaviour; differences are bugs unless chosen.
3. Memory is a feature: each extra open project should cost only what it must.
4. Nothing the IDE starts outlives it.
