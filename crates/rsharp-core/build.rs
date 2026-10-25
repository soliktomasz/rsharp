use std::{path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=src/process/pty_guard.rs");
    if std::env::var("CARGO_CFG_UNIX").is_err() {
        return;
    }
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    let folder = out.ancestors().nth(3).unwrap().join("sidecars");
    std::fs::create_dir_all(&folder).unwrap();
    let compiled = out.join("pty-guard");
    let guard = folder.join("pty-guard");
    let status = Command::new(std::env::var_os("RUSTC").unwrap())
        .args(["--edition=2024", "-O", "--crate-name", "rsharp_pty_guard", "--target"])
        .arg(std::env::var_os("TARGET").unwrap())
        .arg("src/process/pty_guard.rs")
        .arg("-o")
        .arg(&compiled)
        .status()
        .unwrap();
    assert!(status.success(), "could not build the PTY crash guardian");
    let staging = folder.join(format!(".pty-guard-{}", std::process::id()));
    std::fs::copy(&compiled, &staging).unwrap();
    std::fs::rename(staging, &guard).unwrap();
    println!("cargo:rustc-env=RSHARP_PTY_GUARD={}", compiled.display());
}
