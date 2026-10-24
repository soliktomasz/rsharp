use super::*;
use std::time::Instant;

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn wait_for_file(file: &Path) {
    let began = Instant::now();
    while !file.exists() {
        assert!(began.elapsed() < Duration::from_secs(30), "the generating target never reached {}", file.display());
        std::thread::sleep(POLL);
    }
}

#[test]
fn generating_design_time_builds_and_external_holds_take_turns() {
    let (Some(dotnet), Some(carried)) = (rsharp_core::programs::dotnet(), crate::roslyn::carried_host()) else {
        eprintln!("skipped: dotnet or the carried Passenger Host is unavailable");
        return;
    };
    let folder = Folder(std::env::temp_dir().join(format!("rsharp-host-generating-{}", std::process::id())));
    std::fs::create_dir_all(&folder.0).unwrap();
    let mut list = rsharp_core::dotnet::command(&dotnet, ["--list-sdks"], &folder.0);
    list.stdout(Stdio::piped()).stderr(Stdio::null());
    let sdks = process::spawn(list).unwrap().output().unwrap();
    let sdks = String::from_utf8_lossy(&sdks);
    let Some(sdk) = sdks.lines().filter_map(|line| line.split_whitespace().next()).rfind(|v| v.starts_with("10."))
    else {
        eprintln!("skipped: the carried Host needs an installed .NET 10 SDK");
        return;
    };
    std::fs::write(folder.0.join("global.json"), json!({"sdk":{"version":sdk,"rollForward":"disable"}}).to_string())
        .unwrap();
    std::fs::write(
        folder.0.join("NuGet.Config"),
        "<configuration><packageSources><clear /></packageSources></configuration>",
    )
    .unwrap();
    let project = folder.0.join("Generating.csproj");
    std::fs::write(&project, r#"<Project>
  <UsingTask TaskName="WaitForGate" TaskFactory="RoslynCodeTaskFactory" AssemblyFile="$(MSBuildToolsPath)/Microsoft.Build.Tasks.Core.dll">
<ParameterGroup><Folder ParameterType="System.String" Required="true" /></ParameterGroup>
<Task><Code Type="Fragment" Language="cs"><![CDATA[
  System.IO.File.WriteAllText(System.IO.Path.Combine(Folder, "entered"), "running");
  var deadline = System.DateTime.UtcNow.AddSeconds(30);
  while (!System.IO.File.Exists(System.IO.Path.Combine(Folder, "release")) && System.DateTime.UtcNow < deadline)
    System.Threading.Thread.Sleep(20);
  return System.IO.File.Exists(System.IO.Path.Combine(Folder, "release"));
]]></Code></Task>
  </UsingTask>
  <ItemGroup><CscCommandLineArgs Include="/noconfig" /></ItemGroup>
  <Target Name="Compile"><WaitForGate Folder="$(MSBuildProjectDirectory)" /></Target>
  <Target Name="CoreCompile" />
  <Target Name="DesignTimeMarkupCompilation" />
</Project>"#).unwrap();
    let mut command = Command::new(&dotnet);
    command
        .arg(&carried)
        .arg("--folder")
        .arg(&folder.0)
        .current_dir(&folder.0)
        .env("DOTNET_CLI_HOME", folder.0.join("cli"))
        .env("NUGET_PACKAGES", folder.0.join("packages"))
        .env("NuGetAudit", "false")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let running = ProjectHost::start_command(command, None).unwrap();
    let host =
        SharedHost { running: Arc::new(Mutex::new(Some(running))), dotnet, host: carried, folder: folder.0.clone() };
    let build = |host: SharedHost| {
        let project = project.clone();
        let (sender, answer) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = sender.send(host.build(&project, &json!({})));
        });
        answer
    };
    let first = build(host.clone());
    wait_for_file(&folder.0.join("entered"));
    let cancel = Arc::new(AtomicBool::new(false));
    let stopping = cancel.clone();
    let holding = host.clone();
    let (progress, waiting) = mpsc::channel();
    let (sender, acquired) = mpsc::channel();
    std::thread::spawn(move || {
        let answer = holding.hold(&stopping, |projects| {
            let _ = progress.send(projects.to_vec());
        });
        let _ = sender.send(answer);
    });
    let projects = waiting.recv_timeout(Duration::from_secs(3)).expect("a hold did not report its running project");
    assert_eq!(projects, [project.clone()]);
    assert!(acquired.try_recv().is_err(), "an external build could start while the generating target was running");
    cancel.store(true, Ordering::Release);
    assert!(acquired.recv_timeout(Duration::from_secs(1)).expect("Stop waited for the generating build").is_err());
    let holding = host.clone();
    let (progress, waiting) = mpsc::channel();
    let (sender, acquired) = mpsc::channel();
    std::thread::spawn(move || {
        let answer = holding.hold(&AtomicBool::new(false), |projects| {
            let _ = progress.send(projects.to_vec());
        });
        let _ = sender.send(answer);
    });
    waiting.recv_timeout(Duration::from_secs(3)).expect("the next hold did not report its running project");
    std::fs::write(folder.0.join("release"), "").unwrap();
    first.recv_timeout(Duration::from_secs(30)).expect("the first generating build did not finish").unwrap();
    let hold = acquired
        .recv_timeout(Duration::from_secs(3))
        .expect("the drained build did not release the hold waiter")
        .unwrap();
    std::fs::remove_file(folder.0.join("release")).unwrap();
    std::fs::remove_file(folder.0.join("entered")).unwrap();
    let second = build(host.clone());
    assert!(
        second.recv_timeout(Duration::from_millis(150)).is_err(),
        "a generating build ran through the acquired hold"
    );
    assert!(!folder.0.join("entered").exists(), "the generating target started under an external build");
    drop(hold);
    wait_for_file(&folder.0.join("entered"));
    std::fs::write(folder.0.join("release"), "").unwrap();
    second
        .recv_timeout(Duration::from_secs(30))
        .expect("releasing the external hold left a design-time build waiting")
        .unwrap();
}
