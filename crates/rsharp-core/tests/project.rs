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

fn stamped(name: &str) -> (PathBuf, EvaluatedProject) {
    let root = folder(name).join("App");
    std::fs::write(root.join("App.csproj"), "<Project />").unwrap();
    let mut project = project(&root, vec![]);
    project.inputs = vec![rsharp_core::project::Stamp::file(&root.join("App.csproj"))];
    project.folders = vec![rsharp_core::project::Stamp::folder(&root)];
    (root, project)
}

#[test]
fn a_fresh_evaluation_is_not_stale() {
    let (_, project) = stamped("fresh");
    assert!(!project.is_stale(), "a project would be evaluated again with nothing changed");
}

#[test]
fn a_project_is_stale_once_its_project_file_changes() {
    let (root, project) = stamped("changed");
    std::fs::write(root.join("App.csproj"), "<Project Sdk=\"Microsoft.NET.Sdk\" />").unwrap();
    assert!(project.is_stale(), "an edited project file would keep showing what it said before");
}

#[test]
fn a_project_is_stale_once_a_walked_folder_gains_a_file() {
    let (root, project) = stamped("gained");
    std::fs::write(root.join("New.cs"), "").unwrap();
    assert!(project.is_stale(), "a file added outside the IDE would not show");
}

#[test]
fn a_folder_stamp_is_the_hosts_own() {
    // Passenger.Host's Stamps.Folder over "a.cs" and "Models/": FNV-1a over each name's UTF-16 and a newline.
    let root = folder("hash").join("App");
    std::fs::write(root.join("a.cs"), "").unwrap();
    std::fs::create_dir_all(root.join("Models")).unwrap();
    std::fs::create_dir_all(root.join("bin")).unwrap();
    let mut hash: u64 = 14695981039346656037;
    for name in ["Models/", "a.cs"] {
        for c in name.encode_utf16().chain(['\n' as u16]) {
            hash = (hash ^ c as u64).wrapping_mul(1099511628211);
        }
    }
    let stamp = rsharp_core::project::Stamp::folder(&root);
    assert_eq!((stamp.ticks, stamp.length), (hash as i64, 2), "the Host's cache would always read as stale");
}

#[test]
fn a_broken_cache_reads_as_none() {
    let base = folder("cache");
    let store = rsharp_core::state::StateStore::open(base.join("state"));
    let kept = store.folder(&base);
    std::fs::create_dir_all(&kept).unwrap();
    std::fs::write(kept.join("projectModels.json"), r#"{"projects": 7}"#).unwrap();
    let read: rsharp_core::project::Evaluations = store.read_for(&base);
    assert!(read.projects.is_empty(), "a broken cache would be taken as evaluations");
    assert!(kept.join("projectModels.json.bad").exists(), "a broken cache would be read again every start");
}
