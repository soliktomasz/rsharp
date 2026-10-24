//! Where packages are found: a v3 feed (nuget.org's protocol) or a folder of .nupkg files; and All Feeds, every
//! enabled feed asked at once, a package found in several listed once.

use super::sources::Feed;
use super::version;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

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
    V3 { index: String, services: OnceLock<Result<Services, String>> },
    Folder(PathBuf),
}

#[derive(Clone, Debug, Default)]
struct Services {
    search: Option<String>,
    registrations: Option<String>,
    flat: Option<String>,
}

pub struct PackageFeed {
    pub name: String,
    kind: Kind,
}

impl PackageFeed {
    /// A configured source: a URL is a v3 feed, anything else a folder.
    // shortcut: a v2 (OData) URL is asked as v3 and says it has no service index; the next PR reads v2 feeds.
    pub fn for_source(source: &Feed) -> PackageFeed {
        let kind = if source.is_http() {
            Kind::V3 { index: source.source.clone(), services: OnceLock::new() }
        } else {
            Kind::Folder(source.source.clone().into())
        };
        PackageFeed { name: source.name.clone(), kind }
    }

    /// The packages a query finds, the most used first.
    pub fn search(&self, query: &str, prerelease: bool) -> Result<Vec<Package>, String> {
        match &self.kind {
            Kind::V3 { .. } => {
                let services = self.services()?;
                let search = services.search.as_ref().ok_or_else(|| format!("{} offers no search", self.name))?;
                let url =
                    format!("{search}?q={}&take=30&prerelease={prerelease}&semVerLevel=2.0.0", escape(query.trim()));
                let found = get_json(&url)?.unwrap_or(Value::Null);
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
            Kind::V3 { .. } => {
                let services = self.services()?;
                let lower = id.to_lowercase();
                if let Some(registrations) = &services.registrations {
                    let Some(registration) = get_json(&format!("{registrations}{lower}/index.json"))? else {
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
                            fetched = get_json(url)?.unwrap_or(Value::Null);
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
                    get_json(&format!("{flat}{lower}/index.json"))?
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
        let Kind::V3 { index, services } = &self.kind else { return Ok(Services::default()) };
        services
            .get_or_init(|| {
                let found = get_json(index)?.ok_or_else(|| format!("{} has no service index at {index}", self.name))?;
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

fn agent() -> &'static ureq::Agent {
    static AGENT: OnceLock<ureq::Agent> = OnceLock::new();
    AGENT.get_or_init(|| {
        ureq::Agent::config_builder()
            .timeout_global(Some(Duration::from_secs(30)))
            .http_status_as_error(false)
            .user_agent(concat!("RSharp/", env!("CARGO_PKG_VERSION")))
            .tls_config(ureq::tls::TlsConfig::builder().root_certs(ureq::tls::RootCerts::PlatformVerifier).build())
            .build()
            .into()
    })
}

/// A JSON answer, or none where the feed has none (a package it does not hold).
fn get_json(url: &str) -> Result<Option<Value>, String> {
    let mut response = agent().get(url).call().map_err(|e| format!("{url}: {e}"))?;
    match response.status().as_u16() {
        404 => return Ok(None),
        401 | 403 => return Err("the feed needs credentials, which RSharp cannot send yet".into()),
        200..=299 => {}
        status => return Err(format!("{url} answered {status}")),
    }
    // A package with thousands of versions has a registration of several megabytes.
    let text = response.body_mut().with_config().limit(64 << 20).read_to_string().map_err(|e| format!("{url}: {e}"))?;
    serde_json::from_str(&text).map(Some).map_err(|e| format!("{url} is not JSON: {e}"))
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
