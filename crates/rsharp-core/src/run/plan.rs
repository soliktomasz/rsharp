//! What a run configuration launches, worked out once: the program (the project's built assembly under the `dotnet`
//! that builds it, or another program), its arguments, working folder and environment, the build that comes first, and
//! the browser it opens. MSBuild says where the project builds and what its `$(Property)` values are.

use super::command_line;
use super::configuration::{RunConfiguration, project_folder, project_name};
use crate::project::EvaluatedProject;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::process::Command;

/// The project a launch builds first, for one framework when it has several.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuildStep {
    pub project: PathBuf,
    pub framework: Option<String>,
}

/// What MSBuild evaluated for a project, by property name as MSBuild spells it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Properties {
    values: BTreeMap<String, String>,
}

impl Properties {
    pub fn new(values: impl IntoIterator<Item = (String, String)>) -> Self {
        Self { values: values.into_iter().collect() }
    }

    /// `dotnet msbuild -getProperty`'s answer, `{"Properties":{…}}`, as the command writes it.
    pub fn parse(json: &str) -> Result<Self, String> {
        let json = json.trim();
        if json.is_empty() {
            return Err("MSBuild said nothing".into());
        }
        let read: serde_json::Value =
            serde_json::from_str(json).map_err(|error| format!("MSBuild's answer did not read: {error}"))?;
        let properties = read
            .get("Properties")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| format!("MSBuild's answer names no properties: {json}"))?;
        Ok(Self::new(properties.iter().map(|(name, value)| {
            let value = match value.as_str() {
                Some(text) => text.to_string(),
                None => value.to_string(),
            };
            (name.clone(), value)
        })))
    }

    pub fn get(&self, name: &str) -> Option<&str> {
        self.values.iter().find(|(key, _)| key.eq_ignore_ascii_case(name)).map(|(_, value)| value.as_str())
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LaunchPlan {
    /// The configuration's name, as the windows and the status bar say it.
    pub name: String,
    /// The program started: the `dotnet` host for an assembly, or an executable.
    pub program: PathBuf,
    /// The program's arguments, the assembly first when it runs under `dotnet`.
    pub arguments: Vec<String>,
    pub working_directory: PathBuf,
    /// Set over the IDE's own environment, which the program inherits.
    pub environment: BTreeMap<String, String>,
    pub project: PathBuf,
    pub framework: Option<String>,
    /// The build before the launch, or none when the configuration launches what was built last.
    pub build: Option<BuildStep>,
    /// The file the build makes and the launch needs (the project's assembly), checked once the build is done.
    pub built: Option<PathBuf>,
    /// Where the browser opens once the program listens, or none when the configuration opens none.
    pub browser_url: Option<String>,
}

impl LaunchPlan {
    /// Works out what a configuration launches among the properties MSBuild gave for its project. A configuration
    /// that cannot launch says why, in the person's words.
    pub fn resolve(
        configuration: &RunConfiguration,
        project: &EvaluatedProject,
        properties: &Properties,
    ) -> Result<LaunchPlan, String> {
        if let Some(why) = configuration.why_not_here(std::slice::from_ref(project)).or_else(|| project.problem.clone())
        {
            return Err(format!("{} cannot run here: {why}", configuration.name));
        }
        let expand = |text: &str| expand(text, properties);
        let target = properties
            .get("TargetPath")
            .filter(|path| !path.is_empty())
            .map(|path| crate::paths::canonical(Path::new(path)))
            .ok_or_else(|| format!("MSBuild could not say what {} builds", project_name(&project.path)))?;
        if configuration.program.as_deref().is_none_or(str::is_empty)
            && properties.get("OutputType").is_some_and(|output| output.eq_ignore_ascii_case("Library"))
        {
            return Err(format!(
                "{} cannot run here: its project is a library, which has no program",
                configuration.name
            ));
        }
        let output_folder = target.parent().map(Path::to_path_buf).unwrap_or_default();
        let mut arguments = command_line::split(&expand(&configuration.arguments));
        let (program, built) = match configuration.program.as_deref().filter(|program| !program.is_empty()) {
            Some(named) => (executable(&expand(named), &project_folder(&project.path))?, None),
            None if target.extension().is_some_and(|extension| extension.eq_ignore_ascii_case("dll")) => {
                let Some(dotnet) = crate::programs::dotnet() else {
                    return Err("No .NET command line found: install the .NET SDK, or set DOTNET_ROOT".into());
                };
                arguments.insert(0, target.to_string_lossy().into_owned());
                (dotnet, Some(target.clone()))
            }
            None => (target.clone(), Some(target)),
        };
        let framework = configuration.framework.clone();
        let working_directory = match configuration.working_directory.as_deref().map(str::trim) {
            Some(folder) if !folder.is_empty() => {
                crate::paths::canonical(&project_folder(&project.path).join(expand(folder)))
            }
            _ => output_folder,
        };
        Ok(LaunchPlan {
            name: configuration.name.clone(),
            program,
            arguments,
            working_directory,
            environment: configuration.environment.iter().map(|(name, value)| (name.clone(), expand(value))).collect(),
            project: project.path.clone(),
            framework,
            build: configuration
                .build_before_launch
                .then(|| BuildStep { project: project.path.clone(), framework: configuration.framework.clone() }),
            built,
            browser_url: configuration
                .launch_browser
                .then(|| expand(&configuration.browser_url))
                .filter(|url| !url.is_empty()),
        })
    }

    /// The program as its messages name it: the project's assembly, else the executable, without its extension.
    pub fn program_name(&self) -> String {
        let file = self.built.as_ref().unwrap_or(&self.program);
        file.file_stem().map(|stem| stem.to_string_lossy().into_owned()).unwrap_or_default()
    }

    /// The program's command: its arguments, working folder and environment, over the IDE's own.
    pub fn command(&self) -> Command {
        let mut command = Command::new(&self.program);
        command.args(&self.arguments).current_dir(&self.working_directory);
        for (name, value) in &self.environment {
            command.env(name, value);
        }
        command
    }

    /// None when the plan can launch now, its build done; else why not, in the person's words.
    pub fn why_not(&self, verb: &str) -> Option<String> {
        if let Some(built) = &self.built
            && !built.exists()
        {
            let file = built.file_name().unwrap_or_default().to_string_lossy().into_owned();
            return Some(format!("Could not {verb} {}: {file} has not been built", self.program_name()));
        }
        if !self.working_directory.is_dir() {
            return Some(format!(
                "Could not {verb} {}: its working folder {} does not exist",
                self.program_name(),
                self.working_directory.display()
            ));
        }
        None
    }
}

/// A program a configuration names: the .NET command line by name is the one the IDE builds with, a relative path is
/// the project folder's, and a bare name is looked for on the PATH as a shell would.
fn executable(named: &str, folder: &Path) -> Result<PathBuf, String> {
    if named == "dotnet" || named == "dotnet.exe" {
        return crate::programs::dotnet()
            .ok_or_else(|| "No .NET command line found: install the .NET SDK, or set DOTNET_ROOT".into());
    }
    if named.contains(['/', '\\']) || Path::new(named).is_absolute() {
        return Ok(crate::paths::canonical(&folder.join(named)));
    }
    Ok(crate::programs::find(named, &[]).unwrap_or_else(|| PathBuf::from(named)))
}

/// A value with MSBuild's `$(Property)` and the environment's `%NAME%` filled in, as Visual Studio fills a launch
/// profile's: a property or variable that is not there is left as it was written.
fn expand(text: &str, properties: &Properties) -> String {
    expand_environment(&expand_properties(text, properties))
}

fn expand_properties(text: &str, properties: &Properties) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = text[at..].find("$(") {
        let start = at + found;
        let inner = start + 2;
        let Some(end) = text[inner..].find(')').map(|end| inner + end) else { break };
        out.push_str(&text[at..start]);
        match properties.get(&text[inner..end]) {
            Some(value) => out.push_str(value),
            None => out.push_str(&text[start..=end]),
        }
        at = end + 1;
    }
    out.push_str(&text[at..]);
    out
}

