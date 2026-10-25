//! The Terminal tool window driven as a person drives it: shells in the folder they were opened at, tabs, copy and
//! paste through the clipboard, links, and the IDE's own keys still reaching the IDE.

mod support;

use gpui::{Entity, Modifiers, TestAppContext, VisualTestContext, point, px, size};
use rsharp_app::dock::Tool;
use rsharp_app::window::{MainWindow, ToggleTerminal};
use std::time::{Duration, Instant};
use support::Run;

/// Runs the window until `done`, saying whether it came true: a shell, a program and its output all take real time.
fn wait_for(cx: &mut VisualTestContext, what: &str, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(cx) {
        assert!(Instant::now() < deadline, "{what} did not happen within thirty seconds");
        cx.executor().advance_clock(Duration::from_millis(20));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

fn tabs(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> usize {
    main.read_with(cx, |main, cx| main.terminal().read(cx).tabs_len())
}

/// Opens the folder and the Terminal, and waits for the first shell the frame started.
fn open_terminal<'a>(run: &Run, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| main.open_workspace(&run.folder, window, cx));
    cx.dispatch_action(ToggleTerminal);
    cx.run_until_parked();
    let panel = main.read_with(cx, |main, _| main.terminal().clone());
    wait_for(cx, "the first shell", |cx| panel.read_with(cx, |panel, _| panel.active_session().is_some()));
    (main, cx)
}

/// A line typed into the focused terminal, as a person types it and presses Return.
fn type_line(cx: &mut VisualTestContext, line: &str) {
    cx.simulate_input(line);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
}

/// The shell's own answer to where it is, short enough to stay on one line: the echo of the command typed is
/// not the answer, so only a line that begins with the marker is read.
fn answer(text: &str) -> Option<String> {
    text.lines()
        .filter_map(|line| line.trim().strip_prefix("WHERE="))
        .find(|rest| !rest.is_empty() && !rest.starts_with('$'))
        .map(|rest| rest.trim().to_string())
}

/// The folder the active shell is in, asked of the shell itself, once its shell is really running.
fn ask_folder(main: &Entity<MainWindow>, cx: &mut VisualTestContext) -> String {
    wait_for(cx, "the active tab's shell", |cx| {
        main.read_with(cx, |main, cx| main.terminal().read(cx).active_session().is_some())
    });
    type_line(cx, "echo WHERE=$(basename \"$PWD\")");
    let mut said = None;
    wait_for(cx, "the shell's own answer", |cx| {
        said = main
            .read_with(cx, |main, cx| main.terminal().read(cx).active_session())
            .map(|session| session.text())
            .and_then(|text| answer(&text));
        said.is_some()
    });
    said.unwrap_or_default()
}

/// Something the panel draws, clicked where the debug selector says it is.
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    cx.run_until_parked();
    let at = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}")).center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
}

#[gpui::test]
fn terminal_is_lazy_and_its_shortcut_opens_and_hides_the_docked_panel(cx: &mut TestAppContext) {
    let run = Run::new("terminal-lazy", &[]);
    let (main, cx) = run.window(cx);
    assert_eq!(tabs(&main, cx), 0, "an unopened Terminal should start no shell and no worker");
    cx.dispatch_action(ToggleTerminal);
    cx.run_until_parked();
    assert!(main.read_with(cx, |m, _| m.dock().is_open(Tool::Terminal)));
    assert_eq!(tabs(&main, cx), 1, "opening the Terminal should start one shell");
    cx.dispatch_action(ToggleTerminal);
    cx.run_until_parked();
    assert!(!main.read_with(cx, |m, _| m.dock().is_open(Tool::Terminal)));
}

