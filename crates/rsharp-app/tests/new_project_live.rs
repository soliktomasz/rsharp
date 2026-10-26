//! Passenger's NewSolutionTests through real gpui input and the same SDK, with a private hive and local feed.
mod support;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, WindowHandle};
use rsharp_app::{new_project_dialog::NewProjectDialog, templates::Templates, window::MainWindow};
use rsharp_core::templates::{TemplateEnvironment, cache::Catalogs};
use std::{
    path::{Path, PathBuf},
    time::{Duration, Instant},
};
use support::Run;

fn feed() -> PathBuf {
    std::env::var_os("RSHARP_TEMPLATE_TEST_FEED")
        .map(PathBuf::from)
        .unwrap_or_else(|| Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/template-tests/feed"))
}
fn environment(run: &Run) -> Option<TemplateEnvironment> {
    let mut env = match TemplateEnvironment::discover() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("SKIP new_project_live: {e}");
            return None;
        }
    };
    let base = run.folder.parent().unwrap();
    env.cli_home = base.join("cli-home");
    env.hive = Some(base.join("hive"));
    env.nuget_packages = Some(base.join("packages"));
    let source = rsharp_core::paths::canonical(&feed());
    if source.is_dir() {
        let source = source.to_string_lossy().replace('&', "&amp;").replace('"', "&quot;");
        std::fs::write(run.folder.join("nuget.config"),format!("<configuration><packageSources><clear/><add key=\"local\" value=\"{source}\" /></packageSources></configuration>")).unwrap();
    }
    // Roslyn's Host restores in its own process; MSBuild keeps that restore in this run too.
    std::fs::write(
        run.folder.join("Directory.Build.props"),
        format!(
            "<Project><PropertyGroup><RestorePackagesPath>{}</RestorePackagesPath></PropertyGroup></Project>",
            env.nuget_packages.as_ref().unwrap().display()
        ),
    )
    .unwrap();
    Some(env)
}
fn enable(cx: &mut VisualTestContext, env: &TemplateEnvironment) {
    cx.update(|_, cx| {
        let global = cx.global_mut::<Templates>();
        *global.catalogs.lock().unwrap() = Some(Catalogs::new(env.clone()));
        global.enabled = true;
    });
}
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let b = cx.debug_bounds(selector).unwrap_or_else(|| panic!("missing {selector}"));
    cx.simulate_click(b.center(), Modifiers::default());
    cx.run_until_parked();
}
fn field(cx: &mut VisualTestContext, selector: &'static str, value: &str) {
    click(cx, selector);
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(value);
    cx.run_until_parked();
}
fn dialog(
    main: &Entity<MainWindow>,
    cx: &mut VisualTestContext,
) -> (WindowHandle<NewProjectDialog>, VisualTestContext) {
    cx.run_until_parked();
    let handle = main.read_with(cx, |m, _| m.new_project_dialog()).expect("creation dialog did not open");
    let dialog = VisualTestContext::from_window(handle.into(), cx);
    dialog.run_until_parked();
    let problem = handle.read_with(&dialog, |d, _| d.problem().map(str::to_string)).unwrap();
    assert_eq!(problem, None, "catalog load failed");
    assert!(handle.read_with(&dialog, |d, _| d.draft().authoritative).unwrap(), "catalog did not finish loading");
    (handle, dialog)
}
fn created(handle: WindowHandle<NewProjectDialog>, cx: &mut VisualTestContext) {
    click(cx, "button: Create");
    cx.run_until_parked();
    if let Ok(problem) = handle.read_with(cx, |d, _| d.problem().map(str::to_string)) {
        panic!("creation stayed open: {problem:?}");
    }
}
fn command(env: &TemplateEnvironment, args: &[&str], folder: &Path) -> String {
    let mut cmd = env.command(&args.iter().map(|s| (*s).into()).collect::<Vec<_>>(), folder);
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    use std::io::Read;
    let mut child = rsharp_core::process::spawn(cmd).unwrap();
    let mut stderr = child.process.stderr.take().unwrap();
    let error = std::thread::spawn(move || {
        let mut text = String::new();
        stderr.read_to_string(&mut text).unwrap();
        text
    });
    let mut stdout = String::new();
    child.process.stdout.take().unwrap().read_to_string(&mut stdout).unwrap();
    let status = child.wait().unwrap();
    let said = format!("{stdout}{}", error.join().unwrap());
    assert!(status.success(), "dotnet {args:?} failed: {said}");
    said
}
fn wait(cx: &mut VisualTestContext, what: &str, until: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        if until(cx) {
            return;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    panic!("{what} never happened");
}

#[gpui::test]
fn welcome_creates_restores_and_opens_then_grows_to_three_projects_with_references(cx: &mut TestAppContext) {
    let run = Run::new("live-new-solution", &[]);
    let Some(env) = environment(&run) else {
        return;
    };
    if !feed().is_dir() {
        eprintln!("SKIP three-project live test: run python tools/prepare-template-tests.py");
        return;
    }
    let (main, cx) = run.window(cx);
    enable(cx, &env);
    let roslyn = rsharp_lsp::roslyn_server().is_some();
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = roslyn);
    click(cx, "button: New Solution");
    let (handle, mut d) = dialog(&main, cx);
    field(&mut d, "field: Location", run.folder.to_str().unwrap());
    field(&mut d, "field: Solution name", "Greeter");
    click(&mut d, "option: UseProgramMain");
    let (framework, format) =
        handle.read_with(&d, |d, _| (d.draft().framework.clone().unwrap(), d.draft().solution_format.clone())).unwrap();
    created(handle, &mut d);
    cx.run_until_parked();
    let folder = run.folder.join("Greeter");
    let solution = folder.join(format!("Greeter.{format}"));
    let project = folder.join("Greeter/Greeter.csproj");
    assert_eq!(main.read_with(cx, |m, _| m.opened_as().map(Path::to_path_buf)), Some(solution.clone()));
    let program = std::fs::read(folder.join("Greeter/Program.cs")).unwrap();
    assert!(String::from_utf8_lossy(&program).contains("static void Main"));
    let expected = run.folder.join("expected-console");
    command(
        &env,
        &[
            "new",
            "console",
            "--name",
            "Greeter",
            "--output",
            expected.to_str().unwrap(),
            "--framework",
            &framework,
            "--use-program-main",
            "true",
            "--no-restore",
        ],
        &run.folder,
    );
    for name in ["Program.cs", "Greeter.csproj"] {
        assert_eq!(
            std::fs::read(folder.join("Greeter").join(name)).unwrap(),
            std::fs::read(expected.join(name)).unwrap(),
            "{name} differs from the same SDK/template/options used by Passenger"
        );
    }
    if roslyn {
        wait(cx, "initial Roslyn load", |cx| {
            main.read_with(cx, |m, _| m.server_ready() || m.server_failed().is_some())
        });
        assert_eq!(main.read_with(cx, |m, _| m.server_failed().map(str::to_string)), None);
    }
    for (name, query) in [("Greeter.Core", "classlib"), ("Greeter.Tests", "xunit")] {
        main.update_in(cx, |m, w, cx| m.show_new_project(None, w, cx));
        let (handle, mut d) = dialog(&main, cx);
        field(&mut d, "field: Project name", name);
        field(&mut d, "field: Search templates", query);
        created(handle, &mut d);
        cx.run_until_parked();
        assert!(
            main.read_with(cx, |m, _| m.selected_row().is_some_and(|r| r.trim().split(" (").next() == Some(name))),
            "new project {name} was not selected: {:?}",
            main.read_with(cx, |m, _| (m.selected_row(), m.solution_rows(), m.said().map(str::to_string)))
        );
    }
    assert_eq!(rsharp_core::solution::read(&solution).unwrap().projects().len(), 3);
    // The existing Add Reference window, driven by its own real keys.
    let row = main.read_with(cx, |m, _| {
        m.solution_rows().iter().position(|r| r.starts_with("  Greeter (") || r == "  Greeter").unwrap()
    });
    main.update_in(cx, |m, w, cx| m.choose(row, "Add > Reference...", w, cx));
    cx.run_until_parked();
    let reference = main.read_with(cx, |m, _| m.reference_dialog()).unwrap();
    let choices = reference.read_with(cx, |d, _| d.choices());
    let core = choices.iter().position(|(n, _, _)| n == "Greeter.Core").unwrap();
    for _ in 0..core {
        cx.simulate_keystrokes("down");
    }
    cx.simulate_keystrokes("space");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let original = std::fs::read(expected.join("Greeter.csproj")).unwrap();
    let original = String::from_utf8(original).unwrap();
    let newline = if original.contains("\r\n") { "\r\n" } else { "\n" };
    let insertion = format!(
        "  <ItemGroup>{newline}    <ProjectReference Include=\"..\\Greeter.Core\\Greeter.Core.csproj\" />{newline}  </ItemGroup>{newline}{newline}"
    );
    assert_eq!(
        std::fs::read(&project).unwrap(),
        original.replace("</Project>", &format!("{insertion}</Project>")).as_bytes(),
        "Add Reference differs from Passenger's preserving insertion"
    );
    command(&env, &["build", solution.to_str().unwrap()], &folder);
    if roslyn {
        cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-o" } else { "ctrl-t" });
        cx.simulate_input("Class1");
        wait(cx, "Roslyn finding a symbol in the added project", |cx| {
            main.read_with(cx, |m, cx| m.go_to_listed(cx)).unwrap_or_default().iter().any(|l| l.contains("Class1"))
        });
    } else {
        eprintln!("SKIP added-project Roslyn check: sidecars not installed");
    }
}

