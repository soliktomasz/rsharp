//! Where packages are found: a v3 feed (nuget.org's protocol), a v2 feed (OData, as older servers speak it) or a
//! folder of .nupkg files; and All Feeds, every enabled feed asked at once, a package found in several listed once.

use super::secret::Credentials;
use super::sources::Feed;
use super::version;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

/// Where a feed that answers 401 without credentials of its own asks for some: its service index, and whether a
/// first answer was already refused. NuGet's credential providers, unless a test gives its own.
pub type AskCredentials = Arc<dyn Fn(&str, bool) -> Option<Credentials> + Send + Sync>;

/// A package as a search lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Package {
    pub id: String,
    /// Its newest version on the feeds that have it.
    pub version: String,
    pub description: String,
    pub authors: String,
    pub downloads: u64,
    pub feeds: Vec<String>,
}

enum Kind {
    V3 { client: Client, services: OnceLock<Result<Services, String>> },
    V2 { client: Client },
    Folder(PathBuf),
}

#[derive(Clone, Debug, Default)]
struct Services {
    search: Option<String>,
    registrations: Option<String>,
    flat: Option<String>,
    vulnerabilities: Option<String>,
}

pub struct PackageFeed {
    pub name: String,
    kind: Kind,
}

impl PackageFeed {
    /// A configured source: a folder (a path, or a file: URL), a v3 feed (protocolVersion 3, or a URL ending in
    /// .json, as NuGet decides) or a v2 feed; its credentials asked of NuGet's providers when it needs some.
    pub fn for_source(source: &Feed) -> PackageFeed {
        Self::with_credentials(source, Arc::new(super::providers::credentials))
    }

    pub fn with_credentials(source: &Feed, ask: AskCredentials) -> PackageFeed {
        let kind = if source.is_http() {
            let client = Client::new(source, ask);
            let path = source.source.split(['?', '#']).next().unwrap_or("").to_ascii_lowercase();
            if source.protocol_version == Some(3) || path.ends_with(".json") {
                Kind::V3 { client, services: OnceLock::new() }
            } else {
                Kind::V2 { client }
            }
        } else {
            Kind::Folder(source.source.clone().into())
        };
        PackageFeed { name: source.name.clone(), kind }
    }

    /// The address of the feed's known vulnerabilities, a v3 feed's VulnerabilityInfo resource; none for any other.
    pub(super) fn vulnerability_index(&self) -> Result<Option<(String, &Client)>, String> {
        match &self.kind {
            Kind::V3 { client, .. } => Ok(self.services()?.vulnerabilities.map(|v| (v, client))),
            _ => Ok(None),
        }
    }

    /// The packages a query finds, the most used first.
    pub fn search(&self, query: &str, prerelease: bool) -> Result<Vec<Package>, String> {
        match &self.kind {
            Kind::V2 { client } => {
                let filter = if prerelease { "IsAbsoluteLatestVersion" } else { "IsLatestVersion" };
                let url = format!(
                    "{}/Search()?$filter={filter}&searchTerm={}&targetFramework=''&includePrerelease={prerelease}&$skip=0&$top=30&semVerLevel=2.0.0",
                    client.root(),
                    escape(&literal(query.trim()))
                );
                Ok(atom_entries(&client.text(&url)?.unwrap_or_default())?
                    .into_iter()
                    .map(|e| Package {
                        id: e.id,
                        version: e.version,
                        description: e.description,
                        authors: e.authors,
                        downloads: e.downloads,
                        feeds: vec![self.name.clone()],
                    })
                    .collect())
            }
            Kind::V3 { client, .. } => {
                let services = self.services()?;
                let search = services.search.as_ref().ok_or_else(|| format!("{} offers no search", self.name))?;
                let url =
                    format!("{search}?q={}&take=30&prerelease={prerelease}&semVerLevel=2.0.0", escape(query.trim()));
                let found = client.json(&url)?.unwrap_or(Value::Null);
                Ok(found["data"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|hit| {
                        Some(Package {
                            id: hit["id"].as_str()?.into(),
                            version: hit["version"].as_str()?.into(),
                            description: hit["description"].as_str().unwrap_or("").into(),
                            // A list on nuget.org, a string on some private feeds.
                            authors: match &hit["authors"] {
                                Value::Array(list) => {
                                    list.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")
                                }
                                Value::String(one) => one.clone(),
                                _ => String::new(),
                            },
                            downloads: hit["totalDownloads"].as_u64().unwrap_or(0),
                            feeds: vec![self.name.clone()],
                        })
                    })
                    .collect())
            }
            Kind::Folder(folder) => {
                let wanted = query.trim().to_lowercase();
                let mut newest: Vec<Package> = Vec::new();
                for package in read_folder(folder, &self.name, prerelease)? {
                    if !package.id.to_lowercase().contains(&wanted) {
                        continue;
                    }
                    match newest.iter_mut().find(|p| p.id.eq_ignore_ascii_case(&package.id)) {
                        Some(known) if version::compare(&package.version, &known.version).is_gt() => *known = package,
                        Some(_) => {}
                        None => newest.push(package),
                    }
                }
                Ok(newest)
            }
        }
    }

