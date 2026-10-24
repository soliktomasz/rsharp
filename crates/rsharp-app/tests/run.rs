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

/// Drives the window until `done` or the deadline, saying whether it came true.
fn pump_until(cx: &mut VisualTestContext, deadline: Instant, done: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    while Instant::now() < deadline {
        if done(cx) {
            return true;
        }
        cx.executor().advance_clock(Duration::from_millis(100));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
    done(cx)
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
fn stop_during_the_build_before_a_launch_ends_the_run_as_stopped() {
    // Run builds first: Stop then ends that build, and the run is over rather than waiting on a build forever.
    let run = Run::new("run-stop-during-build", &[]);
    app(&run, "System.Console.WriteLine(\"started\");\nSystem.Threading.Thread.Sleep(60000);\n");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the build before the launch", |cx| main.read_with(cx, |main, _| main.is_building()));
    cx.simulate_keystrokes("shift-f5");
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Stopped, "{}\n{}", outcome.message, output(&main, cx));
    assert!(outcome.message.contains("App stopped"), "{}", outcome.message);
    assert!(!output(&main, cx).contains("started"), "a program started after Stop:\n{}", output(&main, cx));
}

#[test]
fn a_build_that_cannot_start_ends_the_run_saying_why() {
    // Another build already running refuses the run's build: the run ends with that reason instead of staying running.
    let run = Run::new("run-build-refused", &[]);
    let gate = if cfg!(windows) {
        "powershell -NoProfile -Command &quot;while (-not (Test-Path release)) { Start-Sleep -Milliseconds 50 }&quot;"
    } else {
        "while [ ! -f release ]; do sleep 0.05; done"
    };
    write(
        &run.folder,
        "App/App.csproj",
        &format!(
            "<Project Sdk=\"Microsoft.NET.Sdk\"><PropertyGroup><OutputType>Exe</OutputType><TargetFramework>net10.0</TargetFramework></PropertyGroup>\
             <Target Name=\"Wait\" BeforeTargets=\"Build\"><Exec Command=\"{gate}\" /></Target></Project>"
        ),
    );
    write(&run.folder, "App/Program.cs", "System.Console.WriteLine(\"started\");");
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("secondary-shift-b");
    wait(cx, "the explicit build's start", |cx| main.read_with(cx, |main, _| main.is_building()));
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the run's refusal", |cx| {
        main.read_with(cx, |main, _| !main.is_running() && main.run_outcome().is_some())
    });
    let outcome = outcome(&main, cx).unwrap();
    assert_eq!(outcome.status, RunStatus::Refused, "{}", outcome.message);
    assert!(outcome.message.contains("did not start"), "{}", outcome.message);
    assert!(outcome.message.contains("Another build is running"), "{}", outcome.message);
    // The explicit build is still the one running, and it ends with its gate released.
    assert!(main.read_with(cx, |main, _| main.is_building()), "the run's refusal ended the explicit build");
    write(&run.folder, "App/release", "released");
    wait(cx, "the explicit build's end", |cx| main.read_with(cx, |main, _| !main.is_building()));
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

/// Opens Rider's dialog from the toolbar's run menu, as a person does.
fn open_dialog(main: &Entity<MainWindow>, cx: &mut VisualTestContext) {
    click(cx, "run widget");
    click(cx, "run configuration: Edit Configurations…");
    assert!(main.read_with(cx, |main, _| main.run_dialog().is_some()), "Edit Configurations opened no dialog");
}

fn dialog(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> Entity<rsharp_app::run_dialog::RunDialog> {
    main.read_with(cx, |main, _| main.run_dialog()).expect("the dialog is closed")
}

#[test]
fn edit_configurations_makes_a_configuration_that_runs_with_its_arguments_and_environment() {
    // Rider's dialog makes a configuration of the folder's own, and Run launches it as it launches a generated one.
    let run = Run::new("run-dialog-make", &[]);
    app(
        &run,
        "System.Console.WriteLine(string.Join(\" \", args));\n\
         System.Console.WriteLine($\"greeting={System.Environment.GetEnvironmentVariable(\"GREETING\")}\");\n",
    );
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    open_dialog(&main, cx);
    click(cx, "run: Add");
    let made = dialog(&main, cx);
    made.update(cx, |dialog, cx| {
        dialog.set_text(rsharp_app::run_dialog::Field::Name, "Mine", cx);
        dialog.set_text(rsharp_app::run_dialog::Field::Arguments, "--seed 5", cx);
        dialog.set_variable(0, "GREETING", "hello", cx);
    });
    click(cx, "button: OK");
    assert!(main.read_with(cx, |main, _| main.run_dialog().is_none()), "OK left the dialog open");
    assert_eq!(
        main.read_with(cx, |main, _| main.run_own().iter().map(|c| c.name.clone()).collect::<Vec<_>>()),
        ["Mine"],
        "the configuration was not kept with the folder"
    );
    assert_eq!(
        main.read_with(cx, |main, _| main.run_configurations().iter().map(|c| c.name.clone()).collect::<Vec<_>>()),
        ["Mine", "App"],
        "the folder's own are listed before the projects' ones"
    );
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("Mine"), "the configuration made is not the one Run launches");
    cx.simulate_keystrokes("ctrl-f5");
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Exited(0), "{}\n{}", outcome.message, output(&main, cx));
    let shown = output(&main, cx);
    assert!(shown.contains("--seed 5"), "the configuration's arguments did not reach the program:\n{shown}");
    assert!(shown.contains("greeting=hello"), "its environment did not reach it:\n{shown}");
}

