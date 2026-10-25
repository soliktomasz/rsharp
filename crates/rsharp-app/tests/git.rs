//! Git's working tree through the same window actions a person uses.
mod support;

use gpui::{TestAppContext, VisualTestContext};
use rsharp_app::dock::Tool;
use rsharp_app::window::{ToggleCommit, ToggleGit};
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use support::Run;

fn git(folder: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(folder)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .env("GIT_AUTHOR_NAME", "Test")
        .env("GIT_AUTHOR_EMAIL", "test@example.invalid")
        .env("GIT_COMMITTER_NAME", "Test")
        .env("GIT_COMMITTER_EMAIL", "test@example.invalid")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = rsharp_core::process::spawn(command).unwrap();
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut child.process.stdout.take().unwrap(), &mut stdout).unwrap();
    let mut stderr = String::new();
    std::io::Read::read_to_string(&mut child.process.stderr.take().unwrap(), &mut stderr).unwrap();
    assert!(child.wait().unwrap().success(), "git {args:?}: {stderr}");
    stdout
}

fn wait(cx: &mut VisualTestContext, mut done: impl FnMut(&mut VisualTestContext) -> bool) {
    let deadline = Instant::now() + Duration::from_secs(30);
    while !done(cx) {
        assert!(Instant::now() < deadline, "Git did not settle within thirty seconds");
        cx.executor().advance_clock(Duration::from_millis(150));
        cx.run_until_parked();
        std::thread::sleep(Duration::from_millis(10));
    }
}

#[gpui::test]
fn a_repository_opened_below_its_root_shows_its_branch_and_changes(cx: &mut TestAppContext) {
    let run = Run::new("git-subfolder", &[("src/a.cs", "class A {}\n")]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    std::fs::write(run.folder.join("src/a.cs"), "class A { int n; }\n").unwrap();
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder.join("src"), w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some()));
    let status = main.read_with(cx, |m, _| m.git_status().unwrap().clone());
    assert_eq!(status.branch.as_deref(), Some("main"));
    assert_eq!(status.changes.len(), 1, "the Commit window lost a tracked modification");
    cx.dispatch_action(ToggleCommit);
    assert!(main.read_with(cx, |m, _| m.dock().is_open(Tool::Commit)));
    cx.dispatch_action(ToggleGit);
    assert!(main.read_with(cx, |m, _| m.dock().is_open(Tool::Git)));
}

#[gpui::test]
fn changing_workspace_discards_the_previous_repository(cx: &mut TestAppContext) {
    let run = Run::new("git-switch", &[("a.cs", "class A {}\n")]);
    git(&run.folder, &["init", "-b", "main"]);
    let other = run.folder.parent().unwrap().join("other");
    std::fs::create_dir(&other).unwrap();
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some()));
    main.update_in(cx, |m, w, cx| m.open_workspace(&other, w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| !m.git_busy()));
    assert!(main.read_with(cx, |m, _| m.git_status().is_none()), "the old branch leaked into another workspace");
}

#[gpui::test]
fn an_external_checkout_refreshes_a_workspace_opened_in_a_subfolder(cx: &mut TestAppContext) {
    let run = Run::new("git-external", &[("src/a.cs", "class A {}\n")]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder.join("src"), w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    cx.run_until_parked();
    git(&run.folder, &["checkout", "-b", "external"]);
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some_and(|s| s.branch.as_deref() == Some("external"))));
}

