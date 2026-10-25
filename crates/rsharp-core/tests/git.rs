use rsharp_core::git::{ChangeKind, Operation, Repository, Side};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

struct Fixture(PathBuf);
impl Fixture {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("rsharp-git-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        let f = Self(rsharp_core::paths::canonical(&path));
        f.git(&["init", "-b", "main"]);
        f.git(&["config", "user.name", "Test"]);
        f.git(&["config", "user.email", "test@example.invalid"]);
        f.git(&["config", "commit.gpgsign", "false"]);
        f.git(&["config", "core.hooksPath", "disabled-hooks"]);
        f
    }
    fn git(&self, args: &[&str]) -> String {
        git(&self.0, args)
    }
    fn write(&self, name: &str, text: &str) -> PathBuf {
        let path = self.0.join(name);
        std::fs::write(&path, text).unwrap();
        path
    }
    fn repo(&self) -> Repository {
        Repository::open(&self.0).unwrap().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
fn git(root: &Path, args: &[&str]) -> String {
    let mut command = Command::new("git");
    command
        .current_dir(root)
        .args(args)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", if cfg!(windows) { "NUL" } else { "/dev/null" })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = rsharp_core::process::spawn(command).unwrap();
    let stdout = child.process.stdout.take().unwrap();
    let output = std::thread::spawn(move || {
        let mut stdout = stdout;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut stdout, &mut bytes).unwrap();
        bytes
    });
    let stderr = child.process.stderr.take().unwrap();
    let errors = std::thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        std::io::Read::read_to_end(&mut stderr, &mut bytes).unwrap();
        bytes
    });
    let status = child.wait().unwrap();
    let bytes = output.join().unwrap();
    let error = errors.join().unwrap();
    assert!(status.success(), "git {args:?}: {}", String::from_utf8_lossy(&error));
    String::from_utf8(bytes).unwrap().trim_end().to_string()
}

#[test]
fn discovery_status_selected_commits_and_message_only_amend_preserve_the_index() {
    let f = Fixture::new("selected");
    let repo = f.repo();
    assert_eq!(repo.status().unwrap().head, None);
    assert!(repo.log_all(10, 0).unwrap().is_empty());
    assert!(repo.bytes("HEAD", &f.0.join("not-created")).unwrap().is_none());
    assert_eq!(repo.status().unwrap().branch.as_deref(), Some("main"));
    let odd = f.write("-[a] café.txt", "first\n");
    let other = f.write("other.txt", "old\n");
    repo.commit(&[odd.clone(), other.clone()], "first", false).unwrap();
    f.write("other.txt", "staged\n");
    f.git(&["add", "other.txt"]);
    f.write("other.txt", "unstaged\n");
    f.write("-[a] café.txt", "second\n");
    repo.commit(&[odd.clone()], "chosen", false).unwrap();
    assert_eq!(repo.text("HEAD", &odd).unwrap().as_deref(), Some("second\n"));
    assert_eq!(f.git(&["show", ":other.txt"]), "staged");
    assert_eq!(repo.text("HEAD", &other).unwrap().as_deref(), Some("old\n"));
    repo.commit(&[], "message amended", true).unwrap();
    assert_eq!(repo.text("HEAD", &other).unwrap().as_deref(), Some("old\n"));
    assert_eq!(f.git(&["show", ":other.txt"]), "staged");
    assert_eq!(repo.log(None, 10, 0).unwrap()[0].subject, "message amended");
    assert!(repo.bytes("HEAD", &f.0.join("missing")).unwrap().is_none());
    assert!(repo.bytes("bad-revision", &odd).is_err());
    assert!(repo.commit(&[], "empty selection", false).is_err());
    assert!(repo.commit(&[odd], "  ", false).is_err());
}

