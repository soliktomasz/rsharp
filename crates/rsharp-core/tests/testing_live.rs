use rsharp_core::project::EvaluatedProject;
use rsharp_core::testing::{TestEvent, TestRunners, TestState, resolve_assemblies};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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

fn command(dotnet: &Path, folder: &Path, arguments: &[&str]) -> String {
    let mut output = String::new();
    let code = rsharp_core::build::run(
        rsharp_core::dotnet::command(dotnet, arguments, folder),
        &AtomicBool::new(false),
        |text, _| output.push_str(text),
    )
    .unwrap();
    assert_eq!(code, Some(0), "The SDK fixture did not build: {output}");
    output
}

#[test]
fn real_sdk_runs_selected_xunit_v3_and_vstest_tests_and_cancels_their_hosts() {
    if let Err(error) = std::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)) {
        eprintln!("SKIP real test runners: loopback sockets unavailable: {error}");
        return;
    }
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("SKIP real test runners: no .NET SDK");
        return;
    };
    for (package, version) in [
        ("xunit.v3", "3.2.2"),
        ("xunit", "2.9.3"),
        ("xunit.runner.visualstudio", "3.1.5"),
        ("microsoft.net.test.sdk", "18.0.1"),
    ] {
        if !cached(package, version) {
            eprintln!("SKIP real test runners: {package} {version} is not cached");
            return;
        }
    }
    let folder = Folder(std::env::temp_dir().join(format!("rsharp-runners-{}", std::process::id())));
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
        std::fs::write(&project, format!(r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework>{executable}<ImplicitUsings>enable</ImplicitUsings><NuGetAudit>false</NuGetAudit><OutputPath>custom-output/</OutputPath></PropertyGroup><ItemGroup>{packages}</ItemGroup></Project>"#)).unwrap();
        let source = if name == "Platform" { "using Xunit;" } else { "using Xunit; using Xunit.Abstractions;" };
        let attachment =
            if name == "Platform" { "TestContext.Current.AddAttachment(\"note\", \"test attachment\");" } else { "" };
        std::fs::write(
            project_folder.join("Tests.cs"),
            format!(
                r#"{source}
namespace Example;
public sealed record Shape(int Sides);
public class Tests(ITestOutputHelper output) {{
 [Fact, Trait("Category", "Smoke")] public void Pass() {{ output.WriteLine("test output"); {attachment} }}
 [Fact] public void Fail() {{ Assert.Equal(5, 2 + 2); }}
 [Fact(Skip = "later")] public void Skip() {{ }}
 [Theory, InlineData(1), InlineData(2)] public void Rows(int value) {{ Assert.True(value > 0); }}
 public static IEnumerable<object[]> Shapes => [[new Shape(3)], [new Shape(4)]];
 [Theory, MemberData(nameof(Shapes))] public void Dynamic(Shape shape) {{ Assert.True(shape.Sides > 2); }}
 [Fact] public void Slow() {{ File.WriteAllText(@"{}", Environment.ProcessId.ToString()); Thread.Sleep(30000); }}
}}
"#,
                project_folder.join("running.txt").display()
            ),
        )
        .unwrap();
        command(&dotnet, &folder.0, &["build", project.to_str().unwrap(), "-nologo", "-v:q", "-nodeReuse:false"]);
        projects.push(EvaluatedProject {
            path: rsharp_core::paths::canonical(&project),
            is_test_project: true,
            target_frameworks: vec!["net10.0".into()],
            ..Default::default()
        });
    }
    let cancel = AtomicBool::new(false);
    let assemblies = resolve_assemblies(&dotnet, &folder.0, &projects, &cancel).unwrap();
    assert_eq!(assemblies.len(), 2, "MSBuild lost a test project's custom output path: {assemblies:?}");
    assert!(
        assemblies
            .iter()
            .all(|assembly| assembly.path.is_file() && assembly.path.to_string_lossy().contains("custom-output"))
    );
    let mut runners = TestRunners::new(dotnet, folder.0.clone());
    let tests = runners
        .discover(&assemblies, &cancel, &mut |event| {
            if let TestEvent::Log(log) = event {
                eprintln!("{log}");
            }
        })
        .unwrap();
    assert_eq!(tests.len(), 14, "The native protocols lost tests or theory rows: {tests:#?}");
    for project in &projects {
        let selected: Vec<_> = tests
            .iter()
            .filter(|test| test.project == project.path && !test.method().ends_with(".Slow"))
            .cloned()
            .collect();
        assert_eq!(selected.len(), 6);
        let mut streamed = Vec::new();
        let summary = runners
            .run(&selected, &cancel, &mut |event| match event {
                TestEvent::Outcomes(batch) => streamed.extend(batch),
                TestEvent::Log(log) => eprintln!("{log}"),
                _ => {}
            })
            .unwrap();
        assert!(!summary.aborted && !summary.canceled, "The test host failed: {summary:?}");
        assert_eq!(summary.outcomes.len(), 7, "A selected run lost its final batch: {summary:?}");
        assert_eq!(streamed, summary.outcomes, "Streamed results differ from the completed run");
        assert_eq!(summary.outcomes.iter().filter(|outcome| outcome.state == TestState::Passed).count(), 5);
        assert_eq!(summary.outcomes.iter().filter(|outcome| outcome.state == TestState::Failed).count(), 1);
        assert_eq!(summary.outcomes.iter().filter(|outcome| outcome.state == TestState::Skipped).count(), 1);
        let pass = summary.outcomes.iter().find(|outcome| outcome.test.method().ends_with(".Pass")).unwrap();
        assert!(
            pass.output.as_deref().is_some_and(|output| output.contains("test output")),
            "Test output was lost: {pass:?}"
        );
        assert!(pass.test.traits.contains(&("Category".into(), "Smoke".into())), "The adapter's traits were lost");
        if project.path.file_stem().unwrap() == "Platform" {
            assert_eq!(pass.attachments.len(), 1, "MTP lost the test's attachment: {pass:?}");
            assert!(std::fs::read_to_string(&pass.attachments[0].path).unwrap().contains("test attachment"));
        }
        let dynamic: Vec<_> =
            summary.outcomes.iter().filter(|outcome| outcome.test.method().ends_with(".Dynamic")).collect();
        assert_eq!(dynamic.len(), 2, "Runtime data rows were lost");
        assert_ne!(dynamic[0].test.id(), dynamic[1].test.id(), "Runtime data rows need distinct identities");
        let row = selected.iter().find(|test| test.method().ends_with(".Rows")).unwrap().clone();
        let one = runners.run(&[row.clone()], &cancel, &mut |_| {}).unwrap();
        assert_eq!(one.outcomes.len(), 1, "Running one row ran other rows too: {one:?}");
        assert_eq!(one.outcomes[0].test.id(), row.id());
        let fail = summary.outcomes.iter().find(|outcome| outcome.state == TestState::Failed).unwrap();
        assert!(fail.message.as_deref().is_some_and(|message| message.contains("5")) && fail.stack_trace.is_some());
        let slow: Vec<_> = tests
            .iter()
            .filter(|test| test.project == project.path && test.method().ends_with(".Slow"))
            .cloned()
            .collect();
        let began = Instant::now();
        let stopped = AtomicBool::new(false);
        let running = project.path.parent().unwrap().join("running.txt");
        std::thread::scope(|scope| {
            let cancel = &stopped;
            let running = &running;
            scope.spawn(move || {
                let began = Instant::now();
                while !running.is_file() && began.elapsed() < Duration::from_secs(30) {
                    std::thread::sleep(Duration::from_millis(20));
                }
                cancel.store(true, Ordering::Relaxed);
            });
            let summary = runners.run(&slow, &stopped, &mut |_| {}).unwrap();
            assert!(summary.canceled, "Stop did not cancel the test host: {summary:?}");
            assert!(summary.outcomes.is_empty(), "Canceled tests must remain not run");
        });
        assert!(running.is_file(), "The selected slow test never started");
        assert!(began.elapsed() < Duration::from_secs(15), "A test host ignored bounded cancellation");
        #[cfg(unix)]
        {
            let pid: i32 = std::fs::read_to_string(running).unwrap().parse().unwrap();
            let began = Instant::now();
            while unsafe { libc::kill(pid, 0) } == 0 && began.elapsed() < Duration::from_secs(5) {
                std::thread::sleep(Duration::from_millis(20));
            }
            assert_ne!(unsafe { libc::kill(pid, 0) }, 0, "The test host survived Stop");
        }
    }
}

