//! PTY processes use the same crash-cleanup boundary as ordinary children, with session-wide Unix cleanup.

use super::{LIVE, Lifeline};
use portable_pty::{CommandBuilder, MasterPty, PtySize};

pub struct PtyChild {
    pub master: Box<dyn MasterPty + Send>,
    child: Box<dyn portable_pty::Child + Send + Sync>,
    lifeline: Option<Lifeline>,
    pid: u32,
}

pub fn spawn_pty(command: CommandBuilder, size: PtySize) -> Result<PtyChild, String> {
    let pair = portable_pty::native_pty_system().openpty(size).map_err(|e| e.to_string())?;
    let mut child = pair.slave.spawn_command(command).map_err(|e| e.to_string())?;
    drop(pair.slave);
    let pid = child.process_id().ok_or("The PTY process has no process id")?;
    #[cfg(unix)]
    let watched = Lifeline::watch_pid(pid, true);
    #[cfg(windows)]
    let watched = child
        .as_raw_handle()
        .ok_or_else(|| std::io::Error::other("The PTY process has no process handle"))
        .and_then(|handle| Lifeline::watch_handle(handle as _));
    let lifeline = match watched {
        Ok(lifeline) => lifeline,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("Could not protect the terminal process: {error}"));
        }
    };
    LIVE.lock().unwrap().push(pid);
    Ok(PtyChild { master: pair.master, child, lifeline: Some(lifeline), pid })
}

impl PtyChild {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<u32>> {
        self.child.try_wait().map(|status| status.map(|s| s.exit_code()))
    }

    pub fn kill(&mut self) {
        // Let the watchdog end every group in the session before reaping its leader.
        drop(self.lifeline.take());
        #[cfg(unix)]
        unsafe {
            libc::kill(self.pid as i32, libc::SIGKILL);
        }
        #[cfg(windows)]
        let _ = self.child.kill();
        let until = std::time::Instant::now() + std::time::Duration::from_secs(2);
        while matches!(self.child.try_wait(), Ok(None)) && std::time::Instant::now() < until {
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        LIVE.lock().unwrap().retain(|pid| *pid != self.pid);
    }
}

impl Drop for PtyChild {
    fn drop(&mut self) {
        self.kill();
    }
}
