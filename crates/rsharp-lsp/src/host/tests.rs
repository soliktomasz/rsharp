use super::*;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(0);

struct Fixture {
    folder: PathBuf,
    host: SharedHost,
}

impl Fixture {
    fn new() -> Self {
        let folder = std::env::temp_dir().join(format!(
            "rsharp-host-protocol-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&folder).unwrap();
        let script = folder.join("host.sh");
        std::fs::write(
            &script,
            r#"
echo $$ >> starts
while [ -f slow-ready ]; do sleep 0.02; done
hold=
blocked=
echo '{"id":0,"result":{}}'
prefix=
if [ -f stall-read ]; then
    dd bs=1 count=1 of=first-byte 2>/dev/null
    echo stalled > stalled
    sleep 2
    prefix=$(cat first-byte)
fi
while IFS= read -r line; do
    line="$prefix$line"; prefix=
id=${line#*\"id\":}; id=${id%%,*}
case "$line" in
    *bad.csproj*) echo "{\"id\":$id,\"error\":\"bad project\"}" ;;
    *eof.csproj*) exit 0 ;;
    *invalid.csproj*) echo 'not JSON'; exit 0 ;;
    *blocked.csproj*) blocked=$id; echo parked >> parked ;;
    *\"method\":\"hold\"*)
        hold=$id
        echo "{\"id\":$id,\"partial\":{\"running\":[\"Running.csproj\"],\"held\":false}}"
        if [ ! -f wait ]; then
            echo "{\"id\":$id,\"partial\":{\"running\":[],\"held\":true}}"
        fi
        ;;
    *\"method\":\"cancel\"*)
        echo "{\"id\":$id,\"result\":true}"
        echo cancelled >> cancelled
        while [ -f delay-terminal ]; do sleep 0.02; done
        if [ -n "$blocked" ]; then
            echo "{\"id\":$blocked,\"partial\":{\"infos\":\"released\"}}"
            echo "{\"id\":$blocked,\"result\":true}"
            blocked=
        fi
        echo "{\"id\":$hold,\"error\":\"cancelled\"}"
        hold=
        echo terminal >> terminal
        ;;
    *\"method\":\"evaluate\"*)
        echo "{\"id\":$id,\"partial\":{\"complete\":false}}"
        echo "{\"id\":$id,\"partial\":{\"targetFrameworks\":[\"net8.0\",\"net9.0\"]}}"
        echo "{\"id\":$id,\"result\":true}"
        ;;
    *\"method\":\"inspect\"*)
        echo "{\"id\":$id,\"result\":{\"evaluated\":true,\"project\":\"inspected.csproj\"}}"
        ;;
    *)
        echo "{\"id\":$id,\"partial\":{\"infos\":\"[$$]\"}}"
        echo "{\"id\":$id,\"result\":true}"
        ;;
esac
done
"#,
        )
        .unwrap();
        let host = SharedHost {
            running: Arc::default(),
            dotnet: PathBuf::from("/bin/sh"),
            host: script,
            folder: folder.clone(),
        };
        Self { folder, host }
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.folder);
    }
}

#[test]
fn a_request_error_does_not_restart_a_valid_host() {
    let fixture = Fixture::new();
    assert!(fixture.host.build(Path::new("bad.csproj"), &json!({})).is_err());
    fixture.host.build(Path::new("good.csproj"), &json!({})).unwrap();
    let starts = std::fs::read_to_string(fixture.folder.join("starts")).unwrap();
    assert_eq!(starts.lines().count(), 1, "one bad project restarted the shared Host");
}

fn wait_for(mut ready: impl FnMut() -> bool) {
    let began = std::time::Instant::now();
    while !ready() {
        assert!(began.elapsed() < Duration::from_secs(3), "the Host protocol did not advance");
        std::thread::sleep(POLL);
    }
}

#[test]
fn hold_partials_and_other_requests_keep_their_own_replies() {
    let fixture = Fixture::new();
    let cancel = AtomicBool::new(false);
    let mut waiting = Vec::new();
    let hold = fixture.host.hold(&cancel, |paths| waiting.extend_from_slice(paths)).unwrap();
    assert_eq!(waiting, [PathBuf::from("Running.csproj")]);
    let host = fixture.host.clone();
    let (sender, built) = mpsc::channel();
    std::thread::spawn(move || {
        sender.send(host.build(Path::new("blocked.csproj"), &json!({}))).unwrap();
    });
    wait_for(|| fixture.folder.join("parked").exists());
    let evaluated = fixture.host.evaluate(Path::new("multi.csproj"), None).unwrap();
    assert_eq!(evaluated.target_frameworks, ["net8.0", "net9.0"], "a multi-target evaluation lost its final partial");
    assert!(evaluated.complete);
    let inspected = fixture.host.inspect(Path::new("inspected.csproj"), None, &HashMap::new()).unwrap();
    assert!(inspected.evaluated, "another request consumed the inspection's reply");
    assert!(fixture.host.build(Path::new("bad.csproj"), &json!({})).is_err());
    assert!(built.try_recv().is_err(), "a generating build ran through the hold");
    assert_eq!(hold.failed(), None);
    drop(hold);
    let built = built.recv_timeout(Duration::from_secs(3)).expect("releasing the hold left a build waiting").unwrap();
    assert_eq!(built.infos.as_deref(), Some("released"));
    fixture.host.build(Path::new("good.csproj"), &json!({})).unwrap();
}

