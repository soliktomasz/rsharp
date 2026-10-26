//! Building from the window, with output and diagnostics where the person can act on them.

mod support;

use gpui::{AppContext, Entity, Focusable, Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::console::OUTPUT_LIMIT;
use rsharp_app::dock::{Place, Tool};
use rsharp_app::settings::{self, BuildResults};
use rsharp_app::window::{
    BuildSelected, BuildStatus, CleanSelected, CleanSolution, CloseSolution, MainWindow, OpenTabInNewWindow,
    ProblemsTab, RebuildSelected, RebuildSolution,
};
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const GOOD_SOURCE: &str = "System.Console.WriteLine(\"hello\");\n";
const BAD_SOURCE: &str = "System.Console.WriteLine(missing);";
const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// Pin the installed SDK and make restore use only this fixture's package directory and no feeds.
fn offline(run: &Run) -> bool {
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("skipped: dotnet is not installed");
        return false;
    };
    let command = rsharp_core::dotnet::command(&dotnet, ["--list-sdks"], &run.folder);
    let mut listed = String::new();
    let result = rsharp_core::build::run(command, &std::sync::atomic::AtomicBool::new(false), |text, error| {
        if !error {
            listed.push_str(text);
        }
    });
    let sdk = listed.lines().find_map(|line| {
        line.split_whitespace().next().filter(|version| version.starts_with("10.0.1")).map(str::to_string)
    });
    let Some(sdk) = sdk.filter(|_| matches!(result, Ok(Some(0)))) else {
        eprintln!("skipped: no installed SDK 10.0.1xx for the offline build fixture");
        return false;
    };
    write(&run.folder, "global.json", &format!(r#"{{"sdk":{{"version":"{sdk}","rollForward":"disable"}}}}"#));
    write(
        &run.folder,
        "nuget.config",
        "<configuration><packageSources><clear /></packageSources><config><add key=\"globalPackagesFolder\" value=\".packages\" /></config></configuration>",
    );
    write(
        &run.folder,
        "Directory.Build.props",
        "<Project><PropertyGroup><RestorePackagesPath>$(MSBuildThisFileDirectory).packages</RestorePackagesPath><NuGetAudit>false</NuGetAudit></PropertyGroup></Project>",
    );
    true
}

fn open<'a>(run: &Run, path: &Path, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(path, window, cx));
    cx.run_until_parked();
    (main, cx)
}

fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

fn wait(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(45);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} did not complete within 45 seconds");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn pump(cx: &mut VisualTestContext, duration: Duration) {
    let deadline = Instant::now() + duration;
    while Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn finished(main: &Entity<MainWindow>, cx: &mut VisualTestContext, expected: BuildStatus) {
    wait(cx, "the build result", |cx| {
        main.read_with(cx, |main, _| !main.is_building() && main.build_outcome().is_some())
    });
    let outcome = main.read_with(cx, |main, _| main.build_outcome().unwrap().clone());
    assert_eq!(
        outcome.status,
        expected,
        "{}\n{}",
        outcome.message,
        main.read_with(cx, |main, cx| main.build_output(cx))
    );
}

fn start(cx: &mut VisualTestContext) {
    cx.simulate_keystrokes("secondary-shift-b");
    cx.run_until_parked();
}

fn gate(run: &Run) {
    let command = if cfg!(windows) {
        "powershell -NoProfile -Command &quot;while (-not (Test-Path release)) { Start-Sleep -Milliseconds 50 }; Set-Content gate-completed complete; Write-Output old-workspace-late-output&quot;"
    } else {
        "while [ ! -f release ]; do sleep 0.05; done; printf complete &gt; gate-completed; echo old-workspace-late-output"
    };
    write(
        &run.folder,
        "Directory.Build.targets",
        &format!(
            "<Project><Target Name=\"WaitForRelease\" BeforeTargets=\"BeforeBuild\"><WriteLinesToFile File=\"$(MSBuildProjectDirectory)/gate-started\" Lines=\"started\" Overwrite=\"true\" /><Message Text=\"Gate waiting\" Importance=\"high\" /><Exec Command=\"{command}\" /></Target></Project>"
        ),
    );
}

#[gpui::test]
fn build_is_reachable_by_name_and_explains_why_it_cannot_run_without_a_workspace(cx: &mut TestAppContext) {
    let run = Run::new("build-unavailable", &[]);
    let (main, cx) = run.window(cx);
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("build solution");
    cx.run_until_parked();
    let listed = main.read_with(cx, |m, cx| m.find_action_listed(cx)).unwrap();
    assert_eq!(listed.first(), Some(&"Build Solution"), "Find Action cannot find Build Solution: {listed:?}");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.said().map(str::to_string)).as_deref(),
        Some("Build Solution is not available here"),
        "Build without a workspace did not explain why it cannot run"
    );
}

