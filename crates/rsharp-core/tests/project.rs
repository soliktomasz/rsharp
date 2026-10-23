use rsharp_core::project::{EvaluatedItem, EvaluatedProject, ProjectFiles};
use std::path::{Path, PathBuf};

fn folder(name: &str) -> PathBuf {
    let folder = std::env::temp_dir().join(format!("rsharp-project-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&folder);
    std::fs::create_dir_all(folder.join("App")).unwrap();
    rsharp_core::paths::canonical(&folder)
}

fn item(kind: &str, path: &Path, link: Option<&str>) -> EvaluatedItem {
    EvaluatedItem { r#type: kind.into(), path: path.to_path_buf(), link: link.map(Into::into), dependent_upon: None }
}

fn project(root: &Path, items: Vec<EvaluatedItem>) -> EvaluatedProject {
    let mut project: EvaluatedProject = serde_json::from_str(r#"{"path": ""}"#).unwrap();
    project.path = root.join("App.csproj");
    project.items = items;
    project
}

fn names(files: &[(String, PathBuf)]) -> Vec<&str> {
    files.iter().map(|(name, _)| name.as_str()).collect()
}

#[test]
fn an_evaluated_project_reads_from_the_hosts_json() {
    let line = r#"{"path":"/s/App/App.csproj","outputType":"Exe","targetFrameworks":["net10.0"],
        "dependencies":[{"framework":null,"packages":[{"name":"Humanizer","version":"2.14.1","implicit":false}],
        "projects":["/s/Core/Core.csproj"],"frameworkReferences":["Microsoft.NETCore.App"]}],
        "items":[{"type":"Compile","path":"/s/App/Program.cs"}],"inputs":[{"path":"/s/App/App.csproj","ticks":5,"length":9}],
        "somethingNew":1}"#;
    let project: EvaluatedProject = serde_json::from_str(line).expect("the Host's evaluation did not read");
    assert!(project.complete, "an evaluation that does not say would be taken as incomplete");
    assert_eq!(project.output_type.as_deref(), Some("Exe"));
    assert_eq!(project.dependencies[0].packages[0].version.as_deref(), Some("2.14.1"));
    assert_eq!(project.dependencies[0].projects, [PathBuf::from("/s/Core/Core.csproj")]);
    assert_eq!(project.items[0].r#type, "Compile");
    assert!(!project.inputs[0].folder);
}

#[test]
fn a_projects_files_show_in_their_folders_and_a_linked_one_at_its_link() {
    let root = folder("placed").join("App");
    let shared = root.parent().unwrap().join("Shared");
    let project = project(
        &root,
        vec![
            item("Compile", &root.join("Program.cs"), None),
            item("Compile", &root.join("Models/Order.cs"), None),
            item("Compile", &shared.join("Version.cs"), Some("Properties\\Version.cs")),
            item("None", &root.parent().unwrap().join("README.md"), None),
            item("Folder", &root.join("Empty/"), None),
        ],
    );
    let files = ProjectFiles::new(&project);
    assert_eq!(files.folders_in(""), ["Empty", "Models", "Properties"], "the project's folders would not show");
    assert_eq!(
        names(&files.files_in("")),
        ["Program.cs", "README.md"],
        "a file from outside would not show at the top"
    );
    assert_eq!(
        files.files_in("Properties"),
        [("Version.cs".to_string(), shared.join("Version.cs"))],
        "a linked file would not show at its Link"
    );
    assert_eq!(names(&files.files_in("Models")), ["Order.cs"]);
}

#[test]
fn a_file_listed_twice_shows_once() {
    let root = folder("twice").join("App");
    let project = project(
        &root,
        vec![item("Compile", &root.join("Program.cs"), None), item("None", &root.join("Program.cs"), None)],
    );
    assert_eq!(names(&ProjectFiles::new(&project).files_in("")), ["Program.cs"], "a file would show twice");
}

#[test]
fn an_empty_folder_on_disk_shows_and_bin_obj_and_dot_folders_do_not() {
    let root = folder("empty").join("App");
    for made in ["Drafts", "bin", "obj", ".vs", "Full"] {
        std::fs::create_dir_all(root.join(made)).unwrap();
    }
    // A folder holding a file the project leaves out is not empty, so it does not show.
    std::fs::write(root.join("Full/Left.txt"), "").unwrap();
    let project = project(&root, vec![item("Compile", &root.join("Program.cs"), None)]);
    assert_eq!(ProjectFiles::new(&project).folders_in(""), ["Drafts"], "a folder just made would vanish");
}

#[test]
fn ds_store_and_thumbs_db_never_show() {
    let root = folder("hidden").join("App");
    let project = project(
        &root,
        vec![
            item("None", &root.join(".DS_Store"), None),
            item("None", &root.join("Thumbs.db"), None),
            item("None", &root.join("notes.md"), None),
        ],
    );
    assert_eq!(names(&ProjectFiles::new(&project).files_in("")), ["notes.md"], "the system's own files would show");
}
