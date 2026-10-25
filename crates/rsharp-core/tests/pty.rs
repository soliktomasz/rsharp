#[cfg(unix)]
#[test]
fn a_pty_program_has_a_controlling_terminal_and_keeps_its_last_output_and_exit_code() {
    use std::io::Read;
    let mut command = portable_pty::CommandBuilder::new("/bin/sh");
    command.args(["-c", "test -t 0 && test -t 1 && stty size; printf 'last output'; exit 3"]);
    let mut child = rsharp_core::process::spawn_pty(
        command,
        portable_pty::PtySize { rows: 31, cols: 93, pixel_width: 0, pixel_height: 0 },
    )
    .unwrap();
    let mut reader = child.master.try_clone_reader().unwrap();
    let mut output = String::new();
    let mut bytes = [0; 4096];
    loop {
        match reader.read(&mut bytes) {
            Ok(0) => break,
            Ok(count) => output.push_str(&String::from_utf8_lossy(&bytes[..count])),
            // Linux reports a terminal whose program has ended as EIO rather than as end of file.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => break,
            Err(error) => panic!("reading the PTY failed: {error}"),
        }
    }
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    };
    assert!(output.contains("31 93"), "the program did not see the view's terminal size: {output}");
    assert!(output.ends_with("last output"), "the process lost its final output: {output}");
    assert_eq!(status, 3, "the PTY must retain a program's actual exit code");
}
#[cfg(unix)]
#[test]
fn closing_a_pty_ends_jobs_in_other_process_groups_too() {
    use std::io::BufRead;
    let mut command = portable_pty::CommandBuilder::new("/bin/bash");
    command.args(["-c", "set -m; sleep 60 & printf 'JOB %s\\n' $!; wait"]);
    let child = rsharp_core::process::spawn_pty(command, portable_pty::PtySize::default()).unwrap();
    let mut reader = std::io::BufReader::new(child.master.try_clone_reader().unwrap());
    let mut line = String::new();
    reader.read_line(&mut line).unwrap();
    let pid: u32 = line.trim().strip_prefix("JOB ").unwrap().parse().unwrap();
    drop(child);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while unsafe { libc::kill(pid as i32, 0) } == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_ne!(
        unsafe { libc::kill(pid as i32, 0) },
        0,
        "a shell job in another process group outlived its terminal tab"
    );
}
#[cfg(unix)]
#[test]
#[ignore]
fn stand_in_pty_ide() {
    use std::io::BufRead;
    let mut command = portable_pty::CommandBuilder::new("/bin/bash");
    command.args(["-c", "set -m; sleep 60 & printf 'JOB %s\\n' $!; wait"]);
    let child = rsharp_core::process::spawn_pty(command, portable_pty::PtySize::default()).unwrap();
    let mut line = String::new();
    std::io::BufReader::new(child.master.try_clone_reader().unwrap()).read_line(&mut line).unwrap();
    println!("{line}");
    use std::io::Write;
    std::io::stdout().flush().unwrap();
    std::thread::sleep(std::time::Duration::from_secs(60));
}

#[cfg(unix)]
#[test]
fn a_pty_job_ends_even_when_the_ide_is_killed_outright() {
    use std::io::BufRead;
    let mut ide = std::process::Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "stand_in_pty_ide", "--ignored", "--nocapture"])
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    let reader = std::io::BufReader::new(ide.stdout.take().unwrap());
    let pid = reader.lines().find_map(|line| line.ok()?.split_once("JOB ")?.1.trim().parse::<u32>().ok()).unwrap();
    ide.kill().unwrap();
    ide.wait().unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while unsafe { libc::kill(pid as i32, 0) } == 0 && std::time::Instant::now() < deadline {
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert_ne!(unsafe { libc::kill(pid as i32, 0) }, 0, "a terminal job outlived the killed IDE");
}
