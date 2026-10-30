//! Builds Passenger's sidecars from the pinned submodule (vendor/passenger, decision 1 of ROADMAP.md): its project Host
//! (MSBuild's design-time builds in a process of their own) and its extension to Roslyn's server, compiled against the
//! very server version Passenger pins, whose folder this finds in the NuGet cache. Where they went is compiled in
//! (RSHARP_SIDECARS, RSHARP_ROSLYN_SERVER). A machine without dotnet or the submodule's checkout builds without them, said
//! as a warning, and runs the person's own `roslyn-language-server` tool instead. RSharp's own XAML host (`xaml-host/`,
//! Passenger.Xaml behind JSON lines) is published beside them into `sidecars/xaml`, its previewer's entry into `sidecars/xaml/preview`.
//! The web language servers Passenger pins go to `sidecars/web` (RSHARP_WEB), whatever became of the rest.

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
    for input in [
        "Program.cs",
        "Preview.cs",
        "RSharp.XamlHost.csproj",
        "packages.lock.json",
        "preview/Program.cs",
        "preview/RSharp.Preview.csproj",
        "preview/packages.lock.json",
    ] {
        println!("cargo:rerun-if-changed={}", manifest.join("xaml-host").join(input).display());
    }
    for input in ["Directory.Packages.props", "Directory.Build.props", "Directory.Build.targets", "global.json"] {
        println!("cargo:rerun-if-changed={}", passenger.join("..").join(input).display());
    }
    // target/<profile>/sidecars, beside the executables, whichever crate's build asked.
    let out = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let sidecars = out.ancestors().nth(3).expect("OUT_DIR is target/<profile>/build/<crate>/out").join("sidecars");
    web_servers(&passenger.join("Passenger.Lsp/WebServers"), &sidecars.join("web"));
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
    // The previewer's entry, which the host names to the HostApp's dotnet (Preview.cs's Entry).
    run(
        &["publish", "-c", "Release", "-o", xaml_out.join("preview").to_str().unwrap()],
        &manifest.join("xaml-host/preview/RSharp.Preview.csproj"),
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

/// The language servers for TypeScript and JavaScript, HTML, CSS and JSON, as Passenger's WebServers.targets carries
/// them: its package-lock.json put in place by npm, each package checked against the sha512 it pins, the HTML server's
/// own TypeScript lib files moved to where it reads them (`../../node_modules/typescript/lib` from its folder). Done again
/// only when the lock file changes. Without the submodule or npm, web files keep their colours only, said as a warning.
// shortcut: the HTML server's own TypeScript keeps its tsserver and translations (Passenger leaves out a third of the
// size); leave them out when the packages ship.
fn web_servers(pins: &Path, out: &Path) {
    let lock_file = pins.join("package-lock.json");
    println!("cargo:rerun-if-changed={}", lock_file.display());
    let stamp = out.join("rsharp.stamp");
    let none = |why: &str| {
        println!(
            "cargo:warning=The web language servers were not put in place ({why}); web files keep their colours only"
        );
        println!("cargo:rustc-env=RSHARP_WEB=");
    };
    let Ok(lock) = std::fs::read(&lock_file) else { return none("vendor/passenger is not checked out") };
    if std::fs::read(&stamp).is_ok_and(|done| done == lock) {
        println!("cargo:rustc-env=RSHARP_WEB={}", out.display());
        return;
    }
    let mut also = vec![PathBuf::from("/opt/homebrew/bin"), PathBuf::from("/usr/local/bin")];
    if cfg!(windows) {
        also.push(r"C:\Program Files\nodejs".into());
    }
    let Some(npm) = rsharp_core::programs::find(if cfg!(windows) { "npm.cmd" } else { "npm" }, &also) else {
        return none("npm was not found");
    };
    let _ = std::fs::remove_dir_all(out);
    std::fs::create_dir_all(out).unwrap();
    for file in ["package.json", "package-lock.json"] {
        std::fs::copy(pins.join(file), out.join(file)).unwrap();
    }
    let status = std::process::Command::new(&npm)
        .args(["ci", "--ignore-scripts", "--no-audit", "--no-fund", "--loglevel=error"])
        .current_dir(out)
        .status()
        .unwrap_or_else(|e| panic!("npm could not run: {e}"));
    assert!(status.success(), "npm ci in {} failed", out.display());
    let modules = out.join("node_modules");
    let from = modules.join("vscode-langservers-extracted/node_modules/typescript/lib");
    let to = modules.join("node_modules/typescript/lib");
    std::fs::create_dir_all(&to).unwrap();
    for entry in std::fs::read_dir(&from).unwrap_or_else(|e| panic!("{} is not there: {e}", from.display())) {
        let name = entry.unwrap().file_name();
        let name = name.to_string_lossy();
        if name.starts_with("lib") && name.ends_with(".d.ts") {
            std::fs::copy(from.join(&*name), to.join(&*name)).unwrap();
        }
    }
    std::fs::write(&stamp, lock).unwrap();
    println!("cargo:rustc-env=RSHARP_WEB={}", out.display());
}
