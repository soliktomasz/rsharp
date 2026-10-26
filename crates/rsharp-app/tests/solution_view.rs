//! The Solution view as Rider's: a solution's folders and projects, each project's Dependencies and its files as
//! MSBuild evaluates them, nested by Rider's rules. Servers are off, so the folder's cache of evaluations is all the
//! view has, as on a window's first frame.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::state::store;
use rsharp_app::window::MainWindow;
use rsharp_core::project::{EvaluatedItem, EvaluatedProject, Evaluations, FrameworkDependencies, PackageDependency};
use std::path::Path;
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\" />\n";

const SLNX: &str = "<Solution>\n  <Folder Name=\"/src/\">\n    <Project Path=\"Web/Web.csproj\" />\n  </Folder>\n  \
    <Folder Name=\"/build/\" />\n  <Project Path=\"Api/Api.csproj\" />\n</Solution>\n";

fn files() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Shop.slnx", SLNX),
        ("Api/Api.csproj", PROJECT),
        ("Api/Program.cs", "class Program { }\n"),
        ("Api/Models/Order.cs", "class Order { }\n"),
        ("Api/appsettings.json", "{}\n"),
        ("Api/appsettings.Development.json", "{}\n"),
        ("Api/bin/Api.dll", ""),
        ("Core/Core.csproj", PROJECT),
        ("Web/Web.csproj", PROJECT),
        ("Web/Index.cs", "class Index { }\n"),
        ("Shared/Version.cs", "class Version { }\n"),
    ]
}

fn item(kind: &str, path: &Path) -> EvaluatedItem {
    EvaluatedItem { r#type: kind.into(), path: path.to_path_buf(), link: None, dependent_upon: None }
}

fn api(folder: &Path) -> EvaluatedProject {
    let root = folder.join("Api");
    EvaluatedProject {
        path: root.join("Api.csproj"),
        complete: true,
        target_frameworks: vec!["net10.0".into()],
        dependencies: vec![FrameworkDependencies {
            framework: None,
            packages: vec![
                PackageDependency { name: "Humanizer".into(), version: Some("2.14.1".into()), implicit: false },
                PackageDependency { name: "Microsoft.NET.ILLink.Tasks".into(), version: None, implicit: true },
            ],
            projects: vec![folder.join("Core/Core.csproj")],
            ..Default::default()
        }],
        items: vec![
            item("Compile", &root.join("Program.cs")),
            item("Compile", &root.join("Models/Order.cs")),
            item("Content", &root.join("appsettings.json")),
            item("Content", &root.join("appsettings.Development.json")),
            EvaluatedItem {
                link: Some("Properties/Version.cs".into()),
                ..item("Compile", &folder.join("Shared/Version.cs"))
            },
        ],
        ..Default::default()
    }
}

fn web(folder: &Path) -> EvaluatedProject {
    EvaluatedProject {
        path: folder.join("Web/Web.csproj"),
        complete: true,
        target_frameworks: vec!["net10.0".into()],
        // Index.cs is on disk but the project removes it.
        items: vec![],
        ..Default::default()
    }
}

/// A window on the run's folder, its cache of evaluations as given.
fn open<'a>(
    run: &Run,
    cx: &'a mut TestAppContext,
    evaluated: Vec<EvaluatedProject>,
) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    cx.update(|_, cx| store(cx).write_for(&folder, &Evaluations { projects: evaluated }));
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
    cx.run_until_parked();
    (main, cx)
}

