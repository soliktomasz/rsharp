//! The NuGet window as a person uses it, against a folder feed the test writes and NuGet folders of the test's own:
//! the Sources tab, a search, a package's versions, then Install and Remove through `dotnet add package` with the
//! Host evaluating the project after each. No test reaches nuget.org.

mod support;

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::dock::Tool;
use rsharp_app::nuget_view::{ALL_FEEDS, NuGetView, Tab};
use rsharp_app::window::MainWindow;
use rsharp_core::nuget::sources::Places;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n\n</Project>\n";

/// A real .nupkg: a zip with its .nuspec at the root.
fn nupkg(path: &Path, id: &str, version: &str) {
    use async_zip::base::write::ZipFileWriter;
    use async_zip::{Compression, ZipEntryBuilder};
    let nuspec = format!(
        r#"<?xml version="1.0"?><package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd"><metadata><id>{id}</id><version>{version}</version><authors>Folk</authors><description>The {id} package</description></metadata></package>"#
    );
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    let mut bytes = Vec::new();
    futures::executor::block_on(async {
        let mut zip = ZipFileWriter::new(futures::io::Cursor::new(&mut bytes));
        zip.write_entry_whole(
            ZipEntryBuilder::new(format!("{id}.nuspec").into(), Compression::Deflate),
            nuspec.as_bytes(),
        )
        .await
        .unwrap();
        zip.close().await.unwrap();
    });
    std::fs::write(path, bytes).unwrap();
}

/// A solution with one project, its nuget.config naming only a folder feed of Demo.Core 1.0.0 and 2.0.0.
fn run(name: &str) -> Run {
    let run = Run::new(
        name,
        &[
            ("Shop.slnx", "<Solution>\n  <Project Path=\"Demo/Demo.csproj\" />\n</Solution>\n"),
            ("Demo/Demo.csproj", PROJECT),
            (
                "nuget.config",
                "<configuration><packageSources><clear /><add key=\"local\" value=\"feed\" /></packageSources></configuration>",
            ),
        ],
    );
    nupkg(&run.folder.join("feed/Demo.Core.1.0.0.nupkg"), "Demo.Core", "1.0.0");
    nupkg(&run.folder.join("feed/Demo.Core.2.0.0.nupkg"), "Demo.Core", "2.0.0");
    run
}

fn open(run: &Run, main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Entity<NuGetView> {
    let nuget = main.read_with(cx, |m, _| m.nuget().clone());
    let home = run.folder.parent().unwrap().join("nuget-home");
    nuget.update(cx, |v, cx| v.set_places(Places { user: home.join("user"), machine: home.join("machine") }, cx));
    let folder = run.folder.clone();
    main.update_in(cx, |m, window, cx| m.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-7" } else { "alt-7" });
    cx.run_until_parked();
    nuget
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn search(nuget: &Entity<NuGetView>, text: &str, cx: &mut VisualTestContext) {
    let field = nuget.read_with(cx, |v, _| v.search_field().clone());
    cx.update(|window, cx| window.focus(&field.focus_handle(cx)));
    cx.simulate_input(text);
    cx.executor().advance_clock(Duration::from_millis(350));
    cx.run_until_parked();
}

#[gpui::test]
fn the_window_lists_the_sources_finds_a_package_and_offers_its_versions(cx: &mut TestAppContext) {
    let run = run("nuget-search");
    let (main, cx) = run.window(cx);
    let nuget = open(&run, &main, cx);
    assert!(main.read_with(cx, |m, _| m.dock().is_open(Tool::NuGet)), "the NuGet key did not open the window");

    click(cx, "nuget tab: Sources");
    let sources = nuget.read_with(cx, |v, _| {
        let s = v.sources().expect("the sources were never read");
        (s.files.len(), s.feeds.iter().map(|f| f.name.clone()).collect::<Vec<_>>())
    });
    assert_eq!(sources, (1, vec!["local".to_string()]), "the Sources tab did not show the solution's file alone");
    click(cx, "nuget tab: Packages");
    assert_eq!(nuget.read_with(cx, |v, _| v.tab()), Tab::Packages);
    assert_eq!(nuget.read_with(cx, |v, _| v.status().to_string()), "No packages installed");

    search(&nuget, "demo", cx);
    let rows =
        nuget.read_with(cx, |v, _| v.rows().iter().map(|r| (r.id.clone(), r.version.clone())).collect::<Vec<_>>());
    assert_eq!(rows, [("Demo.Core".to_string(), "2.0.0".to_string())], "the folder feed's newest was not found");
    assert_eq!(
        nuget.read_with(cx, |v, _| v.versions().map(|v| v.to_vec()).map_err(str::to_string)),
        Ok(vec!["2.0.0".to_string(), "1.0.0".to_string()]),
        "the chosen package's versions were not offered newest first"
    );
    assert_eq!(nuget.read_with(cx, |v, _| v.version().map(str::to_string)).as_deref(), Some("2.0.0"));
    assert!(cx.debug_bounds("nuget install: Demo").is_some(), "the project had no Install button");

    click(cx, "nuget version");
    click(cx, "nuget choice: 1.0.0");
    assert_eq!(nuget.read_with(cx, |v, _| v.version().map(str::to_string)).as_deref(), Some("1.0.0"));
    click(cx, "nuget feed");
    assert_eq!(nuget.read_with(cx, |v, _| v.feed_choices()), [ALL_FEEDS, "local"]);
}

/// Waits, in real time, for what runs on threads of its own (dotnet, the Host) to reach the window.
fn until(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} never happened");
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(50));
    }
}

