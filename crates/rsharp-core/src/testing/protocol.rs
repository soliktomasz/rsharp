use super::*;
use serde_json::Value;
use std::io::{Read, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::Command;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::Instant;

const MAX_MESSAGE: usize = 32 * 1024 * 1024;
const POLL: Duration = Duration::from_millis(50);
pub(super) const STOP_LIMIT: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
pub(super) enum Encoding {
    Platform,
    VsTest,
}

fn encode(encoding: Encoding, message: &Value) -> Result<Vec<u8>, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "PascalCase")]
    struct Versioned<'a> {
        version: &'a Value,
        message_type: &'a Value,
        payload: &'a Value,
    }
    // VSTest's fast header parser only reads Version when it is the first field.
    let body = if matches!(encoding, Encoding::VsTest) && message.get("Version").is_some() {
        serde_json::to_vec(&Versioned {
            version: &message["Version"],
            message_type: &message["MessageType"],
            payload: &message["Payload"],
        })
    } else {
        serde_json::to_vec(message)
    }
    .map_err(|e| e.to_string())?;
    if body.len() > MAX_MESSAGE {
        return Err("The test protocol message exceeds 32 MB".into());
    }
    let mut frame = Vec::new();
    match encoding {
        Encoding::Platform => frame.extend_from_slice(format!("Content-Length: {}\r\n\r\n", body.len()).as_bytes()),
        Encoding::VsTest => {
            let mut length = body.len();
            while length >= 128 {
                frame.push((length as u8) | 128);
                length >>= 7;
            }
            frame.push(length as u8);
        }
    }
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn decode(encoding: Encoding, buffer: &mut Vec<u8>) -> Result<Option<Value>, String> {
    let (header, length) = match encoding {
        Encoding::Platform => {
            let Some(at) = buffer.windows(4).position(|end| end == b"\r\n\r\n") else {
                if buffer.len() > 8192 {
                    return Err("The test protocol header exceeds 8 KB".into());
                }
                return Ok(None);
            };
            if at > 8192 {
                return Err("The test protocol header exceeds 8 KB".into());
            }
            let header = std::str::from_utf8(&buffer[..at]).map_err(|e| e.to_string())?;
            let length = header
                .lines()
                .find_map(|line| {
                    let (name, value) = line.split_once(':')?;
                    name.eq_ignore_ascii_case("Content-Length").then(|| value.trim().parse::<usize>().ok()).flatten()
                })
                .ok_or("The test protocol message has no valid Content-Length")?;
            (at + 4, length)
        }
        Encoding::VsTest => {
            let mut length = 0usize;
            let mut end = None;
            for (at, byte) in buffer.iter().take(5).enumerate() {
                length |= ((byte & 127) as usize) << (at * 7);
                if byte & 128 == 0 {
                    end = Some(at + 1);
                    break;
                }
            }
            let Some(end) = end else {
                if buffer.len() >= 5 {
                    return Err("The VSTest message length is malformed".into());
                }
                return Ok(None);
            };
            (end, length)
        }
    };
    if length > MAX_MESSAGE {
        return Err("The test protocol message exceeds 32 MB".into());
    }
    if buffer.len() < header + length {
        return Ok(None);
    }
    let message = serde_json::from_slice(&buffer[header..header + length])
        .map_err(|e| format!("Invalid test protocol JSON: {e}"))?;
    buffer.drain(..header + length);
    Ok(Some(message))
}

/// One request's process and socket. Its bounded reads let Stop work even halfway through a message.
pub(super) struct Host {
    child: crate::process::Child,
    stream: TcpStream,
    buffer: Vec<u8>,
    encoding: Encoding,
    written: Arc<Mutex<Vec<u8>>>,
    readers: Vec<JoinHandle<()>>,
}

impl Host {
    pub fn start(
        encoding: Encoding,
        cancel: &AtomicBool,
        command: impl FnOnce(u16) -> Command,
    ) -> Result<Self, String> {
        if canceled(cancel) {
            return Err("Test request stopped".into());
        }
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).map_err(|e| e.to_string())?;
        listener.set_nonblocking(true).map_err(|e| e.to_string())?;
        let mut command = command(listener.local_addr().map_err(|e| e.to_string())?.port());
        command
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped());
        let mut child = crate::process::spawn(command).map_err(|e| format!("The test host could not start: {e}"))?;
        let written = Arc::new(Mutex::new(Vec::new()));
        let readers = vec![
            drain(child.process.stdout.take().unwrap(), written.clone()),
            drain(child.process.stderr.take().unwrap(), written.clone()),
        ];
        let began = Instant::now();
        let stream = loop {
            match listener.accept() {
                Ok((stream, _)) => break stream,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) => return Err(error.to_string()),
            }
            if canceled(cancel) {
                child.kill();
                for reader in readers {
                    let _ = reader.join();
                }
                return Err("Test request stopped".into());
            }
            if let Some(status) = child.process.try_wait().map_err(|e| e.to_string())? {
                // End children that may still hold its pipes before quoting the last output.
                child.kill();
                for reader in readers {
                    let _ = reader.join();
                }
                return Err(with_output(format!("The test host ended ({status}) before it connected"), &written));
            }
            if began.elapsed() > Duration::from_secs(60) {
                child.kill();
                for reader in readers {
                    let _ = reader.join();
                }
                return Err(with_output("The test host did not connect within 60 seconds".into(), &written));
            }
            std::thread::sleep(POLL);
        };
        stream.set_read_timeout(Some(POLL)).map_err(|e| e.to_string())?;
        stream.set_write_timeout(Some(Duration::from_secs(1))).map_err(|e| e.to_string())?;
        Ok(Self { child, stream, buffer: Vec::new(), encoding, written, readers })
    }

    pub fn send(&mut self, message: &Value) -> Result<(), String> {
        self.stream
            .write_all(&encode(self.encoding, message)?)
            .map_err(|e| format!("Could not send a test request: {e}"))
    }

    pub fn receive(&mut self) -> Result<Option<Value>, String> {
        if let Some(message) = decode(self.encoding, &mut self.buffer)? {
            return Ok(Some(message));
        }
        let mut chunk = [0; 16384];
        match self.stream.read(&mut chunk) {
            Ok(0) => Err(with_output(
                "The test host closed its connection before the request finished".into(),
                &self.written,
            )),
            Ok(count) => {
                self.buffer.extend_from_slice(&chunk[..count]);
                decode(self.encoding, &mut self.buffer)
            }
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut | std::io::ErrorKind::Interrupted
                ) =>
            {
                Ok(None)
            }
            Err(error) => Err(with_output(format!("The test connection failed: {error}"), &self.written)),
        }
    }

    pub fn finish(&mut self, limit: Duration) {
        let began = Instant::now();
        while matches!(self.child.process.try_wait(), Ok(None)) {
            if began.elapsed() >= limit {
                self.child.kill();
                break;
            }
            std::thread::sleep(POLL);
        }
        let _ = self.child.wait();
        let _ = self.stream.shutdown(Shutdown::Both);
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        let _ = self.stream.shutdown(Shutdown::Both);
        if matches!(self.child.process.try_wait(), Ok(None)) {
            self.child.kill();
        } else {
            let _ = self.child.wait();
        }
        for reader in self.readers.drain(..) {
            let _ = reader.join();
        }
    }
}