#[test]
fn local_remote_rebase_conflict_is_resolved_continued_and_pushed() {
    let f = Fixture::new("acceptance");
    let file = f.write("file.txt", "base\n");
    let repo = f.repo();
    repo.commit(&[file.clone()], "base", false).unwrap();
    let remote = f.0.join("remote.git");
    git(&f.0, &["init", "--bare", remote.to_str().unwrap()]);
    f.git(&["remote", "add", "origin", remote.to_str().unwrap()]);
    repo.push().unwrap();
    repo.create_branch("feature", None).unwrap();
    f.write("file.txt", "my feature\n");
    repo.commit(&[file.clone()], "feature", false).unwrap();
    let peer = f.0.join("peer");
    f.git(&["clone", "--branch", "main", remote.to_str().unwrap(), peer.to_str().unwrap()]);
    git(&peer, &["config", "user.name", "Peer"]);
    git(&peer, &["config", "user.email", "peer@example.invalid"]);
    git(&peer, &["config", "commit.gpgsign", "false"]);
    git(&peer, &["config", "core.hooksPath", "disabled-hooks"]);
    std::fs::write(peer.join("file.txt"), "upstream\n").unwrap();
    let peer_repo = Repository::open(&peer).unwrap().unwrap();
    peer_repo.commit(&[peer.join("file.txt")], "upstream", false).unwrap();
    peer_repo.push().unwrap();
    repo.fetch().unwrap();
    assert!(repo.rebase("origin/main").is_err());
    let status = repo.status().unwrap();
    assert_eq!(status.operation, Operation::Rebase);
    assert_eq!(status.changes[0].kind, ChangeKind::Conflicted);
    let stages = repo.stages(&file).unwrap();
    assert_eq!(stages.ours.as_deref(), Some(&b"upstream\n"[..]));
    assert_eq!(stages.theirs.as_deref(), Some(&b"my feature\n"[..]));
    assert_eq!(stages.side(Operation::Rebase, Side::Yours), stages.theirs.as_deref());
    repo.accept_side(&file, Side::Yours).unwrap();
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "my feature\n");
    repo.continue_operation().unwrap();
    assert_eq!(repo.status().unwrap().operation, Operation::Idle);
    repo.push().unwrap();
    assert_eq!(git(&remote, &["show", "feature:file.txt"]), "my feature");
}

#[test]
fn discovery_distinguishes_absence_corruption_worktrees_and_detached_head() {
    let f = Fixture::new("discovery");
    let file = f.write("file.txt", "base\n");
    let repo = f.repo();
    repo.commit(&[file], "base", false).unwrap();
    let nested = f.0.join("nested");
    std::fs::create_dir(&nested).unwrap();
    assert_eq!(Repository::open(&nested).unwrap().unwrap().root, repo.root);
    let linked = f.0.join("linked");
    f.git(&["worktree", "add", "-b", "linked", linked.to_str().unwrap()]);
    let linked = Repository::open(&linked).unwrap().unwrap();
    assert_ne!(linked.git_dir, linked.root.join(".git"));
    assert_eq!(linked.metadata_dirs().unwrap().len(), 2);
    assert_eq!(linked.status().unwrap().branch.as_deref(), Some("linked"));
    f.git(&["checkout", "--detach"]);
    assert!(repo.status().unwrap().branch.is_none());
    assert!(repo.push().is_err());
    let outside = Fixture::new("outside");
    std::fs::remove_dir_all(outside.0.join(".git")).unwrap();
    assert!(Repository::open(&outside.0).unwrap().is_none());
    std::fs::write(outside.0.join(".git"), "gitdir: nonexistent\n").unwrap();
    assert!(Repository::open(&outside.0).is_err());
}

#[test]
fn staged_rename_preserves_the_blob_and_selected_commit_tracks_both_names() {
    let f = Fixture::new("rename");
    let from = f.write("[old].txt", "base\nsecond line\nthird line\n");
    let to = f.0.join("-new café.txt");
    let repo = f.repo();
    repo.commit(&[from.clone()], "base", false).unwrap();
    f.write("[old].txt", "staged\nsecond line\nthird line\n");
    f.git(&["add", "--", "[old].txt"]);
    f.write("[old].txt", "unstaged\nsecond line\nthird line\n");
    std::fs::rename(&from, &to).unwrap();
    repo.move_index(&from, &to).unwrap();
    assert_eq!(f.git(&["show", ":-new café.txt"]), "staged\nsecond line\nthird line");
    assert_eq!(std::fs::read_to_string(&to).unwrap(), "unstaged\nsecond line\nthird line\n");
    let change = repo.status().unwrap().changes.into_iter().find(|c| c.path == to).unwrap();
    assert_eq!(change.kind, ChangeKind::Renamed);
    assert_eq!(change.from, Some(from.clone()));
    repo.commit(&[to.clone()], "renamed", false).unwrap();
    assert!(repo.bytes("HEAD", &from).unwrap().is_none());
    assert_eq!(repo.text("HEAD", &to).unwrap().as_deref(), Some("unstaged\nsecond line\nthird line\n"));
    assert_eq!(repo.history(&to, 10).unwrap().len(), 2);
    assert_eq!(repo.commit_files("HEAD").unwrap().len(), 1);
}

