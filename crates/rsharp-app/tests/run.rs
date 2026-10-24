//! Running from the window: the folder's configurations, the program in the Run console, its browser, and Stop.

mod support;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::window::{MainWindow, RunStatus};
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use support::Run;

const EXE_PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>";

fn write(root: &Path, name: &str, text: &str) {
    let path = root.join(name);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A project that prints what it was given, then ends.
fn app(run: &Run, source: &str) {
    write(&run.folder, "App/App.csproj", EXE_PROJECT);
    write(&run.folder, "App/Program.cs", source);
}

/// A launch profile as the SDK writes one.
fn profile(run: &Run, settings: &str) {
    write(&run.folder, "App/Properties/launchSettings.json", settings);
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
        eprintln!("skipped: no installed SDK 10.0.1xx for the offline run fixture");
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

fn open<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    let folder = run.folder.clone();
    main.update_in(cx, |main, window, cx| main.open_workspace(&folder, window, cx));
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
    let deadline = Instant::now() + Duration::from_secs(120);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} did not complete within two minutes");
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn outcome(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Option<rsharp_app::window::RunOutcome> {
    main.read_with(cx, |main, _| main.run_outcome().cloned())
}

fn finished(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> rsharp_app::window::RunOutcome {
    wait(cx, "the run's end", |cx| main.read_with(cx, |main, _| !main.is_running() && main.run_outcome().is_some()));
    outcome(main, cx).unwrap()
}

fn output(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |main, cx| main.run_output(cx))
}

fn record_browser(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Arc<Mutex<Vec<String>>> {
    let opened = Arc::new(Mutex::new(Vec::new()));
    let recorder = opened.clone();
    main.update(cx, |main, _| {
        main.set_browser_opener(move |url| recorder.lock().unwrap().push(url));
    });
    opened
}

#[test]
fn a_console_program_runs_and_its_exit_code_is_said() {
    // Run builds what it launches and shows what the program wrote, as Rider's Run window does.
    let run = Run::new("run-console", &[]);
    app(&run, "System.Console.WriteLine(\"hello from App\");\nSystem.Console.Error.WriteLine(\"to error\");\n");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    let names: Vec<String> =
        main.read_with(cx, |main, _| main.run_configurations().iter().map(|c| c.name.clone()).collect());
    assert_eq!(names, ["App"], "a project with no launch profile is run as itself");
    cx.simulate_keystrokes("ctrl-f5");
    cx.run_until_parked();
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Exited(0), "{}\n{}", outcome.message, output(&main, cx));
    assert_eq!(outcome.message, "App exited with code 0");
    let shown = output(&main, cx);
    assert!(shown.contains("hello from App"), "the program's output is in the Run console:\n{shown}");
    assert!(shown.contains("to error"), "its standard error is there too:\n{shown}");
}

#[test]
fn a_launch_profiles_arguments_and_environment_reach_the_program() {
    // What the profile says is what runs: its arguments, its variables, and its application URL as ASP.NET Core's.
    let run = Run::new("run-profile", &[]);
    app(
        &run,
        "System.Console.WriteLine(string.Join(\" \", args));\n\
         System.Console.WriteLine($\"greeting={System.Environment.GetEnvironmentVariable(\"GREETING\")}\");\n\
         System.Console.WriteLine($\"urls={System.Environment.GetEnvironmentVariable(\"ASPNETCORE_URLS\")}\");\n",
    );
    profile(
        &run,
        r#"{"profiles":{"Args":{"commandName":"Project","commandLineArgs":"--seed 7 one","applicationUrl":"http://127.0.0.1:5099","environmentVariables":{"GREETING":"hi"}}}}"#,
    );
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    let names: Vec<String> =
        main.read_with(cx, |main, _| main.run_configurations().iter().map(|c| c.name.clone()).collect());
    assert_eq!(names, ["App: Args"], "a project is listed once per launch profile, as Rider lists it");
    cx.simulate_keystrokes("ctrl-f5");
    cx.run_until_parked();
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Exited(0), "{}\n{}", outcome.message, output(&main, cx));
    let shown = output(&main, cx);
    assert!(shown.contains("--seed 7 one"), "the profile's arguments reach the program:\n{shown}");
    assert!(shown.contains("greeting=hi"), "the profile's environment reaches it:\n{shown}");
    assert!(shown.contains("urls=http://127.0.0.1:5099"), "its application URL is ASP.NET Core's:\n{shown}");
}

#[test]
fn stop_ends_the_program() {
    // A program that does not end on its own ends with Stop, and the status bar says it was stopped.
    let run = Run::new("run-stop", &[]);
    app(&run, "System.Console.WriteLine(\"started\");\nSystem.Threading.Thread.Sleep(60000);\n");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the program's start", |cx| {
        main.read_with(cx, |main, cx| main.is_running() && main.run_output(cx).contains("started"))
    });
    // The Run window is Rider's: its own console, and Rerun, Stop and Clear on its toolbar.
    assert!(
        main.read_with(cx, |main, _| main.dock().is_open(rsharp_app::dock::Tool::Run)),
        "the Run window did not open with the run"
    );
    for button in ["run console", "run: Rerun", "run: Stop", "run: Clear"] {
        assert!(cx.debug_bounds(button).is_some(), "the Run window shows no {button}");
    }
    cx.simulate_keystrokes("shift-f5");
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Stopped, "{}\n{}", outcome.message, output(&main, cx));
    assert_eq!(outcome.message, "App stopped");
    assert!(!main.read_with(cx, |main, _| main.is_running()), "the program is still said to run after Stop");
}