#[test]
fn a_configuration_without_a_name_cannot_be_kept() {
    // Rider keeps nothing that cannot launch: a name and a project are what Apply asks for, and it says which is missing.
    let run = Run::new("run-dialog-nameless", &[("App/App.csproj", EXE_PROJECT)]);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    open_dialog(&main, cx);
    click(cx, "run: Add");
    let made = dialog(&main, cx);
    made.update(cx, |dialog, cx| dialog.set_text(rsharp_app::run_dialog::Field::Name, "  ", cx));
    click(cx, "button: OK");
    let said = made.read_with(cx, |dialog, _| dialog.problem());
    assert_eq!(said, "A configuration needs a name", "Apply kept a configuration with no name");
    assert!(main.read_with(cx, |main, _| main.run_dialog().is_some()), "a refusal closed the dialog");
    assert!(main.read_with(cx, |main, _| main.run_own().is_empty()), "a refused configuration was kept");
}

#[test]
fn cancel_changes_nothing_and_apply_keeps_the_dialog_open() {
    // Apply keeps the list and lets the person keep editing; Cancel writes nothing at all.
    let run = Run::new("run-dialog-apply", &[("App/App.csproj", EXE_PROJECT)]);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    open_dialog(&main, cx);
    click(cx, "run: Add");
    let made = dialog(&main, cx);
    made.update(cx, |dialog, cx| dialog.set_text(rsharp_app::run_dialog::Field::Name, "Applied", cx));
    click(cx, "button: Apply");
    assert!(main.read_with(cx, |main, _| main.run_dialog().is_some()), "Apply closed the dialog");
    assert_eq!(
        main.read_with(cx, |main, _| main.run_own().iter().map(|c| c.name.clone()).collect::<Vec<_>>()),
        ["Applied"],
        "Apply did not keep the configuration with the folder"
    );
    // A second one, cancelled, leaves the first as it was.
    click(cx, "run: Add");
    let made = dialog(&main, cx);
    made.update(cx, |dialog, cx| dialog.set_text(rsharp_app::run_dialog::Field::Name, "Discarded", cx));
    click(cx, "button: Cancel");
    assert!(main.read_with(cx, |main, _| main.run_dialog().is_none()), "Cancel left the dialog open");
    assert_eq!(
        main.read_with(cx, |main, _| main.run_own().iter().map(|c| c.name.clone()).collect::<Vec<_>>()),
        ["Applied"],
        "Cancel kept the configuration it was meant to discard"
    );
}

#[test]
fn a_copy_of_a_generated_configuration_keeps_its_launch_profiles_values() {
    // A generated configuration changes only by copying it, as Rider shows a project's launch profiles.
    let run = Run::new("run-dialog-copy", &[("App/App.csproj", EXE_PROJECT)]);
    profile(
        &run,
        r#"{"profiles":{"Args":{"commandName":"Project","commandLineArgs":"--seed 7","environmentVariables":{"GREETING":"hi"}}}}"#,
    );
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    open_dialog(&main, cx);
    assert_eq!(
        dialog(&main, cx).read_with(cx, |dialog, _| dialog.rows()),
        [("App: Args".to_string(), true)],
        "the dialog does not list the project's configurations"
    );
    click(cx, "run row: App: Args");
    click(cx, "run: Copy");
    assert_eq!(
        dialog(&main, cx).read_with(cx, |dialog, _| dialog.rows()),
        [("App: Args (1)".to_string(), false), ("App: Args".to_string(), true)],
        "the copy is not one of the folder's own, under a name no other has"
    );
    click(cx, "button: OK");
    let own = main.read_with(cx, |main, _| main.run_own().to_vec());
    assert_eq!(own.len(), 1);
    assert_eq!(own[0].arguments, "--seed 7", "the copy lost its launch profile's arguments");
    assert_eq!(own[0].environment.get("GREETING").map(String::as_str), Some("hi"));
    assert_eq!(own[0].launch_profile.as_deref(), Some("Args"));
}

