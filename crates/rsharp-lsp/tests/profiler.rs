//! The profiler host against a real program: Passenger's LoopingConsoleApp (it spins in managed code until ended) started
//! under it, its heap read while it runs, its snapshot taken by Stop. Skips, saying why, without dotnet or the host.

use rsharp_lsp::profiler::{Ended, Profiler, Target};
use std::path::{Path, PathBuf};
use std::time::Duration;

fn looping() -> Option<PathBuf> {
    let dotnet = rsharp_core::programs::dotnet()?;
    let project =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../vendor/passenger/tests/Debuggees/LoopingConsoleApp");
    let output = Path::new(env!("CARGO_TARGET_TMPDIR")).join("profiled").join("LoopingConsoleApp");
    let status = std::process::Command::new(dotnet)
        .arg("build")
        .arg(project.join("LoopingConsoleApp.csproj"))
        .args(["-c", "Debug", "-o"])
        .arg(&output)
        .args(["-nologo", "-v", "q"])
        .status()
        .ok()?;
    status.success().then(|| output.join("LoopingConsoleApp.dll"))
}

#[test]
fn a_running_program_is_sampled_its_heap_read_and_its_snapshot_taken_by_stop() {
    if !rsharp_lsp::profiler::available() {
        eprintln!("skipped: RSharp's build carries no profiler host");
        return;
    }
    let Some(assembly) = looping() else {
        eprintln!("skipped: dotnet or the looping debuggee is not there");
        return;
    };
    let folder = Path::new(env!("CARGO_TARGET_TMPDIR")).join("profiler-traces");
    std::fs::create_dir_all(&folder).unwrap();
    let profiler = Profiler::start().expect("the host starts");
    let target = Target {
        program: rsharp_core::programs::dotnet().unwrap(),
        arguments: vec![assembly.display().to_string()],
        working_directory: assembly.parent().unwrap().to_path_buf(),
        environment: Default::default(),
    };
    let started = profiler.profile(&target, &folder).expect("the program starts under the profiler");
    assert!(started.trace.starts_with(&folder), "the trace is not written where it was asked: {:?}", started.trace);

    let following = {
        let profiler = profiler.clone();
        std::thread::spawn(move || profiler.run(|_, _| {}))
    };
    std::thread::sleep(Duration::from_secs(3));
    let memory = profiler.memory().expect("the heap is read while the program runs");
    assert!(memory["objects"].as_i64().unwrap_or(0) > 0, "a running program's heap holds no objects: {memory}");

    let snapshot = profiler.stop().expect("stop takes the snapshot");
    assert!(snapshot["samples"].as_i64().unwrap_or(0) > 0, "a program spinning for seconds was never sampled");
    assert!(
        snapshot["hotSpots"].as_array().is_some_and(|h| !h.is_empty())
            && snapshot["threads"].as_array().is_some_and(|t| !t.is_empty()),
        "the snapshot has no hot spots or no call tree"
    );
    assert_eq!(following.join().unwrap().expect("the run ends"), Ended::Stopped, "Stop is a stopped run");
    let _ = profiler.kill();
}
