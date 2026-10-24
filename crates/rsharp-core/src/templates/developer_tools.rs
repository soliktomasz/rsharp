//! Removes only Passenger's Avalonia Accelerate package/call lines; every other byte stays as generated.

use regex::Regex;
use std::path::{Path, PathBuf};
use std::sync::LazyLock;

pub fn take_out(folder: &Path) -> Result<Vec<PathBuf>, String> {
    static PACKAGE: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r#"(?mis)^[ \t]*<Package(?:Reference|Version)\b[^>]*\bInclude="AvaloniaUI\.DiagnosticsSupport"[^>]*?(?:/>|>.*?</Package(?:Reference|Version)>)[ \t]*(?:\r?\n)?"#).unwrap()
    });
    static DEBUG: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^[ \t]*#if DEBUG[ \t]*\r?\n(?:[ \t]*[^\r\n]*\b(?:WithDeveloperTools|AttachDeveloperTools)\(\)[^\r\n]*\r?\n)+[ \t]*#endif[ \t]*(?:\r?\n)?").unwrap()
    });
    static CALL: LazyLock<Regex> = LazyLock::new(|| {
        Regex::new(r"(?m)^[ \t]*(?:\.WithDeveloperTools\(\)|this\.AttachDeveloperTools\(\)(?:[ \t]*\|>[ \t]*ignore)?;?)[ \t]*(?:\r?\n)?").unwrap()
    });
    let mut files = Vec::new();
    super::create::walk(folder, false, &mut files)?;
    let mut changed = Vec::new();
    for file in files {
        let extension = file.extension().unwrap_or_default().to_string_lossy().to_ascii_lowercase();
        let edits: &[&Regex] = match extension.as_str() {
            "csproj" | "fsproj" | "vbproj" | "props" | "targets" => &[&PACKAGE],
            "cs" | "fs" | "vb" => &[&DEBUG, &CALL],
            _ => continue,
        };
        let bytes = std::fs::read(&file).map_err(|e| format!("{}: {e}", file.display()))?;
        let (text, charset) = crate::encodings::decode(&bytes, None);
        let mut edited = text.clone();
        for edit in edits {
            edited = edit.replace_all(&edited, "").into_owned();
        }
        if edited != text {
            let bytes = crate::encodings::encode(&edited, charset)
                .map_err(|c| format!("{}: {c} cannot be written back", file.display()))?;
            crate::files::write(&file, &bytes).map_err(|e| format!("{}: {e}", file.display()))?;
            changed.push(file);
        }
    }
    Ok(changed)
}