    /// A package's versions that can be installed, newest first: none its owner unlisted.
    pub fn versions(&self, id: &str, prerelease: bool) -> Result<Vec<String>, String> {
        let mut versions: Vec<String> = match &self.kind {
            Kind::V2 { client } => {
                // More pages than a package's versions fill, so a server that pages forever is not followed forever.
                let mut next =
                    Some(format!("{}/FindPackagesById()?id={}&semVerLevel=2.0.0", client.root(), escape(&literal(id))));
                let mut versions = Vec::new();
                for _ in 0..50 {
                    let Some(url) = next.take() else { break };
                    let text = client.text(&url)?.unwrap_or_default();
                    versions.extend(atom_entries(&text)?.into_iter().filter(|e| e.listed).map(|e| e.version));
                    next = atom_next(&text).map(|href| client.absolute(&href));
                }
                versions
            }
            Kind::V3 { client, .. } => {
                let services = self.services()?;
                let lower = id.to_lowercase();
                if let Some(registrations) = &services.registrations {
                    let Some(registration) = client.json(&format!("{registrations}{lower}/index.json"))? else {
                        return Ok(vec![]);
                    };
                    let mut versions = Vec::new();
                    for page in registration["items"].as_array().into_iter().flatten() {
                        // A package with many versions has its pages by reference, each fetched on its own.
                        let fetched;
                        let page = if page["items"].is_array() {
                            page
                        } else {
                            let url = page["@id"].as_str().ok_or("A registration page has no address")?;
                            fetched = client.json(url)?.unwrap_or(Value::Null);
                            &fetched
                        };
                        for leaf in page["items"].as_array().into_iter().flatten() {
                            let entry = &leaf["catalogEntry"];
                            if entry["listed"].as_bool() != Some(false)
                                && let Some(v) = entry["version"].as_str()
                            {
                                versions.push(v.to_string());
                            }
                        }
                    }
                    versions
                } else if let Some(flat) = &services.flat {
                    client
                        .json(&format!("{flat}{lower}/index.json"))?
                        .map(|v| {
                            v["versions"]
                                .as_array()
                                .into_iter()
                                .flatten()
                                .filter_map(Value::as_str)
                                .map(String::from)
                                .collect()
                        })
                        .unwrap_or_default()
                } else {
                    return Err(format!("{} offers neither registrations nor a flat container", self.name));
                }
            }
            Kind::Folder(folder) => read_folder(folder, &self.name, true)?
                .into_iter()
                .filter(|p| p.id.eq_ignore_ascii_case(id))
                .map(|p| p.version)
                .collect(),
        };
        versions.retain(|v| prerelease || !version::is_prerelease(v));
        sort_newest_first(&mut versions);
        Ok(versions)
    }