#[test]
fn a_failed_build_starts_nothing() {
    // A build that fails before the launch is where the run ends: the program never starts.
    let run = Run::new("run-build-fails", &[]);
    app(&run, "System.Console.WriteLine(missing);\n");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("ctrl-f5");
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Refused, "{}", outcome.message);
    assert!(outcome.message.contains("did not start: its build failed"), "{}", outcome.message);
    assert!(!output(&main, cx).contains("hello"), "a program whose build failed has not run");
}

#[test]
fn the_browser_opens_once_the_program_listens() {
    // A web project's browser opens at its URL once something listens there, and only then.
    let run = Run::new("run-browser", &[]);
    app(&run, "System.Threading.Thread.Sleep(3000);\n");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    profile(
        &run,
        &format!(
            r#"{{"profiles":{{"web":{{"commandName":"Project","applicationUrl":"http://127.0.0.1:{port}","launchBrowser":true}}}}}}"#
        ),
    );
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    let opened = record_browser(&main, cx);
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the browser opening", |_| !opened.lock().unwrap().is_empty());
    assert_eq!(opened.lock().unwrap().as_slice(), [format!("http://127.0.0.1:{port}")]);
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Exited(0), "{}", outcome.message);
}

#[test]
fn the_selector_lists_the_configurations_and_keeps_the_one_chosen() {
    // Rider's toolbar widget: every configuration, the chosen one marked, and the choice kept with the folder.
    let run = Run::new("run-selector", &[("App/App.csproj", EXE_PROJECT)]);
    profile(
        &run,
        r#"{"profiles":{"http":{"commandName":"Project","applicationUrl":"http://localhost:5000"},
            "https":{"commandName":"Project","applicationUrl":"https://localhost:5001"}}}"#,
    );
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    let names: Vec<String> =
        main.read_with(cx, |main, _| main.run_configurations().iter().map(|c| c.name.clone()).collect());
    assert_eq!(names, ["App: http", "App: https"]);
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("App: http"), "the first configuration stands until another is chosen");
    click(cx, "run widget");
    click(cx, "run configuration: App: https");
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("App: https"), "the menu's choice is the one Run launches");

    let (main, cx) = open(&run, cx);
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("App: https"), "the choice is kept with the folder");
}

#[test]
fn leaving_the_folder_ends_the_program() {
    // A program does not outlive its workspace: closing the solution ends it, as Rider ends a run with its folder.
    let run = Run::new("run-leave", &[]);
    app(&run, "System.Console.WriteLine(\"started\");\nSystem.Threading.Thread.Sleep(60000);\n");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the program's start", |cx| {
        main.read_with(cx, |main, cx| main.is_running() && main.run_output(cx).contains("started"))
    });
    main.update_in(cx, |main, window, cx| main.close_solution(window, cx));
    cx.run_until_parked();
    assert!(!main.read_with(cx, |main, _| main.is_running()), "the program runs after its folder was closed");
    assert_eq!(main.read_with(cx, |main, cx| main.run_output(cx)), "", "the closed folder's console went with it");
}
