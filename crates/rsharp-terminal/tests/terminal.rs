use rsharp_terminal::{Screen, input};

#[test]
fn resizing_and_wrapped_selection_keep_the_text_as_one_line() {
    use rsharp_terminal::SelectionType;
    let mut screen = Screen::new(12, 4);
    screen.feed("héllo 漢字 tail".as_bytes());
    screen.select(0, 0, false, SelectionType::Simple);
    screen.select(1, 3, true, SelectionType::Simple);
    assert!(screen.selected_text().unwrap().starts_with("héllo 漢字 tail"));
    screen.resize(30, 4);
    assert!(screen.text().contains("héllo 漢字 tail"), "resizing broke a wrapped Unicode line");
}

#[cfg(unix)]
#[test]
fn terminal_queries_receive_the_cursor_position_and_a_fast_exit_is_not_lost() {
    use rsharp_terminal::{CommandBuilder, Session};
    let mut command = CommandBuilder::new("/bin/bash");
    command.args(["-c","stty -echo; printf '\\033[5;10H\\033[6n'; IFS= read -r -d R reply; printf '\\r\\nreply=%sR\\r\\n' \"${reply#??}\"; exit 3"]);
    let session = Session::start(command, 80, 24).unwrap();
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.exit_code().is_none() {
        assert!(std::time::Instant::now() < until, "the program never received its terminal reply");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(session.exit_code(), Some(3));
    assert!(session.text().contains("reply=5;10R"));
}

#[cfg(unix)]
#[test]
fn a_blocked_paste_is_bounded_and_stop_returns_without_waiting_for_the_reader() {
    use rsharp_terminal::{CommandBuilder, Session};
    let mut command = CommandBuilder::new("/bin/sh");
    command.args(["-c", "sleep 60"]);
    let session = Session::start(command, 80, 24).unwrap();
    let started = std::time::Instant::now();
    session.paste(&"x".repeat(900_000)).unwrap();
    assert!(session.paste(&"x".repeat(2_000_000)).is_err(), "an oversized paste must be refused");
    session.stop();
    assert!(started.elapsed() < std::time::Duration::from_secs(1), "paste or Stop blocked on the program's input");
    let until = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while session.exit_code().is_none() {
        assert!(std::time::Instant::now() < until, "Stop left the terminal running");
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn a_full_screen_program_restores_the_shell_and_keeps_unicode_in_its_cells() {
    let mut screen = Screen::new(20, 4);
    screen.feed(b"before\r\n\x1b[?1049h\x1b[H\x1b[2J");
    screen.feed("e\u{301} 漢".as_bytes());
    let snapshot = screen.snapshot();
    assert_eq!(snapshot.cells[0].text, "e\u{301}");
    assert_eq!(snapshot.cells[2].text, "漢");
    screen.feed(b"\x1b[?1049l");
    assert!(screen.text().contains("before"), "leaving the alternate screen lost the shell's output");
    assert!(!screen.text().contains('漢'));
}

#[test]
fn pasted_controls_cannot_escape_the_programs_bracketed_paste() {
    assert_eq!(input::paste("safe\x1b[201~\r\nnext\x03\t", true), "\x1b[200~safe[201~\rnext\t\x1b[201~");
    assert_eq!(input::key("up", false, false, false, true).as_deref(), Some("\x1bOA"));
    assert_eq!(input::key("right", true, false, false, false).as_deref(), Some("\x1b[1;5C"));
    assert_eq!(input::key("c", true, false, false, false).as_deref(), Some("\x03"));
}

#[cfg(unix)]
#[test]
fn ten_megabytes_of_output_are_drained_without_unbounded_scrollback() {
    use rsharp_terminal::{CommandBuilder, Session};
    let mut command = CommandBuilder::new("/bin/sh");
    command.args([
        "-c",
        "dd if=/dev/zero bs=1048576 count=10 2>/dev/null | tr '\\000' x; printf '\\r\\nfinished\\r\\n'; exit 7",
    ]);
    let session = Session::start(command, 80, 24).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
    while session.exit_code().is_none() {
        assert!(std::time::Instant::now() < deadline, "10 MB output did not finish");
        let snapshot = session.snapshot();
        assert!(snapshot.history <= rsharp_terminal::HISTORY_LINES);
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    assert_eq!(session.exit_code(), Some(7));
    assert!(session.text().contains("finished"), "the program's final output was lost");
}