    fn services(&self) -> Result<Services, String> {
        let Kind::V3 { client, services } = &self.kind else { return Ok(Services::default()) };
        services
            .get_or_init(|| {
                let index = &client.index;
                let found =
                    client.json(index)?.ok_or_else(|| format!("{} has no service index at {index}", self.name))?;
                let resources = found["resources"].as_array().cloned().unwrap_or_default();
                let find = |types: &[&str]| {
                    types.iter().find_map(|t| {
                        resources.iter().find(|r| r["@type"].as_str() == Some(t)).and_then(|r| r["@id"].as_str())
                    })
                };
                let folder =
                    |url: Option<&str>| url.map(|u| if u.ends_with('/') { u.to_string() } else { format!("{u}/") });
                Ok(Services {
                    search: find(&[
                        "SearchQueryService/3.5.0",
                        "SearchQueryService/3.0.0-rc",
                        "SearchQueryService/3.0.0-beta",
                        "SearchQueryService",
                    ])
                    .map(String::from),
                    // 3.6.0 is the hive with packages of SemVer 2 versions, which the earlier ones leave out.
                    registrations: folder(find(&[
                        "RegistrationsBaseUrl/3.6.0",
                        "RegistrationsBaseUrl/3.4.0",
                        "RegistrationsBaseUrl/3.0.0-rc",
                        "RegistrationsBaseUrl/3.0.0-beta",
                        "RegistrationsBaseUrl",
                    ])),
                    flat: folder(find(&["PackageBaseAddress/3.0.0"])),
                    vulnerabilities: find(&["VulnerabilityInfo/6.7.0"]).map(String::from),
                })
            })
            .clone()
    }
}

/// What a search of several feeds found: the packages, each once with the feeds that have it; the feeds that did
/// not answer, and why; and how many hits the package source mapping kept from a feed.
#[derive(Clone, Debug, Default)]
pub struct Search {
    pub packages: Vec<Package>,
    pub failed: Vec<(String, String)>,
    pub unmapped: usize,
}

/// The feeds the NuGet window searches at once: All Feeds, or the one chosen.
pub struct FeedSet {
    pub feeds: Vec<PackageFeed>,
}

impl FeedSet {
    pub fn search(&self, query: &str, prerelease: bool, allows: &(dyn Fn(&str, &str) -> bool + Sync)) -> Search {
        let answers: Vec<_> = std::thread::scope(|scope| {
            let asked: Vec<_> =
                self.feeds.iter().map(|feed| scope.spawn(move || (feed, feed.search(query, prerelease)))).collect();
            asked.into_iter().map(|a| a.join().expect("a feed's search panicked")).collect()
        });
        let mut found = Search::default();
        for (feed, answer) in answers {
            match answer {
                Err(why) => found.failed.push((feed.name.clone(), why)),
                Ok(hits) => {
                    for hit in hits {
                        if !allows(&feed.name, &hit.id) {
                            found.unmapped += 1;
                            continue;
                        }
                        match found.packages.iter_mut().find(|p| p.id.eq_ignore_ascii_case(&hit.id)) {
                            None => found.packages.push(hit),
                            Some(known) => {
                                let mut feeds = std::mem::take(&mut known.feeds);
                                feeds.extend(hit.feeds.iter().cloned());
                                let downloads = known.downloads.max(hit.downloads);
                                if version::compare(&hit.version, &known.version).is_gt() {
                                    *known = hit;
                                }
                                known.feeds = feeds;
                                known.downloads = downloads;
                            }
                        }
                    }
                }
            }
        }
        if self.feeds.len() > 1 {
            // By how well the id matches before downloads, which a private feed never counts, so its packages do not
            // sink below nuget.org's.
            let wanted = query.trim().to_lowercase();
            let rank = |id: &str| {
                let id = id.to_lowercase();
                if id == wanted {
                    0
                } else if id.starts_with(&wanted) {
                    1
                } else if id.contains(&wanted) {
                    2
                } else {
                    3
                }
            };
            found.packages.sort_by(|a, b| rank(&a.id).cmp(&rank(&b.id)).then(b.downloads.cmp(&a.downloads)));
        }
        found
    }

