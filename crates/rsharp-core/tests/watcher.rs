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
fn bin_obj_and_git_objects_are_passed_over_but_git_head_and_a_restore_are_not() {
    let folder = folder("passed");
    std::fs::create_dir_all(folder.join("artifacts/obj/App")).unwrap();
    let paths = heard(&folder, || {
        std::fs::write(folder.join("bin/App.dll"), "").unwrap();
        std::fs::write(folder.join("obj/App.dll"), "").unwrap();
        std::fs::write(folder.join("obj/project.assets.json"), "{}").unwrap();
        std::fs::write(folder.join("artifacts/obj/App/project.assets.json"), "{}").unwrap();
        std::fs::write(folder.join(".git/objects/ab"), "").unwrap();
        std::fs::write(folder.join(".git/HEAD"), "ref: refs/heads/main").unwrap();
        std::fs::write(folder.join("last.txt"), "").unwrap();
    });
    let restores = [folder.join("obj/project.assets.json"), folder.join("artifacts/obj/App/project.assets.json")];
    assert!(
        !paths
            .iter()
            .any(|p| (p.starts_with(folder.join("bin")) || p.starts_with(folder.join("obj"))) && !restores.contains(p)),
        "build output heard: {paths:?}"
    );
    for restore in &restores {
        assert!(paths.contains(restore), "a restore would not reach the XAML code help: {paths:?}");
    }
    assert!(!paths.iter().any(|p| p.starts_with(folder.join(".git/objects"))), "Git's objects heard: {paths:?}");
    assert!(paths.contains(&folder.join(".git/HEAD")), "Git's HEAD would not be heard: {paths:?}");
}

#[test]
fn writes_that_never_pause_are_still_handed_over() {
    let folder = folder("busy");
    let (sender, bursts) = mpsc::channel();
    let _watcher = watch(&folder, move |paths| {
        let _ = sender.send(paths);
    })
    .unwrap();
    std::thread::sleep(Duration::from_millis(300));
    // A write every 50 ms for three seconds: never the 150 ms quiet a burst waits for.
    let writer = {
        let folder = folder.clone();
        std::thread::spawn(move || {
            for i in 0..60 {
                std::fs::write(folder.join("src/busy.log"), format!("{i}")).unwrap();
                std::thread::sleep(Duration::from_millis(50));
            }
        })
    };
    let first = bursts.recv_timeout(Duration::from_millis(2500));
    writer.join().unwrap();
    assert!(first.is_ok(), "a folder written to without a pause would never hand over its changes");
}
