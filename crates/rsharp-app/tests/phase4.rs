//! Phase 4's acceptance: a solution built up from New Solution to three projects with references and a package,
//! entirely through RSharp's window, its files byte for byte those Passenger's window makes for the same steps
//! (`fixtures/phase4`, written by `tools/passenger-parity.sh`), the .sln's random project GUIDs aside. The steps, on
//! SDK 10.0.1xx and a local feed: New Solution (Console App "Shop"); Add, New Project twice (Class Library
//! "Shop.Core" and "Shop.Data"); Add, Reference Shop to Shop.Core and Shop.Core to Shop.Data; Install Demo.Core 2.0.0
//! into Shop.Core from the NuGet window. Skips, saying why, without dotnet or an SDK 10.0.1xx.

mod support;

use gpui::{Entity, Focusable, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use rsharp_app::{new_project_dialog::NewProjectDialog, templates::Templates, window::MainWindow};
use rsharp_core::nuget::sources::Places;
use rsharp_core::templates::{TemplateEnvironment, cache::Catalogs};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::Run;

fn golden() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/phase4")
}

/// A package with nothing but its manifest and an empty lib folder, as Passenger's side makes it.
fn nupkg(path: &Path, id: &str, version: &str) {
    use async_zip::base::write::ZipFileWriter;
    use async_zip::{Compression, ZipEntryBuilder};
    let nuspec = format!(
        r#"<?xml version="1.0"?><package xmlns="http://schemas.microsoft.com/packaging/2013/05/nuspec.xsd"><metadata><id>{id}</id><version>{version}</version><authors>Folk</authors><description>The {id} package</description></metadata></package>"#
    );
    let mut bytes = Vec::new();
    futures::executor::block_on(async {
        let mut zip = ZipFileWriter::new(futures::io::Cursor::new(&mut bytes));
        zip.write_entry_whole(
            ZipEntryBuilder::new(format!("{id}.nuspec").into(), Compression::Deflate),
            nuspec.as_bytes(),
        )
        .await
        .unwrap();
        zip.write_entry_whole(
            ZipEntryBuilder::new("lib/netstandard2.0/_._".to_string().into(), Compression::Stored),
            b"",
        )
        .await
        .unwrap();
        zip.close().await.unwrap();
    });
    std::fs::write(path, bytes).unwrap();
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn field(cx: &mut VisualTestContext, selector: &'static str, value: &str) {
    click(cx, selector);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(value);
    cx.run_until_parked();
}

fn wait(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(300);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} never happened");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(30));
    }
}

fn dialog(
    main: &Entity<MainWindow>,
    cx: &mut VisualTestContext,
) -> (WindowHandle<NewProjectDialog>, VisualTestContext) {
    cx.run_until_parked();
    let handle = main.read_with(cx, |m, _| m.new_project_dialog()).expect("the creation dialog did not open");
    let mut dialog = VisualTestContext::from_window(handle.into(), cx);
    wait(&mut dialog, "the templates to be read", |d| {
        handle.read_with(d, |d, _| d.draft().authoritative || d.problem().is_some()).unwrap_or(true)
    });
    assert_eq!(handle.read_with(&dialog, |d, _| d.problem().map(str::to_string)).unwrap(), None);
    (handle, dialog)
}

fn create(handle: WindowHandle<NewProjectDialog>, cx: &mut VisualTestContext) {
    click(cx, "button: Create");
    wait(cx, "dotnet new", |cx| handle.read_with(cx, |d, _| d.problem().is_some()).unwrap_or(true));
    if let Ok(problem) = handle.read_with(cx, |d, _| d.problem().map(str::to_string)) {
        panic!("dotnet new failed: {problem:?}");
    }
}

