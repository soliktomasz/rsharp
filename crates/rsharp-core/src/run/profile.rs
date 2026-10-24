//! A launch profile from a project's `Properties/launchSettings.json`, as `dotnet run`, Visual Studio and Rider read
//! one: what it launches (its `commandName`), with what arguments, environment and working directory, and the browser
//! it opens. The two kinds that run a program run here; every other kind is read too, and says why it does not.

use serde_json_lenient::Value;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// The project's own program, built.
pub const PROJECT: &str = "Project";

/// Another program, named by `executablePath`.
pub const EXECUTABLE: &str = "Executable";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct LaunchProfile {
    pub name: String,
    pub command: Option<String>,
    pub executable_path: Option<String>,
    pub command_line_args: Option<String>,
    pub working_directory: Option<String>,
    pub environment_variables: BTreeMap<String, String>,
    pub application_url: Option<String>,
    pub launch_browser: bool,
    pub launch_url: Option<String>,
}

impl LaunchProfile {
    pub fn settings_path(project_folder: &Path) -> PathBuf {
        project_folder.join("Properties").join("launchSettings.json")
    }

    /// The project's profiles of every kind, in the file's order; none when it has no settings or they do not read.
    pub fn read(project_folder: &Path) -> Vec<LaunchProfile> {
        let Ok(bytes) = std::fs::read(Self::settings_path(project_folder)) else { return Vec::new() };
        let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(&bytes);
        let Ok(settings) = serde_json_lenient::from_slice::<Value>(bytes) else { return Vec::new() };
        let Some(profiles) = settings.get("profiles").and_then(Value::as_object) else { return Vec::new() };
        profiles
            .iter()
            .filter(|(_, profile)| profile.is_object())
            .map(|(name, profile)| LaunchProfile {
                name: name.clone(),
                command: text(profile, "commandName"),
                executable_path: text(profile, "executablePath"),
                command_line_args: text(profile, "commandLineArgs"),
                working_directory: text(profile, "workingDirectory"),
                environment_variables: environment(profile),
                application_url: text(profile, "applicationUrl"),
                launch_browser: profile.get("launchBrowser").and_then(Value::as_bool).unwrap_or(false),
                launch_url: text(profile, "launchUrl"),
            })
            .collect()
    }

    /// Null for a profile that runs here, else why it does not, in Rider's terms for each command it knows.
    pub fn unsupported(&self) -> Option<String> {
        let command = self.command.as_deref().unwrap_or_default();
        if command.is_empty() {
            return Some("its profile names no commandName".into());
        }
        if self.is(PROJECT) || self.is(EXECUTABLE) {
            return None;
        }
        Some(
            match command.to_ascii_lowercase().as_str() {
                "iisexpress" => "an IIS Express profile runs under IIS Express on Windows, which RSharp does not start",
                "iis" => "an IIS profile runs under IIS on Windows, which RSharp does not drive",
                "msixpackage" => {
                    "an MSIX package profile runs a deployed Windows package, which RSharp does not deploy"
                }
                "debugroslyncomponent" => {
                    "a Roslyn component profile runs the compiler over another project, which RSharp does not"
                }
                "docker" | "dockercompose" | "sdkcontainer" => {
                    "a container profile runs in Docker, which RSharp does not start"
                }
                "wsl2" => "a WSL 2 profile runs in Linux under Windows, which RSharp does not start",
                _ => return Some(format!("RSharp does not know the {command} command")),
            }
            .to_string(),
        )
    }

    /// Whether this profile runs the project's own program.
    pub fn runs_project(&self) -> bool {
        self.is(PROJECT)
    }

    fn is(&self, command: &str) -> bool {
        self.command.as_deref().is_some_and(|c| c.eq_ignore_ascii_case(command))
    }

    /// The environment a run of this profile gets: its variables, with its application URL as ASP.NET Core's
    /// `ASPNETCORE_URLS` unless it sets that itself, as `dotnet run` passes it.
    pub fn environment(&self) -> BTreeMap<String, String> {
        let mut environment = self.environment_variables.clone();
        if self.runs_project()
            && let Some(url) = self.application_url.as_deref().map(str::trim).filter(|url| !url.is_empty())
        {
            environment.entry("ASPNETCORE_URLS".into()).or_insert_with(|| url.to_string());
        }
        environment
    }

    /// Where the browser opens when the profile asks for one: its launch URL against its first application
    /// URL unless it is absolute; empty with neither.
    pub fn browser_url(&self) -> String {
        let root =
            self.application_url.as_deref().and_then(|urls| urls.split(';').map(str::trim).find(|url| !url.is_empty()));
        let launch = self.launch_url.as_deref().map(str::trim).filter(|url| !url.is_empty());
        match (launch, root) {
            (Some(launch), _) if is_absolute(launch) => launch.to_string(),
            (Some(launch), Some(root)) => format!("{}/{}", root.trim_end_matches('/'), launch.trim_start_matches('/')),
            (Some(launch), None) => launch.to_string(),
            (None, Some(root)) => root.to_string(),
            (None, None) => String::new(),
        }
    }
}

