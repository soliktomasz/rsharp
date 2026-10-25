use std::path::{Path, PathBuf};

/// Finds a program as the login shell would: on PATH, then in the places its installers put it, which a GUI app
/// started from the Dock does not have on its PATH.
pub fn find(name: &str, also: &[PathBuf]) -> Option<PathBuf> {
    let exe = program_name(name);
    std::env::var_os("PATH")
        .into_iter()
        .flat_map(|paths| std::env::split_paths(&paths).collect::<Vec<_>>())
        .chain(also.iter().cloned())
        .map(|dir| dir.join(&exe))
        .find(|candidate| is_program(candidate))
}

/// The file a program's name is looked for as. On Windows that is a `.exe` unless the name already says which kind
/// of program it is — `cmd.exe` must not become `cmd.exe.exe`, while a dotted name such as a server's still needs
/// its extension.
fn program_name(name: &str) -> String {
    let said = Path::new(name)
        .extension()
        .is_some_and(|extension| ["exe", "com", "bat", "cmd"].iter().any(|kind| extension.eq_ignore_ascii_case(kind)));
    if cfg!(windows) && !said { format!("{name}.exe") } else { name.to_string() }
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

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn a_program_is_looked_for_under_the_name_windows_gives_it() {
        // `--agent cmd.exe` names a program on PATH; searching for `cmd.exe.exe` would never find it.
        assert_eq!(program_name("cmd.exe"), "cmd.exe");
        assert_eq!(program_name("CMD.EXE"), "CMD.EXE", "a name's own capitals are the person's");
        assert_eq!(program_name("pwsh"), "pwsh.exe");
        // A dotted name that names no kind of program still needs its extension.
        assert_eq!(program_name("Microsoft.CodeAnalysis.LanguageServer"), "Microsoft.CodeAnalysis.LanguageServer.exe");
        assert!(find("cmd.exe", &[]).is_some(), "cmd.exe is on PATH and was not found");
    }
}
