mod support;

use gpui::{AppContext, Focusable, Modifiers, TestAppContext};
use rsharp_app::git_view::{GitRequest, GitView, MergeView};
use support::Run;

#[gpui::test]
fn a_commit_draft_keeps_paragraphs_and_checked_files(cx: &mut TestAppContext) {
    let run = Run::new("git-draft", &[]);
    let (_, cx) = run.window(cx);
    let view = cx.new(GitView::new);
    view.update(cx, |v, cx| {
        v.set_message("Describe the change\n\nExplain why it matters", cx);
        v.set_checked("chosen.cs".into(), true, cx);
        v.set_problem(Some("The commit hook refused this message".into()), cx);
    });
    assert_eq!(view.read_with(cx, |v, cx| v.message(cx)), "Describe the change\n\nExplain why it matters");
    assert_eq!(view.read_with(cx, |v, _| v.checked_paths()), [std::path::PathBuf::from("chosen.cs")]);
}

#[gpui::test]
fn editing_a_merge_result_does_not_acknowledge_conflicts(cx: &mut TestAppContext) {
    let run = Run::new("git-merge-result", &[]);
    let (_, cx) = run.window(cx);
    let view = cx.new(|cx| MergeView::new("a\n".into(), "ours\n".into(), "theirs\n".into(), cx));
    view.update(cx, |v, cx| v.set_result("resolved by hand\n".into(), cx));
    cx.run_until_parked();
    assert_eq!(view.read_with(cx, |v, _| v.unresolved_count()), 1);
    assert_eq!(view.read_with(cx, |v, cx| v.result(cx)), "resolved by hand\n");
    view.update(cx, |v, cx| v.acknowledge_chunk(0, cx));
    assert_eq!(view.read_with(cx, |v, _| v.unresolved_count()), 0);
    assert_eq!(view.read_with(cx, |v, cx| v.result(cx)), "resolved by hand\n");
}

#[gpui::test]
fn status_refresh_keeps_unchecked_files_and_new_files_start_excluded(cx: &mut TestAppContext) {
    use rsharp_core::git::{ChangeKind, FileChange, Operation, Status};
    let run = Run::new("git-chosen-refresh", &[]);
    let (_, cx) = run.window(cx);
    let view = cx.new(GitView::new);
    let status = Status {
        branch: Some("main".into()),
        head: None,
        operation: Operation::Idle,
        changes: vec![
            FileChange { path: "a.cs".into(), from: None, kind: ChangeKind::Modified, index: '.', worktree: 'M' },
            FileChange { path: "new.cs".into(), from: None, kind: ChangeKind::Unversioned, index: '?', worktree: '?' },
        ],
    };
    view.update(cx, |v, cx| v.set_status(Some(status.clone()), cx));
    assert_eq!(view.read_with(cx, |v, _| v.checked_paths()), [std::path::PathBuf::from("a.cs")]);
    view.update(cx, |v, cx| {
        v.set_checked("a.cs".into(), false, cx);
        v.set_checked("new.cs".into(), true, cx);
        v.set_status(Some(status), cx);
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.checked_paths()),
        [std::path::PathBuf::from("new.cs")],
        "refresh changed the person's file choices"
    );
}

#[gpui::test]
fn the_commit_button_emits_only_checked_paths_and_the_multiline_draft(cx: &mut TestAppContext) {
    use rsharp_core::git::{ChangeKind, FileChange, Operation, Status};
    let run = Run::new("git-commit-click", &[]);
    run.window(cx);
    let (view, cx) = cx.add_window_view(|_, cx| GitView::new(cx));
    let requests = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let received = requests.clone();
    cx.update(|_, cx| {
        cx.subscribe(&view, move |_, request: &GitRequest, _| received.lock().unwrap().push(request.clone())).detach()
    });
    view.update(cx, |v, cx| {
        v.set_status(
            Some(Status {
                branch: Some("main".into()),
                head: None,
                operation: Operation::Idle,
                changes: vec![
                    FileChange {
                        path: "a.cs".into(),
                        from: None,
                        kind: ChangeKind::Modified,
                        index: '.',
                        worktree: 'M',
                    },
                    FileChange {
                        path: "b.cs".into(),
                        from: None,
                        kind: ChangeKind::Modified,
                        index: '.',
                        worktree: 'M',
                    },
                ],
            }),
            cx,
        )
    });
    cx.run_until_parked();
    let at = cx.debug_bounds("git checked: b.cs").expect("the changed file has no checkbox").center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
    let editor = view.read_with(cx, |v, _| v.message_editor().clone());
    cx.update(|window, cx| window.focus(&editor.focus_handle(cx)));
    cx.simulate_input("Explain the change");
    cx.simulate_keystrokes("enter");
    cx.simulate_input("And its reason");
    cx.run_until_parked();
    view.update(cx, |v, cx| v.set_busy(true, cx));
    cx.run_until_parked();
    let at = cx.debug_bounds("git commit").expect("Commit was not visible").center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
    assert!(requests.lock().unwrap().is_empty(), "a disabled Commit button emitted an operation");
    view.update(cx, |v, cx| v.set_busy(false, cx));
    cx.run_until_parked();
    let at = cx.debug_bounds("git commit").expect("Commit was not visible").center();
    cx.simulate_click(at, Modifiers::default());
    cx.run_until_parked();
    assert!(requests.lock().unwrap().iter().any(|r|matches!(r,GitRequest::Commit{paths,message,amend:false,push:false}if paths==&vec![std::path::PathBuf::from("a.cs")]&&message=="Explain the change\nAnd its reason")),"Commit lost either the checkbox choice or a message paragraph");
}

#[gpui::test]
fn refreshing_the_repository_keeps_the_scoped_history_tab(cx: &mut TestAppContext) {
    use rsharp_core::git::CommitInfo;
    let run = Run::new("git-history-tabs", &[]);
    let (_, cx) = run.window(cx);
    let view = cx.new(GitView::new);
    let commit = |hash: &str| CommitInfo {
        hash: hash.into(),
        parents: Vec::new(),
        author: "Test".into(),
        email: String::new(),
        time: 0,
        refs: Vec::new(),
        subject: hash.into(),
    };
    view.update(cx, |v, cx| {
        v.set_log(vec![commit("initial")], Vec::new(), cx);
        v.set_history("History: a.cs".into(), vec![commit("file")], Vec::new(), cx);
        v.set_log(vec![commit("refreshed")], Vec::new(), cx);
    });
    assert_eq!(
        view.read_with(cx, |v, _| v.commits()[0].hash.clone()),
        "file",
        "a repository refresh replaced the selected file history"
    );
    view.update(cx, |v, cx| v.show_log(cx));
    assert_eq!(view.read_with(cx, |v, _| v.commits()[0].hash.clone()), "refreshed");
}
