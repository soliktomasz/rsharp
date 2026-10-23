use rsharp_core::trash;

#[test]
fn a_file_moved_to_the_set_folder_leaves_its_place() {
    let base = std::env::temp_dir().join(format!("rsharp-trash-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("Lib")).unwrap();
    std::fs::write(base.join("Lib/Lib.csproj"), "<Project />").unwrap();
    std::fs::write(base.join("notes.md"), "a").unwrap();
    trash::set_folder(Some(base.join("Trash")));
    trash::move_to_trash(&base.join("Lib")).unwrap();
    trash::move_to_trash(&base.join("notes.md")).unwrap();
    std::fs::write(base.join("notes.md"), "b").unwrap();
    trash::move_to_trash(&base.join("notes.md")).unwrap();
    trash::set_folder(None);
    assert!(!base.join("Lib").exists() && !base.join("notes.md").exists(), "what was trashed would stay");
    assert!(base.join("Trash/Lib/Lib.csproj").is_file(), "the folder would not be in the Trash");
    let kept = std::fs::read_dir(base.join("Trash")).unwrap().count();
    assert_eq!(kept, 3, "a second file of a name would replace the first in the Trash");
}
