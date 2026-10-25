//! Kept outside the terminal session: stdin's EOF ends every process still in that session, across job groups.

use std::io::Read;

unsafe extern "C" {
    fn getsid(pid: i32) -> i32;
    fn kill(pid: i32, signal: i32) -> i32;
}

#[cfg(target_os = "macos")]
fn pids() -> Vec<i32> {
    #[link(name = "proc")]
    unsafe extern "C" {
        fn proc_listallpids(buffer: *mut std::ffi::c_void, size: i32) -> i32;
    }
    let count = unsafe { proc_listallpids(std::ptr::null_mut(), 0) }.max(0) as usize;
    let mut pids = vec![0; count + 1024];
    let n = unsafe { proc_listallpids(pids.as_mut_ptr().cast(), (pids.len() * 4) as i32) }.max(0) as usize;
    pids.truncate(n.min(pids.len()));
    pids
}
#[cfg(not(target_os = "macos"))]
fn pids() -> Vec<i32> {
    std::fs::read_dir("/proc")
        .into_iter()
        .flatten()
        .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
        .collect()
}
fn main() {
    let Some(sid) = std::env::args().nth(1).and_then(|s| s.parse::<i32>().ok()).filter(|p| *p > 1) else {
        std::process::exit(2);
    };
    // The parent holds this process's stdin: it is open for as long as the IDE lives, however the IDE ends.
    use std::io::Write;
    let _ = std::io::stdout().write_all(b"ready\n");
    let _ = std::io::stdout().flush();
    let mut byte = [0; 64];
    loop {
        match std::io::stdin().read(&mut byte) {
            Ok(0) | Err(_) => break,
            Ok(_) => {}
        }
    }
    // Every process of the session, which holds the jobs of a shell's other process groups too.
    for pid in pids() {
        if pid > 1 && pid != sid && unsafe { getsid(pid) } == sid {
            unsafe {
                kill(pid, 9);
            }
        }
    }
    // Let a shell waiting for a job reap it before taking away the controlling terminal.
    std::thread::sleep(std::time::Duration::from_millis(20));
    unsafe {
        kill(sid, 9);
    }
}
