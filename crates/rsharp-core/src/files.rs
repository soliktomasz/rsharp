use std::io;
use std::path::Path;

/// Writes a workspace file as every save must: to a temporary file beside it, then renamed over it, so a crash
/// mid-write never leaves half a file, its permissions kept and a symlink written through. A read-only file is
/// refused, which the rename would otherwise replace.
pub fn write(path: &Path, contents: &[u8]) -> io::Result<()> {
    let target = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    if std::fs::metadata(&target).is_ok_and(|m| m.permissions().readonly()) {
        return Err(io::Error::new(io::ErrorKind::PermissionDenied, format!("{} is read-only", target.display())));
    }
    let name = target.file_name().ok_or_else(|| io::Error::other("a file to write has a name"))?;
    let temporary = target.with_file_name(format!(".{}.rsharp-save", name.to_string_lossy()));
    std::fs::write(&temporary, contents)?;
    if let Ok(metadata) = std::fs::metadata(&target) {
        std::fs::set_permissions(&temporary, metadata.permissions())?;
    }
    std::fs::rename(&temporary, &target).inspect_err(|_| {
        let _ = std::fs::remove_file(&temporary);
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_save_replaces_the_file_whole_and_leaves_nothing_beside_it() {
        let folder = std::env::temp_dir().join(format!("rsharp-files-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("Program.cs");
        std::fs::write(&file, "old text that is longer").unwrap();
        super::write(&file, b"new").unwrap();
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new");
        assert_eq!(std::fs::read_dir(&folder).unwrap().count(), 1, "the temporary file was left behind");
        std::fs::remove_dir_all(&folder).unwrap();
    }

    #[test]
    fn a_read_only_file_is_refused_not_replaced() {
        let folder = std::env::temp_dir().join(format!("rsharp-files-ro-{}", std::process::id()));
        std::fs::create_dir_all(&folder).unwrap();
        let file = folder.join("Locked.cs");
        std::fs::write(&file, "kept").unwrap();
        let mut permissions = std::fs::metadata(&file).unwrap().permissions();
        permissions.set_readonly(true);
        std::fs::set_permissions(&file, permissions).unwrap();
        assert!(super::write(&file, b"new").is_err(), "a read-only file was written over");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "kept");
        let mut permissions = std::fs::metadata(&file).unwrap().permissions();
        #[allow(clippy::permissions_set_readonly_false)]
        permissions.set_readonly(false);
        std::fs::set_permissions(&file, permissions).unwrap();
        std::fs::remove_dir_all(&folder).unwrap();
    }
}
