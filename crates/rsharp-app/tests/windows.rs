//! Several projects in one RSharp: each in a window of its own, never twice, and all of them kept at quit.

mod support;

use gpui::TestAppContext;
use rsharp_app::projects::{Session, keep_session, open_project, to_reopen, windows};
use support::Run;

#[gpui::test]
fn a_second_project_opens_in_a_new_window_and_an_open_one_comes_forward(cx: &mut TestAppContext) {
    let first = Run::new("windows-first", &[("A.slnx", "<Solution />")]);
    let second = Run::new("windows-second", &[("B.slnx", "<Solution />")]);
    let (_, cx) = first.window(cx);

    cx.cx.update(|cx| open_project(&first.folder, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| windows(cx).len()), 1, "the Welcome window did not take the first project");

    cx.cx.update(|cx| open_project(&second.folder.join("B.slnx"), cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| windows(cx).len()), 2, "a second project did not get a window of its own");

    cx.cx.update(|cx| open_project(&first.folder, cx));
    cx.run_until_parked();
    assert_eq!(cx.update(|_, cx| windows(cx).len()), 2, "opening a project already open made another window for it");

    cx.cx.update(|cx| keep_session(cx));
    let kept = cx.update(|_, cx| rsharp_app::state::store(cx).read::<Session>());
    assert_eq!(kept.open.len(), 2, "quit did not keep both windows' projects");
    assert!(
        kept.open.contains(&second.folder.join("B.slnx")),
        "a solution opened by its file was not kept as that file"
    );
    assert_eq!(cx.update(|_, cx| to_reopen(cx)).len(), 2, "the next start would not open both again");
}
