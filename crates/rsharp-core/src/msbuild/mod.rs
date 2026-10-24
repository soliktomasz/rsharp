//! Code help in MSBuild files: what the Host's evaluation knows of a project, MSBuild's schemas, where the caret is in
//! the text as typed, and what to offer, show and go to there.

pub mod context;
pub mod help;
pub mod insight;
pub mod schema;

use std::path::Path;

/// A project file, .props or .targets, whose help is MSBuild's.
pub fn is_msbuild(path: &Path) -> bool {
    let extension = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    extension.ends_with("proj") || matches!(extension.as_str(), "props" | "targets")
}