#[test]
fn moving_a_folder_moves_every_tracked_file_in_the_index() {
    let f = Fixture::new("move-folder");
    std::fs::create_dir_all(f.0.join("old/nested")).unwrap();
    let one = f.write("old/one.txt", "one\n");
    let two = f.write("old/nested/two.txt", "two\n");
    let repo = f.repo();
    repo.commit(&[one, two], "base", false).unwrap();
    let from = f.0.join("old");
    let to = f.0.join("new");
    std::fs::rename(&from, &to).unwrap();
    repo.move_index(&from, &to).unwrap();
    assert_eq!(
        f.git(&["diff", "--cached", "--name-status"]),
        "R100\told/nested/two.txt\tnew/nested/two.txt\nR100\told/one.txt\tnew/one.txt",
        "a moved folder kept only one of its files' index entries"
    );
}

#[test]
fn merge_completion_excludes_unchosen_staged_changes_and_rollback_restores_only_chosen_files() {
    let f = Fixture::new("merge");
    let a = f.write("a.txt", "base\n");
    let b = f.write("b.txt", "base\n");
    let c = f.write("c.txt", "base\n");
    let repo = f.repo();
    repo.commit(&[a.clone(), b.clone(), c.clone()], "base", false).unwrap();
    repo.create_branch("feature", None).unwrap();
    f.write("a.txt", "feature\n");
    repo.commit(&[a.clone()], "feature", false).unwrap();
    repo.checkout("main").unwrap();
    f.write("b.txt", "main\n");
    repo.commit(&[b.clone()], "main", false).unwrap();
    f.git(&["merge", "--no-commit", "feature"]);
    f.write("c.txt", "leave out\n");
    f.git(&["add", "c.txt"]);
    repo.commit(&[a.clone()], "merge selected", false).unwrap();
    assert_eq!(repo.commit_files("HEAD").unwrap()[0].path, a);
    assert_eq!(repo.text("HEAD", &c).unwrap().as_deref(), Some("base\n"));
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "leave out\n");
    assert_eq!(repo.status().unwrap().operation, Operation::Idle);
    f.write("a.txt", "rollback\n");
    let selected = repo.status().unwrap().changes.into_iter().filter(|change| change.path == a).collect::<Vec<_>>();
    repo.rollback(&selected).unwrap();
    assert_eq!(std::fs::read_to_string(&a).unwrap(), "feature\n");
    assert_eq!(std::fs::read_to_string(&c).unwrap(), "leave out\n");
}

#[test]
fn modify_delete_acceptance_can_choose_deletion_and_abort_restores_the_branch() {
    let f = Fixture::new("delete-conflict");
    let file = f.write("file.txt", "base\n");
    let repo = f.repo();
    repo.commit(&[file.clone()], "base", false).unwrap();
    repo.create_branch("feature", None).unwrap();
    std::fs::remove_file(&file).unwrap();
    repo.commit(&[file.clone()], "delete", false).unwrap();
    repo.checkout("main").unwrap();
    f.write("file.txt", "main\n");
    repo.commit(&[file.clone()], "main", false).unwrap();
    repo.checkout("feature").unwrap();
    assert!(repo.rebase("main").is_err());
    assert!(repo.stages(&file).unwrap().theirs.is_none());
    repo.accept_side(&file, Side::Yours).unwrap();
    assert!(!file.exists());
    repo.abort_operation().unwrap();
    assert_eq!(repo.status().unwrap().operation, Operation::Idle);
    assert!(!file.exists());
}