#[gpui::test]
fn install_and_remove_change_the_project_and_its_listed_version(cx: &mut TestAppContext) {
    if rsharp_lsp::SharedHost::carried(Path::new(".")).is_none() {
        eprintln!("skipped: Passenger's sidecars were not built (no dotnet or no vendor/passenger at build time)");
        return;
    }
    let Ok(mut environment) = rsharp_core::templates::TemplateEnvironment::discover() else {
        eprintln!("skipped: dotnet is not installed");
        return;
    };
    let run = run("nuget-install");
    let (main, cx) = run.window(cx);
    let own = run.folder.parent().unwrap();
    environment.cli_home = own.join("cli-home");
    environment.nuget_packages = Some(own.join("packages"));
    cx.update(|_, cx| {
        let shared = rsharp_app::templates::shared(cx);
        *shared.lock().unwrap() = Some(rsharp_core::templates::cache::Catalogs::new(environment));
        cx.global_mut::<rsharp_app::state::Ide>().servers = true;
    });
    let nuget = open(&run, &main, cx);
    until(cx, "the project's evaluation", |cx| !main.read_with(cx, |m, _| m.evaluating()));
    search(&nuget, "Demo.Core", cx);
    until(cx, "the versions", |cx| nuget.read_with(cx, |v, _| v.versions().is_ok_and(|v| !v.is_empty())));

    click(cx, "nuget install: Demo");
    until(cx, "Install", |cx| !nuget.read_with(cx, |v, _| v.is_busy()));
    let project = run.folder.join("Demo/Demo.csproj");
    let written = std::fs::read_to_string(&project).unwrap();
    assert!(
        written.contains("<PackageReference Include=\"Demo.Core\" Version=\"2.0.0\" />"),
        "Install did not write the reference: {written}\nlog: {}",
        nuget.read_with(cx, |v, _| v.log().to_string())
    );
    until(cx, "the project's version listed", |cx| cx.debug_bounds("nuget remove: Demo").is_some());

    click(cx, "nuget remove: Demo");
    until(cx, "Remove", |cx| !nuget.read_with(cx, |v, _| v.is_busy()));
    assert!(
        !std::fs::read_to_string(&project).unwrap().contains("Demo.Core"),
        "Remove left the reference; status {:?}, log: {}",
        nuget.read_with(cx, |v, _| v.status().to_string()),
        nuget.read_with(cx, |v, _| v.log().to_string())
    );
    until(cx, "the project's Install offered again", |cx| cx.debug_bounds("nuget install: Demo").is_some());
}

fn type_into(field: &Entity<rsharp_app::field::TextField>, text: &str, cx: &mut VisualTestContext) {
    cx.update(|window, cx| window.focus(&field.focus_handle(cx)));
    cx.simulate_input(text);
    cx.run_until_parked();
}

fn config(run: &Run) -> String {
    std::fs::read_to_string(run.folder.join("nuget.config")).unwrap()
}

#[gpui::test]
fn the_sources_tab_adds_disables_edits_and_deletes_a_feed_in_the_solutions_file(cx: &mut TestAppContext) {
    let run = run("nuget-sources-edit");
    let (main, cx) = run.window(cx);
    let nuget = open(&run, &main, cx);
    click(cx, "nuget tab: Sources");
    assert_eq!(
        nuget.read_with(cx, |v, _| v.chosen_file().map(|f| f.path.clone())),
        Some(run.folder.join("nuget.config")),
        "the solution's own file was not the one shown"
    );

    click(cx, "nuget add feed");
    click(cx, "nuget form ok");
    assert!(cx.debug_bounds("nuget form ok").is_some(), "a nameless feed was accepted");
    let form = nuget.read_with(cx, |v, _| {
        let f = v.form().unwrap();
        (f.name.clone(), f.url.clone(), f.user.clone(), f.password.clone())
    });
    type_into(&form.0, "corp", cx);
    type_into(&form.1, "https://corp.example/v3/index.json", cx);
    click(cx, "nuget form ok");
    assert!(
        config(&run).contains("<add key=\"corp\" value=\"https://corp.example/v3/index.json\" />"),
        "Add Feed did not write the feed: {}; form {:?}; problem {:?}",
        config(&run),
        nuget.read_with(cx, |v, cx| v.form().map(|f| (f.name.read(cx).text(), f.url.read(cx).text()))),
        nuget.read_with(cx, |v, _| v.sources_problem().map(str::to_string))
    );
    assert!(
        nuget.read_with(cx, |v, _| v.feed_choices()).contains(&"corp".to_string()),
        "the sources were not read again"
    );

    click(cx, "nuget feed enabled: corp");
    assert!(config(&run).contains("<add key=\"corp\" value=\"true\" />"), "disabling wrote nothing: {}", config(&run));
    assert!(!nuget.read_with(cx, |v, _| v.feed_choices()).contains(&"corp".to_string()), "a disabled feed was offered");
    click(cx, "nuget feed enabled: corp");
    assert!(!config(&run).contains("value=\"true\""), "enabling left the disabling: {}", config(&run));

    click(cx, "nuget edit feed: corp");
    let form = nuget.read_with(cx, |v, _| {
        let f = v.form().unwrap();
        (f.user.clone(), f.password.clone())
    });
    type_into(&form.0, "ann", cx);
    click(cx, "nuget form ok");
    assert!(nuget.read_with(cx, |v, _| v.form().is_some()), "a user without a password was accepted");
    type_into(&form.1, "pw", cx);
    click(cx, "nuget form ok");
    assert!(config(&run).contains("<add key=\"Username\" value=\"ann\" />"), "{}", config(&run));
    assert!(!format!("{:?}", nuget.read_with(cx, |v, _| v.sources().map(|s| format!("{s:?}")))).contains("\"pw\""));

    click(cx, "nuget delete feed: corp");
    assert!(!config(&run).contains("corp"), "Delete left the feed or its credentials: {}", config(&run));
    assert_eq!(nuget.read_with(cx, |v, _| v.sources_problem().map(str::to_string)), None);
}

