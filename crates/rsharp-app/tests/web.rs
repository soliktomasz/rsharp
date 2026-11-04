//! The web languages as a person uses them, against the real servers on Node.js: a TypeScript file's problem marked,
//! completion after a dot, and Reformat Code. Skips, saying why, without Node.js or the servers.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::time::{Duration, Instant};
use support::Run;

const SITE: &str = "const count: number = \"three\";\nconsole.\nfunction  greet( ){return 1}\n";

/// Waits, in real time, for what the server does on threads of its own to reach the window; false after a minute.
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

#[gpui::test]
fn typescript_is_marked_completed_and_reformatted_by_its_server(cx: &mut TestAppContext) {
    if rsharp_core::programs::node().is_none() {
        eprintln!("skipped: Node.js was not found");
        return;
    }
    if let Err(why) = rsharp_lsp::web::web_servers() {
        eprintln!("skipped: {why}");
        return;
    }
    let run = Run::new("web", &[("site.ts", SITE)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("site.ts"), Some((1, 8)), window, cx);
    });
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let problems = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>())
    };
    let marked = wait_for(cx, |cx| problems(cx).iter().any(|p| p.contains("not assignable to type 'number'")));
    assert!(
        marked,
        "a string given to a number was never marked; the file shows {:?} and the status bar says {:?}",
        problems(cx),
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );

    cx.simulate_keystrokes("ctrl-space");
    assert!(
        wait_for(cx, |cx| editor.read_with(cx, |e, _| e.completion_shown().is_some())),
        "completion after `console.` never showed"
    );
    cx.simulate_input("lo");
    cx.run_until_parked();
    let shown = editor.read_with(cx, |e, _| e.completion_shown()).unwrap_or_default();
    assert_eq!(shown.first().map(String::as_str), Some("log"), "completion for `lo` showed {shown:?}");
    cx.simulate_keystrokes("escape secondary-z");
    cx.run_until_parked();
    assert_eq!(text(&main, cx), SITE, "Undo did not take back what was typed");

    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-alt-l" } else { "ctrl-alt-enter" });
    assert!(
        wait_for(cx, |cx| text(&main, cx).contains("function greet() { return 1 }")),
        "Reformat Code did not lay out the function as TypeScript's formatter does:\n{}",
        text(&main, cx)
    );
}

#[gpui::test]
fn a_node_interpreter_settings_name_that_is_not_there_is_said_and_the_file_keeps_its_colours(cx: &mut TestAppContext) {
    let run = Run::new("web-no-node", &[("site.ts", SITE)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| {
        cx.global_mut::<rsharp_app::state::Ide>().servers = true;
        let mut named = rsharp_app::settings::settings(cx).clone();
        named.node_interpreter = "/no/such/node".into();
        rsharp_app::settings::apply(named, cx);
    });
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("site.ts"), None, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        main.read_with(cx, |m, _| m.said().map(str::to_string)),
        Some("TypeScript has its colours only: Settings name /no/such/node as Node.js, which is not there".into()),
        "the status bar did not say why TypeScript's server could not start"
    );
}

/// The pids under this process whose command line has `needle`, from `ps`'s own table of parents.
#[cfg(unix)]
fn descendants_running(needle: &str) -> Vec<u32> {
    let table = std::process::Command::new("ps").args(["-A", "-o", "pid=,ppid=,command="]).output().unwrap().stdout;
    let rows: Vec<(u32, u32, String)> = String::from_utf8_lossy(&table)
        .lines()
        .filter_map(|l| {
            let mut parts = l.trim().splitn(3, ' ');
            Some((parts.next()?.parse().ok()?, parts.next()?.trim().parse().ok()?, parts.next()?.to_string()))
        })
        .collect();
    let mut family = vec![std::process::id()];
    loop {
        let more: Vec<u32> =
            rows.iter().filter(|(p, parent, _)| family.contains(parent) && !family.contains(p)).map(|r| r.0).collect();
        if more.is_empty() {
            break;
        }
        family.extend(more);
    }
    rows.iter().filter(|(p, _, c)| family.contains(p) && c.contains(needle)).map(|r| r.0).collect()
}

#[cfg(unix)]
#[gpui::test]
fn a_web_server_that_ends_is_started_again_and_marks_the_file_once_more(cx: &mut TestAppContext) {
    if rsharp_core::programs::node().is_none() {
        eprintln!("skipped: Node.js was not found");
        return;
    }
    if let Err(why) = rsharp_lsp::web::web_servers() {
        eprintln!("skipped: {why}");
        return;
    }
    let run = Run::new("web-restart", &[("site.ts", SITE)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join("site.ts"), Some((1, 8)), window, cx);
    });
    let editor = main.read_with(cx, |m, _| m.editor().unwrap());
    let problems = |cx: &mut VisualTestContext| {
        editor
            .read_with(cx, |e, cx| e.buffer.read(cx).diagnostics.iter().map(|d| d.message.clone()).collect::<Vec<_>>())
    };
    assert!(wait_for(cx, |cx| !problems(cx).is_empty()), "the first server never marked the file");

    let servers = descendants_running("typescript-language-server");
    assert!(!servers.is_empty(), "no typescript-language-server runs under the test");
    for pid in servers {
        let _ = std::process::Command::new("kill").args(["-9", &pid.to_string()]).status();
    }
    // A new mistake, which only a server that runs again can mark.
    cx.simulate_input("flag: boolean");
    assert!(
        wait_for(cx, |cx| problems(cx).iter().any(|p| p.contains("'boolean' only refers to a type"))),
        "the language server that was killed was not started again; the file shows {:?} and the status bar says {:?}",
        problems(cx),
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
}