    /// A package's versions on every feed that may serve it, newest first; an error only when each of them failed.
    pub fn versions(
        &self,
        id: &str,
        prerelease: bool,
        allows: &(dyn Fn(&str, &str) -> bool + Sync),
    ) -> Result<Vec<String>, String> {
        let asked: Vec<_> = self.feeds.iter().filter(|f| allows(&f.name, id)).collect();
        let answers: Vec<_> = std::thread::scope(|scope| {
            let running: Vec<_> = asked.iter().map(|feed| scope.spawn(move || feed.versions(id, prerelease))).collect();
            running.into_iter().map(|a| a.join().expect("a feed's versions panicked")).collect()
        });
        if !answers.is_empty() && answers.iter().all(Result::is_err) {
            return Err(answers.into_iter().find_map(Result::err).unwrap());
        }
        let mut versions: Vec<String> = Vec::new();
        for v in answers.into_iter().flatten().flatten() {
            if !versions.iter().any(|known| version::compare(known, &v).is_eq()) {
                versions.push(v);
            }
        }
        sort_newest_first(&mut versions);
        Ok(versions)
    }
}

fn sort_newest_first(versions: &mut [String]) {
    versions.sort_by(|a, b| version::compare(b, a));
}

/// A URL's query value, every byte but the unreserved ones escaped.
fn escape(value: &str) -> String {
    value
        .bytes()
        .map(|b| match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => (b as char).to_string(),
            _ => format!("%{b:02X}"),
        })
        .collect()
}

/// An OData string literal: in quotes, a quote in it doubled.
fn literal(value: &str) -> String {
    format!("'{}'", value.replace('\'', "''"))
}

fn agent(skip_certificate_check: bool) -> &'static ureq::Agent {
    static CHECKED: OnceLock<ureq::Agent> = OnceLock::new();
    static UNCHECKED: OnceLock<ureq::Agent> = OnceLock::new();
    let make = |verify: bool| -> ureq::Agent {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("RSharp/", env!("CARGO_PKG_VERSION")))
            .tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .disable_verification(!verify)
                    .build(),
            )
            .build()
            .into()
    };
    if skip_certificate_check { UNCHECKED.get_or_init(|| make(false)) } else { CHECKED.get_or_init(|| make(true)) }
}

/// A feed's HTTP: its credentials sent to its own host alone (its nuget.config's, else what a credential provider
/// gives once it answers 401 without any), the certificate check its configuration may turn off, and HTTP refused
/// unless the feed allows insecure connections, as restore refuses it. Credentials are never logged.
pub(super) struct Client {
    name: String,
    index: String,
    credentials: Mutex<Option<Credentials>>,
    configured: bool,
    allow_insecure: bool,
    skip_certificate_check: bool,
    ask: AskCredentials,
}

impl Client {
    fn new(source: &Feed, ask: AskCredentials) -> Client {
        Client {
            name: source.name.clone(),
            index: source.source.clone(),
            configured: source.credentials.is_some(),
            credentials: Mutex::new(source.credentials.clone()),
            allow_insecure: source.allow_insecure,
            skip_certificate_check: source.skip_certificate_check,
            ask,
        }
    }

    /// A v2 feed's root: its URL without the slash after it.
    fn root(&self) -> &str {
        self.index.trim_end_matches('/')
    }

    fn authority(url: &str) -> &str {
        let rest = url.split_once("://").map_or(url, |(_, r)| r);
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        &url[..url.len() - rest.len() + end]
    }

    /// A link the feed gave, made absolute against its own host.
    fn absolute(&self, href: &str) -> String {
        if href.contains("://") {
            href.to_string()
        } else {
            format!("{}/{}", Self::authority(&self.index), href.trim_start_matches('/'))
        }
    }

    fn send(&self, url: &str, with: Option<&Credentials>) -> Result<ureq::http::Response<ureq::Body>, String> {
        let mut request = agent(self.skip_certificate_check).get(url);
        if let Some(credentials) = with {
            request = request.header("Authorization", credentials.basic());
        }
        request.call().map_err(|e| format!("{url}: {e}"))
    }