fn expand_environment(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut at = 0;
    while let Some(found) = text[at..].find('%') {
        let start = at + found;
        let inner = start + 1;
        let Some(end) = text[inner..].find('%').map(|end| inner + end) else { break };
        out.push_str(&text[at..start]);
        let name = &text[inner..end];
        match std::env::var(name).ok().filter(|value| !value.is_empty() && !name.is_empty()) {
            Some(value) => out.push_str(&value),
            None => out.push_str(&text[start..=end]),
        }
        at = end + 1;
    }
    out.push_str(&text[at..]);
    out
}

/// The MSBuild properties a configuration's values name, which must be evaluated with them: `TargetPath` and
/// `ProjectDir` first, then every `$(Name)` its program, arguments, working folder, browser URL or environment holds.
pub fn needed_properties(configuration: &RunConfiguration) -> Vec<String> {
    let mut names: BTreeSet<String> = ["OutputType", "ProjectDir", "TargetPath"].map(str::to_string).into();
    let texts = std::iter::once(configuration.arguments.as_str())
        .chain(configuration.program.as_deref())
        .chain(configuration.working_directory.as_deref())
        .chain(std::iter::once(configuration.browser_url.as_str()))
        .chain(configuration.environment.values().map(String::as_str));
    for text in texts {
        let mut at = 0;
        while let Some(found) = text[at..].find("$(") {
            let inner = at + found + 2;
            let Some(end) = text[inner..].find(')').map(|end| inner + end) else { break };
            let name = &text[inner..end];
            if !name.is_empty() && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-')) {
                names.insert(name.to_string());
            }
            at = end + 1;
        }
    }
    let first = ["TargetPath", "ProjectDir", "OutputType"];
    let mut ordered: Vec<String> = first.iter().map(|name| name.to_string()).collect();
    ordered.extend(names.into_iter().filter(|name| !first.contains(&name.as_str())));
    ordered
}

