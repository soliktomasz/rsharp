//! Unit Tests follows source before a build and the same shared buffers as the editor.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use rsharp_core::project::EvaluatedProject;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><TargetFramework>net10.0</TargetFramework><IsTestProject>true</IsTestProject></PropertyGroup></Project>";
const SOURCE: &str = "namespace Shop;\npublic class Orders\n{\n    [Fact]\n    public void Keeps() { }\n}\n";

fn open<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.set_evaluated(
            EvaluatedProject {
                path: run.folder.join("Tests/Tests.csproj"),
                is_test_project: true,
                ..Default::default()
            },
            cx,
        );
    });
    cx.run_until_parked();
    (main, cx)
}

fn names(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Vec<String> {
    cx.executor().advance_clock(Duration::from_millis(200));
    cx.run_until_parked();
    main.read_with(cx, |main, cx| {
        main.test_view().read(cx).tests().iter().map(|test| test.fully_qualified_name.clone()).collect()
    })
}

fn fixture(name: &str) -> Run {
    Run::new(
        name,
        &[
            ("Shop.slnx", "<Solution><Project Path=\"Tests/Tests.csproj\" /></Solution>"),
            ("Tests/Tests.csproj", PROJECT),
            ("Tests/Orders.cs", SOURCE),
        ],
    )
}

#[test]
fn tests_are_listed_before_any_build_and_open_from_find_action() {
    let run = fixture("unit-tests-source");
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    assert_eq!(names(&main, cx), ["Shop.Orders.Keeps"], "Unit Tests needs no build to show source tests");
    assert!(main.read_with(cx, |main, _| main.build_outcome().is_none()));
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("unit tests window");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.dock().is_open(rsharp_app::dock::Tool::UnitTests)));
    assert!(cx.debug_bounds("unit tests").is_some(), "the action opened no Unit Tests content");
}

#[test]
fn unsaved_test_edits_replace_disk_discovery_and_leaving_clears_tests() {
    let run = fixture("unit-tests-unsaved");
    let can_build = offline(&run);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Tests/Orders.cs"), None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input(&SOURCE.replace("Keeps", "Changes"));
    cx.run_until_parked();
    assert_eq!(names(&main, cx), ["Shop.Orders.Changes"], "the tree still shows the saved method after typing");
    assert_eq!(std::fs::read_to_string(run.folder.join("Tests/Orders.cs")).unwrap(), SOURCE);
    cx.simulate_keystrokes("secondary-w");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, cx| main.active_tab_name(cx).is_none()), "Close Tab did not close the editor");
    wait(cx, "disk discovery after the last editor closes", |cx| names(&main, cx) == ["Shop.Orders.Keeps"]);
    assert_eq!(names(&main, cx), ["Shop.Orders.Keeps"], "closing an unsaved buffer did not restore its disk tests");
    if can_build {
        main.update(cx, |main, cx| main.run_tests(None, cx));
        wait(cx, "the build after closing unsaved tests", |cx| !main.read_with(cx, |main, _| main.is_testing()));
        assert_eq!(
            std::fs::read_to_string(run.folder.join("Tests/Orders.cs")).unwrap(),
            SOURCE,
            "the test build saved discarded edits from a closed view"
        );
    }
    main.update_in(cx, |main, window, cx| main.close_solution(window, cx));
    cx.run_until_parked();
    assert!(names(&main, cx).is_empty(), "the closed workspace left its tests behind");
}

