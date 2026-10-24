use std::io::{self, Read};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::time::Duration;

/// Streams both pipes until the child and its descendants end. The callback must not block indefinitely.
pub fn run(command: Command, cancel: &AtomicBool, output: impl FnMut(&str, bool)) -> io::Result<Option<i32>> {
    run_while(command, cancel, || true, output)
}

/// Also stops when an external prerequisite, such as the Host hold, is lost.
pub fn run_while(
    mut command: Command,
    cancel: &AtomicBool,
    mut still_running: impl FnMut() -> bool,
    mut output: impl FnMut(&str, bool),
) -> io::Result<Option<i32>> {
    if cancel.load(Ordering::Acquire) || !still_running() {
        return Ok(None);
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = crate::process::spawn(command)?;
    let stdout = child.process.stdout.take().unwrap();
    let stderr = child.process.stderr.take().unwrap();
    std::thread::scope(|scope| {
        // Sixteen reads can wait here, even when the UI's bounded queue is full.
        let (sender, receiver) = mpsc::sync_channel(16);
        let errors = sender.clone();
        scope.spawn(move || pump(stdout, false, sender));
        scope.spawn(move || pump(stderr, true, errors));
        let mut ended = None;
        let mut stopped = false;
        let mut pipes_open = true;
        loop {
            if ended.is_none() {
                if cancel.load(Ordering::Acquire) || !still_running() {
                    stopped = true;
                    child.kill();
                }
                match child.process.try_wait() {
                    Ok(Some(status)) => {
                        ended = Some(status.code().unwrap_or(-1));
                        // Releasing the Lifeline also closes pipes inherited by a remaining descendant.
                        child.wait()?;
                    }
                    Ok(None) => {}
                    Err(error) => {
                        child.kill();
                        return Err(error);
                    }
                }
            }
            if !pipes_open {
                if let Some(code) = ended {
                    return Ok(if stopped { None } else { Some(code) });
                }
                std::thread::sleep(Duration::from_millis(20));
                continue;
            }
            match receiver.recv_timeout(Duration::from_millis(20)) {
                Ok(Ok((text, stderr))) => output(&text, stderr),
                Ok(Err(error)) => {
                    child.kill();
                    return Err(error);
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => pipes_open = false,
            }
        }
    })
}

fn pump(mut reader: impl Read, stderr: bool, sender: SyncSender<io::Result<(String, bool)>>) {
    let mut buffer = [0; 8192];
    let mut decoder = Decoder::default();
    loop {
        let read = match reader.read(&mut buffer) {
            Ok(read) => read,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let _ = sender.send(Err(error));
                return;
            }
        };
        let text = decoder.push(&buffer[..read], read == 0);
        if !text.is_empty() && sender.send(Ok((text, stderr))).is_err() {
            return;
        }
        if read == 0 {
            return;
        }
    }
}

#[derive(Default)]
enum Escape {
    #[default]
    None,
    Start,
    Intermediate,
    Csi,
    String,
    StringEscape,
}

#[derive(Default)]
struct Decoder {
    pending: Vec<u8>,
    escape: Escape,
}

impl Decoder {
    fn push(&mut self, bytes: &[u8], final_chunk: bool) -> String {
        for &byte in bytes {
            self.escape = match self.escape {
                Escape::None => {
                    if byte == 0x1b {
                        Escape::Start
                    } else {
                        if byte >= 0x20 || matches!(byte, b'\n' | b'\r' | b'\t') {
                            self.pending.push(byte);
                        }
                        Escape::None
                    }
                }
                Escape::Start => match byte {
                    b'[' => Escape::Csi,
                    b']' | b'P' | b'X' | b'^' | b'_' => Escape::String,
                    0x20..=0x2f => Escape::Intermediate,
                    _ => Escape::None,
                },
                Escape::Intermediate => {
                    if (0x30..=0x7e).contains(&byte) {
                        Escape::None
                    } else {
                        Escape::Intermediate
                    }
                }
                Escape::Csi => {
                    if (0x40..=0x7e).contains(&byte) {
                        Escape::None
                    } else {
                        Escape::Csi
                    }
                }
                Escape::String => match byte {
                    7 => Escape::None,
                    0x1b => Escape::StringEscape,
                    _ => Escape::String,
                },
                Escape::StringEscape => {
                    if byte == b'\\' {
                        Escape::None
                    } else {
                        Escape::String
                    }
                }
            };
        }
        let mut text = String::new();
        let mut consumed = 0;
        while consumed < self.pending.len() {
            match std::str::from_utf8(&self.pending[consumed..]) {
                Ok(valid) => {
                    text.push_str(valid);
                    consumed = self.pending.len();
                }
                Err(error) => {
                    let valid_end = consumed + error.valid_up_to();
                    text.push_str(std::str::from_utf8(&self.pending[consumed..valid_end]).unwrap());
                    consumed = valid_end;
                    match error.error_len() {
                        Some(length) => {
                            text.push('�');
                            consumed += length;
                        }
                        None if final_chunk => {
                            text.push('�');
                            consumed = self.pending.len();
                        }
                        None => break,
                    }
                }
            }
        }
        self.pending.drain(..consumed);
        text
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::Ordering;
    use std::time::{Duration, Instant};

    #[test]
    fn utf8_and_ansi_sequences_survive_every_read_boundary_and_the_last_partial_character() {
        let text = "\x1b[31mZażółć\x1b[0m\x1b]8;;https://example.com\x07 link\x1b]8;;\x1b\\\nfinal";
        for split in 0..=text.len() {
            let mut decoder = Decoder::default();
            let mut decoded = decoder.push(&text.as_bytes()[..split], false);
            decoded.push_str(&decoder.push(&text.as_bytes()[split..], true));
            assert_eq!(decoded, "Zażółć link\nfinal", "split at {split}");
        }
        let mut decoder = Decoder::default();
        assert_eq!(decoder.push(&[b'a', 0xe2], false), "a");
        assert_eq!(decoder.push(&[], true), "�");
    }

    #[test]
    fn invalid_utf8_does_not_consume_the_next_incomplete_character() {
        let mut decoder = Decoder::default();
        assert_eq!(decoder.push(&[0xff, 0xe2], false), "�");
        assert_eq!(decoder.push(&[0x82, 0xac], true), "€");
    }

    #[test]
    fn a_lost_host_prerequisite_stops_a_quiet_child_without_waiting_for_output() {
        let mut polls = 0;
        let began = Instant::now();
        let result = run_while(
            fixture("quiet"),
            &AtomicBool::new(false),
            || {
                polls += 1;
                polls < 4
            },
            |_, _| {},
        )
        .unwrap();
        assert_eq!(result, None);
        assert!(began.elapsed() < Duration::from_secs(5));
    }

    fn fixture(mode: &str) -> Command {
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "build::output::tests::child", "--ignored", "--nocapture"])
            .env("RSHARP_BUILD_FIXTURE", mode);
        command
    }

    #[test]
    fn both_pipes_drain_without_deadlock_and_deliver_the_final_unterminated_line() {
        let mut stdout = String::new();
        let mut stderr = String::new();
        let result = run(fixture("output"), &AtomicBool::new(false), |text, error| {
            if error {
                stderr.push_str(text);
            } else {
                stdout.push_str(text);
            }
        })
        .unwrap();
        assert_eq!(result, Some(7));
        assert!(stdout.contains("Zażółć\nfinal"), "the final output was missing: {stdout:?}");
        assert!(!stdout.contains('\x1b'));
        assert_eq!(stderr.len(), 256 * 1024);
    }

    #[test]
    fn an_already_stopped_build_does_not_start_a_missing_program() {
        assert_eq!(run(Command::new("/missing/dotnet"), &AtomicBool::new(true), |_, _| {}).unwrap(), None);
    }

    #[test]
    fn stopping_while_output_is_backpressured_releases_the_child_and_readers() {
        let cancel = AtomicBool::new(false);
        let began = Instant::now();
        let mut calls = 0;
        let result = run(fixture("flood"), &cancel, |_, _| {
            calls += 1;
            std::thread::sleep(Duration::from_millis(5));
            cancel.store(true, Ordering::Release);
        })
        .unwrap();
        assert_eq!(result, None);
        assert!(calls > 0);
        assert!(began.elapsed() < Duration::from_secs(5), "Stop did not release backpressured readers");
    }

    #[cfg(unix)]
    #[test]
    fn a_finished_parent_does_not_leave_its_descendants_holding_the_output_open() {
        let mut command = Command::new("/bin/sh");
        command.args(["-c", "sleep 60 & printf final"]);
        let began = Instant::now();
        let mut output = String::new();
        assert_eq!(run(command, &AtomicBool::new(false), |text, _| output.push_str(text)).unwrap(), Some(0));
        assert_eq!(output, "final");
        assert!(began.elapsed() < Duration::from_secs(5), "an inherited pipe outlived the build");
    }

    #[test]
    #[ignore = "subprocess fixture, started only by build tests"]
    fn child() {
        use std::io::Write;
        let Some(mode) = std::env::var_os("RSHARP_BUILD_FIXTURE") else {
            return;
        };
        if mode == "quiet" {
            std::thread::sleep(Duration::from_secs(60));
            return;
        }
        if mode == "flood" {
            loop {
                std::io::stdout().write_all(&[b'x'; 8192]).unwrap();
            }
        }
        std::io::stderr().write_all(&[b'e'; 256 * 1024]).unwrap();
        for byte in "\x1b[31mZażółć\x1b[0m\nfinal".as_bytes() {
            std::io::stdout().write_all(&[*byte]).unwrap();
            std::io::stdout().flush().unwrap();
        }
        std::process::exit(7);
    }
}