#[gpui::test]
fn an_agent_tab_takes_typed_input_and_tool_window_keys_still_reach_the_ide(cx: &mut TestAppContext) {
    if cfg!(windows) {
        return;
    }
    let run = Run::new("terminal-input", &[("file.cs", "class C {}")]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    let id = main.update_in(cx, |m, w, cx| {
        m.run_agent(
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "printf 'ready\\r\\n'; read answer; printf 'got=%s\\r\\n' \"$answer\"".into(),
            ],
            w,
            cx,
        )
    });
    let panel = main.read_with(cx, |m, _| m.terminal().clone());
    wait_for(cx, "the Agent tab's real shell", |cx| {
        panel.read_with(cx, |panel, _| panel.session(id).is_some_and(|s| s.text().contains("ready")))
    });
    cx.simulate_input("hello terminal");
    cx.simulate_keystrokes("enter");
    wait_for(cx, "typed text reaching the terminal process", |cx| {
        panel.read_with(cx, |panel, _| {
            panel.session(id).is_some_and(|s| s.exit_code() == Some(0) && s.text().contains("got=hello terminal"))
        })
    });
    let was_open = main.read_with(cx, |m, _| m.dock().is_open(Tool::Solution));
    cx.simulate_keystrokes(if cfg!(target_os = "macos") { "cmd-1" } else { "alt-1" });
    assert!(
        main.read_with(cx, |m, _| m.dock().is_open(Tool::Solution)) != was_open,
        "a tool-window key was swallowed by the terminal"
    );
    assert_eq!(tabs(&main, cx), 1, "an ended Agent tab should retain its output");

    // Rider's own key for the Terminal, pressed in the terminal: it hides the window rather than being typed
    // into the shell.
    let screen = cx.debug_bounds("terminal screen").expect("the terminal is on screen").center();
    cx.simulate_click(screen, Modifiers::default());
    cx.run_until_parked();
    cx.simulate_keystrokes("alt-f12");
    cx.run_until_parked();
    assert!(
        !main.read_with(cx, |m, _| m.dock().is_open(Tool::Terminal)),
        "the Terminal's own key was swallowed by the terminal instead of hiding it"
    );
}

#[gpui::test]
fn shells_open_in_their_own_folder_and_a_closed_tab_takes_its_shell_with_it(cx: &mut TestAppContext) {
    let run = Run::new("terminal-folders", &[("src/a.cs", "class A { }\n")]);
    let (main, cx) = open_terminal(&run, cx);
    assert_eq!(tabs(&main, cx), 1, "opening the Terminal should start one shell");
    assert_eq!(ask_folder(&main, cx), "folder", "the first shell did not start in the workspace folder");
    assert!(
        run.state.join("terminal").is_dir(),
        "a test's shell history must live under the run's own state folder, never the person's"
    );

    click(cx, "terminal: New Session");
    wait_for(cx, "the second shell", |cx| tabs(&main, cx) == 2);
    assert!(cx.debug_bounds("terminal tab: Local (2)").is_some(), "a second session has no tab of its own");
    assert_eq!(ask_folder(&main, cx), "folder", "New Session did not start in the workspace folder");

    let rows = main.read_with(cx, |m, _| m.solution_rows());
    let row = rows.iter().position(|r| r.trim_start() == "src").unwrap_or_else(|| panic!("no src row in {rows:?}"));
    assert!(
        main.read_with(cx, |m, _| m.solution_menu(row)).iter().any(|item| item == "Open In > Terminal"),
        "a Solution view row should offer Open In > Terminal"
    );
    main.update_in(cx, |m, w, cx| m.choose(row, "Open In > Terminal", w, cx));
    wait_for(cx, "the shell opened at src", |cx| tabs(&main, cx) == 3);
    assert_eq!(ask_folder(&main, cx), "src", "Open In > Terminal did not start in the row's own folder");

    type_line(cx, "exit");
    wait_for(cx, "the exited shell's tab closing", |cx| tabs(&main, cx) == 2);
    click(cx, "terminal tab: Local");
    type_line(cx, "exit");
    wait_for(cx, "the second shell's tab closing", |cx| tabs(&main, cx) == 1);
    type_line(cx, "exit");
    wait_for(cx, "the last tab closing the tool window", |cx| {
        !main.read_with(cx, |m, _| m.dock().is_open(Tool::Terminal))
    });
}

#[gpui::test]
fn a_start_that_fails_keeps_its_tab_and_says_why(cx: &mut TestAppContext) {
    let run = Run::new("terminal-refused", &[]);
    let (main, cx) = open_terminal(&run, cx);
    let folder = run.folder.clone();
    let id = main.update_in(cx, |m, _, cx| {
        m.terminal().update(cx, |panel, cx| {
            panel.start("Broken".into(), rsharp_terminal::CommandBuilder::new("/no/such/shell"), folder, true, cx)
        })
    });
    let panel = main.read_with(cx, |m, _| m.terminal().clone());
    let mut why = None;
    wait_for(cx, "the refused shell to say why", |cx| {
        why = panel.read_with(cx, |panel, _| panel.problem(id).map(str::to_string));
        why.is_some()
    });
    assert_eq!(tabs(&main, cx), 2, "a shell that would not start should keep its tab");
    assert!(
        cx.debug_bounds("terminal tab: Broken").is_some(),
        "the tab of a shell that never started is not on screen: {why:?}"
    );
}