fn added(before: &[u8], after: &[u8]) -> Vec<String> {
    assert_eq!(before.starts_with(b"\xef\xbb\xbf"), after.starts_with(b"\xef\xbb\xbf"));
    let before = std::str::from_utf8(before).unwrap();
    let after = std::str::from_utf8(after).unwrap();
    let mut was = before.split_inclusive('\n').peekable();
    let mut extra = vec![];
    for line in after.split_inclusive('\n') {
        if was.peek() == Some(&line) {
            was.next();
        } else {
            extra.push(line.into());
        }
    }
    assert!(was.next().is_none(), "the solution lost or changed an existing byte");
    extra
}

#[gpui::test]
fn a_class_library_in_src_matches_passengers_added_lines_for_both_solution_formats(cx: &mut TestAppContext) {
    for format in ["slnx", "sln"] {
        let before = if format == "slnx" {
            include_bytes!("fixtures/new_project/Shop.slnx").as_slice()
        } else {
            include_bytes!("fixtures/new_project/Shop.sln").as_slice()
        };
        let run = Run::new(
            &format!("live-add-{format}"),
            &[
                (
                    "Shop.Web/Shop.Web.csproj",
                    "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>",
                ),
                (
                    "src/Shop.Core/Shop.Core.csproj",
                    "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>",
                ),
            ],
        );
        let Some(env) = environment(&run) else {
            return;
        };
        let solution = run.folder.join(format!("Shop.{format}"));
        std::fs::write(&solution, before).unwrap();
        let (main, cx) = run.window(cx);
        enable(cx, &env);
        main.update_in(cx, |m, w, cx| m.open_workspace(&solution, w, cx));
        cx.run_until_parked();
        let row = main.read_with(cx, |m, _| m.solution_rows().iter().position(|r| r.trim() == "src").unwrap());
        main.update_in(cx, |m, w, cx| m.choose(row, "Add > New Project...", w, cx));
        let (handle, mut d) = dialog(&main, cx);
        assert_eq!(handle.read_with(&d, |d, _| d.draft().directory.clone()).unwrap(), run.folder.join("src"));
        field(&mut d, "field: Project name", "Shop.Data");
        field(&mut d, "field: Search templates", "classlib");
        let framework = handle.read_with(&d, |d, _| d.draft().framework.clone().unwrap()).unwrap();
        created(handle, &mut d);
        cx.run_until_parked();
        let extra = added(before, &std::fs::read(&solution).unwrap());
        if format == "slnx" {
            assert_eq!(extra, ["    <Project Path=\"src/Shop.Data/Shop.Data.csproj\" />\n"]);
        } else {
            let id=regex::Regex::new(r#"^Project\("\{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC\}"\) = "Shop.Data", "src\\Shop.Data\\Shop.Data.csproj", "(\{[0-9A-F-]{36}\})"\r\n$"#).unwrap().captures(&extra[0]).unwrap()[1].to_string();
            assert_eq!(
                extra,
                vec![
                    extra[0].clone(),
                    "EndProject\r\n".into(),
                    format!("\t\t{id}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\r\n"),
                    format!("\t\t{id}.Debug|Any CPU.Build.0 = Debug|Any CPU\r\n"),
                    format!("\t\t{id}.Release|Any CPU.ActiveCfg = Release|Any CPU\r\n"),
                    format!("\t\t{id}.Release|Any CPU.Build.0 = Release|Any CPU\r\n"),
                    format!("\t\t{id} = {{B1000000-0000-0000-0000-000000000002}}\r\n")
                ]
            );
        }
        let expected = run.folder.join("expected");
        command(
            &env,
            &[
                "new",
                "classlib",
                "--name",
                "Shop.Data",
                "--output",
                expected.to_str().unwrap(),
                "--framework",
                &framework,
                "--no-restore",
            ],
            &run.folder,
        );
        for file in ["Shop.Data.csproj", "Class1.cs"] {
            assert_eq!(
                std::fs::read(run.folder.join("src/Shop.Data").join(file)).unwrap(),
                std::fs::read(expected.join(file)).unwrap()
            );
        }
        command(&env, &["build", solution.to_str().unwrap()], &run.folder);
    }
}

#[gpui::test]
fn avalonia_installs_from_a_local_feed_and_creates_a_buildable_app_without_accelerate(cx: &mut TestAppContext) {
    if !feed().join("avalonia.templates.12.1.4.nupkg").is_file() {
        eprintln!("SKIP Avalonia live test: run python tools/prepare-template-tests.py");
        return;
    }
    let run = Run::new("live-avalonia-install", &[]);
    let Some(env) = environment(&run) else {
        return;
    };
    let (main, cx) = run.window(cx);
    enable(cx, &env);
    click(cx, "button: New Solution");
    let (handle, mut d) = dialog(&main, cx);
    field(&mut d, "field: Location", run.folder.to_str().unwrap());
    field(&mut d, "field: Search templates", "avalonia.app");
    assert!(handle.read_with(&d, |d, _| d.draft().chosen().unwrap().pack.is_some()).unwrap());
    click(&mut d, "button: Install");
    d.run_until_parked();
    let problem = handle.read_with(&d, |d, _| d.problem().map(str::to_string)).unwrap();
    assert_eq!(problem, None);
    assert!(
        handle
            .read_with(&d, |d, _| d
                .draft()
                .chosen()
                .is_some_and(|t| t.short_name() == "avalonia.app" && t.pack.is_none()))
            .unwrap()
    );
    field(&mut d, "field: Solution name", "Gallery");
    created(handle, &mut d);
    cx.run_until_parked();
    let folder = run.folder.join("Gallery");
    let project = folder.join("Gallery/Gallery.csproj");
    let text = std::fs::read_to_string(&project).unwrap();
    assert!(!text.contains("AvaloniaUI.DiagnosticsSupport"));
    assert!(text.contains("Avalonia.Desktop"));
    assert!(!std::fs::read_to_string(folder.join("Gallery/Program.cs")).unwrap().contains("DeveloperTools"));
    let solution = main.read_with(cx, |m, _| m.opened_as().unwrap().to_path_buf());
    // A new solution builds once after opening; the test's own build waits for that one, or MSBuild writes the
    // same deps file twice.
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    wait(cx, "the new solution's own build", |cx| !main.read_with(cx, |m, _| m.is_building()));
    command(&env, &["build", solution.to_str().unwrap()], &folder);
}

#[gpui::test]
fn a_solution_template_contributes_every_project_once_and_is_not_offered_when_adding(cx: &mut TestAppContext) {
    let run = Run::new("live-solution-template", &[]);
    let Some(env) = environment(&run) else {
        return;
    };
    let template = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/new_project/whole");
    command(
        &env,
        &[
            "new",
            "install",
            template.to_str().unwrap(),
            "--debug:custom-hive",
            env.hive.as_ref().unwrap().to_str().unwrap(),
        ],
        &run.folder,
    );
    let (main, cx) = run.window(cx);
    enable(cx, &env);
    click(cx, "button: New Solution");
    let (handle, mut d) = dialog(&main, cx);
    field(&mut d, "field: Location", run.folder.to_str().unwrap());
    field(&mut d, "field: Search templates", "rsharp-test-solution");
    assert!(!handle.read_with(&d, |d, _| d.draft().asks_project_name()).unwrap());
    field(&mut d, "field: Solution name", "Whole");
    created(handle, &mut d);
    cx.run_until_parked();
    let solution = run.folder.join("Whole/Whole.slnx");
    let read = rsharp_core::solution::read(&solution).unwrap();
    assert_eq!(read.projects().len(), 2);
    assert_eq!(
        std::fs::read_to_string(&solution).unwrap(),
        "<Solution>\n  <Project Path=\"Whole.Core/Whole.Core.csproj\" />\n  <Project Path=\"Whole.Extra/Whole.Extra.csproj\" />\n</Solution>\n"
    );
    // A new solution builds once after opening; the test's own build waits for that one, or MSBuild writes the
    // same deps file twice.
    cx.executor().advance_clock(Duration::from_millis(300));
    cx.run_until_parked();
    wait(cx, "the new solution's own build", |cx| !main.read_with(cx, |m, _| m.is_building()));
    command(&env, &["build", solution.to_str().unwrap()], &run.folder);
    main.update_in(cx, |m, w, cx| m.show_new_project(None, w, cx));
    let (handle, _) = dialog(&main, cx);
    assert!(
        handle.read_with(cx, |d, _| d.draft().families.iter().flat_map(|f| &f.types).all(|t| !t.is_solution)).unwrap()
    );
}