    /// A GET of one of the feed's URLs, credentials tried as NuGet tries them; none where it has nothing (404).
    pub(super) fn text(&self, url: &str) -> Result<Option<String>, String> {
        if url.to_ascii_lowercase().starts_with("http://") && !self.allow_insecure {
            return Err(format!(
                "{} is reached over HTTP, which NuGet uses only with allowInsecureConnections set for it",
                self.name
            ));
        }
        let own = Self::authority(url).eq_ignore_ascii_case(Self::authority(&self.index));
        let sent = if own { self.credentials.lock().unwrap().clone() } else { None };
        let mut response = self.send(url, sent.as_ref())?;
        if response.status().as_u16() == 401 && own {
            if self.configured {
                return Err(format!("{} refused the credentials its nuget.config gives (401 Unauthorized)", self.name));
            }
            let known = self.credentials.lock().unwrap().clone();
            let retry = match known.filter(|k| Some(k) != sent.as_ref()) {
                Some(newer) => newer,
                None => (self.ask)(&self.index, sent.is_some()).ok_or_else(|| {
                    format!(
                        "{} needs credentials (401 Unauthorized): none in its nuget.config, and no credential provider gave any",
                        self.name
                    )
                })?,
            };
            *self.credentials.lock().unwrap() = Some(retry.clone());
            response = self.send(url, Some(&retry))?;
            if response.status().as_u16() == 401 {
                return Err(format!("{} refused the credentials given for it (401 Unauthorized)", self.name));
            }
        }
        match response.status().as_u16() {
            404 => return Ok(None),
            401 | 403 => return Err(format!("{} refused access ({})", self.name, response.status())),
            200..=299 => {}
            status => return Err(format!("{url} answered {status}")),
        }
        // A package with thousands of versions has a registration of several megabytes.
        response.body_mut().with_config().limit(64 << 20).read_to_string().map(Some).map_err(|e| format!("{url}: {e}"))
    }

    /// A JSON answer, or none where the feed has none (a package it does not hold).
    pub(super) fn json(&self, url: &str) -> Result<Option<Value>, String> {
        self.text(url)?.map(|t| serde_json::from_str(&t).map_err(|e| format!("{url} is not JSON: {e}"))).transpose()
    }
}

struct AtomEntry {
    id: String,
    version: String,
    description: String,
    authors: String,
    downloads: u64,
    listed: bool,
}

