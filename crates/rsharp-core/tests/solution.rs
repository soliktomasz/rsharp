use rsharp_core::solution::{self, SolutionEntry};
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rsharp-solution-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("src/App")).unwrap();
    std::fs::write(folder.join("src/App/App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
    std::fs::write(folder.join("README.md"), "# Shop").unwrap();
    rsharp_core::paths::canonical(&folder)
}

/// The tree as names: a folder as `name/`, with its path in the solution, its children indented under it.
fn shape(entries: &[SolutionEntry], depth: usize, out: &mut Vec<String>) {
    for entry in entries {
        let indent = "  ".repeat(depth);
        match entry {
            SolutionEntry::Folder { name, path, children } => {
                out.push(format!("{indent}{name}/ {path}"));
                shape(children, depth + 1, out);
            }
            SolutionEntry::Project(project) => out.push(format!("{indent}{} {}", project.name, project.exists)),
            SolutionEntry::Item { name, exists, .. } => out.push(format!("{indent}{name} {exists}")),
        }
    }
}

fn read(path: &Path) -> Vec<String> {
    let solution = solution::read(path).expect("the solution did not read");
    let mut out = vec![solution.name.clone()];
    shape(&solution.entries, 1, &mut out);
    out
}

const EXPECTED: &[&str] = &["Shop", "  src/ /src/", "    App true", "    tools/ /src/tools/", "      README.md true"];

#[test]
fn a_slnx_reads_its_folders_files_and_projects_nested_by_their_paths() {
    let folder = folder("slnx");
    let path = folder.join("Shop.slnx");
    std::fs::write(
        &path,
        "<Solution>\n  <Folder Name=\"/src/\">\n    <Project Path=\"src/App/App.csproj\" />\n  </Folder>\n  \
         <Folder Name=\"/src/tools/\">\n    <File Path=\"README.md\" />\n  </Folder>\n</Solution>\n",
    )
    .unwrap();
    assert_eq!(read(&path), EXPECTED, "the Solution view would not show the .slnx's folders as it nests them");
    let solution = solution::read(&path).unwrap();
    assert_eq!(
        solution.projects().iter().map(|p| p.path.clone()).collect::<Vec<_>>(),
        [folder.join("src/App/App.csproj")],
        "the project's path would not be the file on disk"
    );
}

#[test]
fn a_sln_nests_projects_and_solution_items_by_nested_projects() {
    let folder = folder("sln");
    let path = folder.join("Shop.sln");
    let text = "\u{feff}\r\nMicrosoft Visual Studio Solution File, Format Version 12.00\r\n\
        Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"src\", \"src\", \"{AAAAAAAA-0000-0000-0000-000000000001}\"\r\n\
        EndProject\r\n\
        Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"tools\", \"tools\", \"{AAAAAAAA-0000-0000-0000-000000000002}\"\r\n\
        \tProjectSection(SolutionItems) = preProject\r\n\t\tREADME.md = README.md\r\n\tEndProjectSection\r\n\
        EndProject\r\n\
        Project(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"App\", \"src\\App\\App.csproj\", \"{BBBBBBBB-0000-0000-0000-000000000001}\"\r\n\
        EndProject\r\n\
        Global\r\n\tGlobalSection(NestedProjects) = preSolution\r\n\
        \t\t{bbbbbbbb-0000-0000-0000-000000000001} = {aaaaaaaa-0000-0000-0000-000000000001}\r\n\
        \t\t{AAAAAAAA-0000-0000-0000-000000000002} = {AAAAAAAA-0000-0000-0000-000000000001}\r\n\
        \tEndGlobalSection\r\nEndGlobal\r\n";
    std::fs::write(&path, text).unwrap();
    // The .sln declares tools after src, and App last: the order within a level is the file's.
    let mut expected: Vec<String> = EXPECTED.iter().map(|s| s.to_string()).collect();
    expected.swap(2, 3);
    expected.swap(3, 4);
    assert_eq!(
        read(&path),
        expected,
        "the Solution view would not show the .sln's folders as NestedProjects nests them"
    );
}

#[test]
fn a_missing_project_is_kept_and_marked() {
    let folder = folder("missing");
    let path = folder.join("Shop.slnx");
    std::fs::write(&path, "<Solution><Project Path=\"gone/Gone.csproj\" /></Solution>").unwrap();
    assert_eq!(read(&path), ["Shop", "  Gone false"], "a project not on disk would vanish from the Solution view");
}

#[test]
fn find_prefers_the_solution_named_for_the_folder_then_slnx() {
    let folder = std::env::temp_dir().join(format!("rsharp-solution-find-{}", std::process::id())).join("Shop");
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(&folder).unwrap();
    for name in ["A.slnx", "Shop.sln", "Shop.slnx"] {
        std::fs::write(folder.join(name), "").unwrap();
    }
    assert_eq!(
        solution::find(&folder).and_then(|p| p.file_name().map(|n| n.to_string_lossy().into_owned())),
        Some("Shop.slnx".into()),
        "the folder would open another solution than the one named for it"
    );
}

#[test]
fn an_empty_folder_holds_nothing_that_follows_it() {
    let folder = folder("empty-folder");
    let path = folder.join("Shop.slnx");
    std::fs::write(
        &path,
        "<Solution>\n  <Folder Name=\"/build/\" />\n  <Project Path=\"src/App/App.csproj\" />\n</Solution>\n",
    )
    .unwrap();
    assert_eq!(
        read(&path),
        ["Shop", "  build/ /build/", "  App true"],
        "a project after an empty folder would go into it"
    );
}

#[test]
fn a_windows_1252_sln_reads() {
    let folder = folder("1252");
    let path = folder.join("Shop.sln");
    let text = "# Caf\u{e9}\r\nProject(\"{FAE04EC0-301F-11D3-BF4B-00C04F79EFBC}\") = \"App\", \"src\\App\\App.csproj\", \"{BBBBBBBB-0000-0000-0000-000000000001}\"\r\nEndProject\r\n";
    std::fs::write(&path, encoding_rs::WINDOWS_1252.encode(text).0).unwrap();
    assert_eq!(read(&path), ["Shop", "  App true"], "a .sln in the system's code page would not read");
}

#[test]
fn a_file_that_is_no_solution_reads_as_none() {
    let folder = folder("not-a-solution");
    assert!(
        solution::read(&folder.join("src/App/App.csproj")).is_none(),
        "a project file would read as an empty solution"
    );
}
