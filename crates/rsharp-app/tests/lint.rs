//! ESLint and Prettier as Rider runs them, from the project's own packages (the versions Passenger's tests pin, put in
//! place once by npm): a problem the project's ESLint configuration names marked as typed, and Reformat Code laying a
//! file out by its .prettierrc, as one undo step, leaving what .prettierignore names. Skips, saying why, without
//! Node.js, npm or the web servers.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};
use support::Run;

const PACKAGE: &str = "{ \"name\": \"shop-client\", \"private\": true, \"type\": \"module\" }\n";
const CONFIG: &str = "export default [\n  {\n    files: [\"**/*.js\"],\n    rules: {\n      \"prefer-const\": \"error\",\n    },\n  },\n];\n";
const APP: &str = "let total = 1;\nconsole.log(total);\n";
const FORMAT: &str = "const  label = {name:\"tea\",price:3};\nconsole.log(label);\n";
const LEGACY: &str = "const  kept   =   \"as written\";\nconsole.log(kept);\n";

/// Held by each test for its length, so they run one at a time.
// shortcut: in one test process, a window ending (its ESLint server with it) while another window's ESLint server
// looked up its eslint made that lookup fail (eslint/noLibrary, a node spawn's ENOENT); not yet explained. Run them in
// parallel again once it is, and check two real windows, one closed while the other's ESLint starts.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// The folder whose node_modules holds the tests' eslint and prettier, put in place by npm from Passenger's test lock
/// file once (again when the lock changes); none, saying why, where they cannot be.
fn packages() -> Option<PathBuf> {
    let skip = |why: String| {
        eprintln!("skipped: {why}");
        None
    };
    if rsharp_core::programs::node().is_none() {
        return skip("Node.js was not found".into());
    }
    if let Err(why) = rsharp_lsp::web::web_servers() {
        return skip(why);
    }
    let pins = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/passenger/tests/Passenger.Tests/NodePackages");
    let Ok(lock) = std::fs::read(pins.join("package-lock.json")) else {
        return skip("vendor/passenger is not checked out".into());
    };
    let folder = std::env::temp_dir().join("rsharp-test-node-packages");
    let stamp = folder.join("rsharp.stamp");
    if std::fs::read(&stamp).is_ok_and(|done| done == lock) {
        return Some(rsharp_core::paths::canonical(&folder));
    }
    let also = [PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];
    let Some(npm) = rsharp_core::programs::find(if cfg!(windows) { "npm.cmd" } else { "npm" }, &also) else {
        return skip("npm was not found".into());
    };
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(pins.join(file), folder.join(file)).unwrap();
    }
    let installed = std::process::Command::new(npm)
        .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund", "--loglevel=error"])
        .current_dir(&folder)
        .status();
    if !installed.is_ok_and(|s| s.success()) {
        return skip(format!("npm could not put eslint and prettier in {}", folder.display()));
    }
    std::fs::write(&stamp, lock).unwrap();
    Some(rsharp_core::paths::canonical(&folder))
}

/// A run whose folder is a JavaScript project under the packages' folder, so Node.js finds them from its files.
fn project(name: &str, packages: &Path) -> Run {
    let mut run = Run::new(name, &[]);
    run.folder = packages.join("projects").join(format!("{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&run.folder);
    for (file, text) in [
        ("package.json", PACKAGE),
        ("eslint.config.js", CONFIG),
        (".prettierrc", "{ \"semi\": false, \"singleQuote\": true }\n"),
        (".prettierignore", "src/legacy.js\n"),
        ("src/app.js", APP),
        ("src/format.js", FORMAT),
        ("src/legacy.js", LEGACY),
    ] {
        let path = run.folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }
    run
}

fn open<'a>(
    run: &Run,
    file: &str,
    caret: (usize, usize),
    cx: &'a mut TestAppContext,
) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join(file), Some(caret), window, cx);
    });
    (main, cx)
}

/// Waits, in real time, for what Node.js does to reach the window; false after a minute.
fn wait_for(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(60);
    while Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn text(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    main.read_with(cx, |m, cx| m.editor().map(|e| e.read(cx).text().to_string()).unwrap_or_default())
}

const REFORMAT: &str = if cfg!(target_os = "macos") { "cmd-alt-l" } else { "ctrl-alt-enter" };

#[gpui::test]
fn the_projects_eslint_marks_what_its_configuration_names(cx: &mut TestAppContext) {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let Some(packages) = packages() else { return };
    let run = project("eslint", &packages);
    let (main, cx) = open(&run, "src/app.js", (0, 0), cx);
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let problems = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>())
    };
    let wanted = "ESLint: 'total' is never reassigned. Use 'const' instead. (prefer-const)";
    assert!(
        wait_for(cx, |cx| problems(cx).iter().any(|p| p == wanted)),
        "the project's prefer-const was never marked; the file shows {:?} and the status bar says {:?}",
        problems(cx),
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
    let _ = std::fs::remove_dir_all(&run.folder);
}

#[gpui::test]
fn reformat_code_lays_a_file_out_by_the_projects_prettier_as_one_undo(cx: &mut TestAppContext) {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(|p| p.into_inner());
    let Some(packages) = packages() else { return };
    let run = project("prettier", &packages);
    // The caret after `pri` of `price`.
    let (main, cx) = open(&run, "src/format.js", (0, 30), cx);
    cx.simulate_keystrokes(REFORMAT);
    let pretty = "const label = { name: 'tea', price: 3 }\nconsole.log(label)\n";
    assert!(
        wait_for(cx, |cx| text(&main, cx) == pretty),
        "the file was not laid out by its .prettierrc (single quotes, no semicolons):\n{}\nthe status bar says {:?}",
        text(&main, cx),
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
    let caret = main.read_with(cx, |m, cx| m.editor().unwrap().read(cx).caret());
    assert_eq!((caret.line, caret.column), (0, 32), "the caret did not stay after `pri`");
    cx.simulate_keystrokes("secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&main, cx), FORMAT, "one Undo did not take the whole format back");

    main.update_in(cx, |main, window, cx| main.open_file(&run.folder.join("src/legacy.js"), Some((0, 0)), window, cx));
    cx.simulate_keystrokes(REFORMAT);
    assert!(
        wait_for(cx, |cx| main.read_with(cx, |m, _| m.said().is_some_and(|s| s.contains(".prettierignore")))),
        "Reformat Code on a file .prettierignore names did not say why it left it; it says {:?}",
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
    assert_eq!(text(&main, cx), LEGACY, "a file .prettierignore names was formatted");
    let _ = std::fs::remove_dir_all(&run.folder);
}
