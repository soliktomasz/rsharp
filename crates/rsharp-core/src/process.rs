//! Every program RSharp starts goes through [`spawn`], so none outlives the IDE: on macOS and Linux each child
//! leads a process group of its own, and ending it ends everything it started.

use std::io;
use std::process::{Command, ExitStatus};
use std::sync::Mutex;

static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// A program RSharp started. Dropping it ends the program and all it started.
pub struct Child {
    pub process: std::process::Child,
}

/// Starts `command` as a child the IDE ends with itself.
pub fn spawn(mut command: Command) -> io::Result<Child> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let process = command.spawn()?;
    LIVE.lock().unwrap().push(process.id());
    Ok(Child { process })
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
        forget(self.process.id());
    }

    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        let status = self.process.wait();
        forget(self.process.id());
        status
    }
}

impl Drop for Child {
    fn drop(&mut self) {
        if let Ok(None) = self.process.try_wait() {
            self.kill();
        } else {
            forget(self.process.id());
        }
    }
}

/// Ends every child still running: called as the IDE quits and from its panic hook.
pub fn end_all() {
    for pid in std::mem::take(&mut *LIVE.lock().unwrap()) {
        end(pid);
    }
}

fn forget(pid: u32) {
    LIVE.lock().unwrap().retain(|live| *live != pid);
}

// shortcut: a crash that skips the panic hook (SIGKILL, abort) leaves children running, and on Windows only the
// child itself ends, not what it started; a watchdog process (Passenger's Lifeline) or a job object closes both.
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
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    #[test]
    fn a_program_the_ide_ends_takes_what_it_started_with_it() {
        let mut command = Command::new("sh");
        command.args(["-c", "sleep 60 & echo $!; wait"]).stdout(Stdio::piped());
        let mut child = spawn(command).unwrap();
        let mut line = String::new();
        BufReader::new(child.process.stdout.take().unwrap()).read_line(&mut line).unwrap();
        let grandchild: libc::pid_t = line.trim().parse().unwrap();

        drop(child);

        let start = Instant::now();
        while unsafe { libc::kill(grandchild, 0) } == 0 {
            assert!(start.elapsed() < Duration::from_secs(5), "the program's own child outlived it");
            std::thread::sleep(Duration::from_millis(20));
        }
    }
}