fn offline(run: &Run) -> bool {
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("skipped: dotnet is not installed");
        return false;
    };
    let mut sdks = String::new();
    let _ = rsharp_core::build::run(
        rsharp_core::dotnet::command(&dotnet, ["--list-sdks"], &run.folder),
        &std::sync::atomic::AtomicBool::new(false),
        |text, _| sdks.push_str(text),
    );
    let Some(sdk) = sdks.lines().filter_map(|line| line.split_whitespace().next()).find(|s| s.starts_with("10.0.1"))
    else {
        eprintln!("skipped: SDK 10.0.1xx is not installed");
        return false;
    };
    std::fs::write(
        run.folder.join("global.json"),
        format!(r#"{{"sdk":{{"version":"{sdk}","rollForward":"disable"}}}}"#),
    )
    .unwrap();
    std::fs::write(
        run.folder.join("nuget.config"),
        "<configuration><packageSources><clear /></packageSources></configuration>",
    )
    .unwrap();
    true
}

fn wait(cx: &mut VisualTestContext, what: &str, done: impl Fn(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(60);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} did not complete within a minute");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn running_tests_saves_the_editor_and_a_failed_build_starts_no_tests() {
    let run = fixture("unit-tests-build-fails");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("Tests/Orders.cs"), None, window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    let changed = SOURCE.replace("Keeps", "SavedBeforeBuilding");
    cx.simulate_input(&changed);
    cx.run_until_parked();
    main.update(cx, |main, cx| main.run_tests(None, cx));
    wait(cx, "the test build's failure", |cx| !main.read_with(cx, |main, _| main.is_testing()));
    assert_eq!(std::fs::read_to_string(run.folder.join("Tests/Orders.cs")).unwrap(), changed);
    assert_eq!(
        main.read_with(cx, |main, _| main.build_outcome().map(|o| o.status)),
        Some(rsharp_app::window::BuildStatus::Failed)
    );
    let status = main.read_with(cx, |main, cx| main.test_view().read(cx).status().to_string());
    assert!(status.contains("build failed"), "Unit Tests did not explain why nothing ran: {status}");
}

#[test]
fn stop_during_the_test_build_ends_testing() {
    let run = fixture("unit-tests-stop-build");
    if !offline(&run) {
        return;
    }
    let gate =
        if cfg!(windows) { "powershell -NoProfile -Command &quot;Start-Sleep -Seconds 60&quot;" } else { "sleep 60" };
    let project = PROJECT.replace(
        "</Project>",
        &format!(
            "<Target Name=\"WaitForTests\" BeforeTargets=\"Build\"><Exec Command=\"{gate}\" /></Target></Project>"
        ),
    );
    std::fs::write(run.folder.join("Tests/Tests.csproj"), project).unwrap();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update(cx, |main, cx| main.run_tests(None, cx));
    wait(cx, "the test build's start", |cx| main.read_with(cx, |main, _| main.is_building()));
    cx.simulate_keystrokes("shift-f5");
    wait(cx, "Stop during test build", |cx| !main.read_with(cx, |main, _| main.is_testing() || main.is_building()));
    assert_eq!(main.read_with(cx, |main, cx| main.test_view().read(cx).status().to_string()), "Tests stopped");
}