/// A v2 feed's entries: each one's properties (Id, NormalizedVersion, Description, Authors, DownloadCount), its
/// title, summary and author where they are missing; one not listed, or published in 1900, is unlisted.
fn atom_entries(xml: &str) -> Result<Vec<AtomEntry>, String> {
    use quick_xml::events::Event;
    if xml.trim().is_empty() {
        return Ok(vec![]);
    }
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    let mut entries = Vec::new();
    let mut fields: std::collections::HashMap<String, String> = Default::default();
    loop {
        match reader.read_event().map_err(|e| format!("the feed's answer is not Atom: {e}"))? {
            Event::Start(e) => {
                let name = String::from_utf8_lossy(e.local_name().as_ref()).into_owned();
                if name == "entry" {
                    fields.clear();
                }
                path.push(name);
            }
            Event::End(e) => {
                path.pop();
                if e.local_name().as_ref() == b"entry" {
                    let field = |k: &str| fields.get(k).cloned();
                    if let (Some(id), Some(version)) = (
                        field("Id").or_else(|| field("title")),
                        field("NormalizedVersion").or_else(|| field("Version")),
                    ) {
                        entries.push(AtomEntry {
                            id,
                            version,
                            description: field("Description").or_else(|| field("summary")).unwrap_or_default(),
                            authors: field("author.name").or_else(|| field("Authors")).unwrap_or_default(),
                            downloads: field("DownloadCount").and_then(|d| d.parse().ok()).unwrap_or(0),
                            listed: !field("Listed").is_some_and(|l| l.eq_ignore_ascii_case("false"))
                                && !field("Published").is_some_and(|p| p.starts_with("1900")),
                        });
                    }
                }
            }
            Event::Text(t) => {
                let in_entry = path.iter().any(|p| p == "entry");
                if let (true, Some(last)) = (in_entry, path.last()) {
                    let key = if path.len() >= 2 && path[path.len() - 2] == "author" && last == "name" {
                        "author.name".to_string()
                    } else {
                        last.clone()
                    };
                    let text = t.unescape().map(|t| t.into_owned()).unwrap_or_default();
                    fields.entry(key).or_default().push_str(&text);
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(entries)
}

/// The feed's next page, from its `<link rel="next" href="…">`.
fn atom_next(xml: &str) -> Option<String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut depth = 0;
    loop {
        match reader.read_event().ok()? {
            Event::Start(_) => depth += 1,
            Event::End(_) => depth -= 1,
            Event::Empty(e) if depth == 1 && e.local_name().as_ref() == b"link" => {
                let value = |k: &[u8]| {
                    e.attributes()
                        .flatten()
                        .find(|a| a.key.local_name().as_ref() == k)
                        .and_then(|a| a.unescape_value().ok().map(|v| v.into_owned()))
                };
                if value(b"rel").as_deref() == Some("next") {
                    return value(b"href");
                }
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}

/// A folder's packages, flat or in NuGet's `id/version/` layout, each from its .nuspec; a file that is not a package
/// passed over.
fn read_folder(folder: &Path, name: &str, prerelease: bool) -> Result<Vec<Package>, String> {
    if !folder.is_dir() {
        return Err(format!("{name} is no folder: {} is not there", folder.display()));
    }
    let nupkgs = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_file() && p.extension().is_some_and(|e| e.eq_ignore_ascii_case("nupkg")))
            .collect()
    };
    let subfolders = |dir: &Path| -> Vec<PathBuf> {
        std::fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.is_dir())
            .collect()
    };
    let mut files = nupkgs(folder);
    for id in subfolders(folder) {
        for version in subfolders(&id) {
            files.extend(nupkgs(&version));
        }
    }
    files.sort();
    Ok(files
        .iter()
        .filter_map(|f| nuspec(f))
        .filter(|p| prerelease || !version::is_prerelease(&p.version))
        .map(|mut p| {
            p.feeds = vec![name.to_string()];
            p
        })
        .collect())
}

fn nuspec(nupkg: &Path) -> Option<Package> {
    use async_zip::base::read::seek::ZipFileReader;
    use futures::io::{AllowStdIo, AsyncReadExt, BufReader};
    let file = std::fs::File::open(nupkg).ok()?;
    let bytes = futures::executor::block_on(async {
        let mut zip = ZipFileReader::new(BufReader::new(AllowStdIo::new(file))).await.ok()?;
        let index = zip.file().entries().iter().position(|e| {
            e.filename().as_str().is_ok_and(|n| !n.contains('/') && n.to_lowercase().ends_with(".nuspec"))
        })?;
        let mut reader = zip.reader_without_entry(index).await.ok()?;
        let mut bytes = Vec::new();
        reader.read_to_end(&mut bytes).await.ok()?;
        Some(bytes)
    })?;
    let (text, _) = crate::encodings::decode(&bytes, None);
    let field = |name: &str| metadata_field(&text, name);
    let (id, version) = (field("id")?, field("version")?);
    Some(Package {
        id,
        version,
        description: field("description").unwrap_or_default(),
        authors: field("authors").unwrap_or_default(),
        downloads: 0,
        feeds: vec![],
    })
}

/// The text of `<metadata><name>`, whatever the nuspec's namespace.
fn metadata_field(xml: &str, name: &str) -> Option<String> {
    use quick_xml::events::Event;
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut path: Vec<String> = Vec::new();
    loop {
        match reader.read_event().ok()? {
            Event::Start(e) => path.push(String::from_utf8_lossy(e.local_name().as_ref()).into_owned()),
            Event::End(_) => {
                path.pop();
            }
            Event::Text(t) if path.len() == 3 && path[1] == "metadata" && path[2] == name => {
                return t.unescape().ok().map(|t| t.trim().to_string());
            }
            Event::Eof => return None,
            _ => {}
        }
    }
}
