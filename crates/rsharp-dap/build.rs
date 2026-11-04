//! Builds Passenger's debug adapter from the pinned submodule (vendor/passenger, decision 1 of ROADMAP.md): its engine
//! behind the Debug Adapter Protocol, in a process of its own, as Passenger carries it under `debugger/`. Where it went
//! is compiled in (RSHARP_DEBUG_ADAPTER). A machine without dotnet or the submodule's checkout builds without it, said
//! as a warning, and RSharp then says why it cannot debug.

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let passenger = manifest.join("../../vendor/passenger/src");
    for input in [
        "Passenger.DebugAdapter",
        "Passenger.Debugger",
        "Passenger.Debugger.Interop",
        "Passenger.Dap",
        "Passenger.Core",
        "Passenger.Inspector",
        "Passenger.Inspector.Wpf",
        "Publicize.targets",
    ] {
        println!("cargo:rerun-if-changed={}", passenger.join(input).display());
    }
    for input in ["Directory.Packages.props", "Directory.Build.props", "Directory.Build.targets", "global.json"] {
        println!("cargo:rerun-if-changed={}", passenger.join("..").join(input).display());
    }
    println!("cargo:rerun-if-env-changed=CI");
    let without = |why: &str| {
        assert!(std::env::var("CI").as_deref() != Ok("true"), "Passenger's debug adapter is required in CI: {why}");
        println!("cargo:warning=Passenger's debug adapter was not built ({why}); RSharp cannot debug");
        println!("cargo:rustc-env=RSHARP_DEBUG_ADAPTER=");
        println!("cargo:rustc-env=RSHARP_INSPECTOR=");
        println!("cargo:rustc-env=RSHARP_INSPECTOR_WPF=");
    };
    let project = passenger.join("Passenger.DebugAdapter/Passenger.DebugAdapter.csproj");
    if !project.is_file() {
        return without("vendor/passenger is not checked out: git submodule update --init");
    }
    let Some(dotnet) = rsharp_core::programs::dotnet() else { return without("dotnet was not found") };
    // target/<profile>/sidecars/debugger, beside the other sidecars, whichever crate's build asked.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sidecars = out.ancestors().nth(3).expect("OUT_DIR is target/<profile>/build/<crate>/out").join("sidecars");
    let output =
        match rsharp_core::dotnet::command(&dotnet, ["publish", "-c", "Release", "-o"], passenger.parent().unwrap())
            .arg(&sidecars.join("debugger"))
            .arg(&project)
            // NuGet's audit is not this build's business: the feeds' advisories are the NuGet window's, and a machine that
            // cannot write its audit cache (a sandbox, an offline runner) must still build the adapter.
            .args(["-nologo", "-v", "q", "-p:WarningsNotAsErrors=NU1900"])
            .output()
        {
            Ok(output) => output,
            Err(error) => return without(&format!("dotnet publish could not run: {error}")),
        };
    let said = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
    let (answer, errors) = (said(&output.stdout), said(&output.stderr));
    if !output.status.success() {
        return without(&format!("dotnet publish {} failed:\n{answer}{errors}", project.display()));
    }
    let adapter = sidecars.join("debugger").join("Passenger.DebugAdapter.dll");
    if !adapter.is_file() {
        return without(&format!("the debug adapter is not at {}", adapter.display()));
    }
    println!("cargo:rustc-env=RSHARP_DEBUG_ADAPTER={}", adapter.display());
    // Passenger's inspector, which a debugged Avalonia app loads as a startup hook for the Live Visual Tree; its own
    // assembly alone, as the app's Avalonia must be the one it uses.
    let inspector =
        rsharp_core::dotnet::command(&dotnet, ["publish", "-c", "Release", "-o"], passenger.parent().unwrap())
            .arg(sidecars.join("inspector"))
            .arg(passenger.join("Passenger.Inspector/Passenger.Inspector.csproj"))
            .args(["-nologo", "-v", "q", "-p:WarningsNotAsErrors=NU1900"])
            .output();
    let carried = sidecars.join("inspector").join("Passenger.Inspector.dll");
    match inspector {
        Ok(output) if output.status.success() && carried.is_file() => {
            // The app brings the Avalonia the inspector uses: anything else published beside it would load instead.
            for entry in std::fs::read_dir(sidecars.join("inspector")).into_iter().flatten().flatten() {
                if !entry.file_name().to_string_lossy().starts_with("Passenger.Inspector.") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
            println!("cargo:rustc-env=RSHARP_INSPECTOR={}", carried.display())
        }
        _ => {
            println!("cargo:warning=Passenger's inspector was not built; the Live Visual Tree is not available");
            println!("cargo:rustc-env=RSHARP_INSPECTOR=");
        }
    }
    // WPF's counterpart, carried on Windows alone, as Passenger carries it, where WPF runs; it needs no assembly of the
    // app's, WPF being the app's own shared framework, so what is published beside it goes.
    let wpf = sidecars.join("inspector").join("wpf");
    let carried = wpf.join("Passenger.Inspector.Wpf.dll");
    let built = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows")
        && rsharp_core::dotnet::command(&dotnet, ["publish", "-c", "Release", "-o"], passenger.parent().unwrap())
            .arg(&wpf)
            .arg(passenger.join("Passenger.Inspector.Wpf/Passenger.Inspector.Wpf.csproj"))
            .args(["-nologo", "-v", "q", "-p:WarningsNotAsErrors=NU1900"])
            .output()
            .is_ok_and(|output| output.status.success())
        && carried.is_file();
    if built {
        for entry in std::fs::read_dir(&wpf).into_iter().flatten().flatten() {
            if !entry.file_name().to_string_lossy().starts_with("Passenger.Inspector.Wpf.") {
                let _ = std::fs::remove_file(entry.path());
            }
        }
        println!("cargo:rustc-env=RSHARP_INSPECTOR_WPF={}", carried.display());
    } else {
        println!("cargo:rustc-env=RSHARP_INSPECTOR_WPF=");
    }
}
