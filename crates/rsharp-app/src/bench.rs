//! `--bench`: what RSharp's edge is measured by, on the real window and the real server. It waits for
//! Roslyn to load the solution, reads the memory of the IDE and all it started, types into the open file
//! timing each key to its drawn frame, prints one JSON line and quits.

use crate::window::MainWindow;
use gpui::{AsyncApp, Keystroke, WindowHandle};
use std::time::{Duration, Instant};

const KEYS: usize = 200;

pub async fn run(window: WindowHandle<MainWindow>, started: Instant, during_startup: bool, cx: &mut AsyncApp) {
    let (drawn, frame) = futures::channel::oneshot::channel::<Instant>();
    let _ = window.update(cx, |_, window, _| {
        window.on_next_frame(move |_, _| {
            let _ = drawn.send(Instant::now());
        })
    });
    let first_frame = frame.await.map_or(Duration::ZERO, |at| at.duration_since(started));
    let waited = Instant::now();
    let ready = loop {
        if during_startup {
            break Ok(Duration::ZERO);
        }
        let state = window.read_with(cx, |main, _| (main.server_ready(), main.server_failed().map(str::to_string)));
        match state {
            Ok((true, _)) => break Ok(waited.elapsed()),
            Ok((_, Some(why))) => break Err(why),
            Err(_) => return,
            _ if waited.elapsed() > Duration::from_secs(300) => {
                break Err("Roslyn did not load the solution in 300 s".into());
            }
            _ => cx.background_executor().timer(Duration::from_millis(100)).await,
        }
    };
    // Roslyn settles after its load (analyzers, the first diagnostics); memory is read once it has.
    if !during_startup {
        cx.background_executor().timer(Duration::from_secs(5)).await;
    }
    let (ide_kb, all_kb) = memory();
    let typing = Instant::now();

    let mut latencies = Vec::with_capacity(KEYS);
    for i in 0..KEYS {
        let key = if i % 20 == 19 { "enter" } else { "x" };
        let keystroke = Keystroke::parse(key).expect("a plain key parses");
        let (drawn, frame) = futures::channel::oneshot::channel::<Instant>();
        let pressed = Instant::now();
        // Through the window, not its view: a key the window itself handles would otherwise re-enter it.
        let any: gpui::AnyWindowHandle = window.into();
        let _ = any.update(cx, |_, window, cx| {
            window.on_next_frame(move |_, _| {
                let _ = drawn.send(Instant::now());
            });
            window.dispatch_keystroke(keystroke, cx);
            window.refresh();
        });
        if let Ok(at) = frame.await {
            latencies.push(at.duration_since(pressed).as_secs_f64() * 1000.);
        }
    }
    let typing = typing.elapsed();
    latencies.sort_by(f64::total_cmp);
    let percentile =
        |p: f64| latencies.get(((latencies.len() as f64 - 1.) * p).round() as usize).copied().unwrap_or(f64::NAN);
    println!(
        "{}",
        serde_json::json!({
            "firstFrameMs": first_frame.as_secs_f64() * 1000.,
            "solutionLoadedMs": if during_startup {None}else{ready.as_ref().map(|d| d.as_secs_f64() * 1000.).ok()},
            "duringStartup":during_startup,
            "serverProblem": ready.err(),
            "ideRssMb": ide_kb as f64 / 1024.,
            "totalRssMb": all_kb as f64 / 1024.,
            "keyToFrameMedianMs": percentile(0.5),
            "keyToFrameP95Ms": percentile(0.95),
            "keyToFrameMaxMs": latencies.last(),
            // A key whose frame waited over a second: macOS draws no frame while the window is hidden behind
            // others, so a stall says the run was disturbed, not what a key costs.
            "stalls": latencies.iter().filter(|ms| **ms > 1000.).count(),
            "typingSeconds": typing.as_secs_f64(),
            "keys": latencies.len(),
            "runSeconds": started.elapsed().as_secs_f64(),
        })
    );
    let _ = cx.update(|cx| cx.quit());
}

/// Resident memory in KB of this process, and of it with everything it started, from `ps`.
fn memory() -> (u64, u64) {
    let mut command = std::process::Command::new("ps");
    command.args(["-A", "-o", "pid=,ppid=,rss="]).stdout(std::process::Stdio::piped());
    let Ok(child) = rsharp_core::process::spawn(command) else { return (0, 0) };
    let Ok(output) = child.output() else { return (0, 0) };
    let rows: Vec<(u32, u32, u64)> = String::from_utf8_lossy(&output)
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace().map(|v| v.parse::<u64>().ok());
            Some((f.next()?? as u32, f.next()?? as u32, f.next()??))
        })
        .collect();
    let me = std::process::id();
    let mut family = vec![me];
    let mut at = 0;
    while at < family.len() {
        let parent = family[at];
        family.extend(rows.iter().filter(|(_, ppid, _)| *ppid == parent).map(|(pid, _, _)| *pid));
        at += 1;
    }
    let rss = |pid: u32| rows.iter().find(|(p, _, _)| *p == pid).map_or(0, |(_, _, kb)| *kb);
    (rss(me), family.iter().map(|p| rss(*p)).sum())
}