/// A v3 feed on loopback with search, a flat container and one advisory: Demo.Core below 1.2.0 is vulnerable.
fn audit_feed() -> String {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let served = base.clone();
    std::thread::spawn(move || {
        use std::io::{BufRead, Write};
        for stream in listener.incoming().flatten() {
            let mut reader = std::io::BufReader::new(stream.try_clone().unwrap());
            let mut line = String::new();
            reader.read_line(&mut line).unwrap();
            while reader.read_line(&mut String::new()).unwrap_or(0) > 2 {}
            let path = line.split_whitespace().nth(1).unwrap_or("/").to_string();
            let body = match path.split('?').next().unwrap() {
                "/v3/index.json" => format!(
                    r#"{{"resources":[{{"@id":"{served}/query","@type":"SearchQueryService/3.5.0"}},{{"@id":"{served}/flat","@type":"PackageBaseAddress/3.0.0"}},{{"@id":"{served}/vuln.json","@type":"VulnerabilityInfo/6.7.0"}}]}}"#
                ),
                "/vuln.json" => format!(r#"[{{"@name":"base","@id":"{served}/base.json"}}]"#),
                "/base.json" => {
                    r#"{"demo.core":[{"severity":2,"url":"https://advisory.example/demo","versions":"(, 1.2.0)"}]}"#
                        .into()
                }
                "/flat/demo.core/index.json" => r#"{"versions":["1.0.0","1.2.0","2.0.0"]}"#.into(),
                "/query" => r#"{"data":[]}"#.into(),
                _ => String::new(),
            };
            let status = if body.is_empty() { 404 } else { 200 };
            let mut stream = stream;
            let _ = write!(
                stream,
                "HTTP/1.1 {status} X\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            );
        }
    });
    base
}

#[gpui::test]
fn a_vulnerable_installed_package_is_marked_and_update_offers_the_fix(cx: &mut TestAppContext) {
    let base = audit_feed();
    let run = Run::new(
        "nuget-audit",
        &[
            ("Shop.slnx", "<Solution>\n  <Project Path=\"Demo/Demo.csproj\" />\n</Solution>\n"),
            ("Demo/Demo.csproj", PROJECT),
            (
                "nuget.config",
                &format!(
                    "<configuration><packageSources><clear /><add key=\"audit\" value=\"{base}/v3/index.json\" protocolVersion=\"3\" allowInsecureConnections=\"true\" /></packageSources></configuration>"
                ),
            ),
        ],
    );
    let (main, cx) = run.window(cx);
    let nuget = open(&run, &main, cx);
    nuget.update(cx, |v, cx| {
        v.set_projects(
            vec![rsharp_app::nuget_view::ProjectPackages {
                path: run.folder.join("Demo/Demo.csproj"),
                name: "Demo".into(),
                packages: vec![("Demo.Core".into(), Some("1.0.0".into()))],
            }],
            cx,
        )
    });
    until(cx, "the advisories", |cx| nuget.read_with(cx, |v, _| !v.advisories("Demo.Core").is_empty()));
    nuget.update(cx, |v, cx| v.select(Some(0), cx));
    until(cx, "the versions", |cx| nuget.read_with(cx, |v, _| v.versions().is_ok_and(|v| !v.is_empty())));
    assert_eq!(
        nuget.read_with(cx, |v, _| v.version().map(str::to_string)).as_deref(),
        Some("1.2.0"),
        "Update did not offer the least version that fixes the advisory"
    );
    assert!(cx.debug_bounds("nuget advisory: 0").is_some(), "the advisory was not shown");
}