#[gpui::test]
fn copy_and_paste_go_through_the_clipboard_and_a_selection_outlives_a_resize(cx: &mut TestAppContext) {
    if cfg!(windows) {
        return;
    }
    let run = Run::new("terminal-copy", &[("file.cs", "class C { }\n")]);
    let (main, cx) = open_terminal(&run, cx);
    let id = main.update_in(cx, |m, w, cx| {
        m.run_agent(
            vec![
                "/bin/sh".into(),
                "-c".into(),
                "stty -echo; printf 'héllo 漢字  two words\\r\\n'; IFS= read -r answer; \
                 printf 'got=%s\\r\\n' \"$answer\""
                    .into(),
            ],
            w,
            cx,
        )
    });
    let panel = main.read_with(cx, |m, _| m.terminal().clone());
    let text =
        |cx: &mut VisualTestContext| panel.read_with(cx, |p, _| p.session(id).map(|s| s.text()).unwrap_or_default());
    wait_for(cx, "the program's first line", |cx| text(cx).contains("two words"));

    let mac = cfg!(target_os = "macos");
    cx.simulate_keystrokes(if mac { "cmd-a cmd-c" } else { "ctrl-shift-a ctrl-shift-c" });
    cx.run_until_parked();
    let copied = cx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap_or_default();
    assert!(copied.contains("héllo 漢字  two words"), "Select All and Copy did not put the screen on the clipboard");

    cx.simulate_resize(size(px(720.), px(520.)));
    cx.run_until_parked();
    cx.simulate_keystrokes(if mac { "cmd-c" } else { "ctrl-shift-c" });
    cx.run_until_parked();
    let resized = cx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap_or_default();
    assert_eq!(resized, copied, "resizing the window lost what was selected");

    cx.update(|_, cx| cx.write_to_clipboard(gpui::ClipboardItem::new_string("pasted 漢字".into())));
    cx.simulate_keystrokes(if mac { "cmd-v" } else { "ctrl-shift-v" });
    cx.simulate_keystrokes("enter");
    wait_for(cx, "the pasted text reaching the program", |cx| text(cx).contains("got=pasted 漢字"));
}

#[gpui::test]
fn ten_megabytes_of_output_leave_the_editor_typing_and_the_scrollback_bounded(cx: &mut TestAppContext) {
    if cfg!(windows) {
        return;
    }
    let run = Run::new("terminal-flood", &[("file.cs", "class C { }\n")]);
    let (main, cx) = open_terminal(&run, cx);
    let folder = run.folder.clone();
    let id = main.update_in(cx, |m, _, cx| {
        m.terminal().update(cx, |panel, cx| {
            let command = rsharp_terminal::CommandBuilder::new("/bin/sh");
            let mut command = command;
            command.args([
                "-c",
                "dd if=/dev/zero bs=1048576 count=10 2>/dev/null | tr '\\000' x; printf '\\r\\nflood done\\r\\n'",
            ]);
            command.cwd(&folder);
            panel.start("Flood".into(), command, folder.clone(), true, cx)
        })
    });
    let panel = main.read_with(cx, |m, _| m.terminal().clone());
    // The editor takes the keys while the flood arrives, as a person typing during `cat` of a large file would.
    cx.run_until_parked();
    main.update_in(cx, |m, w, cx| m.open_file(&run.folder.join("file.cs"), Some((0, 0)), w, cx));
    cx.run_until_parked();
    cx.simulate_input("int x;");
    cx.run_until_parked();
    let typed = main.read_with(cx, |m, cx| m.editor().expect("the file is open").read(cx).text().to_string());
    assert_eq!(typed, "int x;class C { }\n", "typing into the editor waited for the terminal's output");

    wait_for(cx, "the 10 MB flood to finish", |cx| {
        panel.read_with(cx, |p, _| p.session(id).is_some_and(|s| s.exit_code() == Some(0)))
    });
    let (history, text) = panel.read_with(cx, |p, _| {
        let session = p.session(id).expect("the flood's tab keeps its output");
        (session.snapshot().history, session.text())
    });
    assert!(history <= rsharp_terminal::HISTORY_LINES, "the scrollback grew past its bound: {history} lines");
    assert!(text.contains("flood done"), "the program's last output was lost");
}

