use rsharp_lsp::SharedHost;

#[test]
fn the_host_evaluates_a_project_into_its_files_and_packages() {
    let base = std::env::temp_dir().join(format!("rsharp-host-evaluate-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("App")).unwrap();
    std::fs::create_dir_all(base.join("Shared")).unwrap();
    let base = rsharp_core::paths::canonical(&base);
    let Some(host) = SharedHost::carried(&base) else {
        eprintln!("skipped: Passenger's sidecars were not built (no dotnet or no vendor/passenger at build time)");
        return;
    };
    std::fs::write(
        base.join("App/App.csproj"),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>Exe</OutputType>\n    \
         <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    \
         <PackageReference Include=\"Humanizer\" Version=\"2.14.1\" />\n    \
         <Compile Include=\"../Shared/Version.cs\" Link=\"Properties/Version.cs\" />\n  </ItemGroup>\n</Project>\n",
    )
    .unwrap();
    std::fs::write(base.join("App/Program.cs"), "System.Console.WriteLine();\n").unwrap();
    std::fs::write(base.join("Shared/Version.cs"), "class Version { }\n").unwrap();
    let project = host.evaluate(&base.join("App/App.csproj"), None).expect("the Host did not evaluate the project");
    assert_eq!(project.problem, None, "the project would show as failed to load");
    assert_eq!(project.target_frameworks, ["net10.0"], "the project's row would not name its framework");
    assert!(
        project.dependencies[0]
            .packages
            .iter()
            .any(|p| p.name == "Humanizer" && p.version.as_deref() == Some("2.14.1")),
        "Dependencies would not list the package: {:?}",
        project.dependencies
    );
    assert!(
        project.items.iter().any(|i| i.link.as_deref() == Some("Properties/Version.cs")),
        "the linked file would not show at its Link: {:?}",
        project.items
    );
    let _ = std::fs::remove_dir_all(&base);
}
