//! Every program RSharp starts goes through [`spawn`], so none outlives the IDE, however the IDE ended: quit,
//! crash, SIGTERM or SIGKILL. On macOS and Linux each child leads a process group and a watchdog shell
//! (Passenger's Lifeline) kills the group once the IDE's end of a pipe to it closes; on Windows each child is in
//! a job of its own that ends everything in it when the IDE's handle to it closes.

use std::io;
use std::process::{Command, ExitStatus};
use std::sync::Mutex;

static LIVE: Mutex<Vec<u32>> = Mutex::new(Vec::new());

/// A program RSharp started. Dropping it ends the program and all it started.
pub struct Child {
    pub process: std::process::Child,
    lifeline: Option<Lifeline>,
}

/// Starts `command` as a child the IDE ends with itself.
pub fn spawn(mut command: Command) -> io::Result<Child> {
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    let mut process = command.spawn()?;
    LIVE.lock().unwrap().push(process.id());
    match Lifeline::watch(&process) {
        Ok(lifeline) => Ok(Child { process, lifeline: Some(lifeline) }),
        Err(error) => {
            let _ = process.kill();
            let _ = process.wait();
            LIVE.lock().unwrap().retain(|live| *live != process.id());
            Err(error)
        }
    }
}

impl Child {
    pub fn id(&self) -> u32 {
        self.process.id()
    }

    /// Ends the program and everything it started.
    pub fn kill(&mut self) {
        match &self.lifeline {
            Some(lifeline) => lifeline.end_now(self.process.id()),
            None => end(self.process.id()),
        }
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

    /// The program has ended: its lifeline is let go, which ends anything it left running.
    fn release(&mut self) {
        LIVE.lock().unwrap().retain(|live| *live != self.process.id());
        drop(self.lifeline.take());
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

#[cfg(unix)]
fn end(pid: u32) {
    unsafe {
        libc::killpg(pid as libc::pid_t, libc::SIGKILL);
    }
}

// Windows ends a child's tree through its job, whose handle closes as the process ends.
#[cfg(windows)]
fn end(_: u32) {}

/// A shell in a group of its own that reads its input until the IDE's end closes, then kills the child's group.
#[cfg(unix)]
struct Lifeline(std::process::Child);

#[cfg(unix)]
impl Lifeline {
    fn watch(child: &std::process::Child) -> io::Result<Self> {
        let mut watchdog = Command::new("/bin/sh");
        // No `--`: dash, Debian's and Ubuntu's /bin/sh, reads it as a pid and kills nothing.
        watchdog
            .args(["-c", "read _ ; kill -KILL -$0 2>/dev/null", &child.id().to_string()])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        std::os::unix::process::CommandExt::process_group(&mut watchdog, 0);
        watchdog.spawn().map(Self)
    }

    fn end_now(&self, pid: u32) {
        end(pid);
    }
}

#[cfg(unix)]
impl Drop for Lifeline {
    fn drop(&mut self) {
        drop(self.0.stdin.take());
        let _ = self.0.wait();
    }
}

/// A job of the child's own, which ends every process in it when its last handle, the IDE's, closes.
// shortcut: what the child starts before it is assigned to the job escapes it; starting it suspended closes
// that gap, as Passenger's runtime does.
#[cfg(windows)]
struct Lifeline(windows_sys::Win32::Foundation::HANDLE);

// A job's handle may be used and closed from any thread; the language server's child is shared across them.
#[cfg(windows)]
unsafe impl Send for Lifeline {}
#[cfg(windows)]
unsafe impl Sync for Lifeline {}

#[cfg(windows)]
impl Lifeline {
    fn watch(child: &std::process::Child) -> io::Result<Self> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(io::Error::last_os_error());
            }
            let lifeline = Self(job);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            let set = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                (&limits as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            );
            if set == 0 || AssignProcessToJobObject(job, child.as_raw_handle() as _) == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(lifeline)
        }
    }

    fn end_now(&self, _: u32) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.0, 1);
        }
    }
}

#[cfg(windows)]
impl Drop for Lifeline {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::process::Stdio;
    use std::time::Duration;

    /// A program that starts another, prints that one's process id and waits for it.
    fn starts_a_grandchild() -> Command {
        if cfg!(windows) {
            let mut command = Command::new("powershell");
            command.args([
                "-NoProfile",
                "-Command",
                "$p = Start-Process -FilePath ping -ArgumentList '-n','60','127.0.0.1' -PassThru -WindowStyle Hidden; \
                 Write-Output $p.Id; Wait-Process -Id $p.Id",
            ]);
            command
        } else {
            let mut command = Command::new("sh");
            command.args(["-c", "sleep 60 & echo $!; wait"]);
            command
        }
    }

    #[cfg(unix)]
    fn ends_within(pid: u32, limit: Duration) -> bool {
        let start = std::time::Instant::now();
        while unsafe { libc::kill(pid as libc::pid_t, 0) } == 0 {
            if start.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        true
    }

    #[cfg(windows)]
    fn ends_within(pid: u32, limit: Duration) -> bool {
        use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
        use windows_sys::Win32::System::Threading::{OpenProcess, PROCESS_SYNCHRONIZE, WaitForSingleObject};
        unsafe {
            let process = OpenProcess(PROCESS_SYNCHRONIZE, 0, pid);
            if process.is_null() {
                return true;
            }
            let ended = WaitForSingleObject(process, limit.as_millis() as u32) == WAIT_OBJECT_0;
            CloseHandle(process);
            ended
        }
    }

    fn grandchild_of(out: impl std::io::Read) -> u32 {
        // libtest prints its own `test … ...` before a stand-in's line.
        for line in BufReader::new(out).lines() {
            let line = line.unwrap();
            if let Some((_, pid)) = line.trim().split_once("GRANDCHILD ") {
                return pid.trim().parse().unwrap();
            }
            if let Ok(pid) = line.trim().parse() {
                return pid;
            }
        }
        panic!("the program ended without saying its grandchild's id");
    }

    #[test]
    fn a_program_the_ide_ends_takes_what_it_started_with_it() {
        let mut command = starts_a_grandchild();
        command.stdout(Stdio::piped());
        let mut child = spawn(command).unwrap();
        let grandchild = grandchild_of(child.process.stdout.take().unwrap());

        drop(child);

        assert!(ends_within(grandchild, Duration::from_secs(5)), "the program's own child outlived it");
    }

    /// Run alone by the test below as a stand-in IDE: starts a program that starts another, says the
    /// grandchild's id, and waits to be killed.
    #[test]
    #[ignore]
    fn stand_in_ide() {
        if std::env::var_os("RSHARP_STAND_IN_IDE").is_none() {
            return;
        }
        let mut command = starts_a_grandchild();
        command.stdout(Stdio::piped());
        let mut child = spawn(command).unwrap();
        println!("GRANDCHILD {}", grandchild_of(child.process.stdout.take().unwrap()));
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
        let grandchild = grandchild_of(ide.stdout.take().unwrap());

        ide.kill().unwrap(); // SIGKILL, or TerminateProcess: no hook and no Drop runs in the stand-in.
        ide.wait().unwrap();

        assert!(
            ends_within(grandchild, Duration::from_secs(5)),
            "a program the killed IDE had started was still running"
        );
    }
}