#[test]
fn branch_validation_and_literal_history_do_not_consume_option_like_names() {
    let f = Fixture::new("history");
    let file = f.write("-[a] café.txt", "line one\nline two\n");
    let repo = f.repo();
    repo.commit(&[file.clone()], "base", false).unwrap();
    f.write("-[a] café.txt", "changed\nline two\n");
    repo.commit(&[file.clone()], "edit", false).unwrap();
    assert_eq!(repo.history_lines(&file, 1, 1, 10).unwrap().len(), 2);
    assert!(repo.create_branch("--orphan=bad", None).is_err());
    repo.create_branch("new", Some("HEAD~1")).unwrap();
    assert_eq!(repo.log_all(10, 0).unwrap().len(), 2);
    assert_eq!(repo.log(None, 10, 0).unwrap().len(), 1);
    repo.rename_branch("new", "renamed").unwrap();
    assert!(repo.delete_branch("renamed", true).is_err());
    repo.checkout("main").unwrap();
    repo.delete_branch("renamed", false).unwrap();
    assert!(repo.refs().unwrap().iter().all(|r| r.name != "renamed"));
    assert!(repo.mark_resolved(&f.0.join("../outside.txt")).is_err());
    assert!(repo.mark_resolved(&f.0.join(".git/config")).is_err());
}

#[cfg(unix)]
#[test]
fn failed_hook_is_reported_and_cancelling_a_hook_ends_its_process_tree() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("hook");
    let file = f.write("file.txt", "base\n");
    let repo = f.repo();
    repo.commit(&[file.clone()], "base", false).unwrap();
    f.write("file.txt", "staged\n");
    f.write("other.txt", "unselected staged\n");
    f.git(&["add", "file.txt", "other.txt"]);
    f.write("file.txt", "unstaged\n");
    let hooks = f.0.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    f.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let hook = hooks.join("pre-commit");
    std::fs::write(&hook, "#!/bin/sh\nprintf 'rejected by test hook\\n' >&2\nexit 1\n").unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert!(repo.commit(&[file.clone()], "fails", false).unwrap_err().contains("rejected by test hook"));
    assert_eq!(
        f.git(&["show", ":file.txt"]),
        "staged",
        "a failed commit replaced the selected file's previously staged blob"
    );
    assert_eq!(
        f.git(&["show", ":other.txt"]),
        "unselected staged",
        "a failed commit changed another file's staged blob"
    );
    std::fs::write(&hook, "#!/bin/sh\nhead -c 2097152 /dev/zero >&2\nexit 1\n").unwrap();
    assert!(repo.commit(&[file.clone()], "large stderr", false).unwrap_err().contains("exceeded"));
    std::fs::write(&hook, "#!/bin/sh\nprintf ready > hook-started\nsleep 60 &\nwait\n").unwrap();
    let worker = repo.clone();
    let chosen = file.clone();
    let thread = std::thread::spawn(move || worker.commit(&[chosen], "cancelled", false));
    let start = std::time::Instant::now();
    while !f.0.join("hook-started").exists() {
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "hook did not start");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    repo.cancel();
    assert!(thread.join().unwrap().unwrap_err().contains("cancelled"));
    assert!(start.elapsed() < std::time::Duration::from_secs(5), "cancellation waited for the hook");
    assert_eq!(repo.log(None, 10, 0).unwrap().len(), 1);
    assert_eq!(f.git(&["show", ":file.txt"]), "staged", "cancellation changed the real index");
    assert_eq!(f.git(&["show", ":other.txt"]), "unselected staged");
    std::fs::write(&hook, "#!/bin/sh\nexit 0\n").unwrap();
    repo.commit(&[file.clone()], "after failed hooks", false).unwrap();
    assert_eq!(repo.text("HEAD", &file).unwrap().as_deref(), Some("unstaged\n"));
    assert_eq!(f.git(&["show", ":file.txt"]), "unstaged");
    assert_eq!(f.git(&["show", ":other.txt"]), "unselected staged");
}

