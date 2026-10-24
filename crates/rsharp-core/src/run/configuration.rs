//! A run configuration as Rider's are: a project (and the framework of it), the program's arguments, environment and
//! working directory, whether the project builds first, and the browser a web project opens. Data alone: what it
//! launches is `plan`'s to work out. The folder's are made from its executable projects and their launch profiles.

use super::profile::LaunchProfile;
use crate::project::EvaluatedProject;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct RunConfiguration {
    pub name: String,
    /// The project's file.
    pub project: PathBuf,
    /// The target framework run, for a project with several; none for its first.
    pub framework: Option<String>,
    /// The launch profile its values came from, when one did.
    pub launch_profile: Option<String>,
    /// A program run in place of the project's own (an `Executable` profile's); none for the project's.
    pub program: Option<String>,
    /// The program's arguments as one command line, split as `command_line::split` splits it.
    pub arguments: String,
    pub environment: BTreeMap<String, String>,
    /// Where the program runs, relative to the project's folder; none for its output folder.
    pub working_directory: Option<String>,
    pub build_before_launch: bool,
    pub launch_browser: bool,
    /// Where the browser opens: a profile's `launchUrl` against its first `applicationUrl`.
    pub browser_url: String,
}

impl Default for RunConfiguration {
    fn default() -> Self {
        Self {
            name: String::new(),
            project: PathBuf::new(),
            framework: None,
            launch_profile: None,
            program: None,
            arguments: String::new(),
            environment: BTreeMap::new(),
            working_directory: None,
            build_before_launch: true,
            launch_browser: false,
            browser_url: String::new(),
        }
    }
}

impl RunConfiguration {
    /// The configurations a folder's projects make: one per launch profile, named "Project: profile" as Rider names
    /// them, and the project itself when none of its profiles runs here. A project no evaluation has reached yet is
    /// listed too, since its launch profiles say what runs and its plan refuses a library when the time comes.
    pub fn generate(projects: &[EvaluatedProject]) -> Vec<RunConfiguration> {
        let mut generated = Vec::new();
        for project in projects.iter().filter(|project| project.problem.is_none()) {
            let profiles = LaunchProfile::read(&project_folder(&project.path));
            let runs_here = profiles.iter().any(|profile| profile.unsupported().is_none());
            let understood = project.output_type.is_some();
            if (understood && !project.is_runnable()) || (!understood && !profiles.is_empty() && !runs_here) {
                continue;
            }
            if !runs_here {
                generated.push(RunConfiguration {
                    name: project_name(&project.path),
                    project: project.path.clone(),
                    framework: project.target_frameworks.first().cloned(),
                    ..RunConfiguration::default()
                });
            }
            generated.extend(profiles.iter().map(|profile| Self::from_profile(project, profile)));
        }
        generated
    }

    /// A configuration with a launch profile's values: what generating one makes, and what choosing a profile fills in.
    pub fn from_profile(project: &EvaluatedProject, profile: &LaunchProfile) -> RunConfiguration {
        RunConfiguration {
            name: format!("{}: {}", project_name(&project.path), profile.name),
            project: project.path.clone(),
            framework: project.target_frameworks.first().cloned(),
            launch_profile: Some(profile.name.clone()),
            program: profile.executable_path.clone().filter(|path| !path.is_empty()),
            arguments: profile.command_line_args.clone().unwrap_or_default(),
            environment: profile.environment(),
            working_directory: profile.working_directory.clone().filter(|dir| !dir.is_empty()),
            build_before_launch: true,
            launch_browser: profile.launch_browser,
            browser_url: profile.browser_url(),
        }
    }

    /// Why this configuration cannot launch here among the projects loaded now, in the person's words; none when it
    /// can: its launch profile's kind, or, when it runs its project's own program, a project that runs on Windows alone.
    pub fn why_not_here(&self, projects: &[EvaluatedProject]) -> Option<String> {
        if let Some(profile) = self.profile_why() {
            return Some(profile);
        }
        if self.program.is_some() {
            return None;
        }
        let project = projects.iter().find(|project| crate::paths::same(&project.path, &self.project))?;
        configure_only_on_windows(project, self.framework.as_deref())
    }

    /// The same configuration under a name no other has, as Rider names a copy.
    pub fn unique_name(wanted: &str, taken: impl IntoIterator<Item = String>) -> String {
        let taken: Vec<String> = taken.into_iter().collect();
        if !taken.iter().any(|name| name.eq_ignore_ascii_case(wanted)) {
            return wanted.to_string();
        }
        (1..)
            .map(|n| format!("{wanted} ({n})"))
            .find(|name| !taken.iter().any(|t| t.eq_ignore_ascii_case(name)))
            .unwrap()
    }

    /// Why its launch profile does not run here, read from the project's settings when the window asks; none either
    /// when its profile runs here or when the settings no longer hold that profile.
    fn profile_why(&self) -> Option<String> {
        let profile = self.launch_profile.as_deref()?;
        let folder = project_folder(&self.project);
        LaunchProfile::read(&folder)
            .into_iter()
            .find(|found| found.name == profile)
            .and_then(|found| found.unsupported())
    }
}

pub fn project_folder(project: &Path) -> PathBuf {
    project.parent().map(Path::to_path_buf).unwrap_or_default()
}