fn rows(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Vec<String> {
    main.read_with(cx, |main, _| main.solution_rows())
}

/// Opens the row named `name` (its text without indent), as a click on its chevron does.
fn open_row(main: &Entity<MainWindow>, cx: &mut VisualTestContext, name: &str) {
    let index = rows(main, cx).iter().position(|r| r.trim_start() == name).unwrap_or_else(|| panic!("no row {name}"));
    main.update(cx, |main, cx| main.toggle_row(index, cx));
}

#[gpui::test]
fn a_solution_shows_its_folders_before_its_projects_each_by_name(cx: &mut TestAppContext) {
    let run = Run::new("sv-order", &files());
    let (main, cx) = open(&run, cx, vec![api(&run.folder)]);
    assert_eq!(
        rows(&main, cx),
        ["Shop", "  build", "  src", "  Api (net10.0)"],
        "the solution's folders would not come before its projects, each by name"
    );
    open_row(&main, cx, "src");
    assert_eq!(rows(&main, cx)[3], "    Web", "a solution folder would not hold its project");
}

#[gpui::test]
fn a_sln_shows_as_its_slnx_would(cx: &mut TestAppContext) {
    let sln = "Microsoft Visual Studio Solution File, Format Version 12.00\r\n\
        Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"Api\", \"Api\\Api.csproj\", \"{BBBBBBBB-0000-0000-0000-000000000001}\"\r\n\
        EndProject\r\n\
        Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"src\", \"src\", \"{AAAAAAAA-0000-0000-0000-000000000001}\"\r\n\
        EndProject\r\n";
    let mut files = files();
    files[0] = ("Shop.sln", sln);
    let run = Run::new("sv-sln", &files);
    let (main, cx) = open(&run, cx, vec![api(&run.folder)]);
    assert_eq!(rows(&main, cx), ["Shop", "  src", "  Api (net10.0)"], "a .sln would not show as Rider shows it");
}

#[gpui::test]
fn a_project_opens_to_its_dependencies_then_its_folders_then_its_files(cx: &mut TestAppContext) {
    let run = Run::new("sv-project", &files());
    let (main, cx) = open(&run, cx, vec![api(&run.folder)]);
    open_row(&main, cx, "Api (net10.0)");
    assert_eq!(
        rows(&main, cx)[4..],
        ["    Dependencies", "    Models", "    Properties", "    appsettings.json", "    Program.cs"],
        "the project would not list Dependencies, then its folders, then its files"
    );
    open_row(&main, cx, "Dependencies");
    assert_eq!(
        rows(&main, cx)[5..8],
        ["      net10.0", "      Core", "      Humanizer (2.14.1)"],
        "Dependencies would not list the framework, the project referenced and the package, without the SDK's own"
    );
}

#[gpui::test]
fn a_multi_targeted_projects_dependencies_are_by_framework(cx: &mut TestAppContext) {
    let run = Run::new("sv-multi", &files());
    let mut api = api(&run.folder);
    api.target_frameworks = vec!["net8.0".into(), "net10.0".into()];
    let mut net8 = api.dependencies[0].clone();
    net8.framework = Some("net8.0".into());
    net8.packages.clear();
    let mut net10 = api.dependencies[0].clone();
    net10.framework = Some("net10.0".into());
    api.dependencies = vec![net8, net10];
    let (main, cx) = open(&run, cx, vec![api]);
    open_row(&main, cx, "Api (net8.0, net10.0)");
    open_row(&main, cx, "Dependencies");
    assert_eq!(
        rows(&main, cx)[5..7],
        ["      net8.0", "      net10.0"],
        "each framework would not have its own dependencies"
    );
    open_row(&main, cx, "net10.0");
    assert_eq!(
        rows(&main, cx)[7..9],
        ["        Core", "        Humanizer (2.14.1)"],
        "a framework would not list its own"
    );
}

#[gpui::test]
fn a_linked_file_shows_at_its_link_and_an_excluded_one_not_at_all(cx: &mut TestAppContext) {
    let run = Run::new("sv-linked", &files());
    let (main, cx) = open(&run, cx, vec![api(&run.folder), web(&run.folder)]);
    open_row(&main, cx, "Api (net10.0)");
    open_row(&main, cx, "Properties");
    assert!(rows(&main, cx).contains(&"      Version.cs".to_string()), "a linked file would not show at its Link");
    open_row(&main, cx, "src");
    open_row(&main, cx, "Web (net10.0)");
    let rows = rows(&main, cx);
    let web = rows.iter().position(|r| r.trim_start() == "Web (net10.0)").unwrap();
    assert_eq!(rows[web + 1], "      Dependencies");
    assert!(!rows.iter().any(|r| r.trim_start() == "Index.cs"), "a file the project removes would show: {rows:?}");
}

#[gpui::test]
fn a_file_nests_under_the_one_it_belongs_to(cx: &mut TestAppContext) {
    let run = Run::new("sv-nesting", &files());
    let (main, cx) = open(&run, cx, vec![api(&run.folder)]);
    open_row(&main, cx, "Api (net10.0)");
    assert!(!rows(&main, cx).iter().any(|r| r.contains("Development")), "a nested file would show beside its parent");
    open_row(&main, cx, "appsettings.json");
    let rows = rows(&main, cx);
    let at = rows.iter().position(|r| r.trim_start() == "appsettings.json").unwrap();
    assert_eq!(rows[at + 1], "      appsettings.Development.json", "a nested file would not show under its parent");
}

#[gpui::test]
fn a_project_never_evaluated_shows_its_folders_files(cx: &mut TestAppContext) {
    let run = Run::new("sv-unevaluated", &files());
    let (main, cx) = open(&run, cx, vec![]);
    assert_eq!(rows(&main, cx)[3], "  Api", "a project not yet evaluated would name frameworks it does not know");
    open_row(&main, cx, "Api");
    assert_eq!(
        rows(&main, cx)[4..],
        ["    Dependencies", "    Models", "    appsettings.json", "    Program.cs"],
        "a project not yet evaluated would not show its folder's files, without its project file and bin"
    );
}

#[gpui::test]
fn a_missing_project_is_dimmed_and_empty(cx: &mut TestAppContext) {
    let run = Run::new("sv-missing", &[("Shop.slnx", "<Solution><Project Path=\"Gone/Gone.csproj\" /></Solution>")]);
    let (main, cx) = open(&run, cx, vec![]);
    assert_eq!(rows(&main, cx), ["Shop", "  Gone (not found)"], "a project not on disk would not say so");
    open_row(&main, cx, "Gone (not found)");
    assert_eq!(rows(&main, cx).len(), 2, "a project not on disk would open to something");
}

#[gpui::test]
fn a_project_that_did_not_evaluate_says_so(cx: &mut TestAppContext) {
    let run = Run::new("sv-failed", &files());
    let failed = EvaluatedProject {
        path: run.folder.join("Api/Api.csproj"),
        problem: Some("Api.csproj(3,5): error MSB4025: The project file could not be loaded.".into()),
        ..Default::default()
    };
    let (main, cx) = open(&run, cx, vec![failed]);
    assert_eq!(rows(&main, cx)[3], "  Api (load failed)", "a project that failed to load would not say so");
    open_row(&main, cx, "Api (load failed)");
    assert!(rows(&main, cx).contains(&"    Program.cs".to_string()), "its folder's files would not show");
}

#[gpui::test]
fn a_folder_without_a_solution_shows_as_before(cx: &mut TestAppContext) {
    let run = Run::new("sv-folder", &[("src/A.cs", "class A { }\n"), ("README.md", "# A\n")]);
    let (main, cx) = open(&run, cx, vec![]);
    let name = run.folder.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(rows(&main, cx), [name, "  src".into(), "  README.md".into()], "a folder's own view would change");
}

#[gpui::test]
fn double_clicking_a_file_row_opens_it_in_the_editor(cx: &mut TestAppContext) {
    let run = Run::new("sv-open", &files());
    let (main, cx) = open(&run, cx, vec![api(&run.folder)]);
    open_row(&main, cx, "Api (net10.0)");
    let at = rows(&main, cx).iter().position(|r| r.trim_start() == "Program.cs").unwrap();
    main.update_in(cx, |main, window, cx| main.activate_row(at, window, cx));
    cx.run_until_parked();
    assert_eq!(main.read_with(cx, |main, cx| main.active_tab_name(cx)).as_deref(), Some("Program.cs"));
}

/// A window on the run's folder with the Host on, once every project's evaluation has landed; none, saying why,
/// without Passenger's sidecars.
fn evaluated<'a>(run: &Run, cx: &'a mut TestAppContext) -> Option<(Entity<MainWindow>, &'a mut VisualTestContext)> {
    if rsharp_lsp::SharedHost::carried(&run.folder).is_none() {
        eprintln!("skipped: Passenger's sidecars were not built (no dotnet or no vendor/passenger at build time)");
        return None;
    }
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
    settle(&main, cx);
    Some((main, cx))
}

