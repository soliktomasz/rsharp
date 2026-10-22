# Third-party notices

RSharp's own code is MIT (LICENSE). What it carries or links from others is listed here, added in the same
change that brings it in.

## Crates RSharp depends on directly

| Crate | Licence | Copyright |
|---|---|---|
| `gpui` | Apache-2.0 | Zed Industries, Inc. |
| `futures` | MIT OR Apache-2.0 | The futures-rs authors |
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

## Run, not carried

Roslyn's language server (`roslyn-language-server`, MIT, .NET Foundation) is the person's own install of the
dotnet tool.