#[test]
fn the_window_builds_and_runs_real_xunit_v3_tests_then_reruns_only_failures() {
    let packages = std::env::var_os("NUGET_PACKAGES").map(std::path::PathBuf::from).or_else(|| {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(|home| std::path::PathBuf::from(home).join(".nuget/packages"))
    });
    if !packages.is_some_and(|root| root.join("xunit.v3/3.2.2").is_dir()) {
        eprintln!("skipped: xunit.v3 3.2.2 is not cached for the offline window test");
        return;
    }
    let run = fixture("unit-tests-real");
    if !offline(&run) {
        return;
    }
    std::fs::write(run.folder.join("Tests/Tests.csproj"), PROJECT.replace("</PropertyGroup>", "<OutputType>Exe</OutputType><NuGetAudit>false</NuGetAudit></PropertyGroup><ItemGroup><PackageReference Include=\"xunit.v3\" Version=\"3.2.2\" /></ItemGroup>")).unwrap();
    std::fs::write(run.folder.join("Tests/Orders.cs"), "using Xunit;\nnamespace Shop;\npublic class Orders {\n [Fact] public void Keeps() { }\n [Theory, InlineData(1), InlineData(2)] public void Rows(int value) => Assert.True(value > 0);\n [Fact] public void Fails() => Assert.Fail(\"the expected failure\");\n [Fact(Skip = \"later\")] public void Skips() { }\n}\n").unwrap();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    assert_eq!(names(&main, cx).len(), 4, "source discovery should list methods before building");
    main.update(cx, |main, cx| main.run_tests(None, cx));
    wait(cx, "the xUnit test run", |cx| !main.read_with(cx, |main, _| main.is_testing()));
    let view = main.read_with(cx, |main, _| main.test_view().clone());
    let status = view.read_with(cx, |v, _| v.status().to_string());
    assert!(status.starts_with("3 passed, 1 failed, 1 skipped"), "the real test run did not complete: {status}");
    assert_eq!(view.read_with(cx, |v, _| v.tests().len()), 5, "the theory's rows were lost or source was duplicated");
    let failed = view.read_with(cx, |v, _| v.failed_tests());
    assert_eq!(failed.len(), 1);
    main.update(cx, |main, cx| main.run_tests(Some(failed), cx));
    wait(cx, "rerunning failed tests", |cx| !main.read_with(cx, |main, _| main.is_testing()));
    let status = view.read_with(cx, |v, _| v.status().to_string());
    assert!(status.starts_with("0 passed, 1 failed, 0 skipped"), "Run Failed ran the wrong tests: {status}");
}

#[test]
fn grouping_and_outcome_filters_are_kept_with_the_folder() {
    use rsharp_app::test_view::GroupLevel;
    use rsharp_core::testing::TestState;
    let run = fixture("unit-tests-state");
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    let view = main.read_with(cx, |main, _| main.test_view().clone());
    view.update(cx, |view, cx| {
        view.grouping(vec![GroupLevel::Category, GroupLevel::TargetFramework], cx);
        view.filter(vec![TestState::Failed], cx);
    });
    main.update_in(cx, |main, window, cx| main.close_solution(window, cx));
    cx.run_until_parked();
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.run_until_parked();
    let state = view.read_with(cx, |view, _| view.state());
    assert_eq!(state.grouping, [GroupLevel::TargetFramework, GroupLevel::Category]);
    assert_eq!(state.outcomes, [TestState::Failed]);
}

