# Third-party notices

RSharp's own code is MIT (LICENSE). What it carries or links from others is listed here, added in the same
change that brings it in.

## Crates RSharp depends on directly

| Crate | Licence | Copyright |
|---|---|---|
| `alacritty_terminal` | Apache-2.0 | The Alacritty Project Contributors |
| `portable-pty` | MIT | Wez Furlong |
| `gpui` | Apache-2.0 | Zed Industries, Inc. |
| `async_zip` | MIT | Harry (Majored) and contributors |
| `serde_json_lenient` | MIT OR Apache-2.0 | Google LLC; Erick Tryzelaar, David Tolnay and contributors |
| `encoding_rs` | (Apache-2.0 OR MIT) AND BSD-3-Clause | Mozilla Foundation, Henri Sivonen; its data from the WHATWG Encoding Standard (BSD-3-Clause) |
| `futures` | MIT OR Apache-2.0 | The futures-rs authors |
| `imara-diff` | Apache-2.0 | Pascal Kuthe |
| `libc` | MIT OR Apache-2.0 | The Rust Project Developers |
| `log` | MIT OR Apache-2.0 | The Rust Project Developers |
| `notify` | CC0-1.0 | Félix Saparelli, Daniel Faust, Aron Heinecke and contributors |
| `quick-xml` | MIT | Johann Tuffe, Mingun |
| `regex` | MIT OR Apache-2.0 | The Rust Project Developers |
| `ropey` | MIT | Nathan Vegdahl |
| `serde` | MIT OR Apache-2.0 | Erick Tryzelaar, David Tolnay |
| `serde_json` | MIT OR Apache-2.0 | Erick Tryzelaar, David Tolnay |
| `trash` | MIT | Artur Kovacs and contributors |
| `tree-sitter` | MIT | Max Brunsfeld |
| `tree-sitter-c-sharp` | MIT | Max Brunsfeld, Damien Guard and contributors |
| `ureq` | MIT OR Apache-2.0 | Martin Algesten and contributors |

`ureq` brings rustls-platform-verifier (MIT OR Apache-2.0), webpki-roots (CDLA-Permissive-2.0, Mozilla's root
certificates as data), ureq-proto, utf8-zero and base64 (MIT OR Apache-2.0); on macOS security-framework (MIT OR
Apache-2.0), on Linux rustls-native-certs and openssl-probe (MIT OR Apache-2.0). Its rustls and ring are gpui's.

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
  Build's `rerun.svg`, `stop.svg` and `tw-build.svg` use `expui/run/rerun_dark.svg`,
  `expui/run/stop_dark.svg` and `expui/toolwindows/build_dark.svg`; `clear.svg` is its `actions/gc_dark.svg`.
  Run's `run.svg` and `tw-run.svg` use `expui/run/run_dark.svg` and `expui/toolwindows/run_dark.svg`.
  Terminal's `tw-terminal.svg` is IntelliJ Community's Apache-2.0 `expui/toolwindow/terminal_dark.svg`, from the
  bundled Terminal platform plugin's `plugins/terminal/lib/terminal.jar`; New Session's `add.svg` is
  `expui/general/add_dark.svg` from `lib/intellij.platform.ide.jar`.

The colours in `crates/rsharp-app/src/theme.rs` are values read from Rider's Islands Dark and Rider Dark themes.

## Carried data

- **MSBuild's schemas** (`crates/rsharp-core/assets/msbuild/Microsoft.Build.Core.xsd` and
  `Microsoft.Build.CommonTypes.xsd`): MIT, Copyright (c) .NET Foundation and Contributors, from `src/MSBuild/MSBuild/` of
  https://github.com/dotnet/msbuild, unchanged, as Passenger carries them (its `src/Passenger.App/MSBuild/Schemas/`).
  Read for the code help in MSBuild files, which the .NET SDK carries no schemas for.

## Built from the Passenger submodule

`vendor/passenger` is Passenger (MIT, Copyright (c) 2026 Ted L, https://github.com/Zaneris/passenger), pinned as a git
submodule. RSharp's build (`crates/rsharp-lsp/build.rs`) publishes two of its projects and runs them, unchanged:
Passenger.Host (MSBuild's design-time builds in a process of their own) and Passenger.Roslyn.Extension (Roslyn's
server's project loader and refactorings with options). The Host carries Microsoft.Build.Locator (MIT, .NET
Foundation); the extension is compiled against Roslyn's server with Krafs.Publicizer (MIT, Krafs), which carries
nothing into the output.

## Ported from Passenger

The project-template family rules, pack metadata and template-creation behavior in
`crates/rsharp-core/src/templates/` are ported from Passenger's `src/Passenger.App/Templates/`, and the NuGet
sources, feeds and window in `crates/rsharp-core/src/nuget/` and `crates/rsharp-app/src/nuget_view.rs` from its
`src/Passenger.App/Packages/` and `ToolWindows/NuGetView.cs`, and the MSBuild code help in `crates/rsharp-core/src/msbuild/`
from its `src/Passenger.App/MSBuild/` (MIT, Copyright (c) 2026 Ted L,
https://github.com/Zaneris/passenger).

## Run, not carried

Roslyn's language server (`roslyn-language-server`, MIT, .NET Foundation): the version Passenger pins, which the
extension's build downloads into the NuGet cache and RSharp runs from there; without the submodule or dotnet at build
time, the person's own install of the dotnet tool.
