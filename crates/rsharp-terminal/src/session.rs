use crate::{Screen, Snapshot, input};
use portable_pty::{CommandBuilder, PtySize};
use rsharp_core::process::PtyChild;
use std::{
    collections::VecDeque,
    io::{Read, Write},
    path::Path,
    sync::{
        Arc, Condvar, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

const INPUT_LIMIT: usize = 1024 * 1024;

#[derive(Default)]
struct Queue {
    pieces: VecDeque<Vec<u8>>,
    bytes: usize,
    closed: bool,
}
#[derive(Default)]
pub(crate) struct Outbox {
    queue: Mutex<Queue>,
    ready: Condvar,
}
impl Outbox {
    pub fn push(&self, bytes: Vec<u8>) -> Result<(), String> {
        let mut queue = self.queue.lock().unwrap();
        if queue.closed {
            return Err("The terminal process has ended".into());
        }
        if bytes.len() > INPUT_LIMIT || queue.bytes + bytes.len() > INPUT_LIMIT {
            return Err("Terminal input is full; wait for the program to read before pasting again".into());
        }
        queue.bytes += bytes.len();
        queue.pieces.push_back(bytes);
        self.ready.notify_one();
        Ok(())
    }
    fn close(&self) {
        self.queue.lock().unwrap().closed = true;
        self.ready.notify_all();
    }
    fn next(&self) -> Option<Vec<u8>> {
        let mut queue = self.queue.lock().unwrap();
        loop {
            if queue.closed {
                return None;
            }
            if let Some(piece) = queue.pieces.pop_front() {
                queue.bytes -= piece.len();
                return Some(piece);
            }
            queue = self.ready.wait(queue).unwrap();
        }
    }
}

pub struct Session {
    pub screen: Arc<Mutex<Screen>>,
    child: Arc<Mutex<Option<PtyChild>>>,
    input: Arc<Outbox>,
    closed: Arc<AtomicBool>,
    revision: Arc<AtomicU64>,
    exit: Arc<Mutex<Option<u32>>>,
    pub pid: u32,
}

impl Session {
    pub fn start(mut command: CommandBuilder, cols: usize, rows: usize) -> Result<Self, String> {
        command.env("TERM", "xterm-256color");
        command.env("COLORTERM", "truecolor");
        command.env("TERM_PROGRAM", "RSharp");
        if cfg!(unix) && std::env::var_os("LANG").is_none() {
            command.env("LANG", "en_US.UTF-8");
        }
        let screen = Arc::new(Mutex::new(Screen::new(cols, rows)));
        let input = screen.lock().unwrap().events.outbox.clone();
        let child = rsharp_core::process::spawn_pty(
            command,
            PtySize {
                rows: rows.max(1).min(u16::MAX as usize) as u16,
                cols: cols.max(2).min(u16::MAX as usize) as u16,
                pixel_width: 0,
                pixel_height: 0,
            },
        )?;
        let pid = child.id();
        let mut reader = child.master.try_clone_reader().map_err(|e| e.to_string())?;
        let mut writer = child.master.take_writer().map_err(|e| e.to_string())?;
        let child = Arc::new(Mutex::new(Some(child)));
        let revision = Arc::new(AtomicU64::new(0));
        let closed = Arc::new(AtomicBool::new(false));
        let read_ended = Arc::new(AtomicBool::new(false));
        let exit = Arc::new(Mutex::new(None));
        let (reading_screen, reading_revision, reading_closed, reading_end) =
            (screen.clone(), revision.clone(), closed.clone(), read_ended.clone());
        std::thread::Builder::new()
            .name("Terminal output".into())
            .spawn(move || {
                let mut bytes = [0; 4096];
                while !reading_closed.load(Ordering::Acquire) {
                    match reader.read(&mut bytes) {
                        Ok(0) | Err(_) => break,
                        Ok(count) => {
                            reading_screen.lock().unwrap().feed(&bytes[..count]);
                            reading_revision.fetch_add(1, Ordering::Release);
                        }
                    }
                }
                reading_end.store(true, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        let writing = input.clone();
        std::thread::Builder::new()
            .name("Terminal input".into())
            .spawn(move || {
                while let Some(bytes) = writing.next() {
                    if writer.write_all(&bytes).and_then(|_| writer.flush()).is_err() {
                        break;
                    }
                }
                writing.close();
            })
            .map_err(|e| e.to_string())?;
        let (waiting_child, waiting_closed, waiting_input, waiting_exit, waiting_revision) =
            (child.clone(), closed.clone(), input.clone(), exit.clone(), revision.clone());
        std::thread::Builder::new()
            .name("Terminal process".into())
            .spawn(move || {
                let code = loop {
                    if waiting_closed.load(Ordering::Acquire) {
                        break 130;
                    }
                    match waiting_child.lock().unwrap().as_mut().and_then(|c| c.try_wait().ok().flatten()) {
                        Some(code) => break code,
                        None => std::thread::sleep(Duration::from_millis(10)),
                    }
                };
                // End orphaned jobs too, so one holding the PTY open cannot postpone EOF forever.
                waiting_child.lock().unwrap().take();
                let deadline = Instant::now() + Duration::from_secs(2);
                while !read_ended.load(Ordering::Acquire)
                    && !waiting_closed.load(Ordering::Acquire)
                    && Instant::now() < deadline
                {
                    std::thread::sleep(Duration::from_millis(5));
                }
                waiting_input.close();
                *waiting_exit.lock().unwrap() = Some(code);
                waiting_revision.fetch_add(1, Ordering::Release);
            })
            .map_err(|e| e.to_string())?;
        Ok(Self { screen, child, input, closed, revision, exit, pid })
    }
    pub fn write(&self, text: &str) -> Result<(), String> {
        self.screen.lock().unwrap().bottom();
        self.write_bytes(text.as_bytes())
    }
    pub fn write_bytes(&self, bytes: &[u8]) -> Result<(), String> {
        self.input.push(bytes.to_vec())
    }
    pub fn paste(&self, text: &str) -> Result<(), String> {
        if text.len() > INPUT_LIMIT {
            return Err("The paste is larger than the terminal's 1 MB input limit".into());
        }
        let bracketed = self.screen.lock().unwrap().mode().contains(crate::TermMode::BRACKETED_PASTE);
        self.write(&input::paste(text, bracketed))
    }
    pub fn resize(&self, cols: usize, rows: usize) {
        self.screen.lock().unwrap().resize(cols, rows);
        if let Some(child) = self.child.lock().unwrap().as_ref() {
            let _ = child.master.resize(PtySize {
                cols: cols.max(2).min(u16::MAX as usize) as u16,
                rows: rows.max(1).min(u16::MAX as usize) as u16,
                pixel_width: 0,
                pixel_height: 0,
            });
        }
    }
    pub fn stop(&self) {
        self.closed.store(true, Ordering::Release);
        self.input.close();
    }
    pub fn revision(&self) -> u64 {
        self.revision.load(Ordering::Acquire)
    }
    pub fn exit_code(&self) -> Option<u32> {
        *self.exit.lock().unwrap()
    }
    pub fn snapshot(&self) -> Snapshot {
        self.screen.lock().unwrap().snapshot()
    }
    pub fn text(&self) -> String {
        self.screen.lock().unwrap().text()
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.stop();
    }
}

pub fn shell(folder: &Path, history: Option<&Path>) -> CommandBuilder {
    let mut command = if cfg!(windows) {
        let program = rsharp_core::programs::find("pwsh", &[]).unwrap_or_else(|| "powershell.exe".into());
        let mut command = CommandBuilder::new(program);
        command.arg("-NoLogo");
        if history.is_some() {
            command.args([
                "-NoExit",
                "-Command",
                "try { Set-PSReadLineOption -HistorySavePath $env:HISTFILE } catch { }",
            ]);
        }
        command
    } else {
        let program = if history.is_some() {
            "/bin/sh".into()
        } else {
            std::env::var_os("SHELL")
                .filter(|s| Path::new(s).is_file())
                .unwrap_or_else(|| if cfg!(target_os = "macos") { "/bin/zsh".into() } else { "/bin/bash".into() })
        };
        let mut command = CommandBuilder::new(program);
        if history.is_none() {
            command.arg("-l");
        }
        command
    };
    command.cwd(folder);
    if let Some(history) = history {
        command.env("HISTFILE", history);
    }
    command
}
