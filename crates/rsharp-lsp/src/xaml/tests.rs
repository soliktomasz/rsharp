use super::*;

struct Folder(PathBuf);

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

const PROJECT: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <OutputType>WinExe</OutputType>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n  <ItemGroup>\n    <PackageReference Include=\"Avalonia\" Version=\"12.1.4\" />\n  </ItemGroup>\n</Project>\n";
const WINDOW: &str = "<Window xmlns=\"https://github.com/avaloniaui\"\n        xmlns:x=\"http://schemas.microsoft.com/winfx/2006/xaml\"\n        x:Class=\"Demo.MainWindow\">\n  <StackPanel>\n    <Button Foregrund=\"Red\" />\n  </StackPanel>\n</Window>\n";

#[test]
fn an_avalonia_window_is_checked_completed_and_described_from_its_restored_packages() {
    let (Some(host), Some(dotnet)) = (XamlHost::carried(), rsharp_core::programs::dotnet()) else {
        eprintln!("skipped: the XAML host was not built (no dotnet or no vendor/passenger at build time)");
        return;
    };
    let folder = Folder(std::env::temp_dir().join(format!("rsharp-xaml-host-{}", std::process::id())));
    std::fs::create_dir_all(&folder.0).unwrap();
    let folder_path = rsharp_core::paths::canonical(&folder.0);
    let (project, window) = (folder_path.join("Demo.csproj"), folder_path.join("MainWindow.axaml"));
    std::fs::write(&project, PROJECT).unwrap();
    std::fs::write(&window, WINDOW).unwrap();
    let code = "namespace Demo;\npublic partial class MainWindow : Avalonia.Controls.Window { }\n";
    std::fs::write(folder_path.join("MainWindow.axaml.cs"), code).unwrap();
    let restore = rsharp_core::dotnet::command(&dotnet, ["restore", "-v", "q"], &folder_path).output().unwrap();
    if !restore.status.success() {
        eprintln!("skipped: Avalonia 12.1.4 could not be restored: {}", String::from_utf8_lossy(&restore.stdout));
        return;
    }

    let problems = host.check(&project, &window, WINDOW).unwrap().expect("an Avalonia project's XAML is read");
    let messages: Vec<_> = problems.iter().map(|p| p.message.as_str()).collect();
    assert_eq!(messages, ["Cannot resolve property 'Foregrund' on Button"], "the misspelt property was not marked");
    assert_eq!(problems[0].span.start, Point::new(4, 12), "the problem is not on the misspelt name");

    let typed = WINDOW.replace("<Button Foregrund=\"Red\" />", "<Butt");
    let list = host.complete(&project, &window, &typed, Point::new(4, 9)).unwrap().expect("a tag's name completes");
    assert_eq!(list.start, Point::new(4, 5), "the list does not replace the name typed");
    assert!(list.items.iter().any(|i| i.label == "Button"), "Avalonia's Button is not offered");

    let card = host.symbol(&project, &window, WINDOW, Point::new(4, 6)).unwrap().expect("a tag names its type");
    assert_eq!(card.description, "class Avalonia.Controls.Button : ContentControl");

    // In a folder of its own: beside Demo.csproj it would read Demo's restore in obj/.
    let plain = folder_path.join("plain").join("Plain.csproj");
    std::fs::create_dir_all(plain.parent().unwrap()).unwrap();
    std::fs::write(&plain, "<Project Sdk=\"Microsoft.NET.Sdk\" />\n").unwrap();
    assert_eq!(host.check(&plain, &window, WINDOW).unwrap(), None, "a project without Avalonia or WPF read as XAML's");
}
