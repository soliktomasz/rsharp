//! The system's Trash (the Recycle Bin on Windows, the freedesktop Trash on Linux), through the `trash` crate; or a
//! folder a test sets in its place, so a test never fills the person's.

use std::cell::RefCell;
use std::path::{Path, PathBuf};

thread_local! {
    // shortcut: kept per thread, as each window test runs on a thread of its own; the process's if a delete ever
    // moves off the UI thread.
    static FOLDER: RefCell<Option<PathBuf>> = const { RefCell::new(None) };
}

/// A folder to use as the Trash instead of the system's, or the system's again with none.
pub fn set_folder(folder: Option<PathBuf>) {
    FOLDER.with(|f| *f.borrow_mut() = folder);
}

/// Moves a file or folder to the Trash. A name the Trash already holds is kept: the second takes the time beside it.
pub fn move_to_trash(path: &Path) -> std::io::Result<()> {
    let Some(bin) = FOLDER.with(|f| f.borrow().clone()) else {
        #[allow(unused_mut)]
        let mut context = trash::TrashContext::default();
        // Finder's way runs osascript, outside `process::spawn`, and asks the person to let RSharp control Finder.
        #[cfg(target_os = "macos")]
        {
            use trash::macos::{DeleteMethod, TrashContextExtMacos};
            context.set_delete_method(DeleteMethod::NsFileManager);
        }
        return context.delete(path).map_err(std::io::Error::other);
    };
    std::fs::create_dir_all(&bin)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let mut target = bin.join(&name);
    if target.exists() {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos();
        let (stem, extension) = match name.rsplit_once('.') {
            Some((stem, extension)) if !stem.is_empty() => (stem.to_string(), format!(".{extension}")),
            _ => (name.clone(), String::new()),
        };
        target = bin.join(format!("{stem} {at}{extension}"));
    }
    std::fs::rename(path, target)
}