#[gpui::test]
fn an_unsaved_compiler_error_builds_the_current_text_and_navigates_to_its_source_then_reruns(cx: &mut TestAppContext) {
    let run = Run::new("build-diagnostics", &[("App.csproj", PROJECT), ("Program.cs", GOOD_SOURCE)]);
    if !offline(&run) {
        return;
    }
    let source = run.folder.join("Program.cs");
    let (main, cx) = open(&run, &run.folder.join("App.csproj"), cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(BAD_SOURCE);
    cx.run_until_parked();
    assert!(
        main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).is_modified(cx)),
        "typing did not dirty the source editor"
    );
    assert_eq!(main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).text().to_string()), BAD_SOURCE);
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        GOOD_SOURCE,
        "the fixture was saved before Build could prove it saves"
    );
    start(cx);
    assert!(
        main.read_with(cx, |main, _| main.dock().is_open(Tool::Build)),
        "an explicit build did not open Build by default"
    );
    finished(&main, cx, BuildStatus::Failed);
    assert_eq!(
        std::fs::read_to_string(&source).unwrap(),
        BAD_SOURCE,
        "Build compiled stale disk text instead of saving the editor"
    );
    let problem = main
        .read_with(cx, |main, _| main.build_problems().iter().find(|problem| problem.code == "CS0103").cloned())
        .expect("the missing-name compiler error never reached Build Problems");
    assert_eq!(problem.path.as_deref(), Some(source.as_path()), "the compiler error points at another source file");

    click(cx, "stripe: Problems");
    click(cx, "problems tab: build");
    assert!(
        main.read_with(cx, |main, cx| main.problems_listed(ProblemsTab::Build, cx))
            .iter()
            .any(|line| line.contains("missing"))
    );
    click(cx, "build problem: 0");
    assert_eq!(main.read_with(cx, |main, cx| main.active_tab_name(cx)).as_deref(), Some("Program.cs"));
    let caret = main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).caret());
    assert_eq!(
        (caret.line, caret.utf16_column),
        (problem.line, problem.column),
        "clicking the build error opened the wrong position"
    );

    // Eviction and Clear affect the console, while the build's diagnostic set stays available.
    let build = main.read_with(cx, |main, _| main.build_view().clone());
    build.update(cx, |view, cx| view.append(&[("é\n".repeat(OUTPUT_LIMIT / 2), false)], cx));
    assert_eq!(
        main.read_with(cx, |main, _| main.build_problems().len()),
        1,
        "console eviction discarded a compiler error"
    );
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-0" } else { "alt-0" });
    click(cx, "build: Clear");
    assert!(main.read_with(cx, |main, cx| main.build_output(cx)).is_empty());
    assert_eq!(main.read_with(cx, |main, _| main.build_problems().len()), 1, "Clear discarded Build Problems");
    let editor = main.read_with(cx, |main, _| main.editor().unwrap());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(GOOD_SOURCE.trim_end());
    click(cx, "build: Rerun");
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(
        main.read_with(cx, |main, _| main.build_problems().is_empty()),
        "Rerun kept errors from the previous build"
    );
    assert!(
        run.folder.join("bin/Debug/net10.0/App.dll").is_file(),
        "the corrected console project produced no assembly"
    );
}

