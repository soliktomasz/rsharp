//! Bounded, cancellable dotnet work on a background thread, with both pipes drained concurrently.

use std::io::Read;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

const KEPT: usize = 256 * 1024;

#[derive(Debug)]
pub(super) struct CommandResult {
    pub status: ExitStatus,
    pub stdout: String,
    pub stderr: String,
}

impl CommandResult {
    pub fn said(&self) -> String {
        let text = if self.stderr.trim().is_empty() { &self.stdout } else { &self.stderr };
        let lines: Vec<_> = text.lines().map(str::trim).filter(|s| !s.is_empty()).take(6).collect();
        if lines.is_empty() { format!("it exited with {}", self.status) } else { lines.join(" ") }
    }
}

enum Pipe {
    Data(bool, Vec<u8>),
    Failed(String),
}

pub(super) fn run(
    mut command: Command,
    limit: Duration,
    cancel: &AtomicBool,
    output: &mut dyn FnMut(&str),
) -> Result<CommandResult, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled".into());
    }
    command.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    let mut child = crate::process::spawn(command).map_err(|e| e.to_string())?;
    let (tx, rx) = mpsc::sync_channel(32);
    let readers = [
        Box::new(child.process.stdout.take().unwrap()) as Box<dyn Read + Send>,
        Box::new(child.process.stderr.take().unwrap()) as Box<dyn Read + Send>,
    ];
    let handles: Vec<_> = readers
        .into_iter()
        .enumerate()
        .map(|(index, mut reader)| {
            let tx = tx.clone();
            std::thread::spawn(move || {
                let mut buffer = [0; 4096];
                loop {
                    match reader.read(&mut buffer) {
                        Ok(0) => break,
                        Ok(n) => {
                            if tx.send(Pipe::Data(index == 1, buffer[..n].to_vec())).is_err() {
                                break;
                            }
                        }
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(e) => {
                            let _ = tx.send(Pipe::Failed(e.to_string()));
                            break;
                        }
                    }
                }
            })
        })
        .collect();
    drop(tx);
    let started = Instant::now();
    let mut captured = [Vec::new(), Vec::new()];
    let mut pending = [Vec::new(), Vec::new()];
    let mut status = None;
    let mut problem = None;
    let mut pipes_closed = false;
    loop {
        // Checked after exit too: whatever outlives dotnet with its pipes open must not hold run past them.
        if problem.is_none() {
            if cancel.load(Ordering::Acquire) {
                problem = Some("Cancelled".to_string());
            } else if started.elapsed() >= limit {
                problem = Some(format!("dotnet timed out after {} seconds", limit.as_secs()));
            }
        }
        if status.is_none() {
            if problem.is_some() {
                child.kill();
                status = child.wait().ok();
            } else {
                match child.process.try_wait() {
                    Ok(Some(_)) => status = Some(child.wait().map_err(|e| e.to_string())?),
                    Ok(None) => {}
                    Err(e) => {
                        problem = Some(e.to_string());
                        child.kill();
                        status = child.wait().ok();
                    }
                }
            }
        }
        if problem.is_some() {
            break;
        }
        if pipes_closed {
            if status.is_some() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
            continue;
        }
        match rx.recv_timeout(Duration::from_millis(10)) {
            Ok(Pipe::Data(stderr, bytes)) => {
                let index = usize::from(stderr);
                captured[index].extend_from_slice(&bytes);
                let excess = captured[index].len().saturating_sub(KEPT);
                captured[index].drain(..excess);
                pending[index].extend(bytes);
                emit_utf8(&mut pending[index], output);
            }
            Ok(Pipe::Failed(why)) => {
                problem = Some(why);
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => pipes_closed = true,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
    // A reader still blocked on an open pipe ends once it closes or its send fails.
    if pipes_closed {
        for handle in handles {
            let _ = handle.join();
        }
    }
    for bytes in pending {
        if !bytes.is_empty() {
            output(&String::from_utf8_lossy(&bytes));
        }
    }
    if let Some(why) = problem {
        return Err(why);
    }
    Ok(CommandResult {
        status: status.ok_or("dotnet ended without an exit status")?,
        stdout: String::from_utf8_lossy(&captured[0]).into_owned(),
        stderr: String::from_utf8_lossy(&captured[1]).into_owned(),
    })
}

fn emit_utf8(pending: &mut Vec<u8>, output: &mut dyn FnMut(&str)) {
    loop {
        match std::str::from_utf8(pending) {
            Ok(text) => {
                output(text);
                pending.clear();
                break;
            }
            Err(error) => {
                let valid = error.valid_up_to();
                if valid > 0 {
                    output(std::str::from_utf8(&pending[..valid]).unwrap());
                    pending.drain(..valid);
                }
                if let Some(bad) = error.error_len() {
                    output("�");
                    pending.drain(..bad);
                } else {
                    break;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::process::Command;
    use std::sync::atomic::AtomicBool;
    use std::time::{Duration, Instant};

    fn child_command(mode: &str) -> Command {
        let mut cmd = Command::new(std::env::current_exe().unwrap());
        cmd.args(["--exact", "templates::command::tests::child", "--ignored", "--nocapture"])
            .env("RSHARP_CHILD_CASE", mode);
        cmd
    }

    #[test]
    #[ignore = "subprocess fixture, started only by command tests"]
    fn child() {
        match std::env::var("RSHARP_CHILD_CASE").unwrap().as_str() {
            "flood" => {
                for _ in 0..512 {
                    std::io::stdout().write_all(&[b'o'; 4096]).unwrap();
                    std::io::stderr().write_all(&[b'e'; 4096]).unwrap();
                }
                println!("\nstdout Ω done");
                eprintln!("\nstderr Ω done");
                std::process::exit(17);
            }
            "wait" => {
                std::thread::sleep(Duration::from_secs(30));
            }
            "descendant" => {
                let child = crate::process::spawn(child_command("wait")).unwrap();
                println!("descendant:{}", child.id());
                std::io::stdout().flush().unwrap();
                std::thread::sleep(Duration::from_secs(30));
            }
            _ => unreachable!(),
        }
    }

    #[test]
    fn drains_both_pipes_and_keeps_nonzero_status() {
        let result =
            run(child_command("flood"), Duration::from_secs(10), &AtomicBool::new(false), &mut |_| {}).unwrap();
        assert_eq!(result.status.code(), Some(17));
        assert!(result.stdout.ends_with("stdout Ω done\n"));
        assert!(result.stderr.ends_with("stderr Ω done\n"));
        assert!(result.stdout.len() <= 256 * 1024);
        assert!(result.said().contains("stderr Ω done"));
    }

    #[test]
    fn timeout_ends_the_child_tree() {
        let started = Instant::now();
        let mut output = String::new();
        let why = run(child_command("descendant"), Duration::from_millis(250), &AtomicBool::new(false), &mut |s| {
            output.push_str(s)
        })
        .unwrap_err();
        assert!(why.contains("timed out"));
        assert!(started.elapsed() < Duration::from_secs(3));
        #[cfg(unix)]
        {
            let pid: i32 = output.lines().find_map(|l| l.strip_prefix("descendant:")).unwrap().parse().unwrap();
            let end = Instant::now() + Duration::from_secs(1);
            while unsafe { libc::kill(pid, 0) } == 0 && Instant::now() < end {
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "the timed-out command left its child running");
        }
    }

    #[test]
    fn an_already_cancelled_command_does_not_start() {
        let why = run(child_command("wait"), Duration::from_secs(10), &AtomicBool::new(true), &mut |_| {}).unwrap_err();
        assert_eq!(why, "Cancelled");
    }
}