#[test]
fn stop_while_acquiring_sends_cancel_and_does_not_leave_a_hold() {
    let fixture = Fixture::new();
    std::fs::write(fixture.folder.join("wait"), "").unwrap();
    let cancel = AtomicBool::new(false);
    let began = std::time::Instant::now();
    let result = fixture.host.hold(&cancel, |_| cancel.store(true, Ordering::Release));
    assert!(result.is_err(), "Stop started the build while a generating project was still running");
    assert!(began.elapsed() < Duration::from_secs(1), "Stop waited for the running design-time build");
    wait_for(|| fixture.folder.join("terminal").exists());
    std::fs::remove_file(fixture.folder.join("wait")).unwrap();
    let next = fixture.host.hold(&AtomicBool::new(false), |_| {}).unwrap();
    drop(next);
    fixture.host.build(Path::new("good.csproj"), &json!({})).unwrap();
}

#[test]
fn an_already_cancelled_hold_does_not_start_a_host() {
    let fixture = Fixture::new();
    assert!(fixture.host.hold(&AtomicBool::new(true), |_| {}).is_err());
    assert!(!fixture.folder.join("starts").exists(), "Stop launched an unnecessary Host");
}

#[test]
fn stop_does_not_wait_behind_a_backpressured_inspection_write() {
    let fixture = Fixture::new();
    std::fs::write(fixture.folder.join("stall-read"), "").unwrap();
    let inspecting = fixture.host.clone();
    let (sender, inspected) = mpsc::channel();
    std::thread::spawn(move || {
        let texts = HashMap::from([(PathBuf::from("large.props"), "x".repeat(1024 * 1024))]);
        let _ = sender.send(inspecting.inspect(Path::new("inspected.csproj"), None, &texts));
    });
    wait_for(|| fixture.folder.join("stalled").exists());
    let cancel = Arc::new(AtomicBool::new(false));
    let stopping = cancel.clone();
    let holding = fixture.host.clone();
    let (sender, stopped) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = sender.send(holding.hold(&stopping, |_| {}).is_err());
    });
    std::thread::sleep(POLL);
    cancel.store(true, Ordering::Release);
    assert!(
        stopped
            .recv_timeout(Duration::from_millis(500))
            .expect("Stop waited behind an inspection's blocked stdin write")
    );
    inspected.recv_timeout(Duration::from_secs(3)).expect("the inspection never drained").unwrap();
    wait_for(|| fixture.folder.join("terminal").exists());
}

#[test]
fn stop_during_the_hosts_initial_reply_returns_promptly() {
    let fixture = Fixture::new();
    std::fs::write(fixture.folder.join("slow-ready"), "").unwrap();
    let cancel = Arc::new(AtomicBool::new(false));
    let stopping = cancel.clone();
    let host = fixture.host.clone();
    let (sender, answer) = mpsc::channel();
    std::thread::spawn(move || {
        sender.send(host.hold(&stopping, |_| {}).is_err()).unwrap();
    });
    wait_for(|| fixture.folder.join("starts").exists());
    cancel.store(true, Ordering::Release);
    assert!(answer.recv_timeout(Duration::from_secs(1)).expect("Stop waited for Host startup"));
    assert!(fixture.host.running.lock().unwrap().is_none());
}

#[test]
fn drop_returns_before_the_terminal_reply_and_keeps_the_connection_pinned() {
    let fixture = Fixture::new();
    std::fs::write(fixture.folder.join("delay-terminal"), "").unwrap();
    let hold = fixture.host.hold(&AtomicBool::new(false), |_| {}).unwrap();
    let connection = hold.connection.clone();
    let began = std::time::Instant::now();
    drop(hold);
    assert!(began.elapsed() < Duration::from_millis(100), "dropping a hold blocked the caller");
    wait_for(|| fixture.folder.join("cancelled").exists());
    assert_eq!(connection.pins.load(Ordering::Acquire), 1, "the cancel acknowledgement released the hold's pin");
    std::fs::remove_file(fixture.folder.join("delay-terminal")).unwrap();
    wait_for(|| connection.pins.load(Ordering::Acquire) == 0);
    fixture.host.build(Path::new("good.csproj"), &json!({})).unwrap();
}

#[test]
fn transport_loss_wakes_every_waiter_and_cannot_replace_a_held_host() {
    for stopped in ["eof.csproj", "invalid.csproj"] {
        let fixture = Fixture::new();
        let hold = fixture.host.hold(&AtomicBool::new(false), |_| {}).unwrap();
        let host = fixture.host.clone();
        let (sender, answer) = mpsc::channel();
        std::thread::spawn(move || {
            sender.send(host.build(Path::new("blocked.csproj"), &json!({})).is_err()).unwrap();
        });
        wait_for(|| fixture.folder.join("parked").exists());
        assert!(fixture.host.build(Path::new(stopped), &json!({})).is_err());
        assert!(answer.recv_timeout(Duration::from_secs(1)).expect("transport loss left another caller waiting"));
        assert!(hold.failed().is_some(), "the external build was not told its Host had stopped");
        assert!(fixture.host.evaluate(Path::new("good.csproj"), None).is_err());
        assert_eq!(
            std::fs::read_to_string(fixture.folder.join("starts")).unwrap().lines().count(),
            1,
            "a replacement Host could generate files concurrently with the external build"
        );
        let connection = hold.connection.clone();
        drop(hold);
        wait_for(|| connection.pins.load(Ordering::Acquire) == 0);
        fixture.host.build(Path::new("good.csproj"), &json!({})).unwrap();
        assert_eq!(std::fs::read_to_string(fixture.folder.join("starts")).unwrap().lines().count(), 2);
    }
}