#[cfg(unix)]
#[test]
fn external_index_changes_during_a_commit_are_never_overwritten() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new("index-race");
    let file = f.write("file.txt", "base\n");
    let other = f.write("other.txt", "base\n");
    let repo = f.repo();
    repo.commit(&[file.clone(), other], "base", false).unwrap();
    f.write("file.txt", "chosen\n");
    f.write("other.txt", "external staged\n");
    let hooks = f.0.join("hooks");
    std::fs::create_dir(&hooks).unwrap();
    let hook = hooks.join("pre-commit");
    std::fs::write(
        &hook,
        "#!/bin/sh\nprintf ready > hook-started\nwhile test ! -f release-hook; do sleep 0.01; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&hook, std::fs::Permissions::from_mode(0o755)).unwrap();
    f.git(&["config", "core.hooksPath", hooks.to_str().unwrap()]);
    let worker = repo.clone();
    let chosen = file.clone();
    let thread = std::thread::spawn(move || worker.commit(&[chosen], "chosen", false));
    let start = std::time::Instant::now();
    while !f.0.join("hook-started").exists() {
        assert!(start.elapsed() < std::time::Duration::from_secs(5), "hook did not start");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    f.git(&["add", "other.txt"]);
    f.write("release-hook", "release");
    let error = thread.join().unwrap().unwrap_err();
    assert!(error.contains("commit succeeded") && error.contains("index changed"), "{error}");
    assert_eq!(f.git(&["show", ":other.txt"]), "external staged");
    assert_eq!(repo.text("HEAD", &file).unwrap().as_deref(), Some("chosen\n"));
}

#[test]
#[ignore]
fn missing_git_child() {
    if std::env::var_os("RSHARP_TEST_NO_GIT").is_some() {
        assert!(Repository::open(Path::new(".")).unwrap_err().contains("Git is not installed"));
    }
}

#[test]
fn missing_git_has_an_actionable_error() {
    let mut command = Command::new(std::env::current_exe().unwrap());
    command.args(["--exact", "missing_git_child", "--ignored"]).env("RSHARP_TEST_NO_GIT", "1").env("PATH", "");
    assert!(rsharp_core::process::spawn(command).unwrap().wait().unwrap().success());
}

#[test]
fn cancelling_an_operation_scope_prevents_queued_and_later_commands() {
    let f = Fixture::new("cancel-scope");
    let file = f.write("file.txt", "base\n");
    let repo = f.repo();
    let queued = repo.operation_scope();
    repo.cancel();
    assert!(queued.commit(&[file.clone()], "queued", false).unwrap_err().contains("cancelled"));
    assert!(repo.status().unwrap().head.is_none());
    let active = repo.operation_scope();
    active.commit(&[file], "first step", false).unwrap();
    repo.cancel();
    assert!(active.clone().create_branch("later-step", None).unwrap_err().contains("cancelled"));
    assert!(repo.refs().unwrap().iter().all(|r| r.name != "later-step"));
    assert!(
        active.operation_scope().status().unwrap_err().contains("cancelled"),
        "nesting a scope revived cancelled work"
    );
    assert!(repo.operation_scope().status().is_ok(), "a new operation could not start after cancellation");
}

#[cfg(unix)]
#[test]
fn non_utf8_and_newline_file_names_survive_status_and_commit() {
    use std::os::unix::ffi::OsStringExt;
    let f = Fixture::new("path-bytes");
    let name = if cfg!(target_os = "macos") { b"line\nname.txt".to_vec() } else { b"line\n\xff.txt".to_vec() };
    let file = f.0.join(std::ffi::OsString::from_vec(name));
    std::fs::write(&file, b"base\n").unwrap();
    let repo = f.repo();
    assert_eq!(repo.status().unwrap().changes[0].path, file);
    repo.commit(&[file.clone()], "odd path", false).unwrap();
    assert_eq!(repo.bytes("HEAD", &file).unwrap().as_deref(), Some(&b"base\n"[..]));
    assert_eq!(repo.history(&file, 10).unwrap().len(), 1);
}

#[cfg(unix)]
#[test]
fn discovery_preserves_newlines_in_repository_folder_names() {
    let f = Fixture::new("root\nnewline");
    assert_eq!(f.repo().root, f.0);
}
