//! Feeds as the NuGet window asks them, offline: a stand-in v3 feed on loopback and folders of .nupkg files the test
//! writes. No test reaches nuget.org.

mod nupkg;

use nupkg::nupkg;
use rsharp_core::nuget::feed::{FeedSet, PackageFeed};
use rsharp_core::nuget::secret::Credentials;
use rsharp_core::nuget::sources::Feed;
use rsharp_core::nuget::vulnerabilities::{Severity, Vulnerabilities};
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A feed on loopback: each path's status and body, and the paths asked with the Authorization each sent, in order.
struct StandIn {
    base: String,
    asked: Arc<Mutex<Vec<String>>>,
    authorized: Arc<Mutex<Vec<Option<String>>>>,
}

impl StandIn {
    fn serve(routes: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> StandIn {
        Self::serve_with(move |path, _| routes(path))
    }

    fn serve_with(routes: impl Fn(&str, Option<&str>) -> (u16, String) + Send + Sync + 'static) -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let authorized = Arc::new(Mutex::new(Vec::new()));
        let (log, auth_log) = (asked.clone(), authorized.clone());
        let routes = Arc::new(routes);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, auth_log, routes) = (log.clone(), auth_log.clone(), routes.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    let mut authorization = None;
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                            break;
                        }
                        if let Some((name, value)) = header.split_once(':')
                            && name.eq_ignore_ascii_case("authorization")
                        {
                            authorization = Some(value.trim().to_string());
                        }
                    }
                    log.lock().unwrap().push(path.clone());
                    auth_log.lock().unwrap().push(authorization.clone());
                    let (status, body) = routes(&path, authorization.as_deref());
                    let mut stream = stream;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        StandIn { base, asked, authorized }
    }

    /// The feed as a nuget.config allowing it HTTP names it, its credential providers giving nothing.
    fn source(&self, name: &str) -> Feed {
        Feed {
            name: name.into(),
            source: format!("{}/v3/index.json", self.base),
            enabled: true,
            protocol_version: Some(3),
            allow_insecure: true,
            ..Default::default()
        }
    }

    fn feed(&self, name: &str) -> PackageFeed {
        PackageFeed::with_credentials(&self.source(name), Arc::new(|_, _| None))
    }
}

fn index(base: &str, registrations: bool) -> String {
    let mut resources = vec![
        json!({"@id": format!("{base}/query"), "@type": "SearchQueryService/3.5.0"}),
        json!({"@id": format!("{base}/flat"), "@type": "PackageBaseAddress/3.0.0"}),
    ];
    if registrations {
        resources.push(json!({"@id": format!("{base}/reg"), "@type": "RegistrationsBaseUrl/3.6.0"}));
    }
    json!({"version": "3.0.0", "resources": resources}).to_string()
}

fn leaf(version: &str, listed: bool) -> serde_json::Value {
    json!({"catalogEntry": {"version": version, "listed": listed}})
}

#[test]
fn a_v3_feed_searches_and_lists_versions_from_inline_and_paged_registrations() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen = base.clone();
    let feed = StandIn::serve(move |path| {
        let base = seen.lock().unwrap().clone();
        match path.split('?').next().unwrap() {
            "/v3/index.json" => (200, index(&base, true)),
            "/query" => (
                200,
                json!({"data": [
                    {"id": "Demo.Core", "version": "2.0.0", "description": "The core", "authors": ["Ann", "Bo"], "totalDownloads": 42},
                    {"id": "Demo.Private", "version": "1.0.0", "authors": "Corp"}
                ]})
                .to_string(),
            ),
            "/reg/demo.core/index.json" => (
                200,
                json!({"items": [
                    {"@id": format!("{base}/reg/demo.core/page1.json")},
                    {"@id": "inline", "items": [leaf("2.0.0", true), leaf("2.1.0-beta.1", true)]}
                ]})
                .to_string(),
            ),
            "/reg/demo.core/page1.json" => {
                (200, json!({"items": [leaf("1.0.0", true), leaf("1.5.0", false), leaf("1.10.0", true)]}).to_string())
            }
            _ => (404, String::new()),
        }
    });
    *base.lock().unwrap() = feed.base.clone();
    let demo = feed.feed("demo");

    let found = demo.search("demo core", false).unwrap();
    assert_eq!(found[0].id, "Demo.Core");
    assert_eq!((found[0].authors.as_str(), found[0].downloads), ("Ann, Bo", 42));
    assert_eq!(found[1].authors, "Corp", "a feed's authors as one string were lost");
    let query = feed.asked.lock().unwrap().iter().find(|p| p.starts_with("/query")).cloned().unwrap();
    assert_eq!(query, "/query?q=demo%20core&take=30&prerelease=false&semVerLevel=2.0.0");

    assert_eq!(
        demo.versions("Demo.Core", false).unwrap(),
        ["2.0.0", "1.10.0", "1.0.0"],
        "the paged registration was not followed, an unlisted or prerelease version was offered, or the order is not NuGet's"
    );
    assert_eq!(demo.versions("Demo.Core", true).unwrap()[0], "2.1.0-beta.1");
    assert!(demo.versions("Missing", false).unwrap().is_empty(), "a package the feed does not hold is no failure");
    let indexes = feed.asked.lock().unwrap().iter().filter(|p| *p == "/v3/index.json").count();
    assert_eq!(indexes, 1, "the service index was asked again for each request");
}