/// What MSBuild evaluates for a project, one framework of it at most: `Needed`, in one request that runs no build.
pub fn properties(project: &Path, framework: Option<&str>, needed: &[String]) -> Result<Properties, String> {
    let folder = project_folder(project);
    let dotnet = match crate::programs::dotnet() {
        Some(dotnet) => dotnet,
        None => {
            return Err("No .NET command line found: install the .NET SDK, or set DOTNET_ROOT".into());
        }
    };
    let mut arguments: Vec<String> = vec!["msbuild".into(), project.to_string_lossy().into_owned(), "-nologo".into()];
    arguments.push(format!("-getProperty:{}", needed.join(",")));
    if let Some(framework) = framework {
        arguments.push(format!("-property:TargetFramework={framework}"));
    }
    let command = crate::dotnet::command(&dotnet, &arguments, &folder);
    let mut out = String::new();
    let mut error = String::new();
    let cancel = std::sync::atomic::AtomicBool::new(false);
    match crate::build::run(command, &cancel, |text, is_error| {
        let into = if is_error { &mut error } else { &mut out };
        into.push_str(text);
    }) {
        Ok(Some(0)) => Properties::parse(&out),
        Ok(Some(code)) => Err(format!(
            "MSBuild could not say what {} builds: exit code {code} {}",
            project_name(project),
            error.trim()
        )),
        Ok(None) => Err("MSBuild stopped before it answered".into()),
        Err(failure) => Err(format!("MSBuild could not be started: {failure}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::EvaluatedProject;

    fn project(path: &str) -> EvaluatedProject {
        EvaluatedProject {
            path: PathBuf::from(path),
            output_type: Some("Exe".into()),
            target_frameworks: vec!["net10.0".into()],
            ..EvaluatedProject::default()
        }
    }

    fn properties(target: &str) -> Properties {
        Properties::new([
            ("TargetPath".to_string(), target.to_string()),
            ("ProjectDir".to_string(), "/work/Shop/".to_string()),
            ("TargetDir".to_string(), "/work/Shop/bin/Debug/net10.0/".to_string()),
        ])
    }

    fn configuration(arguments: &str) -> RunConfiguration {
        RunConfiguration {
            name: "Shop".into(),
            project: PathBuf::from("/work/Shop/Shop.csproj"),
            arguments: arguments.into(),
            ..RunConfiguration::default()
        }
    }

    #[test]
    fn an_assembly_launches_under_the_dotnet_host_with_it_first() {
        let plan = LaunchPlan::resolve(
            &configuration("--seed 3"),
            &project("/work/Shop/Shop.csproj"),
            &properties("/work/Shop/bin/Debug/net10.0/Shop.dll"),
        )
        .unwrap();
        assert_eq!(plan.program.file_name().unwrap(), if cfg!(windows) { "dotnet.exe" } else { "dotnet" });
        assert_eq!(plan.arguments, ["/work/Shop/bin/Debug/net10.0/Shop.dll", "--seed", "3"]);
        assert_eq!(plan.working_directory, Path::new("/work/Shop/bin/Debug/net10.0"));
        assert_eq!(plan.program_name(), "Shop");
        assert_eq!(plan.built.as_deref(), Some(Path::new("/work/Shop/bin/Debug/net10.0/Shop.dll")));
        assert!(plan.build.is_some(), "a configuration builds before it launches unless told not to");
    }

    #[test]
    fn an_exe_target_launches_itself_and_another_program_takes_its_place() {
        let self_contained = LaunchPlan::resolve(
            &configuration(""),
            &project("/work/Tool/Tool.csproj"),
            &properties("/work/Tool/bin/Debug/net10.0/Tool"),
        )
        .unwrap();
        assert_eq!(self_contained.program, Path::new("/work/Tool/bin/Debug/net10.0/Tool"));
        assert!(self_contained.arguments.is_empty());
        let other = RunConfiguration { program: Some("$(TargetDir)../nested/program".into()), ..configuration("a b") };
        let plan = LaunchPlan::resolve(
            &other,
            &project("/work/Tool/Tool.csproj"),
            &properties("/work/Tool/bin/Debug/net10.0/Tool"),
        )
        .unwrap();
        assert_eq!(plan.program, Path::new("/work/Shop/bin/Debug/net10.0/../nested/program"));
        assert_eq!(plan.arguments, ["a", "b"], "another program's arguments are the profile's alone");
        assert_eq!(plan.built, None, "a program another program launches is not the project's to check");
    }

    #[test]
    fn a_working_folder_and_an_environment_expand_as_visual_studio_expands_them() {
        let with_environment = RunConfiguration {
            working_directory: Some("$(ProjectDir)src".into()),
            environment: [("COUNT".to_string(), "$(TargetDir)".to_string())].into(),
            ..configuration("")
        };
        let plan = LaunchPlan::resolve(
            &with_environment,
            &project("/work/Shop/Shop.csproj"),
            &properties("/work/Shop/bin/Debug/net10.0/Shop.dll"),
        )
        .unwrap();
        assert_eq!(plan.working_directory, Path::new("/work/Shop/src"));
        assert_eq!(plan.environment.get("COUNT"), Some(&"/work/Shop/bin/Debug/net10.0/".to_string()));
        let unknown =
            LaunchPlan::resolve(&configuration("$(NotThere)"), &project("/w/S.csproj"), &properties("/w/S.dll"))
                .unwrap();
        assert_eq!(
            unknown.arguments.last().map(String::as_str),
            Some("$(NotThere)"),
            "a property MSBuild did not give is left as it was written"
        );
    }

    #[test]
    fn a_plan_says_why_it_cannot_launch_yet() {
        let plan = LaunchPlan::resolve(
            &configuration(""),
            &project("/work/Shop/Shop.csproj"),
            &properties("/nowhere/Shop.dll"),
        )
        .unwrap();
        let missing = plan.why_not("run").unwrap();
        assert!(missing.starts_with("Could not run Shop: Shop.dll has not been built"), "{missing}");
        let built = LaunchPlan { built: None, ..plan };
        let no_folder = built.why_not("run").unwrap();
        assert!(no_folder.contains("working folder"), "{no_folder}");
    }

    #[test]
    fn msbuilds_answer_reads_and_a_project_it_could_not_say_about_is_an_error() {
        assert_eq!(
            Properties::parse(r#"{"Properties":{"TargetPath":"/w/A.dll","ProjectDir":"/w/"}}"#)
                .unwrap()
                .get("targetpath"),
            Some("/w/A.dll"),
            "a property is found however its name is cased"
        );
        assert!(Properties::parse("").unwrap_err().contains("said nothing"));
        assert!(Properties::parse("not json").unwrap_err().contains("did not read"));
        assert!(Properties::parse("{}").unwrap_err().contains("names no properties"));
        let mut configuration = configuration("");
        configuration.arguments = "$(TargetDir) --x %PATH%".into();
        let needed = needed_properties(&configuration);
        assert_eq!(needed[..3], ["TargetPath", "ProjectDir", "OutputType"]);
        assert!(needed.contains(&"TargetDir".to_string()));
    }

    #[test]
    fn a_project_that_does_not_evaluate_is_refused_with_msbuilds_words() {
        let broken = EvaluatedProject {
            path: PathBuf::from("/work/Shop/Shop.csproj"),
            problem: Some("the SDK Microsoft.NET.Sdk.Web is not installed".into()),
            ..EvaluatedProject::default()
        };
        let error = LaunchPlan::resolve(&configuration(""), &broken, &properties("/w/S.dll")).unwrap_err();
        assert!(error.contains("cannot run here") && error.contains("not installed"), "{error}");
    }
}
