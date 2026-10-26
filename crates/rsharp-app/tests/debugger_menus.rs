mod support;

use gpui::{AppContext, Modifiers, TestAppContext};
use rsharp_app::debug_view::{DebugAction, DebugView, VariableRow};
use support::Run;

#[gpui::test]
fn a_data_breakpoint_menu_names_the_member_of_the_clicked_object(cx: &mut TestAppContext) {
    let run = Run::new("data-breakpoint-menu", &[]);
    run.window(cx);
    let (view, cx) = cx.add_window_view(|_, cx| {
        let console = cx.new(|cx| rsharp_app::console::ConsoleView::new("debug", cx));
        DebugView::new(console, cx)
    });
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = requests.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, action: &DebugAction, _| {
            if let DebugAction::SetDataBreakpoint { reference, name } = action {
                received.lock().unwrap().push((*reference, name.clone()));
            }
        })
        .detach();
    });
    view.update(cx, |view, cx| {
        view.stopped("Paused", Vec::new(), cx);
        view.set_variables(
            vec![VariableRow {
                name: "counter".into(),
                value: "Counter".into(),
                r#type: Some("Counter".into()),
                evaluate_name: Some("counter".into()),
                reference: 42,
            }],
            cx,
        );
    });
    cx.run_until_parked();
    let counter = cx.debug_bounds("debug variable counter").unwrap().center();
    cx.simulate_click(counter, Modifiers::default());
    view.update(cx, |view, cx| {
        let tree = view.tree();
        view.set_children(
            &[0],
            tree,
            vec![VariableRow {
                name: "Count".into(),
                value: "0".into(),
                r#type: Some("int".into()),
                evaluate_name: Some("counter.Count".into()),
                reference: 0,
            }],
            cx,
        )
    });
    cx.run_until_parked();
    let count = cx.debug_bounds("debug variable Count").unwrap().center();
    cx.simulate_mouse_down(count, gpui::MouseButton::Right, Modifiers::default());
    cx.simulate_mouse_up(count, gpui::MouseButton::Right, Modifiers::default());
    cx.run_until_parked();
    let set = cx.debug_bounds("debug menu: Set Data Breakpoint").unwrap().center();
    cx.simulate_click(set, Modifiers::default());
    cx.run_until_parked();
    assert_eq!(
        *requests.lock().unwrap(),
        [(42, "Count".to_string())],
        "the data breakpoint must name its parent object and member"
    );
}

/// The stop's marks are the buffer's, which every window showing the file shares: an edit moves them once, not once
/// for each window told of it.
#[gpui::test]
fn the_stops_marks_move_once_with_an_edit_however_many_windows_show_the_file(cx: &mut TestAppContext) {
    let run = Run::new("marks-two-windows", &[("Program.cs", "a\nb\nc\nd\n")]);
    let source = rsharp_core::paths::canonical(&run.folder.join("Program.cs"));
    for _ in 0..2 {
        let (main, cx) = run.window(cx);
        let (folder, source) = (run.folder.clone(), source.clone());
        main.update_in(cx, |main, window, cx| {
            main.open_workspace(&folder, window, cx);
            main.open_file(&source, Some((0, 0)), window, cx);
        });
        cx.run_until_parked();
    }
    let buffer = cx.update(|cx| rsharp_app::buffer::find(&source, cx)).expect("both windows show the file");
    buffer.update(cx, |buffer, cx| {
        buffer.set_execution_line(Some(2), cx);
        buffer.set_inline_values(Some((2, "  i = 1".into())), cx);
    });
    // The second window's editor has the keys: a line broken at the top.
    let window = *cx.windows().last().unwrap();
    let cx = &mut gpui::VisualTestContext::from_window(window, cx);
    cx.simulate_keystrokes("enter");
    cx.run_until_parked();
    let (execution, inline) =
        buffer.read_with(cx, |buffer, _| (buffer.execution_line, buffer.inline_values.as_ref().map(|(line, _)| *line)));
    assert_eq!(
        (execution, inline),
        (Some(3), Some(3)),
        "one line typed above the stop moved its marks by other than one line"
    );
}
