//! Rider's Add Reference, Remove of a reference and a file move made to a project file's text (Passenger's
//! ProjectEdits): the rest of the file keeps its bytes, and a reference added then removed leaves it as it was.

use rsharp_core::project_edits::{add_reference, move_items, remove_reference};
use std::path::{Path, PathBuf};

/// As Passenger's SolutionViewTests writes an executable: CRLF and blank lines between its groups (the BOM is the
/// encoding's).
const EXECUTABLE: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\r\n\r\n  <PropertyGroup>\r\n    <OutputType>Exe</OutputType>\r\n    <TargetFramework>net10.0</TargetFramework>\r\n  </PropertyGroup>\r\n\r\n</Project>\r\n";

const LIBRARY: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <PropertyGroup>\n    <TargetFramework>net10.0</TargetFramework>\n  </PropertyGroup>\n</Project>\n";

const WITH_PACKAGES: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <PackageReference Include=\"Humanizer\" Version=\"2.14.1\" />\n  </ItemGroup>\n</Project>\n";

const WITH_REFERENCES: &str = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <ProjectReference Include=\"..\\Data\\Data.csproj\" />\n  </ItemGroup>\n</Project>\n";

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rsharp-project-edits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    for project in ["Shop.App/Shop.App.csproj", "Shop.Core/Shop.Core.csproj", "Data/Data.csproj"] {
        std::fs::create_dir_all(folder.join(project).parent().unwrap()).unwrap();
        std::fs::write(folder.join(project), "").unwrap();
    }
    rsharp_core::paths::canonical(&folder)
}

#[test]
fn a_reference_is_written_as_visual_studio_writes_it() {
    let root = folder("written");
    let (app, core) = (root.join("Shop.App/Shop.App.csproj"), root.join("Shop.Core/Shop.Core.csproj"));
    assert_eq!(
        add_reference(&app, EXECUTABLE, &core).unwrap(),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\r\n\r\n  <PropertyGroup>\r\n    <OutputType>Exe</OutputType>\r\n    <TargetFramework>net10.0</TargetFramework>\r\n  </PropertyGroup>\r\n\r\n\
         \x20 <ItemGroup>\r\n    <ProjectReference Include=\"..\\Shop.Core\\Shop.Core.csproj\" />\r\n  </ItemGroup>\r\n\r\n</Project>\r\n",
        "the reference would not be written as Visual Studio and Passenger write it"
    );
}

#[test]
fn adding_and_removing_a_reference_leaves_the_project_file_as_it_was() {
    let root = folder("round-trip");
    let (app, core) = (root.join("Shop.App/Shop.App.csproj"), root.join("Shop.Core/Shop.Core.csproj"));
    for text in [EXECUTABLE, LIBRARY, WITH_PACKAGES, WITH_REFERENCES] {
        let added = add_reference(&app, text, &core).unwrap();
        assert!(added.contains("<ProjectReference Include=\"..\\Shop.Core\\Shop.Core.csproj\" />"), "{added}");
        assert_eq!(remove_reference(&app, &added, &core).unwrap(), text, "the project would not be as it was");
    }
}

#[test]
fn a_reference_goes_beside_the_projects_others() {
    let root = folder("beside");
    let (app, core) = (root.join("Shop.App/Shop.App.csproj"), root.join("Shop.Core/Shop.Core.csproj"));
    assert_eq!(
        add_reference(&app, WITH_REFERENCES, &core).unwrap(),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup>\n    <ProjectReference Include=\"..\\Data\\Data.csproj\" />\n    \
         <ProjectReference Include=\"..\\Shop.Core\\Shop.Core.csproj\" />\n  </ItemGroup>\n</Project>\n"
    );
}

#[test]
fn adding_a_reference_already_there_says_so() {
    let root = folder("again");
    let (app, data) = (root.join("Shop.App/Shop.App.csproj"), root.join("Data/Data.csproj"));
    assert!(add_reference(&app, WITH_REFERENCES, &data).is_err(), "a reference would be added twice");
    assert!(remove_reference(&app, LIBRARY, &data).is_err(), "a reference not there would be taken as removed");
}

