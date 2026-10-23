//! Rider's Solution view edits to a .sln and a .slnx, from Passenger's SolutionFoldersTests: each changes only the
//! lines or elements it is about, and undone, the file is as it was, byte for byte.

use rsharp_core::solution_edits::{SolutionEdit, edit, invalid_folder_name};
use std::path::{Path, PathBuf};

const FOLDER_TYPE: &str = "{2150E333-8FDC-42A3-9474-1A3956D46DE8}";

/// As Visual Studio writes a .sln: a blank first line and CRLF (the BOM is the encoding's, not the text's); the folders
/// declared after the project at the top, the Solution Items' files out of order.
const SLN: &str = "\r\nMicrosoft Visual Studio Solution File, Format Version 12.00\r\n# Visual Studio Version 17\r\nVisualStudioVersion = 17.10.35013.160\r\nMinimumVisualStudioVersion = 10.0.40219.1\r\n\
Project(\"{9A19103F-16F7-4668-BE54-9A1E7A4F7556}\") = \"Shop.Web\", \"src\\Shop.Web\\Shop.Web.csproj\", \"{A1000000-0000-0000-0000-000000000001}\"\r\nEndProject\r\n\
Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"Solution Items\", \"Solution Items\", \"{B1000000-0000-0000-0000-000000000001}\"\r\n\
\tProjectSection(SolutionItems) = preProject\r\n\t\tREADME.md = README.md\r\n\t\t.editorconfig = .editorconfig\r\n\tEndProjectSection\r\nEndProject\r\n\
Project(\"{2150E333-8FDC-42A3-9474-1A3956D46DE8}\") = \"src\", \"src\", \"{B1000000-0000-0000-0000-000000000002}\"\r\nEndProject\r\n\
Project(\"{9A19103F-16F7-4668-BE54-9A1E7A4F7556}\") = \"Shop.Core\", \"src\\Shop.Core\\Shop.Core.csproj\", \"{A1000000-0000-0000-0000-000000000002}\"\r\nEndProject\r\n\
Global\r\n\tGlobalSection(SolutionConfigurationPlatforms) = preSolution\r\n\t\tDebug|Any CPU = Debug|Any CPU\r\n\t\tRelease|Any CPU = Release|Any CPU\r\n\tEndGlobalSection\r\n\
\tGlobalSection(ProjectConfigurationPlatforms) = postSolution\r\n\
\t\t{A1000000-0000-0000-0000-000000000001}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\r\n\t\t{A1000000-0000-0000-0000-000000000001}.Debug|Any CPU.Build.0 = Debug|Any CPU\r\n\
\t\t{A1000000-0000-0000-0000-000000000001}.Release|Any CPU.ActiveCfg = Release|Any CPU\r\n\t\t{A1000000-0000-0000-0000-000000000001}.Release|Any CPU.Build.0 = Release|Any CPU\r\n\
\t\t{A1000000-0000-0000-0000-000000000002}.Debug|Any CPU.ActiveCfg = Debug|Any CPU\r\n\t\t{A1000000-0000-0000-0000-000000000002}.Debug|Any CPU.Build.0 = Debug|Any CPU\r\n\
\t\t{A1000000-0000-0000-0000-000000000002}.Release|Any CPU.ActiveCfg = Release|Any CPU\r\n\t\t{A1000000-0000-0000-0000-000000000002}.Release|Any CPU.Build.0 = Release|Any CPU\r\n\
\tEndGlobalSection\r\n\tGlobalSection(SolutionProperties) = preSolution\r\n\t\tHideSolutionNode = FALSE\r\n\tEndGlobalSection\r\n\
\tGlobalSection(NestedProjects) = preSolution\r\n\t\t{A1000000-0000-0000-0000-000000000002} = {B1000000-0000-0000-0000-000000000002}\r\n\tEndGlobalSection\r\n\
\tGlobalSection(ExtensibilityGlobals) = postSolution\r\n\t\tSolutionGuid = {C1000000-0000-0000-0000-000000000001}\r\n\tEndGlobalSection\r\nEndGlobal\r\n";

