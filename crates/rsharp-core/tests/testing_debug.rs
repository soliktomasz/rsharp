//! Debug Selected Tests: each runner hands its test host to the debugger before a test runs.

use rsharp_core::project::EvaluatedProject;
use rsharp_core::testing::{DebugTarget, TestRunners, TestState, resolve_assemblies};
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

struct Folder(PathBuf);
impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn cached(package: &str, version: &str) -> bool {
    let root = std::env::var_os("NUGET_PACKAGES").map(PathBuf::from).or_else(|| {
        std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
            .map(|home| PathBuf::from(home).join(".nuget/packages"))
    });
    root.is_some_and(|root| root.join(package).join(version).is_dir())
}

fn build(dotnet: &Path, folder: &Path, project: &Path) {
    let mut output = String::new();
    let code = rsharp_core::build::run(
        rsharp_core::dotnet::command(
            dotnet,
            ["build", project.to_str().unwrap(), "-nologo", "-v:q", "-nodeReuse:false"],
            folder,
        ),
        &AtomicBool::new(false),
        |text, _| output.push_str(text),
    )
    .unwrap();
    assert_eq!(code, Some(0), "The SDK fixture did not build: {output}");
}

#[cfg(unix)]
fn alive(pid: u32) -> bool {
    unsafe { libc::kill(pid as i32, 0) == 0 }
}
#[cfg(not(unix))]
fn alive(_: u32) -> bool {
    true
}

#[test]
fn debugged_runs_hand_each_runners_host_to_the_debugger() {
    if let Err(error) = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)) {
        eprintln!("SKIP debugged test runs: loopback sockets unavailable: {error}");
        return;
    }
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("SKIP debugged test runs: no .NET SDK");
        return;
    };
    for (package, version) in [
        ("xunit.v3", "3.2.2"),
        ("xunit", "2.9.3"),
        ("xunit.runner.visualstudio", "3.1.5"),
        ("microsoft.net.test.sdk", "18.0.1"),
    ] {
        if !cached(package, version) {
            eprintln!("SKIP debugged test runs: {package} {version} is not cached");
            return;
        }
    }
    let folder = Folder(std::env::temp_dir().join(format!("rsharp-debug-runners-{}", std::process::id())));
    std::fs::create_dir_all(&folder.0).unwrap();
    std::fs::write(
        folder.0.join("NuGet.Config"),
        "<configuration><packageSources><clear /></packageSources></configuration>",
    )
    .unwrap();
    let mut projects = Vec::new();
    for (name, packages, executable) in [
        ("Platform", r#"<PackageReference Include="xunit.v3" Version="3.2.2" />"#, "<OutputType>Exe</OutputType>"),
        (
            "VsTest",
            r#"<PackageReference Include="xunit" Version="2.9.3" /><PackageReference Include="xunit.runner.visualstudio" Version="3.1.5" /><PackageReference Include="Microsoft.NET.Test.Sdk" Version="18.0.1" />"#,
            "",
        ),
    ] {
        let project_folder = folder.0.join(name);
        std::fs::create_dir_all(&project_folder).unwrap();
        let project = project_folder.join(format!("{name}.csproj"));
        std::fs::write(&project, format!(r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework>{executable}<ImplicitUsings>enable</ImplicitUsings><NuGetAudit>false</NuGetAudit></PropertyGroup><ItemGroup>{packages}</ItemGroup></Project>"#)).unwrap();
        std::fs::write(
            project_folder.join("Tests.cs"),
            "using Xunit;\nnamespace Example;\npublic class Tests {\n [Fact] public void Pass() { }\n [Fact] public void Fail() { Assert.Equal(5, 2 + 2); }\n}\n",
        )
        .unwrap();
        build(&dotnet, &folder.0, &project);
        projects.push(EvaluatedProject {
            path: rsharp_core::paths::canonical(&project),
            is_test_project: true,
            target_frameworks: vec!["net10.0".into()],
            ..Default::default()
        });
    }
    let cancel = AtomicBool::new(false);
    let assemblies = resolve_assemblies(&dotnet, &folder.0, &projects, &cancel).unwrap();
    let mut runners = TestRunners::new(dotnet.clone(), folder.0.clone());
    let tests = runners.discover(&assemblies, &cancel, &mut |_| {}).unwrap();
    for project in &projects {
        let platform = project.path.file_stem().unwrap() == "Platform";
        let selected: Vec<_> = tests.iter().filter(|test| test.project == project.path).cloned().collect();
        assert_eq!(selected.len(), 2);
        let assembly = assemblies.iter().find(|a| a.project == project.path).unwrap().path.clone();
        // The debugger's part: start what the runner hands over (as the adapter would), or attach to what it started.
        let mut launched = Vec::new();
        let mut targets = Vec::new();
        let summary = runners
            .run_debugged(&selected, &cancel, &mut |_| {}, &mut |target| {
                let pid = match &target {
                    DebugTarget::Attach { process_id } => {
                        assert!(alive(*process_id), "The runner named a test host that is not running: {process_id}");
                        *process_id
                    }
                    DebugTarget::Launch { program, arguments, working_directory, environment } => {
                        let mut command = std::process::Command::new(program);
                        command
                            .args(arguments)
                            .envs(environment)
                            .current_dir(working_directory)
                            .stdout(std::process::Stdio::null())
                            .stderr(std::process::Stdio::null());
                        let child = rsharp_core::process::spawn(command).map_err(|e| e.to_string())?;
                        let pid = child.process.id();
                        launched.push(child);
                        pid
                    }
                };
                targets.push(target);
                Ok(pid)
            })
            .unwrap();
        assert_eq!(targets.len(), 1, "The debugger was asked {} times for one host: {summary:?}", targets.len());
        match &targets[0] {
            DebugTarget::Launch { program, arguments, .. } => {
                assert!(platform, "VSTest asked the debugger to start its .NET testhost rather than attach to it");
                assert_eq!(program, &dotnet, "The debugger would start the test app without the dotnet host");
                assert_eq!(arguments[..2], ["exec".to_string(), assembly.to_string_lossy().into_owned()]);
                assert_eq!(&arguments[2..6], ["--server", "--client-host", "127.0.0.1", "--client-port"]);
            }
            // VSTest starts a .NET testhost itself and waits for the debugger before it runs a test in it.
            DebugTarget::Attach { .. } => assert!(!platform, "MTP asked the debugger to attach rather than launch"),
        }
        assert!(!summary.aborted, "A debugged run failed: {summary:?}");
        assert_eq!(summary.outcomes.len(), 2, "A debugged run lost results: {summary:?}");
        assert_eq!(summary.outcomes.iter().filter(|o| o.state == TestState::Failed).count(), 1);

        let refused = runners
            .run_debugged(&selected, &cancel, &mut |_| {}, &mut |_| Err("the debugger could not start".into()))
            .unwrap();
        assert!(
            refused.error.as_deref().is_some_and(|e| e.contains("the debugger could not start")),
            "A refused debugger did not end the run with its reason: {refused:?}"
        );
        assert!(refused.outcomes.is_empty(), "Tests ran without the debugger the person asked for");
    }
}
