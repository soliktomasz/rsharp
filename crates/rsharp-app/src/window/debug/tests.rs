use super::*;

#[test]
fn a_queued_save_still_needs_applying_after_its_buffer_closes() {
    let hot = HotReload { pending: true, ..HotReload::default() };
    assert!(hot.has_changes(&[]), "closing the saved buffer must not discard its queued update");
}

#[gpui::test]
fn a_save_during_hot_reload_is_accepted_without_losing_the_waiting_step(cx: &mut gpui::TestAppContext) {
    cx.update(|cx| {
        crate::init(std::env::temp_dir().join(format!("rsharp-hot-reload-save-{}", std::process::id())), cx);
        cx.global_mut::<crate::state::Ide>().servers = false;
        cx.global_mut::<crate::templates::Templates>().enabled = false;
    });
    let (main, cx) = cx.add_window_view(MainWindow::new);
    main.update(cx, |main, cx| {
        main.debug.phase = Phase::Paused;
        main.debug.hot_reload = Some(HotReload { applying: true, ..HotReload::default() });
        main.debug.after_apply = Some(AfterApply::Step(Step::Over));

        assert!(main.hot_reload_apply(None, cx), "a save during an update was dropped instead of queued");
        assert!(main.debug.hot_reload.as_ref().unwrap().pending, "the accepted save will never be applied");
        assert!(
            matches!(main.debug.after_apply, Some(AfterApply::Step(Step::Over))),
            "the queued save replaced the step waiting for the updated code"
        );
    });
}

#[cfg(unix)]
struct Requests {
    path: PathBuf,
    server: Arc<LanguageServer>,
}

#[cfg(unix)]
impl Requests {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("rsharp-hot-reload-{name}-{}.json", std::process::id()));
        let _ = std::fs::remove_file(&path);
        let mut command = std::process::Command::new("/bin/sh");
        command.args(["-c", "cat > \"$1\"", "hot-reload-requests"]).arg(&path);
        let server = Arc::new(LanguageServer::start(command, |_| {}).unwrap());
        Self { path, server }
    }

    fn wait_for(&self, text: &str) -> String {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        loop {
            let requests = std::fs::read_to_string(&self.path).unwrap_or_default();
            if requests.contains(text) {
                return requests;
            }
            assert!(std::time::Instant::now() < deadline, "the language server never received {text}: {requests}");
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
    }
}

#[cfg(unix)]
impl Drop for Requests {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}

#[cfg(unix)]
#[test]
fn discarded_hot_reload_outcome_ends_the_roslyn_session() {
    let requests = Requests::new("discarded-outcome");
    let outcome = HotReloadOutcome::failed("the debugger stopped".into())
        .session(Arc::new(HotReloadSession { number: 17, server: requests.server.clone() }));

    // A worker that finishes after Stop or after the window closes has nobody to take its outcome.
    drop(outcome);

    let written = requests.wait_for("Passenger.Roslyn.Extension.EndHotReload");
    assert!(written.contains("\\\"session\\\":17"), "cleanup ended a different Edit and Continue session: {written}");
}

#[cfg(unix)]
#[test]
fn ending_debugging_keeps_hot_reload_alive_until_its_worker_finishes() {
    let requests = Requests::new("worker-owner");
    let outcome = HotReloadOutcome::failed("the debugger stopped".into())
        .session(Arc::new(HotReloadSession { number: 23, server: requests.server.clone() }));
    let hot = HotReload { session: outcome.session.clone(), ..HotReload::default() };

    // Stop drops the window's state, while the worker still owns the session it is using.
    drop(hot);
    requests.server.notify("test/worker-still-running", json!({}));
    let written = requests.wait_for("test/worker-still-running");
    assert!(
        !written.contains("Passenger.Roslyn.Extension.EndHotReload"),
        "Stop ended Roslyn's session while its worker could still be committing or discarding an update"
    );

    drop(outcome);

    let written = requests.wait_for("Passenger.Roslyn.Extension.EndHotReload");
    assert!(written.contains("\\\"session\\\":23"), "the last owner failed to end its session: {written}");
}
