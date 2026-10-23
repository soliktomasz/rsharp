//! dotnet new's installed templates and Rider's project families, with the options an IDE asks for.

pub mod cache;
pub mod catalog;
mod command;
pub mod create;
pub mod developer_tools;
pub mod families;
pub mod options;
pub mod packs;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateCatalog {
    pub sdk: String,
    pub families: Vec<TemplateFamily>,
    pub solution_formats: Vec<String>,
    pub solution_format: String,
}

impl TemplateCatalog {
    pub fn templates(&self) -> Vec<&ProjectTemplate> {
        self.families.iter().flat_map(|f| &f.types).collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateFamily {
    pub name: String,
    pub types: Vec<ProjectTemplate>,
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct ProjectTemplate {
    pub name: String,
    pub short_names: Vec<String>,
    pub author: String,
    pub classifications: Vec<String>,
    pub default_name: String,
    pub variants: Vec<TemplateVariant>,
    pub family: String,
    pub type_name: String,
    pub is_solution: bool,
    pub pack: Option<TemplatePack>,
}

impl ProjectTemplate {
    pub fn short_name(&self) -> &str {
        &self.short_names[0]
    }

    pub fn languages(&self) -> Vec<String> {
        let mut languages: Vec<_> = self.variants.iter().filter_map(|v| v.language.clone()).collect();
        languages.sort_by(|a, b| language_order(a).cmp(&language_order(b)).then(a.cmp(b)));
        languages.dedup();
        languages
    }

    pub fn frameworks(&self, language: Option<&str>) -> Vec<String> {
        let mut frameworks = Vec::new();
        for framework in self.variants.iter().filter(|v| v.language.as_deref() == language).flat_map(|v| &v.frameworks)
        {
            if !frameworks.contains(framework) {
                frameworks.push(framework.clone());
            }
        }
        frameworks
    }

    pub fn variant(&self, language: Option<&str>, framework: Option<&str>) -> &TemplateVariant {
        self.variants
            .iter()
            .find(|v| {
                v.language.as_deref() == language
                    && framework.is_none_or(|f| v.frameworks.is_empty() || v.frameworks.iter().any(|v| v == f))
            })
            .or_else(|| self.variants.iter().find(|v| v.language.as_deref() == language))
            .unwrap_or(&self.variants[0])
    }
}

pub(crate) fn language_order(language: &str) -> usize {
    match language {
        "C#" => 0,
        "F#" => 1,
        "VB" => 2,
        _ => 3,
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct TemplateVariant {
    pub identity: String,
    pub language: Option<String>,
    pub precedence: i64,
    pub description: String,
    pub framework_option: Option<String>,
    pub frameworks: Vec<String>,
    pub default_framework: Option<String>,
    pub options: Vec<TemplateOption>,
    pub skip_restore_option: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplateOptionKind {
    Bool,
    Choice,
    Text,
    Integer,
    Float,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateOption {
    pub name: String,
    pub option: String,
    pub label: String,
    pub description: Option<String>,
    pub kind: TemplateOptionKind,
    pub default: String,
    pub choices: Vec<TemplateChoice>,
    pub inverted: bool,
    pub required: bool,
    pub enabled_when: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TemplateChoice {
    pub value: String,
    pub label: String,
    pub description: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct TemplatePack {
    pub id: String,
    pub offer: String,
    pub version: String,
    pub author: String,
    pub kinds: Vec<PackKind>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize)]
pub struct PackKind {
    pub name: String,
    pub short_name: String,
    pub is_solution: bool,
    pub languages: Vec<String>,
    pub description: String,
    pub classifications: Vec<String>,
    pub default_name: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TemplatePlatform {
    MacOS,
    Windows,
    Linux,
}

impl TemplatePlatform {
    pub fn current() -> Self {
        if cfg!(windows) {
            Self::Windows
        } else if cfg!(target_os = "macos") {
            Self::MacOS
        } else {
            Self::Linux
        }
    }
}

#[derive(Clone, Debug)]
pub struct TemplateEnvironment {
    pub dotnet: std::path::PathBuf,
    pub cli_home: std::path::PathBuf,
    pub hive: Option<std::path::PathBuf>,
    pub nuget_packages: Option<std::path::PathBuf>,
}

impl TemplateEnvironment {
    pub fn discover() -> Result<Self, String> {
        let dotnet = crate::programs::dotnet().ok_or("dotnet is not installed or could not be found")?;
        let cli_home = std::env::var_os("DOTNET_CLI_HOME")
            .filter(|v| !v.is_empty())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(crate::programs::home);
        if cli_home.as_os_str().is_empty() {
            return Err("Neither DOTNET_CLI_HOME nor the home folder is set".into());
        }
        Ok(Self { dotnet, cli_home, hive: None, nuget_packages: None })
    }

    pub fn command(&self, arguments: &[std::ffi::OsString], folder: &std::path::Path) -> std::process::Command {
        let mut command = std::process::Command::new(&self.dotnet);
        command
            .args(arguments)
            .current_dir(folder)
            .env("DOTNET_CLI_HOME", &self.cli_home)
            .env("DOTNET_NOLOGO", "1")
            .env("DOTNET_SKIP_FIRST_TIME_EXPERIENCE", "1")
            .env("DOTNET_CLI_TELEMETRY_OPTOUT", "1");
        if let Some(packages) = &self.nuget_packages {
            command.env("NUGET_PACKAGES", packages);
        }
        command
    }

    pub fn new_command(&self, arguments: &[std::ffi::OsString], folder: &std::path::Path) -> std::process::Command {
        let mut arguments =
            std::iter::once(std::ffi::OsString::from("new")).chain(arguments.iter().cloned()).collect::<Vec<_>>();
        if let Some(hive) = &self.hive {
            arguments.extend(["--debug:custom-hive".into(), hive.as_os_str().into()]);
        }
        self.command(&arguments, folder)
    }
}

#[derive(Clone, Debug)]
pub struct ProjectTarget {
    pub solution: std::path::PathBuf,
    pub folder: Option<String>,
    pub projects: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct CreateRequest {
    pub target: Option<ProjectTarget>,
    pub template: ProjectTemplate,
    pub language: Option<String>,
    pub framework: Option<String>,
    pub solution_name: String,
    pub project_name: String,
    pub directory: std::path::PathBuf,
    pub same_directory: bool,
    pub solution_format: String,
    pub values: std::collections::BTreeMap<String, String>,
}

impl CreateRequest {
    pub fn name_given(&self) -> &str {
        if self.template.is_solution { self.solution_name.trim() } else { self.project_name.trim() }
    }
    pub fn variant(&self) -> &TemplateVariant {
        self.template.variant(self.language.as_deref(), self.framework.as_deref())
    }
    pub fn values(&self) -> std::collections::BTreeMap<String, String> {
        self.variant()
            .options
            .iter()
            .map(|o| (o.name.clone(), self.values.get(&o.name).unwrap_or(&o.default).clone()))
            .collect()
    }
}