#[test]
fn removing_the_chosen_configuration_leaves_the_first_generated_one_chosen() {
    // A configuration the person removes cannot stay chosen: the list's first stands, as Rider's toolbar does.
    let run = Run::new("run-dialog-remove", &[("App/App.csproj", EXE_PROJECT)]);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    open_dialog(&main, cx);
    click(cx, "run: Add");
    let made = dialog(&main, cx);
    made.update(cx, |dialog, cx| dialog.set_text(rsharp_app::run_dialog::Field::Name, "Mine", cx));
    click(cx, "button: OK");
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("Mine"));
    open_dialog(&main, cx);
    click(cx, "run row: Mine");
    click(cx, "run: Remove");
    click(cx, "button: OK");
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("App"), "the removed configuration is still the one Run would launch");
    assert!(main.read_with(cx, |main, _| main.run_own().is_empty()), "the removed configuration is still kept");
}

#[test]
fn the_chosen_configuration_and_the_folders_own_are_kept_with_it() {
    // Being kept with the folder means a window opened again shows the same list and the same choice.
    let run = Run::new("run-dialog-kept", &[("App/App.csproj", EXE_PROJECT)]);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    open_dialog(&main, cx);
    click(cx, "run: Add");
    let made = dialog(&main, cx);
    made.update(cx, |dialog, cx| {
        dialog.set_text(rsharp_app::run_dialog::Field::Name, "Kept", cx);
        dialog.set_text(rsharp_app::run_dialog::Field::Arguments, "--kept", cx);
    });
    click(cx, "button: OK");
    let (main, cx) = open(&run, cx);
    let own = main.read_with(cx, |main, _| main.run_own().to_vec());
    assert_eq!(own.iter().map(|c| c.name.clone()).collect::<Vec<_>>(), ["Kept"], "the folder's own were not kept");
    assert_eq!(own[0].arguments, "--kept");
    assert_eq!(own[0].project, run.folder.join("App/App.csproj"), "a stored project is read back against the folder");
    let chosen = main.read_with(cx, |main, _| main.chosen_run_configuration().map(|c| c.name.clone()));
    assert_eq!(chosen.as_deref(), Some("Kept"), "the choice was not kept with the folder");
}

/// dotnet watch opens a socket under the temp folder, and .NET refuses a path over 104 characters: a fixture gives it a
/// short one, as a person whose temp folder is deep must. It is made under `target/`, which no test's own state shares.
fn watch_temp(name: &str) -> String {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target").join(format!("watch-{name}"));
    std::fs::create_dir_all(&dir).unwrap();
    // Resolved, since `..` counts towards the limit too.
    std::fs::canonicalize(&dir).unwrap_or(dir).to_string_lossy().into_owned()
}

/// Turns Settings' Hot Reload on, as a person does in Settings, Build, Execution, Deployment, Hot Reload.
fn with_hot_reload(cx: &mut VisualTestContext, on: bool) {
    cx.update(|_, cx| {
        let mut changed = rsharp_app::settings::settings(cx).clone();
        changed.hot_reload = on;
        rsharp_app::settings::apply(changed, cx);
    });
}

#[test]
fn hot_reload_runs_the_program_under_dotnet_watch() {
    // Settings' Hot Reload runs the project under dotnet watch: watch builds it, so no build goes first.
    let run = Run::new("run-watch", &[]);
    app(&run, "System.Console.WriteLine(\"watched\");\nSystem.Threading.Thread.Sleep(60000);\n");
    let temp = watch_temp("first");
    profile(
        &run,
        &format!(
            r#"{{"profiles":{{"watch":{{"commandName":"Project","environmentVariables":{{"TMPDIR":"{temp}"}}}}}}}}"#
        ),
    );
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    with_hot_reload(cx, true);
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the program under dotnet watch", |cx| {
        main.read_with(cx, |main, cx| {
            (main.is_watching() && main.run_output(cx).contains("watched"))
                || (!main.is_running() && main.run_outcome().is_some())
        })
    });
    let shown = output(&main, cx);
    assert!(
        shown.contains("watched"),
        "the program under watch did not run: {:?}\n{shown}",
        outcome(&main, cx).map(|outcome| outcome.message)
    );
    assert!(
        shown.lines().next().is_some_and(|line| line.is_empty())
            && shown.lines().any(|line| line.contains(" watch --non-interactive run --project")),
        "the Run console does not show dotnet watch's command line:\n{shown}"
    );
    assert!(
        main.read_with(cx, |main, _| main.said().unwrap_or_default().contains("Hot Reload")),
        "the status bar does not say the program runs with Hot Reload: {:?}",
        main.read_with(cx, |main, _| main.said().map(str::to_string))
    );
    cx.simulate_keystrokes("shift-f5");
    let outcome = finished(&main, cx);
    assert_eq!(outcome.status, RunStatus::Stopped, "{}", outcome.message);
}