fn is_absolute(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    lower.starts_with("http://") || lower.starts_with("https://")
}

fn text(profile: &Value, key: &str) -> Option<String> {
    profile.get(key).and_then(Value::as_str).map(str::to_string)
}

fn environment(profile: &Value) -> BTreeMap<String, String> {
    let Some(variables) = profile.get("environmentVariables").and_then(Value::as_object) else {
        return BTreeMap::new();
    };
    variables
        .iter()
        .map(|(name, value)| {
            let value = match value.as_str() {
                Some(text) => text.to_string(),
                None => value.to_string(),
            };
            (name.clone(), value)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(name: &str, settings: Option<&str>) -> PathBuf {
        let root = std::env::temp_dir().join(format!("rsharp-profile-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("Properties")).unwrap();
        if let Some(settings) = settings {
            std::fs::write(root.join("Properties").join("launchSettings.json"), settings).unwrap();
        }
        root
    }

    #[test]
    fn a_projects_profiles_read_as_dotnet_run_reads_them() {
        // The values a launch uses come from this file, so every field the SDK writes is read as the SDK means it.
        let root = folder(
            "read",
            Some(
                r#"{
                    // comments and trailing commas, as the SDK's own writer leaves them
                    "profiles": {
                        "Shop": {
                            "commandName": "Project",
                            "commandLineArgs": "--seed 3",
                            "workingDirectory": "src",
                            "applicationUrl": "https://localhost:7001;http://localhost:5001",
                            "launchBrowser": true,
                            "launchUrl": "swagger",
                            "environmentVariables": { "ASPNETCORE_ENVIRONMENT": "Development", "COUNT": 3 },
                        },
                        "Other": { "commandName": "Executable", "executablePath": "bin/tool" },
                        "Container": { "commandName": "Docker" },
                    },
                    "iisSettings": { "windowsAuthentication": false },
                }"#,
            ),
        );
        let profiles = LaunchProfile::read(&root);
        assert_eq!(profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(), ["Shop", "Other", "Container"]);
        let shop = &profiles[0];
        assert_eq!(shop.command_line_args.as_deref(), Some("--seed 3"));
        assert_eq!(shop.working_directory.as_deref(), Some("src"));
        assert!(shop.launch_browser);
        assert_eq!(shop.environment_variables.get("COUNT").map(String::as_str), Some("3"));
        assert_eq!(
            shop.environment().get("ASPNETCORE_URLS").map(String::as_str),
            Some("https://localhost:7001;http://localhost:5001"),
            "every URL an application URL lists reaches ASP.NET Core, as dotnet run passes them"
        );
        assert_eq!(shop.browser_url(), "https://localhost:7001/swagger", "the browser joins the first application URL");
        assert_eq!(shop.unsupported(), None);
        assert!(shop.runs_project());
        assert_eq!(profiles[1].unsupported(), None, "an Executable profile runs another program, which RSharp starts");
        assert!(!profiles[1].runs_project());
        assert_eq!(
            profiles[2].unsupported().as_deref(),
            Some("a container profile runs in Docker, which RSharp does not start")
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_profile_says_why_it_cannot_run_here() {
        let reason = |command: &str| {
            LaunchProfile { command: Some(command.into()), ..LaunchProfile::default() }
                .unsupported()
                .unwrap_or_default()
        };
        assert_eq!(
            LaunchProfile::default().unsupported().as_deref(),
            Some("its profile names no commandName"),
            "a profile that names no command cannot run"
        );
        assert!(reason("IISExpress").contains("IIS Express"));
        assert!(reason("WSL2").contains("WSL 2"));
        assert!(reason("DebugRoslynComponent").contains("compiler"));
        assert!(reason("SomethingNew").contains("SomethingNew"), "a command RSharp does not know is named as it is");
    }

    #[test]
    fn an_environment_keeps_its_own_url_and_an_absolute_launch_url() {
        let profile = LaunchProfile {
            command: Some(PROJECT.into()),
            application_url: Some("http://localhost:5000;http://localhost:5001".into()),
            environment_variables: [("ASPNETCORE_URLS".to_string(), "http://localhost:9999".to_string())].into(),
            launch_url: Some("https://example.test/path".into()),
            ..LaunchProfile::default()
        };
        assert_eq!(
            profile.environment().get("ASPNETCORE_URLS").map(String::as_str),
            Some("http://localhost:9999"),
            "a profile's own ASPNETCORE_URLS is not replaced"
        );
        assert_eq!(profile.browser_url(), "https://example.test/path");
        let relative = LaunchProfile { launch_url: Some("/health".into()), ..profile };
        assert_eq!(relative.browser_url(), "http://localhost:5000/health");
    }

    #[test]
    fn a_project_without_settings_has_no_profiles() {
        let root = folder("missing", None);
        assert!(LaunchProfile::read(&root).is_empty());
        std::fs::write(root.join("Properties").join("launchSettings.json"), "{ this is not json").unwrap();
        assert!(LaunchProfile::read(&root).is_empty(), "a file that does not read gives no profiles");
        let _ = std::fs::remove_dir_all(&root);
    }
}
