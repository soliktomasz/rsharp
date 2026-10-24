//! Install and Remove as Passenger runs them, `dotnet add package` and `dotnet remove package`, against a folder feed
//! the test writes, with NuGet's caches and user settings of the test's own. Skips, saying why, without dotnet.

mod nupkg;

use rsharp_core::nuget::change;
use rsharp_core::templates::TemplateEnvironment;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n\n</Project>\n";

fn setup(name: &str) -> Option<(PathBuf, TemplateEnvironment)> {
    let Ok(mut environment) = TemplateEnvironment::discover() else {
        eprintln!("skipped: dotnet is not installed");
        return None;
    };
    let root = std::env::temp_dir().join(format!("rsharp-nuget-change-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let feed = root.join("feed");
    nupkg::nupkg(&feed.join("Demo.Core.1.0.0.nupkg"), "Demo.Core", "1.0.0");
    nupkg::nupkg(&feed.join("Demo.Core.2.0.0.nupkg"), "Demo.Core", "2.0.0");
    std::fs::create_dir_all(root.join("src/Demo")).unwrap();
    std::fs::write(
        root.join("nuget.config"),
        format!(
            "<configuration><packageSources><clear /><add key=\"local\" value=\"{}\" /></packageSources></configuration>",
            feed.display()
        ),
    )
    .unwrap();
    std::fs::write(root.join("src/Demo/Demo.csproj"), PROJECT).unwrap();
    environment.cli_home = root.join("home");
    environment.nuget_packages = Some(root.join("packages"));
    Some((rsharp_core::paths::canonical(&root), environment))
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn install_update_and_remove_write_the_project_as_dotnet_does() {
    let Some((root, environment)) = setup("plain") else { return };
    let project = root.join("src/Demo/Demo.csproj");
    let cancel = AtomicBool::new(false);
    let mut said = String::new();
    let mut output = |s: &str| said.push_str(s);

    change::install(&environment, &project, "Demo.Core", "1.0.0", &cancel, &mut output)
        .unwrap_or_else(|why| panic!("Install failed: {why}"));
    assert!(
        read(&project).contains("<PackageReference Include=\"Demo.Core\" Version=\"1.0.0\" />"),
        "{}",
        read(&project)
    );
    change::install(&environment, &project, "Demo.Core", "2.0.0", &cancel, &mut output).unwrap();
    assert!(read(&project).contains("Version=\"2.0.0\""), "Update did not move the version: {}", read(&project));
    change::remove(&environment, &project, "Demo.Core", &cancel, &mut output).unwrap();
    assert!(!read(&project).contains("Demo.Core"), "Remove left the reference: {}", read(&project));
    assert!(!said.is_empty(), "dotnet's output never reached the log");
    assert_eq!(change::touched(&project), [project.clone()]);
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn central_management_puts_the_version_in_directory_packages_props() {
    let Some((root, environment)) = setup("central") else { return };
    let props = root.join("Directory.Packages.props");
    std::fs::write(
        &props,
        "<Project>\n  <PropertyGroup>\n    <ManagePackageVersionsCentrally>true</ManagePackageVersionsCentrally>\n  </PropertyGroup>\n</Project>\n",
    )
    .unwrap();
    let project = root.join("src/Demo/Demo.csproj");
    assert_eq!(
        change::touched(&project),
        [project.clone(), props.clone()],
        "Directory.Packages.props was not named as written"
    );
    change::install(&environment, &project, "Demo.Core", "1.0.0", &AtomicBool::new(false), &mut |_| {})
        .unwrap_or_else(|why| panic!("Install failed: {why}"));
    assert!(read(&props).contains("<PackageVersion Include=\"Demo.Core\" Version=\"1.0.0\" />"), "{}", read(&props));
    assert!(read(&project).contains("<PackageReference Include=\"Demo.Core\" />"), "{}", read(&project));
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn a_package_no_feed_has_fails_with_dotnets_reason() {
    let Some((root, environment)) = setup("missing") else { return };
    let project = root.join("src/Demo/Demo.csproj");
    let why = change::install(&environment, &project, "Not.There", "1.0.0", &AtomicBool::new(false), &mut |_| {})
        .unwrap_err();
    assert!(!why.is_empty());
    assert_eq!(read(&project), PROJECT, "a failed install changed the project");
    let _ = std::fs::remove_dir_all(root);
}