#[test]
fn a_test_build_saves_a_linked_source_file_outside_the_workspace() {
    let run = fixture("unit-tests-linked-save");
    if !offline(&run) {
        return;
    }
    let linked = run.folder.parent().unwrap().join("linked/Shared.cs");
    std::fs::create_dir_all(linked.parent().unwrap()).unwrap();
    std::fs::write(&linked, SOURCE).unwrap();
    std::fs::write(
        run.folder.join("Tests/Tests.csproj"),
        PROJECT
            .replace("</Project>", "<ItemGroup><Compile Include=\"../../linked/Shared.cs\" /></ItemGroup></Project>"),
    )
    .unwrap();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| {
        main.set_evaluated(
            EvaluatedProject {
                path: run.folder.join("Tests/Tests.csproj"),
                is_test_project: true,
                items: vec![rsharp_core::project::EvaluatedItem {
                    r#type: "Compile".into(),
                    path: linked.clone(),
                    ..Default::default()
                }],
                ..Default::default()
            },
            cx,
        );
        main.open_file(&linked, None, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    let changed = SOURCE.replace("Keeps", "SavesLinkedChanges");
    cx.simulate_input(&changed);
    cx.run_until_parked();
    main.update(cx, |main, cx| main.run_tests(None, cx));
    wait(cx, "the linked test build", |cx| !main.read_with(cx, |main, _| main.is_testing()));
    assert_eq!(std::fs::read_to_string(&linked).unwrap(), changed, "tests ran the linked file's old saved bytes");
}

#[test]
fn run_unit_tests_uses_the_active_files_project_and_the_solution_selection() {
    let run = fixture("unit-tests-context-project");
    if !offline(&run) {
        return;
    }
    std::fs::create_dir_all(run.folder.join("Other")).unwrap();
    std::fs::write(run.folder.join("Other/Other.csproj"), PROJECT).unwrap();
    std::fs::write(run.folder.join("Other/Orders.cs"), SOURCE.replace("{ }", "{ missing_other_project(); }")).unwrap();
    std::fs::write(
        run.folder.join("Shop.slnx"),
        "<Solution><Project Path=\"Tests/Tests.csproj\" /><Project Path=\"Other/Other.csproj\" /></Solution>",
    )
    .unwrap();
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    main.update_in(cx, |main, window, cx| {
        main.set_evaluated(
            EvaluatedProject {
                path: run.folder.join("Other/Other.csproj"),
                is_test_project: true,
                ..Default::default()
            },
            cx,
        );
        main.open_file(&run.folder.join("Tests/Orders.cs"), Some((4, 0)), window, cx);
    });
    cx.run_until_parked();
    assert_eq!(names(&main, cx).len(), 2);
    cx.simulate_keystrokes("secondary-u r");
    cx.run_until_parked();
    wait(cx, "the context test build", |cx| !main.read_with(cx, |main, _| main.is_testing()));
    assert!(
        !main.read_with(cx, |main, cx| main.build_output(cx)).contains("Other/Orders.cs"),
        "the editor's context built a different project's same-named test"
    );
    main.update_in(cx, |main, window, cx| {
        let index = main.solution_rows().iter().position(|row| row.contains("Other")).unwrap();
        main.select_row(index, cx);
        main.toggle_tool(rsharp_app::dock::Tool::Solution, window, cx);
        main.toggle_tool(rsharp_app::dock::Tool::Solution, window, cx);
    });
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-u r");
    cx.run_until_parked();
    wait(cx, "the selected project's test build", |cx| !main.read_with(cx, |main, _| main.is_testing()));
    let output = main.read_with(cx, |main, cx| main.build_output(cx));
    assert!(
        output.contains("Other/Orders.cs") && !output.contains("Tests/Orders.cs"),
        "Run Unit Tests ignored the Solution selection: {output}"
    );
}

#[test]
fn a_closed_files_change_is_read_again_and_an_old_workspace_cannot_replace_the_new_one() {
    let run = fixture("unit-tests-refresh");
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    std::fs::write(run.folder.join("Tests/Orders.cs"), SOURCE.replace("Keeps", "ChangedOnDisk")).unwrap();
    main.update(cx, |main, cx| main.refresh_tests(cx));
    cx.run_until_parked();
    assert_eq!(names(&main, cx), ["Shop.Orders.ChangedOnDisk"]);
    let other = Run::new("unit-tests-empty", &[("Readme.txt", "no tests")]);
    main.update_in(cx, |main, window, cx| {
        main.refresh_tests(cx);
        main.open_workspace(&other.folder, window, cx);
    });
    cx.run_until_parked();
    assert!(names(&main, cx).is_empty(), "a stale source scan populated the next workspace");
}

#[test]
fn opening_a_test_project_directly_discovers_tests_without_a_solution_or_build() {
    let run = fixture("unit-tests-project-only");
    if !offline(&run) {
        return;
    }
    if rsharp_lsp::SharedHost::carried(&run.folder).is_none() {
        eprintln!("skipped: Passenger's project Host was not built");
        return;
    }
    std::fs::remove_file(run.folder.join("Shop.slnx")).unwrap();
    let mut cx = TestAppContext::single();
    let (main, cx) = run.window(&mut cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder.join("Tests/Tests.csproj"), window, cx));
    cx.run_until_parked();
    wait(cx, "source tests in the directly opened project", |cx| names(&main, cx) == ["Shop.Orders.Keeps"]);
    assert!(main.read_with(cx, |main, _| main.build_outcome().is_none()), "source discovery built the project");
}
