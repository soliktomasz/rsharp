//! Feeds as the NuGet window asks them, offline: a stand-in v3 feed on loopback and folders of .nupkg files the test
//! writes. No test reaches nuget.org.

mod nupkg;

use nupkg::nupkg;
use rsharp_core::nuget::feed::{FeedSet, PackageFeed};
use rsharp_core::nuget::sources::Feed;
use serde_json::json;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

/// A v3 feed on loopback: each path's status and body, and the paths asked, in order.
struct StandIn {
    base: String,
    asked: Arc<Mutex<Vec<String>>>,
}

impl StandIn {
    fn serve(routes: impl Fn(&str) -> (u16, String) + Send + Sync + 'static) -> StandIn {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let asked = Arc::new(Mutex::new(Vec::new()));
        let log = asked.clone();
        let routes = Arc::new(routes);
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let (log, routes) = (log.clone(), routes.clone());
                std::thread::spawn(move || {
                    let mut reader = BufReader::new(stream.try_clone().unwrap());
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
                    loop {
                        let mut header = String::new();
                        if reader.read_line(&mut header).unwrap_or(0) == 0 || header.trim().is_empty() {
                            break;
                        }
                    }
                    log.lock().unwrap().push(path.clone());
                    let (status, body) = routes(&path);
                    let mut stream = stream;
                    let _ = write!(
                        stream,
                        "HTTP/1.1 {status} X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                        body.len()
                    );
                });
            }
        });
        StandIn { base, asked }
    }

    fn feed(&self, name: &str) -> PackageFeed {
        PackageFeed::for_source(&Feed {
            name: name.into(),
            source: format!("{}/v3/index.json", self.base),
            enabled: true,
            protocol_version: Some(3),
            defined_in: None,
        })
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
        protocol_version: None,
        defined_in: None,
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
