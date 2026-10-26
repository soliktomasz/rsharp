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