/// Waits, in real time, for the Host's evaluations to land; they run on threads of their own.
fn settle(main: &Entity<MainWindow>, cx: &mut VisualTestContext) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(120);
    loop {
        cx.run_until_parked();
        if !main.read_with(cx, |m, _| m.evaluating()) {
            return;
        }
        assert!(std::time::Instant::now() < deadline, "the projects were never evaluated");
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
}

const EVALUATED: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    \
    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    \
    <Compile Remove=\"Old.cs\" />\n    <Compile Include=\"../Shared/Version.cs\" Link=\"Properties/Version.cs\" />\n  \
    </ItemGroup>\n</Project>\n";

#[gpui::test]
fn a_solution_opened_shows_each_projects_files_as_msbuild_evaluates_them(cx: &mut TestAppContext) {
    let run = Run::new(
        "sv-evaluated",
        &[
            ("Shop.slnx", "<Solution>\n  <Project Path=\"Api/Api.csproj\" />\n</Solution>\n"),
            ("Api/Api.csproj", EVALUATED),
            ("Api/Program.cs", "class Program { }\n"),
            ("Api/Old.cs", "class Old { }\n"),
            ("Shared/Version.cs", "class Version { }\n"),
        ],
    );
    let Some((main, cx)) = evaluated(&run, cx) else { return };
    assert_eq!(rows(&main, cx)[1], "  Api (net10.0)", "the project would not show its framework once evaluated");
    open_row(&main, cx, "Api (net10.0)");
    open_row(&main, cx, "Properties");
    let rows = rows(&main, cx);
    assert!(!rows.iter().any(|r| r.trim_start() == "Old.cs"), "a file the project removes would show: {rows:?}");
    assert!(rows.contains(&"      Version.cs".to_string()), "a linked file would not show at its Link: {rows:?}");
    let kept: rsharp_core::project::Evaluations = cx.update(|_, cx| store(cx).read_for(&run.folder));
    assert_eq!(kept.projects.len(), 1, "the evaluation would not be kept for the next start");
}