const SLNX: &str = concat!(
    "<Solution>\n  <Project Path=\"src/Shop.Web/Shop.Web.csproj\" />\n",
    "  <Folder Name=\"/Solution Items/\">\n    <File Path=\"README.md\" />\n    <File Path=\".editorconfig\" />\n  </Folder>\n",
    "  <Folder Name=\"/src/\">\n    <Project Path=\"src/Shop.Core/Shop.Core.csproj\" />\n  </Folder>\n</Solution>\n",
);

/// A folder holding the solution's files, so its paths resolve as they would on disk.
fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rsharp-solution-edits-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    for file in ["tools/Importer/Importer.csproj", "docs/notes.md", "README.md", "src/Shop.Core/Shop.Core.csproj"] {
        let path = folder.join(file);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }
    rsharp_core::paths::canonical(&folder)
}

fn step(solution: &Path, text: &str, change: SolutionEdit) -> String {
    edit(solution, text, change).unwrap_or_else(|why| panic!("the edit was refused: {why}"))
}

/// A folder's id in a .sln, by its name.
fn id(text: &str, name: &str) -> String {
    let line = text.lines().find(|l| l.starts_with(&format!("Project(\"{FOLDER_TYPE}\") = \"{name}\""))).unwrap();
    line.rsplit('"').nth(1).unwrap().to_string()
}

#[test]
fn a_solution_folder_added_renamed_given_a_project_and_a_file_and_removed_changes_only_its_own_lines_in_a_slnx() {
    let root = folder("slnx");
    let solution = root.join("Shop.slnx");
    let text = step(&solution, SLNX, SolutionEdit::AddFolder { parent: None, name: "Tools" });
    assert!(
        text.ends_with("    <Project Path=\"src/Shop.Core/Shop.Core.csproj\" />\n  </Folder>\n  <Folder Name=\"/Tools/\" />\n</Solution>\n"),
        "a folder added would not be written as Visual Studio writes it:\n{text}"
    );
    let text = step(&solution, &text, SolutionEdit::RenameFolder { folder: "/Tools/", name: "Utilities" });
    assert!(text.contains("<Folder Name=\"/Utilities/\" />") && !text.contains("Tools"), "the rename:\n{text}");
    let importer = root.join("tools/Importer/Importer.csproj");
    let text = step(&solution, &text, SolutionEdit::AddProject { folder: Some("/Utilities/"), project: &importer });
    assert!(
        text.contains(
            "  <Folder Name=\"/Utilities/\">\n    <Project Path=\"tools/Importer/Importer.csproj\" />\n  </Folder>\n"
        ),
        "a project added to a folder:\n{text}"
    );
    let notes = root.join("docs/notes.md");
    let text = step(&solution, &text, SolutionEdit::AddFile { folder: "/Utilities/", path: &notes });
    let text = step(&solution, &text, SolutionEdit::AddFolder { parent: Some("/Utilities/"), name: "Scripts" });
    assert!(
        text.contains(
            "  <Folder Name=\"/Utilities/\">\n    <File Path=\"docs/notes.md\" />\n    <Project Path=\"tools/Importer/Importer.csproj\" />\n  </Folder>\n  <Folder Name=\"/Utilities/Scripts/\" />\n</Solution>\n"
        ),
        "a file and a folder added to the folder:\n{text}"
    );
    let text = step(&solution, &text, SolutionEdit::RemoveFolder { folder: "/Utilities/" });
    assert_eq!(text, SLNX, "with the folder removed, the .slnx would not be as it was before it was added");
}

