# NuGet (Phase 4, second part)

Follows `2026-10-24-nuget-design.md`. Passenger's `Packages/` (`PackageSources`, `FeedClient`, `V2Feed`,
`CredentialProviders`, `Vulnerabilities`, `NuGetConfigEditor`, `FeedDialog`) and `ToolWindows/NuGetSourcesView.cs` are
the reference.

## Goals

### Credentials and feed settings (`nuget::sources`, `nuget::feed`)

- `packageSourceCredentials` read as NuGet reads it: an element per feed, its name XML-encoded (`My Feed` is
  `My_x0020_Feed`), with `Username` and `ClearTextPassword`, or `Password`. On Windows, `Password` is DPAPI-encrypted
  for the user, with NuGet's entropy (`"NuGet"`) and base64. Elsewhere an encrypted password can't be read, so it counts
  as none. A closer file's credentials win. `NuGetPackageSourceCredentials_<name>` (`Username=…;Password=…`) comes
  first, as NuGet takes it.
- A feed's `allowInsecureConnections` and `disableTLSCertificateValidation` attributes.
- `auditSources`, read like `packageSources`.
- Credentials are kept in memory and printed nowhere: their `Debug` names the user, never the password.
- HTTP requests: a feed's credentials go only to its own host. An `http:` URL is refused unless the feed allows
  insecure connections, as restore refuses it. A 401 with credentials from the config is reported as a refusal of
  those credentials. With none, the credential providers are asked once, and the request is retried with what they
  give. Turning off the certificate check uses an agent of its own.

### v2 feeds (`nuget::feed`)

A URL is a v3 feed when its `protocolVersion` is 3 or its path ends in `.json`, as NuGet decides; any other URL is v2.
Search uses OData's `Search()` (`IsLatestVersion`, or `IsAbsoluteLatestVersion` with prerelease, top 30) and versions
use `FindPackagesById()`, following `next` links for at most 50 pages. A version that isn't listed, or was published in
1900, is left out. Atom is read with `quick-xml`.

### Credential providers (`nuget::providers`)

- Found as NuGet finds them: the paths in `NUGET_NETCORE_PLUGIN_PATHS` (else `NUGET_PLUGIN_PATHS`) and nothing else
  when either is set. Otherwise `~/.nuget/plugins/netcore/<name>/<name>.dll`, run with dotnet, and the
  `nuget-plugin-*` programs on PATH (on Windows only `.exe`).
- NuGet's plugin protocol, a JSON message per line: Handshake (2.0.0), Initialize, GetOperationClaims, then
  GetAuthenticationCredentials (non-interactive, no dialog). The provider's own Handshake, Log and SetLogLevel requests
  are answered as NuGet answers them, and anything else gets NotFound. Each step has 20 seconds and the answer a minute.
- Run through `process::spawn`. Credentials are kept for the session by feed URL; a retry asks again past them.
  A provider that gives nothing is logged by name only.

### Vulnerabilities (`nuget::vulnerabilities`)

- A v3 feed's `VulnerabilityInfo/6.7.0` resource: an index of pages (a base and the updates since), each a map of
  lower-case ids to advisories (severity 0 to 3, a URL, and the versions affected as a NuGet range).
- It's read from the folder's audit sources, or else from its enabled sources, as NuGetAudit does. A feed that fails
  is listed by name, never thrown.
- `Advisory::affects` reads NuGet's range notation; `fixed_version` gives the lowest offered version above the
  installed one that no advisory affects.
- The window reads advisories once its sources are read and a project has a package. An installed version with an
  advisory gets a warning mark on its row. The details list each advisory (severity, then its link, opened in the
  browser), and Update defaults to the version that fixes them.

### Editing nuget.config (`nuget::config_edit`, the Sources tab)

- Add, Update (a renamed feed's disabling, credentials and mapping follow it), Remove (with what the file says of the
  feed elsewhere), and Enable or Disable, written as `dotnet nuget` writes them with `--configfile`. Only the elements
  changed are written, indented like their neighbours; every other byte stays (declaration, comments, quotes, BOM,
  line ends). A missing or empty file starts as NuGet's empty configuration.
- A password is written as `ClearTextPassword`, except on Windows, where it's encrypted as `Password`. An empty
  password keeps the one the file has; no user name takes the credentials out.
- The Sources tab lists the files in effect and the feeds of the chosen one, each with its enabled box, a lock when the
  file holds credentials, Edit and Delete. Add Feed and Edit open an inline form: Name, URL, User, Password (masked),
  Enabled, Allow insecure connections. The machine's file is read-only. Enabling a feed removes its disabling from
  every file that has one, as `dotnet nuget enable source` does.
- A write saves an open copy first, keeps the old bytes in Local History, then reads the sources again. A file that
  won't take the change says why under the tab.

## Tests

- Offline only, as before. Credentials: fixtures for encoded names, clear text, the environment variable and a closer
  file winning, plus a `Debug` that never shows the password. The loopback feed gains Basic authentication and a 401
  path; a stand-in provider is the test binary itself, speaking the protocol.
- v2: a loopback OData feed for search, paged versions and unlisted versions.
- Vulnerabilities: a loopback resource with a base page and an update; range cases; `fixed_version`.
- Editor: byte-for-byte expected files for each operation on a file with comments, CRLF and a BOM.
- DPAPI round-trip test on Windows only.
- Window: the Sources tab adds, disables, edits and deletes a feed in the solution's file; a vulnerable installed
  package shows its mark and Update offers the fix.