/// The SDK the steps pin, as dotnet lists it, or why there is none.
fn sdk10(env: &TemplateEnvironment) -> Option<String> {
    let mut command = env.command(&["--list-sdks".into()], Path::new("/"));
    command.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::null());
    let output = rsharp_core::process::spawn(command).ok()?.output().ok()?;
    String::from_utf8_lossy(&output)
        .lines()
        .find_map(|l| l.split_whitespace().next().filter(|v| v.starts_with("10.0.1")).map(String::from))
}

/// A .sln with each project's GUID named by its order, as dotnet new sln makes them random.
fn without_guids(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes).into_owned();
    let mut seen: Vec<String> = Vec::new();
    regex::Regex::new(r"\{[0-9A-Fa-f-]{36}\}")
        .unwrap()
        .replace_all(&text, |c: &regex::Captures| {
            let guid = c[0].to_uppercase();
            // Project type GUIDs are Visual Studio's own and stay as written.
            if guid == "{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}" || guid == "{2150E333-8FDC-42A3-9474-1A3956D46DE8}" {
                return guid;
            }
            let at = seen.iter().position(|g| *g == guid).unwrap_or_else(|| {
                seen.push(guid.clone());
                seen.len() - 1
            });
            format!("{{PROJECT-{at}}}")
        })
        .into_owned()
}

fn files(folder: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut walk = vec![folder.to_path_buf()];
    while let Some(dir) = walk.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if path.is_dir() {
                if name != "bin" && name != "obj" {
                    walk.push(path);
                }
            } else {
                found.push(path.strip_prefix(folder).unwrap().to_path_buf());
            }
        }
    }
    found.sort();
    found
}