#[test]
fn a_feed_without_registrations_lists_its_flat_container_and_a_refusal_names_credentials() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen = base.clone();
    let feed = StandIn::serve(move |path| match path {
        "/v3/index.json" => (200, index(&seen.lock().unwrap(), false)),
        "/flat/demo.core/index.json" => (200, json!({"versions": ["1.0.0", "3.0.0-rc.1", "2.0.0"]}).to_string()),
        _ => (404, String::new()),
    });
    *base.lock().unwrap() = feed.base.clone();
    assert_eq!(feed.feed("flat").versions("Demo.Core", false).unwrap(), ["2.0.0", "1.0.0"]);

    let locked = StandIn::serve(|_| (401, String::new()));
    let why = locked.feed("private").search("x", false).unwrap_err();
    assert!(why.contains("credentials"), "a 401 did not say the feed needs credentials: {why}");
}

static NEXT: AtomicUsize = AtomicUsize::new(0);

fn folder() -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rsharp-nuget-feed-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn folder_feed(name: &str, dir: &Path) -> PackageFeed {
    PackageFeed::for_source(&Feed {
        name: name.into(),
        source: dir.to_string_lossy().into(),
        enabled: true,
        ..Default::default()
    })
}

#[test]
fn a_folder_feed_reads_flat_and_nested_packages_from_their_nuspecs() {
    let dir = folder();
    nupkg(&dir.join("Demo.Core.1.0.0.nupkg"), "Demo.Core", "1.0.0");
    nupkg(&dir.join("demo.core/2.0.0/demo.core.2.0.0.nupkg"), "Demo.Core", "2.0.0");
    nupkg(&dir.join("Demo.Core.3.0.0-beta.nupkg"), "Demo.Core", "3.0.0-beta");
    nupkg(&dir.join("Other.1.0.0.nupkg"), "Other", "1.0.0");
    std::fs::write(dir.join("broken.nupkg"), "not a zip").unwrap();
    let feed = folder_feed("local", &dir);

    let found = feed.search("core", false).unwrap();
    assert_eq!(found.len(), 1, "the query matched Other, or a broken file was listed: {found:?}");
    assert_eq!((found[0].version.as_str(), found[0].description.as_str()), ("2.0.0", "About Demo.Core & more"));
    assert_eq!(feed.versions("demo.core", true).unwrap(), ["3.0.0-beta", "2.0.0", "1.0.0"]);
    assert!(folder_feed("gone", &dir.join("missing")).search("", false).unwrap_err().contains("no folder"));
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn all_feeds_lists_a_package_once_keeps_mapped_ones_out_and_still_answers_when_one_fails() {
    let (one, two) = (folder(), folder());
    nupkg(&one.join("Demo.Core.1.0.0.nupkg"), "Demo.Core", "1.0.0");
    nupkg(&two.join("Demo.Core.2.0.0.nupkg"), "Demo.Core", "2.0.0");
    nupkg(&two.join("XDemo.Core.Tools.1.0.0.nupkg"), "XDemo.Core.Tools", "1.0.0");
    nupkg(&two.join("Corp.Demo.Core.1.0.0.nupkg"), "Corp.Demo.Core", "1.0.0");
    let set = FeedSet {
        feeds: vec![folder_feed("one", &one), folder_feed("two", &two), folder_feed("down", &one.join("gone"))],
    };
    let mapping: HashMap<&str, &str> = [("corp.demo.core", "one")].into();
    let allows = |feed: &str, id: &str| mapping.get(id.to_lowercase().as_str()).is_none_or(|f| *f == feed);

    let found = set.search("demo.core", false, &allows);
    let listed: Vec<_> = found.packages.iter().map(|p| (p.id.as_str(), p.version.as_str(), p.feeds.clone())).collect();
    assert_eq!(
        listed,
        [
            ("Demo.Core", "2.0.0", vec!["one".to_string(), "two".to_string()]),
            ("XDemo.Core.Tools", "1.0.0", vec!["two".to_string()]),
        ],
        "a package in two feeds was not listed once at its newest, or the closer match did not come first"
    );
    assert_eq!(found.unmapped, 1, "the package the mapping keeps from feed two was offered from it");
    assert_eq!(found.failed.len(), 1);
    assert_eq!(found.failed[0].0, "down");

    assert_eq!(set.versions("Demo.Core", false, &allows).unwrap(), ["2.0.0", "1.0.0"]);
    let _ = (std::fs::remove_dir_all(one), std::fs::remove_dir_all(two));
}

#[test]
fn a_feeds_credentials_go_to_its_own_host_and_http_needs_its_switch() {
    let feed = StandIn::serve_with(|path, auth| match (path, auth) {
        (_, Some("Basic YW5uOnB3")) => (200, json!({"data": []}).to_string().replace("[]", "[]")),
        _ => (401, String::new()),
    });
    let mut source = feed.source("private");
    source.protocol_version = None;
    source.source = format!("{}/v3/index.json", feed.base);
    source.credentials = Some(Credentials { username: "ann".into(), password: "pw".into() });
    let found = PackageFeed::with_credentials(
        &source,
        Arc::new(|_, _| panic!("a provider was asked for configured credentials")),
    );
    // The stand-in answers its index with search data, so the search finds no service: the credentials were taken.
    let why = found.search("x", false).unwrap_err();
    assert!(why.contains("offers no search"), "{why}");
    assert_eq!(feed.authorized.lock().unwrap()[0].as_deref(), Some("Basic YW5uOnB3"));

    source.allow_insecure = false;
    let refused = PackageFeed::with_credentials(&source, Arc::new(|_, _| None)).search("x", false).unwrap_err();
    assert!(refused.contains("allowInsecureConnections"), "an HTTP feed was asked without its switch: {refused}");

    source.allow_insecure = true;
    source.credentials = Some(Credentials { username: "ann".into(), password: "wrong".into() });
    let wrong = PackageFeed::with_credentials(&source, Arc::new(|_, _| None)).search("x", false).unwrap_err();
    assert!(wrong.contains("refused the credentials its nuget.config gives"), "{wrong}");
}

#[test]
fn a_feed_without_credentials_asks_the_providers_once_it_answers_401() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen = base.clone();
    let feed = StandIn::serve_with(move |path, auth| match auth {
        Some("Basic Ym90OnRvaw==") if path == "/v3/index.json" => (200, index(&seen.lock().unwrap(), false)),
        Some("Basic Ym90OnRvaw==") => (200, json!({"versions": ["1.0.0"]}).to_string()),
        _ => (401, String::new()),
    });
    *base.lock().unwrap() = feed.base.clone();
    let asked = Arc::new(Mutex::new(Vec::new()));
    let log = asked.clone();
    let source = feed.source("private");
    let private = PackageFeed::with_credentials(
        &source,
        Arc::new(move |index: &str, retry| {
            log.lock().unwrap().push((index.to_string(), retry));
            Some(Credentials { username: "bot".into(), password: "tok".into() })
        }),
    );
    assert_eq!(private.versions("Demo.Core", false).unwrap(), ["1.0.0"]);
    assert_eq!(
        *asked.lock().unwrap(),
        [(source.source.clone(), false)],
        "the providers were not asked once, for the feed's index"
    );

    let never = PackageFeed::with_credentials(&feed.source("private"), Arc::new(|_, _| None));
    assert!(never.search("x", false).unwrap_err().contains("no credential provider gave any"));
}