pub fn project_name(project: &Path) -> String {
    project.file_stem().map(|name| name.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Why a project's own program runs on Windows alone here: WPF, Windows Forms and the desktop runtime it needs.
fn configure_only_on_windows(project: &EvaluatedProject, framework: Option<&str>) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let frameworks = project
        .dependencies
        .iter()
        .filter(|dependencies| framework.is_none() || dependencies.framework.as_deref() == framework);
    for dependencies in frameworks {
        if dependencies.framework_references.iter().any(|reference| {
            reference.starts_with("Microsoft.WindowsDesktop.App") || reference.starts_with("Microsoft.Windows.SDK.NET")
        }) {
            let name = dependencies.framework.as_deref().unwrap_or("this framework");
            return Some(format!(
                "its {name} program needs Windows' desktop (WPF or Windows Forms), which runs on Windows alone"
            ));
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::FrameworkDependencies;

    fn project(path: &str, output: &str, frameworks: &[&str]) -> EvaluatedProject {
        EvaluatedProject {
            path: PathBuf::from(path),
            output_type: Some(output.into()),
            target_frameworks: frameworks.iter().map(|f| f.to_string()).collect(),
            ..EvaluatedProject::default()
        }
    }

    fn with_settings(name: &str, project_at: &str, settings: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rsharp-config-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let folder = root.join(project_at);
        std::fs::create_dir_all(folder.join("Properties")).unwrap();
        std::fs::write(folder.join("Properties").join("launchSettings.json"), settings).unwrap();
        folder.join("App.csproj")
    }

    #[test]
    fn a_folder_makes_one_configuration_per_launch_profile_and_one_for_a_project_whose_profiles_cannot_run() {
        let app = with_settings(
            "generate",
            "app",
            r#"{"profiles":{"http":{"commandName":"Project","applicationUrl":"http://localhost:5000"},
                "exe":{"commandName":"Executable","executablePath":"tool"},
                "iis":{"commandName":"IISExpress"}}}"#,
        );
        let plain = with_settings("plain", "plain", r#"{"profiles":{"only":{"commandName":"Docker"}}}"#);
        let library = project("/work/Lib/Lib.csproj", "Library", &["net10.0"]);
        let projects = [
            project(&app.to_string_lossy(), "Exe", &["net10.0"]),
            project(&plain.to_string_lossy(), "WinExe", &["net10.0"]),
            library,
        ];
        let names: Vec<String> = RunConfiguration::generate(&projects).into_iter().map(|c| c.name).collect();
        assert_eq!(
            names,
            ["App: http", "App: exe", "App: iis", "App", "App: only"],
            "every profile is listed, and a project whose profiles cannot run here is listed as itself; a library makes nothing"
        );
    }

    #[test]
    fn a_configuration_keeps_its_profiles_values() {
        let folder = std::env::temp_dir().join(format!("rsharp-config-values-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&folder);
        let profile = LaunchProfile {
            name: "https".into(),
            command: Some("Project".into()),
            command_line_args: Some("--seed 3".into()),
            working_directory: Some("src".into()),
            application_url: Some("https://localhost:7001".into()),
            launch_browser: true,
            launch_url: Some("swagger".into()),
            environment_variables: [("COUNT".to_string(), "3".to_string())].into(),
            ..LaunchProfile::default()
        };
        let configuration =
            RunConfiguration::from_profile(&project("/work/Shop/Shop.csproj", "Exe", &["net10.0"]), &profile);
        assert_eq!(configuration.name, "Shop: https");
        assert_eq!(configuration.arguments, "--seed 3");
        assert_eq!(configuration.working_directory.as_deref(), Some("src"));
        assert_eq!(configuration.browser_url, "https://localhost:7001/swagger");
        assert!(configuration.build_before_launch, "Rider builds a project before running it");
        assert_eq!(
            configuration.environment.get("ASPNETCORE_URLS").map(String::as_str),
            Some("https://localhost:7001")
        );
        assert_eq!(configuration.why_not_here(&[]), None, "a configuration whose project is unknown is not refused");
    }

    #[test]
    fn a_names_copy_is_numbered_and_a_windows_only_project_says_so() {
        assert_eq!(RunConfiguration::unique_name("Shop", Vec::new()), "Shop");
        assert_eq!(RunConfiguration::unique_name("Shop", ["shop".to_string()]), "Shop (1)");
        assert_eq!(RunConfiguration::unique_name("Shop", ["Shop".to_string(), "Shop (1)".to_string()]), "Shop (2)");
        let windows = EvaluatedProject {
            path: PathBuf::from("/work/Wpf/Wpf.csproj"),
            output_type: Some("WinExe".into()),
            dependencies: vec![FrameworkDependencies {
                framework: Some("net10.0-windows".into()),
                framework_references: vec!["Microsoft.WindowsDesktop.App.WPF".into()],
                ..FrameworkDependencies::default()
            }],
            ..EvaluatedProject::default()
        };
        let configuration =
            RunConfiguration { name: "Wpf".into(), project: windows.path.clone(), ..RunConfiguration::default() };
        let reason = configuration.why_not_here(std::slice::from_ref(&windows));
        if cfg!(windows) {
            assert_eq!(reason, None, "a Windows desktop program runs where it is built for");
        } else {
            assert!(reason.unwrap().contains("Windows' desktop"), "a WPF program off Windows says why it cannot run");
        }
    }
}