fn drain(mut pipe: impl Read + Send + 'static, written: Arc<Mutex<Vec<u8>>>) -> JoinHandle<()> {
    std::thread::spawn(move || {
        let mut chunk = [0; 4096];
        while let Ok(count) = pipe.read(&mut chunk) {
            if count == 0 {
                break;
            }
            let mut kept = written.lock().unwrap();
            kept.extend_from_slice(&chunk[..count]);
            if kept.len() > 8192 {
                let discard = kept.len() - 8192;
                kept.drain(..discard);
            }
        }
    })
}

fn with_output(why: String, written: &Mutex<Vec<u8>>) -> String {
    let written = written.lock().unwrap();
    let output = String::from_utf8_lossy(&written);
    if output.trim().is_empty() { why } else { format!("{why}:\n{}", output.trim()) }
}

pub(super) fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key)?.as_str()
}
pub(super) fn array<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    value.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn both_wire_formats_handle_fragmented_and_consecutive_utf8_messages() {
        for encoding in [Encoding::Platform, Encoding::VsTest] {
            let message = json!({"text": "a long λ message".repeat(20)});
            let frame = encode(encoding, &message).unwrap();
            let mut buffer = Vec::new();
            for byte in &frame[..frame.len() - 1] {
                buffer.push(*byte);
                assert!(decode(encoding, &mut buffer).unwrap().is_none());
            }
            buffer.extend_from_slice(&frame[frame.len() - 1..]);
            buffer.extend_from_slice(&frame);
            assert_eq!(decode(encoding, &mut buffer).unwrap(), Some(message.clone()));
            assert_eq!(decode(encoding, &mut buffer).unwrap(), Some(message));
            assert!(buffer.is_empty());
        }
    }

    #[test]
    fn malformed_or_oversized_frames_fail_before_allocating_the_body() {
        assert!(decode(Encoding::Platform, &mut b"Content-Length: 999999999\r\n\r\n".to_vec()).is_err());
        assert!(decode(Encoding::Platform, &mut b"Other: 2\r\n\r\n{}".to_vec()).is_err());
        assert!(decode(Encoding::VsTest, &mut vec![0xff; 6]).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn cancellation_before_connection_reaps_a_host_with_both_pipes_open() {
        let Ok(probe) = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)) else {
            eprintln!("skipped: local sockets are unavailable for the startup cancellation test");
            return;
        };
        drop(probe);
        let pid_file = std::env::temp_dir().join(format!(
            "rsharp-connect-cancel-{}-{}.pid",
            std::process::id(),
            super::super::runners::next_request()
        ));
        let cancel = AtomicBool::new(false);
        let began = Instant::now();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                while std::fs::read_to_string(&pid_file).ok().and_then(|pid| pid.parse::<u32>().ok()).is_none()
                    && began.elapsed() < Duration::from_secs(5)
                {
                    std::thread::sleep(Duration::from_millis(10));
                }
                cancel.store(true, std::sync::atomic::Ordering::Release);
            });
            let result = Host::start(Encoding::Platform, &cancel, |_| {
                let mut command = Command::new("sh");
                command
                    .args(["-c", "printf '%s' $$ > \"$1\"; echo waiting; echo waiting >&2; sleep 30", "sh"])
                    .arg(&pid_file);
                command
            });
            assert!(
                matches!(result, Err(error) if error == "Test request stopped"),
                "canceling an unconnected host did not end startup"
            );
        });
        let pid: i32 = std::fs::read_to_string(&pid_file).expect("the test host never started").parse().unwrap();
        let _ = std::fs::remove_file(pid_file);
        assert!(began.elapsed() < Duration::from_secs(6), "startup cancellation hung with pipe readers open");
        assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the unconnected host survived cancellation");
    }

    #[test]
    fn vstest_version_precedes_message_type_for_the_sdks_fast_header_parser() {
        let frame = encode(Encoding::VsTest, &json!({"Version":7,"MessageType":"Run","Payload":{}})).unwrap();
        assert!(
            frame[1..].starts_with(br#"{"Version":7,"MessageType":"Run""#),
            "VSTest otherwise treats modern test cases as protocol zero and loses their Source"
        );
    }
}