#[test]
fn a_v2_feed_searches_and_pages_through_its_versions() {
    let entry = |id: &str, version: &str, listed: bool, published: &str| {
        format!(
            r#"<entry><title type="text">{id}</title><summary>About {id}</summary><author><name>Folk</name></author><m:properties><d:Version>{version}</d:Version><d:NormalizedVersion>{version}</d:NormalizedVersion><d:DownloadCount m:type="Edm.Int32">7</d:DownloadCount><d:Listed m:type="Edm.Boolean">{listed}</d:Listed><d:Published>{published}</d:Published></m:properties></entry>"#
        )
    };
    let atom = |entries: String, next: Option<String>| {
        format!(
            r#"<?xml version="1.0" encoding="utf-8"?><feed xmlns="http://www.w3.org/2005/Atom" xmlns:d="http://schemas.microsoft.com/ado/2007/08/dataservices" xmlns:m="http://schemas.microsoft.com/ado/2007/08/dataservices/metadata">{}{entries}</feed>"#,
            next.map(|n| format!(r#"<link rel="next" href="{n}" />"#)).unwrap_or_default()
        )
    };
    let base = Arc::new(Mutex::new(String::new()));
    let seen = base.clone();
    let feed = StandIn::serve(move |path| {
        let base = seen.lock().unwrap().clone();
        if path.starts_with("/api/v2/Search()") {
            (200, atom(entry("Demo.Core", "2.0.0", true, "2024-01-01"), None))
        } else if path.starts_with("/api/v2/FindPackagesById()") && !path.contains("skip") {
            let page =
                entry("Demo.Core", "1.0.0", true, "2023-01-01") + &entry("Demo.Core", "1.5.0", false, "2023-02-01");
            (200, atom(page, Some(format!("{base}/api/v2/FindPackagesById()?id='Demo.Core'&amp;$skip=2"))))
        } else if path.contains("skip=2") {
            (
                200,
                atom(
                    entry("Demo.Core", "2.0.0", true, "2024-01-01")
                        + &entry("Demo.Core", "0.9.0", true, "1900-01-01T00:00:00"),
                    None,
                ),
            )
        } else {
            (404, String::new())
        }
    });
    *base.lock().unwrap() = feed.base.clone();
    let mut source = feed.source("old");
    source.source = format!("{}/api/v2/", feed.base);
    source.protocol_version = None;
    let old = PackageFeed::with_credentials(&source, Arc::new(|_, _| None));
    let found = old.search("demo", false).unwrap();
    assert_eq!(
        (found[0].id.as_str(), found[0].version.as_str(), found[0].authors.as_str(), found[0].downloads),
        ("Demo.Core", "2.0.0", "Folk", 7)
    );
    let search = feed.asked.lock().unwrap()[0].clone();
    assert!(search.contains("$filter=IsLatestVersion&searchTerm=%27demo%27"), "{search}");
    assert_eq!(
        old.versions("Demo.Core", false).unwrap(),
        ["2.0.0", "1.0.0"],
        "the next page was not followed, or an unlisted or 1900-published version was offered"
    );
}

#[test]
fn vulnerabilities_come_from_the_base_page_and_its_updates() {
    let base = Arc::new(Mutex::new(String::new()));
    let seen = base.clone();
    let feed = StandIn::serve(move |path| {
        let base = seen.lock().unwrap().clone();
        match path {
            "/v3/index.json" => (
                200,
                json!({"resources": [{"@id": format!("{base}/vuln/index.json"), "@type": "VulnerabilityInfo/6.7.0"}]})
                    .to_string(),
            ),
            "/vuln/index.json" => (200, json!([{"@name": "base", "@id": format!("{base}/vuln/base.json")}, {"@name": "update", "@id": "update.json"}]).to_string()),
            "/vuln/base.json" => (200, json!({"demo.core": [{"severity": 2, "url": "https://advisory.example/1", "versions": "(, 1.2.0)"}]}).to_string()),
            "/vuln/update.json" => (200, json!({"demo.core": [{"severity": 3, "url": "https://advisory.example/2", "versions": "[1.0.0]"}]}).to_string()),
            _ => (404, String::new()),
        }
    });
    *base.lock().unwrap() = feed.base.clone();
    let read = Vulnerabilities::read_feeds(&[feed.feed("audit")]);
    assert!(read.failed.is_empty(), "{:?}", read.failed);
    let found: Vec<_> = read.of("Demo.Core", "1.0.0").iter().map(|a| (a.severity, a.url.clone())).collect();
    assert_eq!(
        found,
        [
            (Severity::Critical, "https://advisory.example/2".to_string()),
            (Severity::High, "https://advisory.example/1".to_string())
        ],
        "the update page was not read, or the gravest did not come first"
    );
    assert!(read.of("Demo.Core", "1.2.0").is_empty());
}
