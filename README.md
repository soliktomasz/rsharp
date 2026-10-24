# RSharp

A Rider-inspired IDE for C# and modern .NET, written in Rust on Zed's [gpui](https://crates.io/crates/gpui).

RSharp is the sibling of [Passenger](https://github.com/Zaneris/passenger), the same product written in C# on
Avalonia, and takes Passenger's behaviour as its reference, and Rider's after it. Its edge is being native and
GPU-drawn with no runtime in the IDE process: a fast start, no garbage-collection pauses while typing, and little
memory with several solutions and their language servers open. It is for .NET developers who use, or would use,
Rider: C# on CoreCLR (no .NET Framework, Mono or Unity) on macOS, Windows and Linux.

The C# intelligence is Roslyn's own language server, and MSBuild runs through `dotnet`, both driven through
Passenger's sidecar processes. Nothing comes from Microsoft's vsdbg or C# Dev Kit, and every dependency is
open source ([THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md)).

## Status

Early, and built phase by phase towards Passenger's features ([ROADMAP.md](ROADMAP.md)). In place: the window as
Rider's Islands with Rider's keymaps, the editor (tree-sitter colours, folding, multiple carets, find and replace,
split tabs), Roslyn's diagnostics and code help, the Solution view with Rider's edits to solutions and projects,
New Solution / New Project, NuGet, Local History, code help in MSBuild files, and an MCP server for agents. Next:
build, run and test, then the terminal, Git, the debugger, XAML and the profiler.

## Building

Needs Rust (2024 edition) and the .NET SDK. [`just`](https://github.com/casey/just) is optional: each recipe is
one cargo command.

```sh
git submodule update --init   # vendor/passenger, whose sidecars the build publishes with dotnet
just build                    # cargo build; warnings are errors
just run [PATH]               # the IDE on a folder, solution or project
just test [filter]            # every test
```

Without `dotnet` or the submodule at build time, the build warns and RSharp runs your installed
`roslyn-language-server` tool instead.

`just run` also takes `--file F`, `--caret L:C`, and `--key KEYS` / `--type TEXT`, which go through the window's
real input once it opens: `just run . --key cmd-shift-a --type clo` shows Find Action with "clo" typed.

## RSharp for agents

RSharp runs as an MCP server on standard input and output, with Roslyn behind its tools (`workspace`,
`diagnostics`, `find_symbol`, `outline`, `usages`, `rename`, `edit`):

```sh
claude mcp add rsharp -- /path/to/rsharp --mcp /path/to/folder
```

## Layout

| Crate | What it is |
|---|---|
| `rsharp-core` | processes, paths, solutions and projects, NuGet, templates, .editorconfig, state; no UI |
| `rsharp-text` | the document, selections and every editing command; no UI |
| `rsharp-languages` | tree-sitter parsing, colours, folds and brackets |
| `rsharp-lsp` | the LSP client, Roslyn and Passenger's MSBuild Host |
| `rsharp-mcp` | the MCP server |
| `rsharp-app` | the gpui application |

[CLAUDE.md](CLAUDE.md) maps each area in detail, with the project's rules; [PRODUCT.md](PRODUCT.md) says who it is
for and why, and [DESIGN.md](DESIGN.md) how it looks.

## Licence

MIT, © 2026 Tomasz Solik ([LICENSE](LICENSE)). Passenger's sidecars are MIT, © 2026 Ted L.
