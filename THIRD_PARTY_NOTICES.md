# Third-party notices

RSharp's own code is MIT (LICENSE). What it carries or links from others is listed here, added in the same
change that brings it in.

## Crates RSharp depends on directly

| Crate | Licence | Copyright |
|---|---|---|
| `gpui` | Apache-2.0 | Zed Industries, Inc. |
| `encoding_rs` | (Apache-2.0 OR MIT) AND BSD-3-Clause | Mozilla Foundation, Henri Sivonen; its data from the WHATWG Encoding Standard (BSD-3-Clause) |
| `futures` | MIT OR Apache-2.0 | The futures-rs authors |
| `imara-diff` | Apache-2.0 | Pascal Kuthe |
| `libc` | MIT OR Apache-2.0 | The Rust Project Developers |
| `log` | MIT OR Apache-2.0 | The Rust Project Developers |
| `regex` | MIT OR Apache-2.0 | The Rust Project Developers |
| `ropey` | MIT | Nathan Vegdahl |
| `serde` | MIT OR Apache-2.0 | Erick Tryzelaar, David Tolnay |
| `serde_json` | MIT OR Apache-2.0 | Erick Tryzelaar, David Tolnay |
| `tree-sitter` | MIT | Max Brunsfeld |
| `tree-sitter-c-sharp` | MIT | Max Brunsfeld, Damien Guard and contributors |

gpui brings about 550 crates of its own. Their notices are not yet written out: a release must generate them
(for example with `cargo about`) and fail on any licence that is not open source.

## Carried assets

- **Inter** (`crates/rsharp-app/assets/fonts/Inter-*.otf`): SIL Open Font License 1.1, Copyright 2016 The Inter
  Project Authors (https://github.com/rsms/inter). Copied from the JetBrains Runtime inside Rider.
- **JetBrains Mono** (`crates/rsharp-app/assets/fonts/JetBrainsMono-*.ttf`): SIL Open Font License 1.1, Copyright
  2020 The JetBrains Mono Project Authors (https://github.com/JetBrains/JetBrainsMono).
- **IntelliJ Platform icons** (`crates/rsharp-app/assets/icons/*.svg`): Apache-2.0, Copyright 2000-2024 JetBrains
  s.r.o. and contributors (https://github.com/JetBrains/intellij-community), the New UI ("expui") icons, taken from
  Rider's platform jar (`intellij.platform.ide.jar`), never from Rider's own (`intellij.rider.*`) jars.

The colours in `crates/rsharp-app/src/theme.rs` are values read from Rider's Islands Dark and Rider Dark themes.

## Built from the Passenger submodule

`vendor/passenger` is Passenger (MIT, Copyright (c) 2026 Ted L, https://github.com/Zaneris/passenger), pinned as a git
submodule. RSharp's build (`crates/rsharp-lsp/build.rs`) publishes two of its projects and runs them, unchanged:
Passenger.Host (MSBuild's design-time builds in a process of their own) and Passenger.Roslyn.Extension (Roslyn's
server's project loader and refactorings with options). The Host carries Microsoft.Build.Locator (MIT, .NET
Foundation); the extension is compiled against Roslyn's server with Krafs.Publicizer (MIT, Krafs), which carries
nothing into the output.

## Run, not carried

Roslyn's language server (`roslyn-language-server`, MIT, .NET Foundation): the version Passenger pins, which the
extension's build downloads into the NuGet cache and RSharp runs from there; without the submodule or dotnet at build
time, the person's own install of the dotnet tool.
