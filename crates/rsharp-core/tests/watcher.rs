use rsharp_core::watcher::watch;
use std::path::PathBuf;
use std::sync::mpsc;
use std::time::Duration;

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rsharp-watcher-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    for made in ["src", "bin", "obj", ".git/objects", ".git/refs/heads"] {
        std::fs::create_dir_all(folder.join(made)).unwrap();
    }
    rsharp_core::paths::canonical(&folder)
}

/// The paths of the bursts heard within a few seconds.
fn heard(folder: &PathBuf, change: impl FnOnce()) -> Vec<PathBuf> {
    let (sender, bursts) = mpsc::channel();
    let _watcher = watch(folder, move |paths| {
        let _ = sender.send(paths);
    })
    .expect("the folder could not be watched");
    // FSEvents and inotify start a moment after they are asked.
    std::thread::sleep(Duration::from_millis(300));
    change();
    let mut all = Vec::new();
    while let Ok(paths) = bursts.recv_timeout(Duration::from_secs(3)) {
        all.extend(paths);
        if all.iter().any(|p| p.ends_with("last.txt")) {
            break;
        }
    }
    all
}

#[test]
fn a_file_made_in_the_folder_is_reported_once_its_burst_ends() {
    let folder = folder("made");
    let paths = heard(&folder, || {
        std::fs::write(folder.join("src/Order.cs"), "class Order { }").unwrap();
        std::fs::write(folder.join("src/last.txt"), "").unwrap();
    });
    assert!(paths.contains(&folder.join("src/Order.cs")), "a file made would not be heard: {paths:?}");
}

#[test]
fn bin_obj_and_git_objects_are_passed_over_but_git_head_is_not() {
    let folder = folder("passed");
    let paths = heard(&folder, || {
        std::fs::write(folder.join("bin/App.dll"), "").unwrap();
        std::fs::write(folder.join("obj/App.dll"), "").unwrap();
        std::fs::write(folder.join(".git/objects/ab"), "").unwrap();
        std::fs::write(folder.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(folder.join("last.txt"), "").unwrap();
    });
    assert!(
        !paths.iter().any(|p| p.starts_with(folder.join("bin")) || p.starts_with(folder.join("obj"))),
        "build output heard: {paths:?}"
    );
    assert!(!paths.iter().any(|p| p.starts_with(folder.join(".git/objects"))), "Git's objects heard: {paths:?}");
    assert!(paths.contains(&folder.join(".git/HEAD")), "Git's HEAD would not be heard: {paths:?}");
}