#[test]
fn a_solution_folder_added_renamed_given_a_project_and_a_file_and_removed_changes_only_its_own_lines_in_a_sln() {
    let root = folder("sln");
    let solution = root.join("Shop.sln");
    let text = step(&solution, SLN, SolutionEdit::AddFolder { parent: None, name: "Tools" });
    let tools = text.split('\n').find(|l| l.contains("\"Tools\", \"Tools\"")).expect("no Tools line");
    assert!(
        tools.starts_with(&format!("Project(\"{FOLDER_TYPE}\") = \"Tools\", \"Tools\", \"{{"))
            && tools.ends_with("}\"\r"),
        "a folder added would not be written as Visual Studio writes it: {tools:?}"
    );
    let text = step(&solution, &text, SolutionEdit::RenameFolder { folder: "/Tools/", name: "Utilities" });
    assert!(text.contains("\"Utilities\", \"Utilities\"") && !text.contains("Tools"));
    let importer = root.join("tools/Importer/Importer.csproj");
    let text = step(&solution, &text, SolutionEdit::AddProject { folder: Some("/Utilities/"), project: &importer });
    let line = text
        .lines()
        .find(|l| l.contains("\"Importer\", \"tools\\Importer\\Importer.csproj\", \""))
        .expect("no Importer line");
    let importer_id = line.rsplit('"').nth(1).unwrap().to_string();
    assert!(
        text.contains(&format!("\t\t{importer_id}.Debug|Any CPU.Build.0 = Debug|Any CPU\r\n")),
        "a project not built"
    );
    assert!(text.contains(&format!("\t\t{importer_id}.Release|Any CPU.Build.0 = Release|Any CPU\r\n")));
    assert!(
        text.contains(&format!("\t\t{importer_id} = {}\r\n", id(&text, "Utilities"))),
        "a project not in its folder"
    );
    let notes = root.join("docs/notes.md");
    let text = step(&solution, &text, SolutionEdit::AddFile { folder: "/Utilities/", path: &notes });
    let text = step(&solution, &text, SolutionEdit::AddFolder { parent: Some("/Utilities/"), name: "Scripts" });
    assert!(text.contains(&format!(
        "\"Utilities\", \"Utilities\", \"{}\"\r\n\tProjectSection(SolutionItems) = preProject\r\n\t\tdocs\\notes.md = docs\\notes.md\r\n\tEndProjectSection\r\nEndProject\r\n",
        id(&text, "Utilities")
    )));
    assert!(text.contains(&format!("\t\t{} = {}\r\n", id(&text, "Scripts"), id(&text, "Utilities"))));
    let text = step(&solution, &text, SolutionEdit::RemoveFolder { folder: "/Utilities/" });
    assert_eq!(text, SLN, "with the folder removed, the .sln would not be as it was before it was added");
}

#[test]
fn a_folder_added_to_a_crlf_sln_keeps_its_separators() {
    let root = folder("crlf");
    let text = step(&root.join("Shop.sln"), SLN, SolutionEdit::AddFolder { parent: Some("/src/"), name: "Tools" });
    assert!(!text.replace("\r\n", "").contains('\n'), "a line would end in \\n alone:\n{text:?}");
}

#[test]
fn removing_a_project_takes_its_configurations_and_nesting() {
    let root = folder("remove-project");
    let core = root.join("src/Shop.Core/Shop.Core.csproj");
    let text = step(&root.join("Shop.sln"), SLN, SolutionEdit::RemoveProject { project: &core });
    assert!(!text.contains("A1000000-0000-0000-0000-000000000002"), "a line naming the project would stay:\n{text}");
    assert!(!text.contains("NestedProjects"), "the nesting section left empty would stay:\n{text}");
    let text = step(&root.join("Shop.slnx"), SLNX, SolutionEdit::RemoveProject { project: &core });
    assert!(text.contains("  <Folder Name=\"/src/\" />\n"), "the folder left empty would not close itself:\n{text}");
}

#[test]
fn a_folder_name_with_a_slash_or_dots_is_refused() {
    for name in ["a/b", "..", "  ", "a:b"] {
        assert!(invalid_folder_name(name).is_some(), "{name:?} would be taken as a folder name");
    }
    assert_eq!(invalid_folder_name("Tools"), None);
}

#[test]
fn an_edit_to_a_folder_that_is_not_there_says_so() {
    let root = folder("missing");
    for (solution, text) in [("Shop.sln", SLN), ("Shop.slnx", SLNX)] {
        let refused = edit(&root.join(solution), text, SolutionEdit::RenameFolder { folder: "/Nope/", name: "X" });
        assert!(refused.is_err(), "renaming a folder not there would be taken in {solution}");
        let taken = edit(&root.join(solution), text, SolutionEdit::AddFolder { parent: None, name: "SRC" });
        assert!(taken.is_err(), "a folder whose name is taken would be added again in {solution}");
    }
}