#[test]
fn moving_an_item_named_in_the_project_follows_it() {
    let project = Path::new("/Shop/Shop.csproj");
    let text = "<Project>\n  <ItemGroup>\n    <Compile Update=\"Form.Designer.cs\">\n      <DependentUpon>Form.cs</DependentUpon>\n    </Compile>\n  </ItemGroup>\n</Project>\n";
    let moves = [
        (PathBuf::from("/Shop/Form.cs"), PathBuf::from("/Shop/Sub/Form.cs")),
        (PathBuf::from("/Shop/Form.Designer.cs"), PathBuf::from("/Shop/Sub/Form.Designer.cs")),
    ];
    assert_eq!(
        move_items(project, text, &moves).as_deref(),
        Some(
            "<Project>\n  <ItemGroup>\n    <Compile Update=\"Sub/Form.Designer.cs\">\n      <DependentUpon>Form.cs</DependentUpon>\n    </Compile>\n  </ItemGroup>\n</Project>\n"
        ),
        "the project would lose track of the files moved"
    );
    let moved_parent = [(PathBuf::from("/Shop/Form.cs"), PathBuf::from("/Shop/Other.cs"))];
    assert_eq!(
        move_items(project, text, &moved_parent).as_deref(),
        Some(
            "<Project>\n  <ItemGroup>\n    <Compile Update=\"Form.Designer.cs\">\n      <DependentUpon>Other.cs</DependentUpon>\n    </Compile>\n  </ItemGroup>\n</Project>\n"
        ),
        "DependentUpon would not follow its file"
    );
}

#[test]
fn moving_a_file_the_project_holds_by_glob_changes_nothing() {
    let project = Path::new("/Shop/Shop.csproj");
    let text = "<Project>\n  <ItemGroup>\n    <Compile Include=\"**/*.cs\" />\n  </ItemGroup>\n</Project>\n";
    let moves = [(PathBuf::from("/Shop/A.cs"), PathBuf::from("/Shop/B.cs"))];
    assert_eq!(move_items(project, text, &moves), None, "a glob would be rewritten");
}

#[test]
fn a_reference_beside_others_on_one_line_goes_inside_their_group() {
    let root = folder("one-line-refs");
    let (app, core) = (root.join("Shop.App/Shop.App.csproj"), root.join("Shop.Core/Shop.Core.csproj"));
    let text = "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup><ProjectReference Include=\"..\\Data\\Data.csproj\" /></ItemGroup>\n</Project>\n";
    assert_eq!(
        add_reference(&app, text, &core).unwrap(),
        "<Project Sdk=\"Microsoft.NET.Sdk\">\n  <ItemGroup><ProjectReference Include=\"..\\Data\\Data.csproj\" /><ProjectReference Include=\"..\\Shop.Core\\Shop.Core.csproj\" /></ItemGroup>\n</Project>\n",
        "a reference would land outside its group, or copy what stood before it on its line"
    );
}

#[test]
fn a_reference_to_a_project_written_on_one_line_goes_inside_it() {
    let root = folder("one-line-project");
    let (app, core) = (root.join("Shop.App/Shop.App.csproj"), root.join("Shop.Core/Shop.Core.csproj"));
    let added = add_reference(&app, "<Project Sdk=\"Microsoft.NET.Sdk\"></Project>\n", &core).unwrap();
    assert!(added.starts_with("<Project Sdk=\"Microsoft.NET.Sdk\">"), "the group would go before <Project>: {added:?}");
    assert!(added.ends_with("</ItemGroup>\n</Project>\n"), "the group would not go inside the project: {added:?}");
    assert_eq!(remove_reference(&app, &added, &core).unwrap().matches("<Project").count(), 1);
}