#[test]
fn msbuild_assembly_resolution_keeps_imported_framework_order_and_custom_target_paths() {
    let Some(dotnet) = rsharp_core::programs::dotnet() else {
        eprintln!("SKIP assembly resolution: no .NET SDK");
        return;
    };
    let folder = Folder(std::env::temp_dir().join(format!("rsharp-assembly-resolution-{}", std::process::id())));
    std::fs::create_dir_all(&folder.0).unwrap();
    let project = folder.0.join("Odd (project);name.proj");
    std::fs::write(
        folder.0.join("frameworks.props"),
        r#"<Project><PropertyGroup><TestFrameworks>custom-b;custom-a</TestFrameworks></PropertyGroup></Project>"#,
    )
    .unwrap();
    std::fs::write(&project, r#"<Project><Import Project="frameworks.props" />
<Target Name="GetTargetFrameworks" Returns="@(_ThisProject)"><ItemGroup><_ThisProject Include="$(MSBuildProjectFullPath)" TargetFrameworks="$(TestFrameworks)" HasSingleTargetFramework="false" /></ItemGroup></Target>
<Target Name="GetTargetPath" Returns="@(_Target)"><ItemGroup><_Target Include="$(MSBuildProjectDirectory)/unusual-output/$(TargetFramework)/RealName.dll" /></ItemGroup></Target>
</Project>"#).unwrap();
    // The caller may identify tests by their framework package even when IsTestProject is false.
    let projects = [EvaluatedProject { path: project.clone(), ..Default::default() }];
    let assemblies = resolve_assemblies(&dotnet, &folder.0, &projects, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        assemblies.iter().map(|assembly| assembly.framework.as_str()).collect::<Vec<_>>(),
        ["custom-b", "custom-a"]
    );
    assert_eq!(assemblies[0].project, project);
    assert_eq!(assemblies[1].path, folder.0.join("unusual-output/custom-a/RealName.dll"));
    assert!(resolve_assemblies(&dotnet, &folder.0, &projects, &AtomicBool::new(true)).unwrap_err().contains("stopped"));
}
