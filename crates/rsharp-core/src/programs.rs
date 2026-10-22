use std::path::{Path, PathBuf};

/// Finds a program as the login shell would: on PATH, then in the places its installers put it, which a GUI app
/// started from the Dock does not have on its PATH.
pub fn find(name: &str, also: &[PathBuf]) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{name}.exe") } else { name.to_string() };
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .chain(also.iter().cloned())
        .map(|dir| dir.join(&exe))
        .find(|candidate| is_program(candidate))
}

/// The `dotnet` the .NET installers put in place.
pub fn dotnet() -> Option<PathBuf> {
    let mut also = vec![home().join(".dotnet")];
    if cfg!(target_os = "macos") {
        also.push("/usr/local/share/dotnet".into());
    } else if cfg!(windows) {
        also.push(r"C:\Program Files\dotnet".into());
    } else {
        also.extend(["/usr/share/dotnet".into(), "/usr/lib/dotnet".into()]);
    }
    find("dotnet", &also)
}

/// The person's home folder.
pub fn home() -> PathBuf {
    std::env::var_os(if cfg!(windows) { "USERPROFILE" } else { "HOME" }).map(PathBuf::from).unwrap_or_default()
}

fn is_program(path: &Path) -> bool {
    path.is_file()
}
