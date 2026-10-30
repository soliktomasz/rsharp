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
| `image` | MIT OR Apache-2.0 | The image-rs Developers |
| `libc` | MIT OR Apache-2.0 | The Rust Project Developers |
| `log` | MIT OR Apache-2.0 | The Rust Project Developers |
| `notify` | CC0-1.0 | Félix Saparelli, Daniel Faust, Aron Heinecke and contributors |
| `quick-xml` | MIT | Johann Tuffe, Mingun |
| `regex` | MIT OR Apache-2.0 | The Rust Project Developers |
| `ropey` | MIT | Nathan Vegdahl |
| `serde` | MIT OR Apache-2.0 | Erick Tryzelaar, David Tolnay |
| `serde_json` | MIT OR Apache-2.0 | Erick Tryzelaar, David Tolnay |
| `symbolic-ppdb` | MIT | Sentry (https://github.com/getsentry/symbolic) |
| `trash` | MIT | Artur Kovacs and contributors |
| `tree-sitter` | MIT | Max Brunsfeld |
| `tree-sitter-c-sharp` | MIT | Max Brunsfeld, Damien Guard and contributors |
| `tree-sitter-css` | MIT | Max Brunsfeld, Amaan Qureshi |
| `tree-sitter-html` | MIT | Max Brunsfeld, Amaan Qureshi |
| `tree-sitter-javascript` | MIT | Max Brunsfeld, Amaan Qureshi |
| `tree-sitter-json` | MIT | Max Brunsfeld, Amaan Qureshi |
| `tree-sitter-md` | MIT | Matthias Deiml |
| `tree-sitter-typescript` | MIT | Max Brunsfeld, Amaan Qureshi |
| `tree-sitter-xml` | MIT | ObserverOfTime, Amaan Qureshi |
| `tree-sitter-yaml` | MIT | Ika, Amaan Qureshi (tree-sitter-grammars contributors) |
| `ureq` | MIT OR Apache-2.0 | Martin Algesten and contributors |

`symbolic-ppdb` reads the sources the debugger's decompiled PDBs embed: it brings symbolic-common (MIT) and watto
(Apache-2.0), and uses flate2 and thiserror (MIT OR Apache-2.0), indexmap and uuid (Apache-2.0 OR MIT), and
serde/serde_json, which the workspace already carries.

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
submodule. RSharp's build publishes three of its projects and runs them, unchanged:
`crates/rsharp-lsp/build.rs` publishes Passenger.Host (MSBuild's design-time builds in a process of their own) and
Passenger.Roslyn.Extension (Roslyn's server's project loader and refactorings with options), and
`crates/rsharp-dap/build.rs` publishes Passenger.DebugAdapter (Passenger's own ICorDebug engine behind the Debug
Adapter Protocol, in a process of its own, with ILSpy's decompiler for code without source). The Host carries
Microsoft.Build.Locator (MIT, .NET Foundation); the extension is compiled against Roslyn's server with
Krafs.Publicizer (MIT, Krafs), which carries nothing into the output. RSharp's own XAML host
(`crates/rsharp-lsp/xaml-host/`, published by the same build.rs) is built on Passenger.Xaml and Passenger.Text,
unchanged, and carries them with Microsoft.CodeAnalysis.CSharp (MIT, .NET Foundation), Passenger.Xaml's C# parser,
and Avalonia.Remote.Protocol (MIT, AvaloniaUI OÜ), which the XAML Preview speaks to the project's own Avalonia
previewer.

### Inside `sidecars/web/`

The language servers of JavaScript, TypeScript, HTML, CSS and JSON files, npm packages of plain JavaScript run on the
machine's Node.js. `crates/rsharp-lsp/build.rs` puts in place the ones Passenger's
`src/Passenger.Lsp/WebServers/package-lock.json` pins, through `npm ci`, which checks each tarball against the sha512 it
pins; nothing is downloaded as the app runs. They are carried as their tarballs hold them. Each package is named by its
npm name, with the licence its package.json states and the copyright line its licence file carries (else its author),
as Passenger's notices list them.

| Package | In RSharp | Licence | Copyright |
|---|---|---|---|
| `typescript-language-server` | the TypeScript and JavaScript language server, over TypeScript's own tsserver (a project's own `node_modules/typescript` when it has one) | Apache-2.0 | Copyright (c) 2015 - present Microsoft Corporation |
| `typescript` (6.0.3, and 4.9.5 under vscode-langservers-extracted) | TypeScript's compiler and tsserver for a project without its own; the HTML server's for a page's scripts | Apache-2.0 | no copyright line; its author: Microsoft Corp. |
| `vscode-langservers-extracted` | VS Code's HTML, CSS and JSON language servers, as extracted from VS Code (its ESLint and Markdown servers, carried with them, are not run yet) | MIT | Copyright (c) 2020 hrsh7th |
| `vscode-css-languageservice`, `vscode-html-languageservice`, `vscode-json-languageservice`, `jsonc-parser`, `request-light`, `vscode-uri` | the servers' language services | MIT | Copyright (c) Microsoft |
| `vscode-languageserver`, `vscode-languageserver-protocol`, `vscode-languageserver-types`, `vscode-languageserver-textdocument`, `vscode-jsonrpc`, `vscode-nls`, `vscode-markdown-languageservice` | the servers' side of the Language Server Protocol | MIT | Copyright (c) Microsoft Corporation |
| `@vscode/l10n` | the servers' messages | MIT | no copyright line; its author: Microsoft Corporation |
| `core-js` | the extracted servers' JavaScript library | MIT | Copyright (c) 2013–2025 Denis Pushkarev (zloirock.ru) |
| `regenerator-runtime` | the extracted servers' generators | MIT | Copyright (c) 2014-present, Facebook, Inc. |
| `semver` | version ranges | ISC | Copyright (c) Isaac Z. Schlueter and Contributors |
| `node-html-parser` | the Markdown service's HTML | MIT | Copyright 2019 Tao Qiufeng |
| `css-select`, `css-what`, `domelementtype`, `domhandler`, `domutils`, `entities`, `nth-check` | node-html-parser's | BSD-2-Clause | Copyright (c) Felix Böhm |
| `boolbase` | css-select's | ISC | no copyright line; its author: Felix Boehm |
| `dom-serializer` | domutils' | MIT | Copyright (c) 2014 The cheeriojs contributors |
| `he` | node-html-parser's | MIT | Copyright Mathias Bynens |
| `picomatch` | the Markdown service's globs | MIT | Copyright (c) 2017-present, Jon Schlinkert. |

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