#[gpui::test]
fn a_solution_grows_to_three_projects_with_references_and_a_package_as_passengers_does(cx: &mut TestAppContext) {
    let Ok(mut env) = TemplateEnvironment::discover() else {
        eprintln!("skipped: dotnet is not installed");
        return;
    };
    let Some(sdk) = sdk10(&env) else {
        eprintln!("skipped: no SDK 10.0.1xx, which the steps and Passenger's files are on");
        return;
    };
    let run = Run::new("phase4", &[]);
    let place = run.folder.clone();
    let own = place.parent().unwrap().to_path_buf();
    std::fs::write(
        place.join("global.json"),
        format!("{{\n  \"sdk\": {{\n    \"version\": \"{sdk}\",\n    \"rollForward\": \"disable\"\n  }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(
        place.join("nuget.config"),
        "<configuration>\n  <packageSources>\n    <clear />\n    <add key=\"local\" value=\"feed\" />\n  </packageSources>\n  <config>\n    <add key=\"globalPackagesFolder\" value=\"packages\" />\n  </config>\n</configuration>\n",
    )
    .unwrap();
    std::fs::create_dir_all(place.join("feed")).unwrap();
    nupkg(&place.join("feed/Demo.Core.1.0.0.nupkg"), "Demo.Core", "1.0.0");
    nupkg(&place.join("feed/Demo.Core.2.0.0.nupkg"), "Demo.Core", "2.0.0");
    env.cli_home = own.join("cli-home");
    env.hive = Some(own.join("hive"));

    let (main, cx) = run.window(cx);
    cx.update(|_, cx| {
        let global = cx.global_mut::<Templates>();
        *global.catalogs.lock().unwrap() = Some(Catalogs::new(env.clone()));
        global.enabled = true;
    });
    let nuget = main.read_with(cx, |m, _| m.nuget().clone());
    nuget.update(cx, |v, cx| {
        v.set_places(Places { user: own.join("nuget/user"), machine: own.join("nuget/machine") }, cx)
    });

    // 1. New Solution: Console App, Shop, the rest as it opens.
    click(cx, "button: New Solution");
    let (handle, mut d) = dialog(&main, cx);
    field(&mut d, "field: Location", place.to_str().unwrap());
    let (handle, mut d) = {
        // The Location's own SDK is read again, as the pinned folder's templates are those Passenger read.
        wait(&mut d, "the Location's templates", |d| {
            handle.read_with(d, |d, _| d.draft().authoritative).unwrap_or(true)
        });
        (handle, d)
    };
    field(&mut d, "field: Solution name", "Shop");
    create(handle, &mut d);
    let folder = place.join("Shop");
    wait(cx, "the new solution open", |cx| main.read_with(cx, |m, _| m.opened_as().is_some()));

    // 2 and 3. Add, New Project: two class libraries.
    for name in ["Shop.Core", "Shop.Data"] {
        main.update_in(cx, |m, w, cx| m.show_new_project(None, w, cx));
        let (handle, mut d) = dialog(&main, cx);
        field(&mut d, "field: Search templates", "Class Library");
        field(&mut d, "field: Project name", name);
        create(handle, &mut d);
        wait(cx, "the class library in the solution", |cx| {
            main.read_with(cx, |m, _| m.solution_rows().iter().any(|r| r.trim().split(" (").next() == Some(name)))
        });
    }

    // 4. Add, Reference: Shop to Shop.Core, Shop.Core to Shop.Data.
    for (from, to) in [("Shop", "Shop.Core"), ("Shop.Core", "Shop.Data")] {
        let row = main.read_with(cx, |m, _| {
            // A project's row is one level in; the solution, also Shop, is the first row.
            let rows = m.solution_rows();
            rows.iter()
                .position(|r| r.starts_with("  ") && !r.starts_with("   ") && r.trim().split(" (").next() == Some(from))
                .unwrap_or_else(|| panic!("no project row {from}: {rows:?}"))
        });
        main.update_in(cx, |m, w, cx| m.choose(row, "Add > Reference...", w, cx));
        cx.run_until_parked();
        let reference = main.read_with(cx, |m, _| m.reference_dialog()).expect("Add Reference did not open");
        let at = reference.read_with(cx, |d, _| d.choices()).iter().position(|(n, _, _)| n == to).unwrap();
        cx.update(|window, cx| window.focus(&reference.focus_handle(cx)));
        for _ in 0..at {
            cx.simulate_keystrokes("down");
        }
        cx.simulate_keystrokes("space");
        cx.simulate_keystrokes("enter");
        cx.run_until_parked();
    }

    // 5. The NuGet window: Demo.Core 2.0.0 installed in Shop.Core.
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-7" } else { "alt-7" });
    let search = nuget.read_with(cx, |v, _| v.search_field().clone());
    cx.update(|window, cx| window.focus(&search.focus_handle(cx)));
    cx.simulate_input("Demo.Core");
    cx.executor().advance_clock(Duration::from_millis(350));
    wait(cx, "the package's versions", |cx| nuget.read_with(cx, |v, _| v.versions().is_ok_and(|v| v.len() == 2)));
    assert_eq!(nuget.read_with(cx, |v, _| v.version().map(str::to_string)).as_deref(), Some("2.0.0"));
    click(cx, "nuget install: Shop.Core");
    wait(cx, "dotnet add package's end", |cx| !nuget.read_with(cx, |v, _| v.is_busy()));
    assert!(
        nuget.read_with(cx, |v, _| v.status().ends_with("done")),
        "Install failed: {}",
        nuget.read_with(cx, |v, _| v.log().to_string())
    );

    // Every file as Passenger's window made it.
    let (mine, theirs) = (files(&folder), files(&golden()));
    assert_eq!(mine, theirs, "RSharp made other files than Passenger");
    for file in theirs {
        let (made, expected) =
            (std::fs::read(folder.join(&file)).unwrap(), std::fs::read(golden().join(&file)).unwrap());
        if file.extension().is_some_and(|e| e == "sln") {
            assert_eq!(without_guids(&made), without_guids(&expected), "{} differs from Passenger's", file.display());
        } else {
            assert!(
                made == expected,
                "{} differs from Passenger's:\nRSharp's:\n{}\nPassenger's:\n{}",
                file.display(),
                String::from_utf8_lossy(&made),
                String::from_utf8_lossy(&expected)
            );
        }
    }
}
