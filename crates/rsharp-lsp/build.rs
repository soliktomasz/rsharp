//! Builds Passenger's sidecars from the pinned submodule (vendor/passenger, decision 1 of ROADMAP.md): its project Host
//! (MSBuild's design-time builds in a process of their own) and its extension to Roslyn's server, compiled against the
//! very server version Passenger pins, whose folder this finds in the NuGet cache. Where they went is compiled in
//! (RSHARP_SIDECARS, RSHARP_ROSLYN_SERVER). A machine without dotnet or the submodule's checkout builds without them, said
//! as a warning, and runs the person's own `roslyn-language-server` tool instead. RSharp's own XAML host (`xaml-host/`,
//! Passenger.Xaml behind JSON lines) is published beside them into `sidecars/xaml`.

use std::path::{Path, PathBuf};

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let passenger = manifest.join("../../vendor/passenger/src");
    for input in
        ["Passenger.Host", "Passenger.Roslyn.Extension", "Passenger.Lsp/RoslynServer.targets", "Publicize.targets"]
    {
        println!("cargo:rerun-if-changed={}", passenger.join(input).display());
    }
    println!("cargo:rerun-if-changed={}", passenger.join("Passenger.Xaml").display());
    println!("cargo:rerun-if-changed={}", passenger.join("Passenger.Text").display());
    for input in ["Program.cs", "RSharp.XamlHost.csproj", "packages.lock.json"] {
        println!("cargo:rerun-if-changed={}", manifest.join("xaml-host").join(input).display());
    }
    for input in ["Directory.Packages.props", "Directory.Build.props", "Directory.Build.targets", "global.json"] {
        println!("cargo:rerun-if-changed={}", passenger.join("..").join(input).display());
    }
    let without = |why: &str| {
        println!(
            "cargo:warning=Passenger's sidecars were not built ({why}); RSharp runs the installed roslyn-language-server tool"
        );
        println!("cargo:rustc-env=RSHARP_SIDECARS=");
        println!("cargo:rustc-env=RSHARP_ROSLYN_SERVER=");
    };
    let host = passenger.join("Passenger.Host/Passenger.Host.csproj");
    let extension = passenger.join("Passenger.Roslyn.Extension/Passenger.Roslyn.Extension.csproj");
    if !host.is_file() {
        return without("vendor/passenger is not checked out: git submodule update --init");
    }
    let Some(dotnet) = rsharp_core::programs::dotnet() else { return without("dotnet was not found") };
    // target/<profile>/sidecars, beside the executables, whichever crate's build asked.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sidecars = out.ancestors().nth(3).expect("OUT_DIR is target/<profile>/build/<crate>/out").join("sidecars");
    // What the command printed, its warnings and errors included, told apart from its answer below: MSBuild says
    // "MSBuild server unavailable ... Falling back to an in-process build" on stderr where a server cannot start, and
    // that line must not be read as the property asked for.
    let run = |args: &[&str], project: &Path| {
        let output = rsharp_core::dotnet::command(&dotnet, args, passenger.parent().unwrap())
            .arg(project)
            .args(["-nologo", "-v", "q"])
            .output()
            .unwrap_or_else(|e| panic!("dotnet {} could not run: {e}", args.join(" ")));
        let said = |bytes: &[u8]| String::from_utf8_lossy(bytes).into_owned();
        let (answer, errors) = (said(&output.stdout), said(&output.stderr));
        assert!(output.status.success(), "dotnet {} {} failed:\n{answer}{errors}", args.join(" "), project.display());
        answer
    };
    let host_out = sidecars.join("host");
    let extension_out = sidecars.join("roslyn-extension");
    run(&["publish", "-c", "Release", "-o", host_out.to_str().unwrap()], &host);
    run(&["publish", "-c", "Release", "-o", extension_out.to_str().unwrap()], &extension);
    let xaml_out = sidecars.join("xaml");
    run(
        &["publish", "-c", "Release", "-o", xaml_out.to_str().unwrap()],
        &manifest.join("xaml-host/RSharp.XamlHost.csproj"),
    );
    let server = run(&["msbuild", "-t:FindRoslynServer", "-getProperty:RoslynServerFolder"], &extension);
    let server = server.lines().map(str::trim).rfind(|l| !l.is_empty()).unwrap_or_default().to_string();
    assert!(
        Path::new(&server).join("Microsoft.CodeAnalysis.LanguageServer.dll").is_file(),
        "Roslyn's server is not at {server:?}"
    );
    println!("cargo:rustc-env=RSHARP_SIDECARS={}", sidecars.display());
    println!("cargo:rustc-env=RSHARP_ROSLYN_SERVER={server}");
}