#[test]
fn a_save_applies_its_change_to_the_running_program() {
    // What Hot Reload promises: an edit made here reaches the program that runs, without a second Run.
    let run = Run::new("run-watch-apply", &[]);
    app(
        &run,
        "while (true)\n{\n    System.Console.WriteLine(Values.Text);\n    System.Threading.Thread.Sleep(200);\n}\n",
    );
    write(
        &run.folder,
        "App/Values.cs",
        "public static class Values\n{\n    public static string Text => \"one\";\n}\n",
    );
    let temp = watch_temp("apply");
    profile(
        &run,
        &format!(
            r#"{{"profiles":{{"watch":{{"commandName":"Project","environmentVariables":{{"TMPDIR":"{temp}"}}}}}}}}"#
        ),
    );
    if !offline(&run) {
        return;
    }
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    with_hot_reload(cx, true);
    cx.simulate_keystrokes("ctrl-f5");
    wait(cx, "the program printing its first value", |cx| {
        main.read_with(cx, |main, cx| {
            (main.is_watching() && main.run_output(cx).contains("one"))
                || (!main.is_running() && main.run_outcome().is_some())
        })
    });
    let shown = output(&main, cx);
    assert!(
        shown.contains("one"),
        "the program under watch did not print: {:?}\n{shown}",
        outcome(&main, cx).map(|outcome| outcome.message)
    );
    let values = run.folder.join("App/Values.cs");
    main.update_in(cx, |main, window, cx| main.open_file(&values, Some((0, 0)), window, cx));
    cx.run_until_parked();
    cx.simulate_keystrokes("secondary-a");
    cx.simulate_input("public static class Values\n{\n    public static string Text => \"two\";\n}\n");
    cx.run_until_parked();
    // Apply Hot Reload Changes saves what is open, and dotnet watch reads the files on disk.
    cx.simulate_keystrokes("alt-f10");
    cx.run_until_parked();
    assert_eq!(
        std::fs::read_to_string(&values).unwrap(),
        "public static class Values\n{\n    public static string Text => \"two\";\n}\n",
        "Apply Hot Reload Changes did not save the edit"
    );
    // dotnet watch answers a save itself: either it applies the change, or the program restarts with it. A machine
    // whose file events never reach watch says so rather than failing, as one without the SDK is skipped.
    let two =
        |cx: &mut VisualTestContext| main.read_with(cx, |main, cx| main.run_output(cx).matches("two").count() > 1);
    let seen = pump_until(cx, Instant::now() + Duration::from_secs(45), |cx| {
        two(cx) || output(&main, cx).contains("File changed")
    });
    if !seen {
        cx.simulate_keystrokes("shift-f5");
        eprintln!("skipped: this machine delivers no file change to dotnet watch");
        return;
    }
    pump_until(cx, Instant::now() + Duration::from_secs(90), two);
    let shown = output(&main, cx);
    assert!(
        shown.contains("two"),
        "dotnet watch saw the change and did not apply it ({:?}); the last of its output was:\n{}",
        outcome(&main, cx).map(|outcome| outcome.message),
        shown.lines().rev().take(15).collect::<Vec<_>>().join("\n")
    );
    // The status bar says what watch made of the save, in the person's words.
    let said = main.read_with(cx, |main, _| main.said().map(str::to_string)).unwrap_or_default();
    assert!(
        said.starts_with("Hot Reload:"),
        "the status bar does not say what dotnet watch made of the save: {said:?}"
    );
    cx.simulate_keystrokes("shift-f5");
    finished(&main, cx);
}

#[test]
fn apply_hot_reload_changes_is_unavailable_without_a_run_under_watch() {
    // Rider greys it: no key does anything, and Find Action says what would make it available.
    let run = Run::new("run-watch-unavailable", &[("App/App.csproj", EXE_PROJECT)]);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("apply hot reload");
    cx.run_until_parked();
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |main, _| main.said().map(str::to_string)).as_deref(),
        Some("Apply Hot Reload Changes is not available here"),
        "nothing said why Apply Hot Reload Changes cannot run"
    );
}

#[test]
fn find_action_runs_edit_configurations_by_name() {
    // Rider has no key for it: the Run menu and Find Action are how a person opens it.
    let run = Run::new("run-dialog-find", &[("App/App.csproj", EXE_PROJECT)]);
    let mut cx = TestAppContext::single();
    let (main, cx) = open(&run, &mut cx);
    cx.simulate_keystrokes("secondary-shift-a");
    cx.simulate_input("edit configurations");
    cx.run_until_parked();
    let listed = main.read_with(cx, |main, cx| main.find_action_listed(cx)).unwrap();
    assert_eq!(listed.first(), Some(&"Edit Configurations…"), "Find Action cannot find it: {listed:?}");
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    assert!(main.read_with(cx, |main, _| main.run_dialog().is_some()), "Find Action opened no dialog");
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
