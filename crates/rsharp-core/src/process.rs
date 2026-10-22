//! Every program RSharp starts goes through [`spawn`], so none outlives the IDE: on macOS and Linux each child
//! leads a process group of its own, and a watchdog shell (Passenger's Lifeline) ends that group the moment
//! the IDE's end of a pipe to it closes, however the IDE ended: quit, crash, SIGTERM or SIGKILL.

use std::io;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::Mutex;

static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// A program RSharp started. Dropping it ends the program and all it started.
pub struct Child {
    pub process: std::process::Child,
    /// The watchdog, waiting on a pipe only this process holds open.
    lifeline: Option<std::process::Child>,
}

/// Starts `command` as a child the IDE ends with itself.
pub fn spawn(mut command: Command) -> io::Result<Child> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut process = command.spawn()?;
    LIVE.lock().unwrap().push(process.id());
    let lifeline = match watch(process.id()) {
        Ok(lifeline) => Some(lifeline),
        Err(error) => {
            let _ = process.kill();
            return Err(error);
        }
    };
    Ok(Child { process, lifeline })
}

/// A shell in a group of its own that reads its input until the IDE's end closes, then kills `group`.
#[cfg(unix)]
fn watch(group: u32) -> io::Result<std::process::Child> {
    let mut watchdog = Command::new("/bin/sh");
    watchdog
        .args(["-c", "read _ ; kill -KILL -- -$0 2>/dev/null", &group.to_string()])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    std::os::unix::process::CommandExt::process_group(&mut watchdog, 0);
    watchdog.spawn()
}

// shortcut: on Windows only the child itself ends, not what it started; a job object (Passenger's Lifeline
// there) closes both.
#[cfg(not(unix))]
fn watch(_: u32) -> io::Result<std::process::Child> {
    Err(io::Error::other("no watchdog on this system yet"))
}

impl Child {
    pub fn id(&self) -> u32 {
        self.process.id()
    }

    /// Ends the program and everything it started.
    pub fn kill(&mut self) {
        end(self.process.id());
        let _ = self.process.kill();
        let _ = self.process.wait();
        self.release();
    }

    /// Everything the program writes to its piped output, once it has ended.
    pub fn output(mut self) -> io::Result<Vec<u8>> {
        let mut out = Vec::new();
        if let Some(mut stdout) = self.process.stdout.take() {
            std::io::Read::read_to_end(&mut stdout, &mut out)?;
        }
        self.wait()?;
        Ok(out)
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.process.wait();
        self.release();
        status
    }

    /// The program has ended: its watchdog is let go (its input closed, so it kills an empty group and exits).
    fn release(&mut self) {
        LIVE.lock().unwrap().retain(|live| *live != self.process.id());
        if let Some(mut lifeline) = self.lifeline.take() {
            drop(lifeline.stdin.take());
            let _ = lifeline.wait();
        }
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if let Ok(None) = self.process.try_wait() {
            self.kill();
        } else {
            self.release();
        }
    }
}

/// Ends every child still running: called as the IDE quits and from its panic hook.
pub fn end_all() {
    for pid in std::mem::take(&mut *LIVE.lock().unwrap()) {
        end(pid);
    }
}

fn end(pid: u32) {
    #[cfg(unix)]
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
    #[cfg(not(unix))]
    let _ = pid;
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::time::{Duration, Instant};

    fn gone_soon(pid: libc::pid_t, what: &str) {
        let start = Instant::now();
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(start.elapsed() < Duration::from_secs(5), "{what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn first_line(child: &mut Child) -> String {
        let mut line = String::new();
        BufReader::new(child.process.stdout.take().unwrap()).read_line(&mut line).unwrap();
        line
    }

    #[test]
    fn a_program_the_ide_ends_takes_what_it_started_with_it() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & echo $!; wait"]).stdout(Stdio::piped());
        let mut child = spawn(command).unwrap();
        let grandchild: libc::pid_t = first_line(&mut child).trim().parse().unwrap();

        drop(child);

        gone_soon(grandchild, "the program's own child outlived it");
    }

    /// Run alone by the test below as a stand-in IDE: starts a program that starts another, says the
    /// grandchild's id, and waits to be killed.
    #[test]
    #[ignore]
    fn stand_in_ide() {
        if std::env::var_os("RSHARP_STAND_IN_IDE").is_none() {
            return;
        }
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & echo $!; wait"]).stdout(Stdio::piped());
        let mut child = spawn(command).unwrap();
        println!("GRANDCHILD {}", first_line(&mut child).trim());
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    fn what_the_ide_started_ends_even_when_the_ide_is_killed_outright() {
        let mut ide = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "process::tests::stand_in_ide", "--ignored", "--nocapture", "--test-threads=1"])
            .env("RSHARP_STAND_IN_IDE", "1")
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut out = BufReader::new(ide.stdout.take().unwrap());
        let grandchild: libc::pid_t = loop {
            let mut line = String::new();
            assert!(out.read_line(&mut line).unwrap() > 0, "the stand-in IDE ended before starting its program");
            // libtest prints its own `test … ...` before the stand-in's line.
            if let Some((_, pid)) = line.trim().split_once("GRANDCHILD ") {
                break pid.parse().unwrap();
            }
        };

        ide.kill().unwrap(); // SIGKILL: no hook, no Drop runs in the stand-in.
        ide.wait().unwrap();

        gone_soon(grandchild, "a program the killed IDE had started was still running");
    }
}