#[gpui::test]
fn committing_checked_files_keeps_unrelated_staged_changes(cx: &mut TestAppContext) {
    let run = Run::new("git-commit", &[("a.cs", "class A {}\n"), ("b.cs", "class B {}\n")]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    std::fs::write(run.folder.join("a.cs"), "class A { int a; }\n").unwrap();
    std::fs::write(run.folder.join("b.cs"), "class B { int b; }\n").unwrap();
    git(&run.folder, &["add", "b.cs"]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    main.update_in(cx, |m, w, cx| {
        m.git_request(
            rsharp_app::git_view::GitRequest::Commit {
                paths: vec![run.folder.join("a.cs")],
                message: "chosen file\n\nDetailed explanation".into(),
                amend: false,
                push: false,
            },
            w,
            cx,
        )
    });
    wait(cx, |cx| main.read_with(cx, |m, _| !m.git_busy()));
    assert_eq!(git(&run.folder, &["show", "HEAD:a.cs"]), "class A { int a; }\n");
    assert_eq!(git(&run.folder, &["show", "HEAD:b.cs"]), "class B {}\n", "Commit included an unchecked staged file");
    assert!(git(&run.folder, &["diff", "--cached", "--name-only"]).contains("b.cs"));
}

#[gpui::test]
fn a_commit_saves_a_buffer_that_lives_beside_the_opened_subfolder(cx: &mut TestAppContext) {
    use rsharp_app::git_view::GitRequest;
    let run = Run::new("git-save-outside", &[("src/a.cs", "class A {}\n"), ("notes/b.txt", "one\n")]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, window, cx| {
        m.open_workspace(&run.folder.join("src"), window, cx);
        m.open_file(&run.folder.join("notes/b.txt"), Some((0, 0)), window, cx);
    });
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    let notes = run.folder.join("notes/b.txt");
    cx.simulate_input("typed ");
    cx.run_until_parked();
    assert_eq!(std::fs::read_to_string(&notes).unwrap(), "one\n", "typing alone wrote the file");
    main.update_in(cx, |m, window, cx| {
        m.git_request(
            GitRequest::Commit { paths: vec![notes.clone()], message: "notes".into(), amend: false, push: false },
            window,
            cx,
        )
    });
    wait(cx, |cx| main.read_with(cx, |m, _| !m.git_busy()));
    assert_eq!(
        git(&run.folder, &["show", "HEAD:notes/b.txt"]),
        "typed one\n",
        "the commit took the file as it was on disk, not the buffer opened beside the workspace's subfolder"
    );
}

#[gpui::test]
fn leaving_the_workspace_ends_the_git_work_queued_for_it(cx: &mut TestAppContext) {
    use rsharp_app::git_view::GitRequest;
    let run = Run::new("git-leave", &[("a.cs", "class A {}\n")]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    std::fs::write(run.folder.join("a.cs"), "class A { int a; }\n").unwrap();
    let other = run.folder.parent().unwrap().join("elsewhere");
    std::fs::create_dir(&other).unwrap();
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, window, cx| m.open_workspace(&run.folder, window, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    // A commit and the Log's read are queued, and the workspace changes before either worker is given a turn.
    main.update_in(cx, |m, window, cx| {
        m.git_request(
            GitRequest::Commit {
                paths: vec![run.folder.join("a.cs")],
                message: "queued".into(),
                amend: false,
                push: false,
            },
            window,
            cx,
        );
        m.toggle_tool(Tool::Git, window, cx);
        m.open_workspace(&other, window, cx);
    });
    cx.run_until_parked();
    assert_eq!(
        git(&run.folder, &["log", "--format=%s"]).lines().count(),
        1,
        "work queued for a workspace committed into the repository it had left"
    );
    assert_eq!(std::fs::read_to_string(run.folder.join("a.cs")).unwrap(), "class A { int a; }\n");
    assert!(
        main.read_with(cx, |m, cx| m.git_log_view().read(cx).commits().is_empty()),
        "a reply for the repository that was left filled the Log of the next workspace"
    );
    assert!(main.read_with(cx, |m, _| m.git_status().is_none()));
}

#[gpui::test]
fn a_branch_checked_out_from_the_branch_list_is_named_by_its_full_ref(cx: &mut TestAppContext) {
    use rsharp_app::git_view::GitRequest;
    use rsharp_app::window::ToggleGit;
    let run = Run::new("git-branch-names", &[("a.cs", "class A {}\n")]);
    let remote = run.folder.parent().unwrap().join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare"]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    git(&run.folder, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&run.folder, &["push", "-u", "origin", "main"]);
    git(&run.folder, &["push", "origin", "main:feature"]);
    git(&run.folder, &["fetch", "origin"]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, window, cx| m.open_workspace(&run.folder, window, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    cx.dispatch_action(ToggleGit);
    wait(cx, |cx| {
        main.read_with(cx, |m, cx| {
            m.git_log_view().read(cx).refs().iter().any(|r| r.full_name == "refs/remotes/origin/feature")
        })
    });
    main.update_in(cx, |m, window, cx| {
        m.git_request(GitRequest::Checkout("refs/remotes/origin/feature".into()), window, cx)
    });
    wait(cx, |cx| {
        main.read_with(cx, |m, _| {
            !m.git_busy() && m.git_status().is_some_and(|s| s.branch.as_deref() == Some("feature"))
        })
    });
    assert_eq!(
        git(&run.folder, &["rev-parse", "--abbrev-ref", "feature@{upstream}"]).trim(),
        "origin/feature",
        "the branch made from a remote ref does not track it"
    );
    main.update_in(cx, |m, window, cx| m.git_request(GitRequest::Checkout("refs/heads/main".into()), window, cx));
    wait(cx, |cx| {
        main.read_with(cx, |m, _| !m.git_busy() && m.git_status().is_some_and(|s| s.branch.as_deref() == Some("main")))
    });
    assert!(
        git(&run.folder, &["branch", "--list", "refs/heads/refs"]).trim().is_empty(),
        "a local ref's full name became a branch of its own"
    );
}

#[gpui::test]
fn a_conflicted_rebase_is_resolved_in_the_merge_window_and_pushed(cx: &mut TestAppContext) {
    use rsharp_app::git_view::GitRequest;
    use rsharp_core::git::{ChangeKind, Operation};
    let run = Run::new("git-rebase-window", &[("a.cs", "class A { int n = 0; }\n")]);
    let remote = run.folder.parent().unwrap().join("remote.git");
    std::fs::create_dir(&remote).unwrap();
    git(&remote, &["init", "--bare"]);
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    git(&run.folder, &["remote", "add", "origin", remote.to_str().unwrap()]);
    git(&run.folder, &["push", "-u", "origin", "main"]);
    git(&run.folder, &["checkout", "-b", "feature"]);
    std::fs::write(run.folder.join("a.cs"), "class A { int n = 1; }\n").unwrap();
    git(&run.folder, &["commit", "-am", "feature"]);
    git(&run.folder, &["checkout", "main"]);
    std::fs::write(run.folder.join("a.cs"), "class A { int n = 2; }\n").unwrap();
    git(&run.folder, &["commit", "-am", "main changed"]);
    git(&run.folder, &["checkout", "feature"]);
    git(&run.folder, &["config", "user.name", "Test"]);
    git(&run.folder, &["config", "user.email", "test@example.invalid"]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    main.update_in(cx, |m, w, cx| m.git_request(GitRequest::Rebase("main".into()), w, cx));
    wait(cx, |cx| {
        main.read_with(cx, |m, _| !m.git_busy() && m.git_status().is_some_and(|s| s.operation == Operation::Rebase))
    });
    assert!(
        main.read_with(cx, |m, _| m.git_status().unwrap().changes.iter().any(|c| c.kind == ChangeKind::Conflicted))
    );
    main.update_in(cx, |m, w, cx| m.git_request(GitRequest::Resolve(run.folder.join("a.cs")), w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_merge_window().is_some()));
    let handle = main.read_with(cx, |m, _| m.git_merge_window().unwrap());
    let merge = cx.update(|_, cx| handle.update(cx, |_, _, cx| cx.entity()).unwrap());
    wait(cx, |cx| merge.read_with(cx, |v, _| !v.is_loading()));
    assert_eq!(merge.read_with(cx, |v, _| v.unresolved_count()), 1);
    merge.update(cx, |v, cx| v.accept_chunk(0, rsharp_text::MergeChoice::Left, cx));
    assert_eq!(
        merge.read_with(cx, |v, cx| v.result(cx)),
        "class A { int n = 1; }\n",
        "Yours was reversed during rebase"
    );
    let conflict_bytes = std::fs::read(run.folder.join("a.cs")).unwrap();
    std::fs::write(run.folder.join("a.cs"), "external edit\n").unwrap();
    merge.update(cx, |v, cx| {
        let result = v.result(cx);
        cx.emit(rsharp_app::git_view::MergeRequest::Apply(result));
    });
    wait(cx, |cx| main.read_with(cx, |m, _| !m.git_busy() && m.said().is_some_and(|s| s.contains("conflict changed"))));
    assert_eq!(
        std::fs::read_to_string(run.folder.join("a.cs")).unwrap(),
        "external edit\n",
        "a stale merge overwrote an external edit"
    );
    std::fs::write(run.folder.join("a.cs"), conflict_bytes).unwrap();
    merge.update(cx, |v, cx| {
        let result = v.result(cx);
        cx.emit(rsharp_app::git_view::MergeRequest::Apply(result));
    });
    wait(cx, |cx| {
        main.read_with(cx, |m, _| {
            !m.git_busy() && m.git_status().is_some_and(|s| s.changes.iter().all(|c| c.kind != ChangeKind::Conflicted))
        })
    });
    main.update_in(cx, |m, w, cx| m.git_request(GitRequest::Continue, w, cx));
    wait(cx, |cx| {
        main.read_with(cx, |m, _| !m.git_busy() && m.git_status().is_some_and(|s| s.operation == Operation::Idle))
    });
    main.update_in(cx, |m, w, cx| m.git_request(GitRequest::Push, w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| !m.git_busy()));
    assert_eq!(git(&remote, &["show", "refs/heads/feature:a.cs"]), "class A { int n = 1; }\n");
}

#[gpui::test]
fn a_conflict_resolved_in_the_merge_window_keeps_the_files_encoding(cx: &mut TestAppContext) {
    use rsharp_app::git_view::GitRequest;
    use rsharp_core::git::{ChangeKind, Operation};
    // windows-1252's "é" is one byte, which a merge must write back as it found it.
    let base: &[u8] = b"class A { string s = \"base\"; }\n";
    let ours: &[u8] = b"class A { string s = \"caf\xE9\"; }\n";
    let theirs: &[u8] = b"class A { string s = \"th\xE9\"; }\n";
    let run = Run::new("git-merge-encoding", &[(".editorconfig", "root = true\n\n[*]\ncharset = latin1\n")]);
    std::fs::write(run.folder.join("a.cs"), base).unwrap();
    git(&run.folder, &["init", "-b", "main"]);
    git(&run.folder, &["add", "."]);
    git(&run.folder, &["commit", "-m", "initial"]);
    git(&run.folder, &["checkout", "-b", "feature"]);
    std::fs::write(run.folder.join("a.cs"), ours).unwrap();
    git(&run.folder, &["commit", "-am", "feature"]);
    git(&run.folder, &["checkout", "main"]);
    std::fs::write(run.folder.join("a.cs"), theirs).unwrap();
    git(&run.folder, &["commit", "-am", "main changed"]);
    git(&run.folder, &["checkout", "feature"]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |m, w, cx| m.open_workspace(&run.folder, w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_status().is_some() && !m.git_busy()));
    main.update_in(cx, |m, w, cx| m.git_request(GitRequest::Rebase("main".into()), w, cx));
    wait(cx, |cx| {
        main.read_with(cx, |m, _| !m.git_busy() && m.git_status().is_some_and(|s| s.operation == Operation::Rebase))
    });
    main.update_in(cx, |m, w, cx| m.git_request(GitRequest::Resolve(run.folder.join("a.cs")), w, cx));
    wait(cx, |cx| main.read_with(cx, |m, _| m.git_merge_window().is_some()));
    let handle = main.read_with(cx, |m, _| m.git_merge_window().unwrap());
    let merge = cx.update(|_, cx| handle.update(cx, |_, _, cx| cx.entity()).unwrap());
    wait(cx, |cx| merge.read_with(cx, |v, _| !v.is_loading()));
    merge.update(cx, |v, cx| v.accept_chunk(0, rsharp_text::MergeChoice::Left, cx));
    merge.update(cx, |v, cx| {
        let result = v.result(cx);
        cx.emit(rsharp_app::git_view::MergeRequest::Apply(result));
    });
    wait(cx, |cx| {
        main.read_with(cx, |m, _| {
            !m.git_busy() && m.git_status().is_some_and(|s| s.changes.iter().all(|c| c.kind != ChangeKind::Conflicted))
        })
    });
    assert_eq!(
        std::fs::read(run.folder.join("a.cs")).unwrap(),
        ours,
        "the resolved file was written in another encoding than the one it and its .editorconfig name"
    );
}