#[gpui::test]
fn the_build_console_keeps_a_megabyte_of_valid_text_and_supports_copy_without_editing(cx: &mut TestAppContext) {
    let run = Run::new("build-console", &[]);
    let (main, cx) = open(&run, &run.folder, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-0" } else { "alt-0" });
    let build = main.read_with(cx, |main, _| main.build_view().clone());
    build.update(cx, |view, cx| {
        view.append(&[(format!("discarded prefix\n{}tail", "é🚀\n".repeat(OUTPUT_LIMIT / 7 + 10)), true)], cx);
    });
    let text = main.read_with(cx, |main, cx| main.build_output(cx));
    assert!(text.len() <= OUTPUT_LIMIT && text.len() >= OUTPUT_LIMIT - 4, "console retained {} bytes", text.len());
    assert!(text.ends_with("tail") && !text.contains("discarded prefix"), "console eviction removed the newest output");
    let editor = build.read_with(cx, |view, _| view.editor().clone());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
    cx.simulate_keystrokes("secondary-a secondary-c");
    let copied = cx.update(|_, cx| cx.read_from_clipboard().and_then(|item| item.text())).unwrap_or_default();
    assert!(copied == text, "selected build output could not be copied ({} of {} bytes)", copied.len(), text.len());
    cx.simulate_input("must not be typed into build output");
    cx.simulate_keystrokes("backspace secondary-x secondary-v");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, cx| main.build_output(cx)) == text, "a read-only console accepted an edit");
    assert!(
        editor.read_with(cx, |editor, _| editor.text().to_string()) == text,
        "the console editor diverged from its output"
    );
}

#[gpui::test]
fn solution_and_selected_project_operations_build_rebuild_and_clean_only_their_targets(cx: &mut TestAppContext) {
    let run = Run::new("build-targets", &[]);
    if !offline(&run) {
        return;
    }
    write(
        &run.folder,
        "Shop.slnx",
        "<Solution><Folder Name=\"/Libraries/\"><Project Path=\"Core/Core.csproj\" /></Folder><Project Path=\"App/App.csproj\" /></Solution>",
    );
    write(
        &run.folder,
        "Core/Core.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>",
    );
    write(&run.folder, "Core/Class.cs", "public static class Value { public static string Text => \"core\"; }");
    write(
        &run.folder,
        "App/App.csproj",
        "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup><ItemGroup><ProjectReference Include=\"../Core/Core.csproj\" /></ItemGroup></Project>",
    );
    write(&run.folder, "App/Program.cs", "System.Console.WriteLine(Value.Text);");
    let (main, cx) = open(&run, &run.folder.join("Shop.slnx"), cx);
    let (app, core) =
        (run.folder.join("App/bin/Debug/net10.0/App.dll"), run.folder.join("Core/bin/Debug/net10.0/Core.dll"));
    start(cx);
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(app.is_file() && core.is_file(), "Build Solution did not produce both assemblies");
    write(&run.folder, "App/Program.cs", BAD_SOURCE);
    let library_folder = main
        .read_with(cx, |main, _| main.solution_rows().iter().position(|row| row.trim() == "Libraries"))
        .expect("the solution folder is missing");
    main.update(cx, |main, cx| main.toggle_row(library_folder, cx));
    cx.run_until_parked();
    let select = |name: &str, main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        let rows = main.read_with(cx, |main, _| main.solution_rows());
        let row = rows
            .iter()
            .position(|row| row.trim().split(" (").next() == Some(name))
            .unwrap_or_else(|| panic!("no solution row {name}: {rows:?}"));
        main.update(cx, |main, cx| main.select_row(row, cx));
        cx.run_until_parked();
    };
    select("Core", &main, cx);
    cx.dispatch_action(BuildSelected);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    cx.dispatch_action(RebuildSelected);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    cx.dispatch_action(CleanSelected);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(!core.exists() && app.exists(), "Clean Selected cleaned projects outside the selected project");
    select("Libraries", &main, cx);
    cx.dispatch_action(BuildSelected);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(core.exists(), "building the solution folder ignored its projects");
    write(&run.folder, "App/Program.cs", "System.Console.WriteLine(Value.Text);");
    cx.dispatch_action(RebuildSolution);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    cx.dispatch_action(CleanSolution);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(!app.exists() && !core.exists(), "Clean Solution left a project's assembly behind");
}