#[gpui::test]
fn the_context_menus_copy_takes_the_selection_it_was_opened_over(cx: &mut TestAppContext) {
    if cfg!(windows) {
        return;
    }
    let run = Run::new("terminal-menu", &[("file.cs", "class C { }\n")]);
    let (main, cx) = open_terminal(&run, cx);
    let id = main.update_in(cx, |m, w, cx| {
        m.run_agent(vec!["/bin/sh".into(), "-c".into(), "printf 'menu words\r\n'; sleep 30".into()], w, cx)
    });
    let panel = main.read_with(cx, |m, _| m.terminal().clone());
    wait_for(cx, "the program's line", |cx| {
        panel.read_with(cx, |p, _| p.session(id).is_some_and(|s| s.text().contains("menu words")))
    });
    let mac = cfg!(target_os = "macos");
    cx.simulate_keystrokes(if mac { "cmd-a" } else { "ctrl-shift-a" });
    cx.run_until_parked();

    let screen = cx.debug_bounds("terminal screen").expect("the terminal is on screen").center();
    cx.simulate_mouse_down(screen, gpui::MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(screen, gpui::MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    click(cx, "terminal menu: Copy");
    let copied = cx.update(|_, cx| cx.read_from_clipboard()).and_then(|item| item.text()).unwrap_or_default();
    assert!(
        copied.contains("menu words"),
        "Copy in the menu took the cell under the click instead of the selection: {copied:?}"
    );
}

#[gpui::test]
fn a_url_opens_the_browser_and_a_stack_frames_place_opens_the_file(cx: &mut TestAppContext) {
    if cfg!(windows) {
        return;
    }
    let run =
        Run::new("terminal-links", &[("My App/Program.cs", "class Program\n{\n\n    static void Main() { }\n}\n")]);
    let (main, cx) = open_terminal(&run, cx);
    let program = format!(
        "printf 'Now listening on: http://localhost:5000/swagger\\r\\n\
           at Shop.Program.Main(String[] args) in {}/My App/Program.cs:line 4\\r\\n'; sleep 30",
        run.folder.display()
    );
    let id = main.update_in(cx, |m, w, cx| m.run_agent(vec!["/bin/sh".into(), "-c".into(), program], w, cx));
    let opened = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let recorder = opened.clone();
    main.update(cx, |m, _| m.set_browser_opener(move |url| recorder.lock().unwrap().push(url)));
    let panel = main.read_with(cx, |m, _| m.terminal().clone());
    wait_for(cx, "the program's links", |cx| {
        panel.read_with(cx, |p, _| p.session(id).is_some_and(|s| s.text().contains(":line 4")))
    });

    let screen = cx.debug_bounds("terminal screen").expect("the terminal is on screen");
    let cell = main.read_with(cx, |m, cx| m.terminal().read(cx).view().read(cx).cell());
    let in_cell = |(row, col): (usize, usize)| {
        screen.origin + point(cell.width * (col as f32 + 0.5), cell.height * (row as f32 + 0.5))
    };
    // Where a phrase starts on screen, so the click lands on it however the line wrapped.
    let at = |cx: &mut VisualTestContext, phrase: &str| {
        let cells: Vec<(usize, usize, String)> = panel.read_with(cx, |p, _| {
            p.session(id)
                .map(|s| s.snapshot().cells.into_iter().map(|c| (c.row, c.col, c.text)).collect())
                .unwrap_or_default()
        });
        let mut text = String::new();
        let mut starts = Vec::new();
        for (row, col, piece) in &cells {
            starts.push((text.len(), *row, *col));
            text.push_str(piece);
        }
        let found = text.find(phrase).unwrap_or_else(|| panic!("{phrase:?} is not on the terminal screen: {text:?}"));
        let cell = starts.iter().find(|(start, ..)| *start == found).expect("a cell holds the phrase's start");
        (cell.1, cell.2)
    };

    let target = at(cx, "http://localhost:5000/swagger");
    cx.simulate_click(in_cell(target), Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        opened.lock().unwrap().as_slice(),
        ["http://localhost:5000/swagger"],
        "clicking the URL did not open the browser"
    );

    let target = at(cx, "My App/Program.cs");
    cx.simulate_click(in_cell(target), Modifiers::default());
    cx.run_until_parked();
    let (name, line) = main.read_with(cx, |m, cx| {
        let editor = m.editor().expect("the stack frame's place opened no file").read(cx);
        (editor.path.file_name().unwrap().to_string_lossy().into_owned(), editor.caret().line)
    });
    assert_eq!(name, "Program.cs", "the clicked stack frame did not open its file");
    assert_eq!(line, 3, "the stack frame opened its file at the wrong line");
}
