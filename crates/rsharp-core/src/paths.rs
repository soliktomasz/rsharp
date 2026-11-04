use std::path::{Path, PathBuf};

/// The path with symlinks resolved, as MSBuild and Roslyn report paths; a path compared with another must come
/// from here too. A path that does not exist yet is returned as given.
pub fn canonical(path: &Path) -> PathBuf {
    let resolved = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    // Windows' canonical form is a verbatim path (`\\?\C:\…`), which neither Roslyn nor MSBuild report.
    match resolved.to_str().and_then(|p| p.strip_prefix(r"\\?\")) {
        Some(plain) if plain.as_bytes().get(1) == Some(&b':') => PathBuf::from(plain),
        _ => resolved,
    }
}

/// Whether `path` is `folder` or inside it, both canonical.
pub fn is_under(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder)
}

/// Whether two paths name the same file, both canonical: a solution's project and a model's are written the same way,
/// but a person's configuration is not necessarily.
pub fn same(one: &Path, other: &Path) -> bool {
    one == other || canonical(one) == canonical(other)
}

/// `relative` under `base`, or empty when `base` is: a build that carried nothing must not name a path relative to
/// wherever the app was started.
pub fn under(base: &str, relative: &str) -> PathBuf {
    if base.is_empty() { PathBuf::new() } else { Path::new(base).join(relative) }
}

/// A file or folder RSharp carries (a sidecar, a server, a schema): `sidecars/<relative>` beside the executable, as a
/// build has them and as a folder's copy would; else `Resources/sidecars/<relative>` of the app bundle the executable
/// is in, the only place a sealed bundle lets data be; else `built`, where this build put it, compiled in — a folder
/// that is on the machine that built it alone. `None` when none is there.
pub fn carried(relative: &str, built: &Path) -> Option<PathBuf> {
    carried_in(std::env::current_exe().ok()?.parent()?, relative, built)
}

/// [`carried`] from `exe_dir`, the executable's folder (the app bundle's `Contents/MacOS`, a build's `target/<profile>`).
fn carried_in(exe_dir: &Path, relative: &str, built: &Path) -> Option<PathBuf> {
    let bundle = exe_dir.parent().map(|contents| contents.join("Resources/sidecars"));
    for sidecars in [Some(exe_dir.join("sidecars")), bundle].into_iter().flatten() {
        let path = sidecars.join(relative);
        if path.exists() {
            return Some(path);
        }
    }
    (!built.as_os_str().is_empty() && built.exists()).then(|| built.to_path_buf())
}

/// A `file://` URI for an absolute path, as the language server names documents.
pub fn to_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    // `file:///C:/…`: a drive's path takes the slash a Unix path starts with.
    if !path.to_string_lossy().starts_with('/') {
        uri.push('/');
    }
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' | b':' => uri.push(byte as char),
            b'\\' => uri.push('/'),
            _ => uri.push_str(&format!("%{byte:02X}")),
        }
    }
    uri
}

/// The path a `file://` URI names, or `None` for any other scheme.
pub fn from_uri(uri: &str) -> Option<PathBuf> {
    let rest = uri.strip_prefix("file://")?;
    let mut bytes = Vec::with_capacity(rest.len());
    let mut iter = rest.bytes();
    while let Some(byte) = iter.next() {
        if byte == b'%' {
            let hex = [iter.next()?, iter.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(byte);
        }
    }
    let path = String::from_utf8(bytes).ok()?;
    // `/C:/…` back to `C:/…`.
    let path = match path.as_bytes() {
        [b'/', drive, b':', ..] if drive.is_ascii_alphabetic() => path[1..].to_string(),
        _ => path,
    };
    Some(PathBuf::from(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_sidecar_that_is_neither_beside_the_executable_nor_built_is_none() {
        assert_eq!(carried("nothing/is/here", Path::new("")), None);
        assert_eq!(carried("nothing/is/here", Path::new("/nowhere/at/all")), None);
    }

    #[test]
    fn a_sidecar_is_beside_the_executable_first_then_in_the_bundles_resources_then_where_it_was_built() {
        let root = std::env::temp_dir().join(format!("rsharp-carried-{}", std::process::id()));
        let app = root.join("RSharp.app/Contents");
        let macos = app.join("MacOS");
        let in_resources = app.join("Resources/sidecars/web");
        let beside = macos.join("sidecars/web");
        std::fs::create_dir_all(&in_resources).unwrap();
        assert_eq!(
            carried_in(&macos, "web", Path::new("/nowhere")),
            Some(in_resources),
            "a shipped app's sidecars are found in its Resources: without them web files keep their colours only"
        );
        std::fs::create_dir_all(&beside).unwrap();
        assert_eq!(
            carried_in(&macos, "web", Path::new("/nowhere")),
            Some(beside),
            "a copy beside the executable is nearer, as a build and a folder's distribution have them"
        );
        let built = root.join("built/web");
        std::fs::create_dir_all(&built).unwrap();
        assert_eq!(
            carried_in(&root, "web", &built),
            Some(built),
            "a folder with neither runs the sidecars the build made"
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_path_with_spaces_and_symbols_survives_a_round_trip_through_a_uri() {
        let path = Path::new("/Users/me/My Projects/C# app/Program.cs");
        let uri = to_uri(path);
        assert_eq!(uri, "file:///Users/me/My%20Projects/C%23%20app/Program.cs");
        assert_eq!(from_uri(&uri).as_deref(), Some(path));
    }

    #[test]
    fn a_windows_path_becomes_the_uri_roslyn_reports() {
        let uri = to_uri(Path::new(r"C:\Users\me\App\Program.cs"));
        assert_eq!(uri, "file:///C:/Users/me/App/Program.cs");
        assert_eq!(from_uri(&uri).as_deref(), Some(Path::new("C:/Users/me/App/Program.cs")));
    }
}