#[gpui::test]
fn an_sdk_web_project_builds_rebuilds_and_cleans_without_any_package_feed(cx: &mut TestAppContext) {
    let run = Run::new(
        "build-web",
        &[
            (
                "Web.csproj",
                "<Project Sdk=\"Microsoft.NET.Sdk.Web\"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>",
            ),
            ("Program.cs", "var app = Microsoft.AspNetCore.Builder.WebApplication.Create(args); app.Run();"),
        ],
    );
    if !offline(&run) {
        return;
    }
    let (main, cx) = open(&run, &run.folder.join("Web.csproj"), cx);
    let assembly = run.folder.join("bin/Debug/net10.0/Web.dll");
    start(cx);
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(assembly.is_file());
    cx.dispatch_action(RebuildSolution);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(assembly.is_file());
    cx.dispatch_action(CleanSolution);
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(!assembly.exists(), "Clean left the Web assembly behind");
}

#[gpui::test]
fn stop_ends_a_gated_build_and_its_children_and_a_new_build_can_run(cx: &mut TestAppContext) {
    let run = Run::new("build-stop", &[("App.csproj", PROJECT), ("Program.cs", GOOD_SOURCE)]);
    if !offline(&run) {
        return;
    }
    gate(&run);
    let (main, cx) = open(&run, &run.folder.join("App.csproj"), cx);
    start(cx);
    wait(cx, "the build's gate", |_| run.folder.join("gate-started").exists());
    assert!(main.read_with(cx, |main, _| main.is_building()));
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("build solution");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.is_building()), "an unavailable second Build replaced the active build");
    cx.simulate_keystrokes("shift-f5");
    cx.run_until_parked();
    finished(&main, cx, BuildStatus::Stopped);
    write(&run.folder, "release", "released");
    pump(cx, Duration::from_millis(200));
    assert!(!run.folder.join("gate-completed").exists(), "Stop left the build's shell alive to pass its gate");
    start(cx);
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(run.folder.join("gate-completed").exists(), "the next build could not pass the released gate");
}

#[gpui::test]
fn closing_or_switching_the_workspace_stops_its_build_and_discards_late_output(cx: &mut TestAppContext) {
    let run = Run::new("build-close", &[("App.csproj", PROJECT), ("Program.cs", GOOD_SOURCE)]);
    if !offline(&run) {
        return;
    }
    gate(&run);
    let (main, cx) = open(&run, &run.folder.join("App.csproj"), cx);
    start(cx);
    wait(cx, "the build's gate", |_| run.folder.join("gate-started").exists());
    cx.dispatch_action(CloseSolution);
    cx.run_until_parked();
    assert!(
        !main.read_with(cx, |main, _| main.has_workspace() || main.is_building()),
        "Close Solution left the build session active"
    );
    // The close's cancellation reaches the build's process tree on its own thread: give it its moment before the
    // gate is opened, or a child still being killed passes it under a loaded machine.
    pump(cx, Duration::from_secs(2));
    write(&run.folder, "release", "released");
    pump(cx, Duration::from_millis(350));
    assert!(!run.folder.join("gate-completed").exists(), "the closed workspace left its build child running");
    assert!(main.read_with(cx, |main, cx| main.build_output(cx)).is_empty(), "old output landed on the Welcome screen");
    assert!(
        main.read_with(cx, |main, _| main.build_outcome().is_none() && main.build_problems().is_empty()),
        "an old build outcome landed after Close Solution"
    );
    std::fs::remove_file(run.folder.join("release")).unwrap();
    std::fs::remove_file(run.folder.join("gate-started")).unwrap();
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder.join("App.csproj"), window, cx));
    cx.run_until_parked();
    start(cx);
    wait(cx, "the reopened build's gate", |_| run.folder.join("gate-started").exists());
    let next = Run::new("build-switch-destination", &[]);
    main.update_in(cx, |main, window, cx| main.open_workspace(&next.folder, window, cx));
    cx.run_until_parked();
    pump(cx, Duration::from_secs(2));
    write(&run.folder, "release", "released");
    pump(cx, Duration::from_millis(350));
    assert!(!run.folder.join("gate-completed").exists(), "switching workspaces left the previous build child running");
    assert!(main.read_with(cx, |main, cx| main.build_output(cx)).is_empty(), "old output landed in the new workspace");
    assert!(
        main.read_with(cx, |main, _| !main.is_building() && main.build_outcome().is_none()),
        "the new workspace inherited an old build result"
    );
}