#[gpui::test]
fn a_project_file_saved_in_the_ide_shows_what_it_says_now(cx: &mut TestAppContext) {
    let run = Run::new(
        "sv-saved",
        &[
            ("Shop.slnx", "<Solution>\n  <Project Path=\"Api/Api.csproj\" />\n</Solution>\n"),
            ("Api/Api.csproj", EVALUATED),
            ("Api/Program.cs", "class Program { }\n"),
            ("Shared/Version.cs", "class Version { }\n"),
        ],
    );
    let Some((main, cx)) = evaluated(&run, cx) else { return };
    let project = run.folder.join("Api/Api.csproj");
    main.update_in(cx, |main, window, cx| main.open_file(&project, Some((5, 0)), window, cx));
    cx.simulate_input("    <PackageReference Include=\"Humanizer\" Version=\"2.14.1\" />\n");
    cx.simulate_keystrokes("secondary-s");
    settle(&main, cx);
    open_row(&main, cx, "Api (net10.0)");
    open_row(&main, cx, "Dependencies");
    assert!(
        rows(&main, cx).contains(&"      Humanizer (2.14.1)".to_string()),
        "a package added and saved would not show: {:?}",
        rows(&main, cx)
    );
}

#[gpui::test]
fn a_project_that_does_not_evaluate_says_so_and_the_rest_still_load(cx: &mut TestAppContext) {
    let run = Run::new(
        "sv-broken",
        &[
            (
                "Shop.slnx",
                "<Solution>\n  <Project Path=\"Api/Api.csproj\" />\n  <Project Path=\"Bad/Bad.csproj\" />\n</Solution>\n",
            ),
            ("Api/Api.csproj", EVALUATED),
            ("Api/Program.cs", "class Program { }\n"),
            ("Bad/Bad.csproj", "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n"),
            ("Bad/Thing.cs", "class Thing { }\n"),
            ("Shared/Version.cs", "class Version { }\n"),
        ],
    );
    let Some((main, cx)) = evaluated(&run, cx) else { return };
    assert_eq!(rows(&main, cx)[1..], ["  Api (net10.0)", "  Bad (load failed)"], "a broken project would not say so");
    open_row(&main, cx, "Bad (load failed)");
    assert!(rows(&main, cx).contains(&"    Thing.cs".to_string()), "a broken project would not show its folder");
}

#[gpui::test]
fn the_selection_stays_on_its_row_as_an_evaluation_lands_above_it(cx: &mut TestAppContext) {
    let run = Run::new("sv-selection", &files());
    let (main, cx) = open(&run, cx, vec![]);
    for row in ["src", "Web", "Api"] {
        open_row(&main, cx, row);
    }
    let at = rows(&main, cx).iter().position(|r| r.trim_start() == "Program.cs").unwrap();
    main.update(cx, |main, cx| main.select_row(at, cx));
    let mut web = web(&run.folder);
    web.items = ["A.cs", "B.cs", "C.cs"].iter().map(|f| item("Compile", &run.folder.join("Web").join(f))).collect();
    main.update(cx, |main, cx| main.set_evaluated(web, cx));
    assert_eq!(
        main.read_with(cx, |main, _| main.selected_row()).as_deref().map(str::trim_start),
        Some("Program.cs"),
        "an evaluation landing above the selected row would move the selection to another file"
    );
}

#[gpui::test]
fn a_host_that_cannot_load_an_sdk_says_why_on_the_solution(cx: &mut TestAppContext) {
    let run = Run::new(
        "sv-no-sdk",
        &[
            ("global.json", "{ \"sdk\": { \"version\": \"1.0.100\", \"rollForward\": \"disable\" } }\n"),
            (
                "Shop.slnx",
                "<Solution>\n  <Project Path=\"Api/Api.csproj\" />\n  <Project Path=\"Web/Web.csproj\" />\n</Solution>\n",
            ),
            ("Api/Api.csproj", EVALUATED),
            ("Web/Web.csproj", EVALUATED),
        ],
    );
    let Some((main, cx)) = evaluated(&run, cx) else { return };
    assert_eq!(
        rows(&main, cx)[0],
        "Shop (projects not evaluated)",
        "the Solution view would not say its projects never loaded"
    );
    let tip = main.read_with(cx, |main, _| main.solution_row_tip(0));
    assert!(tip.as_deref().is_some_and(|t| t.contains("SDK")), "the reason would not be given: {tip:?}");
}
