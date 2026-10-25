use std::{sync::mpsc, time::Duration};

#[test]
fn a_git_folder_below_an_objects_directory_still_reports_changes() {
    let root =
        std::env::temp_dir().join(format!("rsharp-git-watch-objects-{}", std::process::id())).join("objects/repo");
    let _ = std::fs::remove_dir_all(&root);
    let metadata = root.join(".git");
    std::fs::create_dir_all(metadata.join("refs/heads")).unwrap();
    std::fs::write(metadata.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let (send, receive) = mpsc::channel();
    let _watcher = rsharp_core::watcher::watch_git(&metadata, move |paths| {
        let _ = send.send(paths);
    })
    .unwrap();
    std::fs::write(metadata.join("HEAD"), "ref: refs/heads/other\n").unwrap();
    let mut found = false;
    for _ in 0..10 {
        if let Ok(paths) = receive.recv_timeout(Duration::from_millis(500)) {
            if paths.iter().any(|p| p.ends_with("HEAD")) {
                found = true;
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(root.parent().unwrap().parent().unwrap());
    assert!(found, "a watch folder below a directory called objects lost every change");
}

#[test]
fn git_metadata_changes_arrive_when_the_workspace_is_elsewhere() {
    let root = std::env::temp_dir().join(format!("rsharp-git-watch-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let metadata = root.join(".git");
    std::fs::create_dir_all(metadata.join("refs/heads")).unwrap();
    std::fs::write(metadata.join("HEAD"), "ref: refs/heads/main\n").unwrap();
    let (send, receive) = mpsc::channel();
    let _watcher = rsharp_core::watcher::watch_git(&metadata, move |paths| {
        let _ = send.send(paths);
    })
    .unwrap();
    std::fs::write(metadata.join("HEAD"), "ref: refs/heads/other\n").unwrap();
    let mut found = false;
    for _ in 0..10 {
        if let Ok(paths) = receive.recv_timeout(Duration::from_millis(500)) {
            if paths.iter().any(|p| p.ends_with("HEAD")) {
                found = true;
                break;
            }
        }
    }
    let _ = std::fs::remove_dir_all(root);
    assert!(found, "the Git watcher missed a HEAD write outside the workspace");
}