#[gpui::test]
fn presentation_settings_control_explicit_opening_and_success_warning_and_error_results(cx: &mut TestAppContext) {
    let run = Run::new("build-presentation", &[("App.csproj", PROJECT), ("Program.cs", GOOD_SOURCE)]);
    if !offline(&run) {
        return;
    }
    write(
        &run.folder,
        "Directory.Build.targets",
        "<Project><Target Name=\"FixtureWarning\" BeforeTargets=\"BeforeBuild\"><Warning Condition=\"Exists('$(MSBuildProjectDirectory)/warn')\" Code=\"TEST0001\" Text=\"fixture warning\" /></Target></Project>",
    );
    let (main, cx) = open(&run, &run.folder.join("App.csproj"), cx);
    assert!(cx.update(|_, cx| settings::settings(cx).open_build_window));
    assert_eq!(cx.update(|_, cx| settings::settings(cx).build_results), BuildResults::Errors);
    for (mode, source, warning, shown, status) in [
        (BuildResults::Errors, GOOD_SOURCE, false, false, BuildStatus::Succeeded),
        (BuildResults::Always, GOOD_SOURCE, false, true, BuildStatus::Succeeded),
        (BuildResults::Never, BAD_SOURCE, false, false, BuildStatus::Failed),
        (BuildResults::Errors, BAD_SOURCE, false, true, BuildStatus::Failed),
        (BuildResults::WarningsOrErrors, GOOD_SOURCE, true, true, BuildStatus::Succeeded),
    ] {
        cx.update(|_, cx| {
            let mut changed = settings::settings(cx).clone();
            changed.open_build_window = false;
            changed.build_results = mode;
            settings::apply(changed, cx);
        });
        if main.read_with(cx, |main, _| main.dock().is_open(Tool::Build)) {
            cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-0" } else { "alt-0" });
        }
        write(&run.folder, "Program.cs", source);
        if warning {
            write(&run.folder, "warn", "warning");
        } else {
            let _ = std::fs::remove_file(run.folder.join("warn"));
        }
        start(cx);
        assert!(
            !main.read_with(cx, |main, _| main.dock().is_open(Tool::Build)),
            "explicit opening ignored its checkbox for {mode:?}"
        );
        finished(&main, cx, status);
        assert_eq!(
            main.read_with(cx, |main, _| main.dock().is_open(Tool::Build)),
            shown,
            "result presentation ignored {mode:?}"
        );
        if warning {
            assert!(
                main.read_with(cx, |main, _| main.build_problems().iter().any(|problem| problem.code == "TEST0001")),
                "the warning result had no warning"
            );
        }
    }
}

#[gpui::test]
fn a_failed_save_prevents_process_start_and_keeps_the_old_file(cx: &mut TestAppContext) {
    let run = Run::new("build-failed-save", &[("App.csproj", PROJECT)]);
    let source = run.folder.join("Program.cs");
    std::fs::write(&source, b"// caf\xe9\n").unwrap();
    let (main, cx) = open(&run, &run.folder.join("App.csproj"), cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_input("😀");
    cx.run_until_parked();
    assert!(
        main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).is_modified(cx)),
        "typing did not dirty the eight-bit source"
    );
    assert!(main.read_with(cx, |main, cx| main.editor().unwrap().read(cx).text().to_string()).contains("😀"));
    start(cx);
    finished(&main, cx, BuildStatus::NotStarted);
    assert!(!main.read_with(cx, |main, _| main.is_building()));
    let message = main.read_with(cx, |main, _| main.build_outcome().unwrap().message.clone());
    assert!(
        message.contains("save") && message.contains("windows-1252"),
        "save failure lost its explanation: {message}"
    );
    assert_eq!(std::fs::read(&source).unwrap(), b"// caf\xe9\n", "the failed save damaged the original source");
    assert!(
        main.read_with(cx, |main, cx| main.build_output(cx)).is_empty(),
        "a process started despite the failed save"
    );
}

