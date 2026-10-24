//! A project's packages changed as Passenger changes them, through `dotnet add package` and `dotnet remove package`:
//! the .NET command line writes the project, or the Directory.Packages.props under central management, as it would
//! for the person, and its restore uses the folder's feeds.

use crate::templates::{TemplateEnvironment, command};
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// A restore that has to download half a framework takes minutes, not more.
const LIMIT: Duration = Duration::from_secs(600);

/// Adds a package at a version, or moves it to that version when the project has it.
pub fn install(
    environment: &TemplateEnvironment,
    project: &Path,
    id: &str,
    version: &str,
    cancel: &AtomicBool,
    output: &mut dyn FnMut(&str),
) -> Result<(), String> {
    refuse(&[id, version])?;
    run(
        environment,
        project,
        &["add".into(), project.into(), "package".into(), id.into(), "--version".into(), version.into()],
        cancel,
        output,
    )
}

pub fn remove(
    environment: &TemplateEnvironment,
    project: &Path,
    id: &str,
    cancel: &AtomicBool,
    output: &mut dyn FnMut(&str),
) -> Result<(), String> {
    refuse(&[id])?;
    run(environment, project, &["remove".into(), project.into(), "package".into(), id.into()], cancel, output)
}

/// The files a change may write: the project, and every Directory.Packages.props above it, the closest of which
/// holds a centrally managed version.
pub fn touched(project: &Path) -> Vec<PathBuf> {
    let mut files = vec![project.to_path_buf()];
    for folder in project.ancestors().skip(1) {
        let props = folder.join("Directory.Packages.props");
        if props.is_file() {
            files.push(props);
        }
    }
    files
}

/// A word the command would read as an option, whoever gave it, is refused before anything runs.
fn refuse(words: &[&str]) -> Result<(), String> {
    match words.iter().find(|w| w.trim().is_empty() || w.starts_with('-')) {
        Some(word) => Err(format!("'{word}' is not a package id or version")),
        None => Ok(()),
    }
}

fn run(
    environment: &TemplateEnvironment,
    project: &Path,
    arguments: &[OsString],
    cancel: &AtomicBool,
    output: &mut dyn FnMut(&str),
) -> Result<(), String> {
    let folder = project.parent().ok_or("The project has no folder")?;
    let result = command::run(environment.command(arguments, folder), LIMIT, cancel, output)?;
    if result.status.success() { Ok(()) } else { Err(result.said()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_word_read_as_an_option_never_starts_dotnet() {
        let environment = TemplateEnvironment {
            dotnet: "/nonexistent/dotnet".into(),
            cli_home: std::env::temp_dir(),
            hive: None,
            nuget_packages: None,
        };
        let cancel = AtomicBool::new(false);
        let project = Path::new("/tmp/Demo/Demo.csproj");
        let why = install(&environment, project, "--source", "1.0.0", &cancel, &mut |_| {}).unwrap_err();
        assert_eq!(why, "'--source' is not a package id or version");
        assert!(
            install(&environment, project, "Demo", " ", &cancel, &mut |_| {}).unwrap_err().contains("not a package")
        );
        assert!(remove(&environment, project, "-x", &cancel, &mut |_| {}).unwrap_err().contains("not a package"));
    }
}
