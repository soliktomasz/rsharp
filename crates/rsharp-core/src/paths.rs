use std::path::{Path, PathBuf};

/// The path with symlinks resolved, as MSBuild and Roslyn report paths; a path compared with another must come
/// from here too. A path that does not exist yet is returned as given.
pub fn canonical(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// Whether `path` is `folder` or inside it, both canonical.
pub fn is_under(path: &Path, folder: &Path) -> bool {
    path.starts_with(folder)
}

/// A `file://` URI for an absolute path, as the language server names documents.
pub fn to_uri(path: &Path) -> String {
    let mut uri = String::from("file://");
    for byte in path.to_string_lossy().bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' | b'/' => uri.push(byte as char),
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
    Some(PathBuf::from(String::from_utf8(bytes).ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_path_with_spaces_and_symbols_survives_a_round_trip_through_a_uri() {
        let path = Path::new("/Users/me/My Projects/C# app/Program.cs");
        let uri = to_uri(path);
        assert_eq!(uri, "file:///Users/me/My%20Projects/C%23%20app/Program.cs");
        assert_eq!(from_uri(&uri).as_deref(), Some(path));
    }
}