#[gpui::test]
fn build_saves_a_dirty_workspace_buffer_that_exists_only_in_a_detached_editor(cx: &mut TestAppContext) {
    let run = Run::new("build-detached-save", &[("App.csproj", PROJECT), ("Program.cs", GOOD_SOURCE)]);
    if !offline(&run) {
        return;
    }
    let source = run.folder.join("Program.cs");
    let (main, cx) = open(&run, &run.folder.join("App.csproj"), cx);
    main.update_in(cx, |main, window, cx| main.open_file(&source, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.dispatch_action(OpenTabInNewWindow);
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.editor().is_none()), "the detached tab remained in the main window");
    let detached = cx
        .update(|_, cx| {
            cx.windows().into_iter().find_map(|window| window.downcast::<rsharp_app::editor_window::EditorWindow>())
        })
        .expect("Open Tab in New Window did not create an editor window");
    let mut other = VisualTestContext::from_window(detached.into(), cx);
    other.simulate_keystrokes("secondary-a");
    other.simulate_input(BAD_SOURCE);
    other.run_until_parked();
    assert_eq!(std::fs::read_to_string(&source).unwrap(), GOOD_SOURCE);
    start(cx);
    finished(&main, cx, BuildStatus::Failed);
    assert_eq!(std::fs::read_to_string(&source).unwrap(), BAD_SOURCE, "Build ignored the dirty detached editor");
    assert!(
        main.read_with(cx, |main, _| main.build_problems().iter().any(|problem| problem.code == "CS0103")),
        "Build compiled stale text from before detaching"
    );
}

#[gpui::test]
fn the_build_window_docks_at_the_bottom_and_its_visibility_is_kept_with_the_workspace(cx: &mut TestAppContext) {
    let run = Run::new("build-dock", &[]);
    let (main, cx) = open(&run, &run.folder, cx);
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-0" } else { "alt-0" });
    cx.run_until_parked();
    assert_eq!(main.read_with(cx, |main, _| main.dock().open_in(Place::BottomLeft)), Some(Tool::Build));
    let again = cx.update(|window, cx| cx.new(|cx| MainWindow::new(window, cx)));
    again.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    assert!(
        again.read_with(cx, |main, _| main.dock().is_open(Tool::Build)),
        "the workspace forgot the Build window was open"
    );
}

#[gpui::test]
fn a_new_solution_builds_once_it_opens_but_a_new_project_added_to_it_does_not(cx: &mut TestAppContext) {
    let solution = "<Solution><Project Path=\"App/App.csproj\" /></Solution>\n";
    let run = Run::new(
        "build-new-solution",
        &[("Made.slnx", solution), ("App/App.csproj", PROJECT), ("App/Program.cs", GOOD_SOURCE)],
    );
    if !offline(&run) {
        return;
    }
    write(&run.folder, "Lib/Lib.csproj", PROJECT);
    write(&run.folder, "Lib/Program.cs", GOOD_SOURCE);
    let (main, cx) = run.window(cx);
    let made = run.folder.join("Made.slnx");
    main.update(cx, |m, cx| m.attach_generated(None, Some(made.clone()), vec![run.folder.join("App/App.csproj")], cx))
        .unwrap();
    cx.run_until_parked();
    assert_eq!(main.read_with(cx, |m, _| m.opened_as().map(Path::to_path_buf)), Some(made.clone()));
    finished(&main, cx, BuildStatus::Succeeded);
    assert!(run.folder.join("App/bin").is_dir(), "the new solution was not built after it opened");

    let first = main.read_with(cx, |m, _| m.build_outcome().cloned()).unwrap();
    let target = rsharp_core::templates::ProjectTarget { solution: made, folder: None, projects: vec![] };
    main.update(cx, |m, cx| m.attach_generated(Some(target), None, vec![run.folder.join("Lib/Lib.csproj")], cx))
        .unwrap();
    pump(cx, Duration::from_millis(500));
    assert!(!main.read_with(cx, |m, _| m.is_building()), "adding a new project started a build");
    assert!(!run.folder.join("Lib/bin").exists(), "adding a new project built it");
    assert_eq!(main.read_with(cx, |m, _| m.build_outcome().map(|o| o.message.clone())), Some(first.message));
}
