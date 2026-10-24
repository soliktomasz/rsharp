//! A validated destination and literal dotnet arguments; the UI never makes paths or commands of its own.

use super::*;
use std::ffi::OsString;
use std::path::{Path, PathBuf};

mod job;
#[cfg(test)]
use job::create_with;
pub(crate) use job::walk;
pub use job::{CreatedProjects, CreationProgress, create};

#[derive(Clone, Debug)]
pub struct CreationPlan {
    pub root: PathBuf,
    pub solution: Option<PathBuf>,
    pub project_folder: PathBuf,
}

pub fn name_problem(name: &str, what: &str) -> Option<String> {
    let name = name.trim();
    if name.is_empty() {
        Some(format!("{what} name cannot be empty"))
    } else if matches!(name, "." | "..") || name.contains(['\0', '/', '\\', ':', '*', '?', '"', '<', '>', '|']) {
        Some(format!("{what} name contains invalid characters"))
    } else {
        None
    }
}

pub fn plan(request: &CreateRequest, catalog: &TemplateCatalog) -> Result<CreationPlan, String> {
    if request.template.variants.is_empty() || request.template.short_names.is_empty() {
        return Err("Choose a template".into());
    }
    if let Some(pack) = &request.template.pack {
        return Err(format!(
            "{} comes with {}, which is not installed: Install it first",
            request.template.name, pack.id
        ));
    }
    if request.target.is_some() && request.template.is_solution {
        return Err("A solution template cannot be added as a project".into());
    }
    if request.target.is_none()
        && let Some(why) = name_problem(&request.solution_name, "Solution")
    {
        return Err(why);
    }
    if !request.template.is_solution
        && let Some(why) = name_problem(&request.project_name, "Project")
    {
        return Err(why);
    }
    if request.directory.as_os_str().is_empty() {
        return Err("A directory is needed".into());
    }
    if !request.directory.is_absolute() {
        return Err(format!("{} is not a full path", request.directory.display()));
    }
    if let Some(target) = &request.target
        && target.projects.iter().any(|p| p.eq_ignore_ascii_case(request.project_name.trim()))
    {
        return Err(format!("Project {} already exists in the solution", request.project_name.trim()));
    }
    if request.target.is_none()
        && !request.template.is_solution
        && !catalog.solution_formats.contains(&request.solution_format)
    {
        return Err(format!("Solution format {} is not supported by SDK {}", request.solution_format, catalog.sdk));
    }
    let (root, solution, project_folder) = if request.target.is_some() {
        let root = request.directory.join(request.project_name.trim());
        (root.clone(), None, root)
    } else {
        let root = request.directory.join(request.solution_name.trim());
        let solution = Some(root.join(format!("{}.{}", request.solution_name.trim(), request.solution_format)));
        let project = if request.template.is_solution || request.same_directory {
            root.clone()
        } else {
            root.join(request.project_name.trim())
        };
        (root, solution, project)
    };
    let what = if request.target.is_some() {
        format!("Project {}", request.project_name.trim())
    } else {
        format!("Solution {}", request.solution_name.trim())
    };
    match std::fs::symlink_metadata(&root) {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(format!("{what} cannot be created: {} is a symbolic link", root.display()));
        }
        Ok(meta) if !meta.is_dir() => return Err(format!("{what} cannot be created: {} is a file", root.display())),
        Ok(_) => {
            if std::fs::read_dir(&root)
                .map_err(|e| e.to_string())?
                .next()
                .transpose()
                .map_err(|e| e.to_string())?
                .is_some()
            {
                return Err(format!("{what} already exists in this location: {} is not empty", root.display()));
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e.to_string()),
    }
    let values = request.values();
    for option in &request.variant().options {
        if !options::is_enabled(option.enabled_when.as_deref(), &values) {
            continue;
        }
        let value = &values[&option.name];
        if option.required && value.is_empty() {
            return Err(format!("{} is needed", option.label));
        }
        if !value.is_empty() {
            if option.kind == TemplateOptionKind::Integer && value.parse::<i64>().is_err() {
                return Err(format!("{}: please enter an integer number", option.label));
            }
            if option.kind == TemplateOptionKind::Float && value.parse::<f64>().is_err() {
                return Err(format!("{}: please enter a float number", option.label));
            }
        }
    }
    Ok(CreationPlan { root, solution, project_folder })
}

pub fn arguments(request: &CreateRequest, output: Option<&Path>) -> Result<Vec<OsString>, String> {
    if request.template.variants.is_empty() || request.template.short_names.is_empty() {
        return Err("Choose a template".into());
    }
    let variant = request.variant();
    let mut args: Vec<OsString> =
        [request.template.short_name(), "--name", request.name_given()].into_iter().map(Into::into).collect();
    if let Some(output) = output {
        args.extend(["--output".into(), output.as_os_str().into(), "--no-update-check".into()]);
        if let Some(skip) = &variant.skip_restore_option {
            args.extend([format!("--{skip}").into(), "true".into()]);
        }
    }
    if let Some(language) = &variant.language {
        args.extend(["--language".into(), language.into()]);
    }
    if let (Some(option), Some(framework)) = (&variant.framework_option, &request.framework) {
        args.extend([format!("--{option}").into(), framework.into()]);
    }
    let values = request.values();
    for each in &variant.options {
        let value = &values[&each.name];
        if options::is_enabled(each.enabled_when.as_deref(), &values) && !value.eq_ignore_ascii_case(&each.default) {
            args.extend([format!("--{}", each.option).into(), value.into()]);
        }
    }
    Ok(args)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Command, ExitStatus};
    use std::sync::atomic::AtomicBool;
    use std::time::Duration;

    #[test]
    fn creation_rechecks_current_target_names_and_logical_folder_before_writing() {
        let (root, environment, catalog, mut request) = setup("current-target", false);
        let solution = root.join("Shop.slnx");
        std::fs::write(&solution, "<Solution><Project Path=\"Taken/Taken.csproj\" /></Solution>").unwrap();
        request.project_name = "Taken".into();
        request.target =
            Some(crate::templates::ProjectTarget { solution: solution.clone(), folder: None, projects: vec![] });
        let mut run = |_: Command,
                       _: Duration,
                       _: &AtomicBool,
                       _: &mut dyn FnMut(&str)|
         -> Result<command::CommandResult, String> {
            panic!("a changed target reached dotnet before validation")
        };
        let error = job::create_with(&environment, &catalog, &request, &AtomicBool::new(false), &mut |_| {}, &mut run)
            .unwrap_err();
        assert!(error.contains("already exists"), "{error}");
        request.project_name = "Fresh".into();
        request.target.as_mut().unwrap().folder = Some("/gone/".into());
        let error = job::create_with(&environment, &catalog, &request, &AtomicBool::new(false), &mut |_| {}, &mut run)
            .unwrap_err();
        assert!(error.contains("gone"), "{error}");
        assert!(!root.join("Fresh").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[cfg(unix)]
    fn status(code: i32) -> ExitStatus {
        use std::os::unix::process::ExitStatusExt;
        ExitStatus::from_raw(code << 8)
    }
    #[cfg(windows)]
    fn status(code: i32) -> ExitStatus {
        use std::os::windows::process::ExitStatusExt;
        ExitStatus::from_raw(code as u32)
    }

    fn setup(name: &str, solution_template: bool) -> (PathBuf, TemplateEnvironment, TemplateCatalog, CreateRequest) {
        let root = std::env::temp_dir().join(format!("rsharp-create-flow-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        let root = crate::paths::canonical(&root);
        let template = ProjectTemplate {
            name: "Console".into(),
            short_names: vec!["console".into()],
            is_solution: solution_template,
            variants: vec![TemplateVariant {
                identity: "c".into(),
                skip_restore_option: Some("no-restore".into()),
                ..Default::default()
            }],
            ..Default::default()
        };
        let catalog = TemplateCatalog {
            sdk: "10".into(),
            families: vec![],
            solution_formats: vec!["slnx".into(), "sln".into()],
            solution_format: "slnx".into(),
        };
        let environment = TemplateEnvironment {
            dotnet: "dotnet".into(),
            cli_home: root.clone(),
            hive: Some(root.join("hive")),
            nuget_packages: None,
        };
        let request = CreateRequest {
            target: None,
            template,
            language: None,
            framework: None,
            solution_name: "Greeter".into(),
            project_name: "Greeter".into(),
            directory: root.clone(),
            same_directory: false,
            solution_format: "slnx".into(),
            values: Default::default(),
        };
        (root, environment, catalog, request)
    }

    fn fake(
        command: Command,
        _limit: Duration,
        _cancel: &AtomicBool,
        _output: &mut dyn FnMut(&str),
    ) -> Result<super::super::command::CommandResult, String> {
        let args: Vec<_> = command.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
        if args[0] == "new" {
            let output = PathBuf::from(&args[args.iter().position(|s| s == "--output").unwrap() + 1]);
            std::fs::create_dir_all(&output).unwrap();
            if args[1] == "sln" {
                std::fs::write(output.join("Greeter.slnx"), "<Solution />\n").unwrap();
            } else {
                std::fs::write(output.join("Greeter.csproj"), "<Project />\n").unwrap();
                std::fs::create_dir_all(output.join("obj/Hidden")).unwrap();
                std::fs::write(output.join("obj/Hidden/Hidden.csproj"), "ignored").unwrap();
            }
        }
        Ok(super::super::command::CommandResult { status: status(0), stdout: String::new(), stderr: String::new() })
    }

    #[test]
    fn project_solution_runs_sln_then_template_then_restore() {
        let (root, env, catalog, request) = setup("order", false);
        let mut commands = Vec::new();
        let made = create_with(
            &env,
            &catalog,
            &request,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |cmd, limit, cancel, output| {
                let args: Vec<_> = cmd.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
                commands.push(args[..if args[0] == "new" { 2 } else { 1 }].join(" "));
                fake(cmd, limit, cancel, output)
            },
        )
        .unwrap();
        assert_eq!(commands, ["new sln", "new console", "restore"]);
        assert_eq!(made.projects, [root.join("Greeter/Greeter/Greeter.csproj")]);
        assert!(made.warnings.is_empty());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn solution_template_keeps_its_solution_or_gets_a_fallback() {
        for with_solution in [false, true] {
            let (root, env, catalog, request) = setup(if with_solution { "with-solution" } else { "fallback" }, true);
            let made = create_with(
                &env,
                &catalog,
                &request,
                &AtomicBool::new(false),
                &mut |_| {},
                &mut |cmd, limit, cancel, output| {
                    let result = fake(cmd, limit, cancel, output);
                    if with_solution {
                        std::fs::write(root.join("Greeter/Greeter.sln"), "template solution").unwrap();
                    }
                    result
                },
            )
            .unwrap();
            assert_eq!(
                made.solution,
                Some(root.join(if with_solution { "Greeter/Greeter.sln" } else { "Greeter/Greeter.slnx" }))
            );
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn restore_failure_retains_generated_projects() {
        let (root, env, catalog, request) = setup("restore", false);
        let made = create_with(
            &env,
            &catalog,
            &request,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |cmd, limit, cancel, output| {
                if cmd.get_args().next().unwrap() == "restore" {
                    return Ok(super::super::command::CommandResult {
                        status: status(1),
                        stdout: String::new(),
                        stderr: "offline feed".into(),
                    });
                }
                fake(cmd, limit, cancel, output)
            },
        )
        .unwrap();
        assert!(made.projects[0].exists());
        assert!(made.warnings[0].contains("offline feed"));
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn failed_creation_and_cancel_clean_only_owned_output() {
        for existed in [false, true] {
            let (root, env, catalog, request) = setup(if existed { "empty-existing" } else { "cancel" }, false);
            if existed {
                std::fs::create_dir(root.join("Greeter")).unwrap();
            }
            let cancel = AtomicBool::new(false);
            let result =
                create_with(&env, &catalog, &request, &cancel, &mut |_| {}, &mut |cmd, limit, token, output| {
                    if cmd.get_args().next().unwrap() == "restore" {
                        cancel.store(true, std::sync::atomic::Ordering::Release);
                        return Err("Cancelled".into());
                    }
                    fake(cmd, limit, token, output)
                });
            assert!(result.unwrap_err().contains("Cancelled"));
            assert_eq!(root.join("Greeter").exists(), existed);
            if existed {
                assert_eq!(std::fs::read_dir(root.join("Greeter")).unwrap().count(), 0);
            }
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[test]
    fn uncertain_cleanup_reports_residue() {
        let (root, env, catalog, request) = setup("residue", false);
        let result = create_with(
            &env,
            &catalog,
            &request,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |cmd, limit, cancel, output| {
                if cmd.get_args().next().unwrap() == "restore" {
                    std::fs::write(root.join("Greeter/notes.txt"), b"keep me\r\n").unwrap();
                    return Err("Cancelled".into());
                }
                fake(cmd, limit, cancel, output)
            },
        );
        assert!(result.unwrap_err().contains("Cleanup left"));
        assert_eq!(std::fs::read(root.join("Greeter/notes.txt")).unwrap(), b"keep me\r\n");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_completed_result_dropped_before_handoff_is_cleaned() {
        let (root, env, catalog, request) = setup("dropped-result", false);
        let made = create_with(&env, &catalog, &request, &AtomicBool::new(false), &mut |_| {}, &mut fake).unwrap();
        assert!(made.projects[0].exists());
        drop(made);
        assert!(!root.join("Greeter").exists());
        let made = create_with(&env, &catalog, &request, &AtomicBool::new(false), &mut |_| {}, &mut fake).unwrap();
        made.keep();
        drop(made);
        assert!(root.join("Greeter/Greeter/Greeter.csproj").exists());
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn projects_are_ordered_and_build_outputs_excluded() {
        let (root, env, catalog, request) = setup("project-order", true);
        let made = create_with(
            &env,
            &catalog,
            &request,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |cmd, limit, cancel, output| {
                let result = fake(cmd, limit, cancel, output);
                std::fs::write(root.join("Greeter/B.fsproj"), "<Project />").unwrap();
                std::fs::write(root.join("Greeter/A.vbproj"), "<Project />").unwrap();
                result
            },
        )
        .unwrap();
        assert_eq!(
            made.projects.iter().map(|p| p.file_name().unwrap().to_str().unwrap()).collect::<Vec<_>>(),
            ["Greeter.csproj", "A.vbproj", "B.fsproj"]
        );
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn a_failed_partial_template_reports_unknown_output_without_deleting_it() {
        let (root, env, catalog, request) = setup("partial", false);
        let why = create_with(
            &env,
            &catalog,
            &request,
            &AtomicBool::new(false),
            &mut |_| {},
            &mut |cmd, limit, cancel, output| {
                let args: Vec<_> = cmd.get_args().map(|s| s.to_string_lossy().into_owned()).collect();
                if args.get(1).map(String::as_str) == Some("console") {
                    std::fs::write(root.join("Greeter/partial.txt"), "unknown author").unwrap();
                    return Ok(super::super::command::CommandResult {
                        status: status(1),
                        stdout: String::new(),
                        stderr: "template failed".into(),
                    });
                }
                fake(cmd, limit, cancel, output)
            },
        )
        .unwrap_err();
        assert!(why.contains("template failed") && why.contains("Cleanup left"));
        assert_eq!(std::fs::read_to_string(root.join("Greeter/partial.txt")).unwrap(), "unknown author");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn replacing_the_root_or_modifying_a_generated_file_prevents_its_deletion() {
        for replace_root in [false, true] {
            let (root, env, catalog, request) =
                setup(if replace_root { "replaced-root" } else { "edited-output" }, false);
            let made = create_with(&env, &catalog, &request, &AtomicBool::new(false), &mut |_| {}, &mut fake).unwrap();
            let kept = if replace_root {
                std::fs::rename(root.join("Greeter"), root.join("Former")).unwrap();
                std::fs::create_dir(root.join("Greeter")).unwrap();
                root.join("Greeter/notes.txt")
            } else {
                made.projects[0].clone()
            };
            std::fs::write(&kept, "a person's edit").unwrap();
            assert!(made.discard().unwrap_err().contains("Cleanup left"));
            assert_eq!(std::fs::read_to_string(kept).unwrap(), "a person's edit");
            let _ = std::fs::remove_dir_all(root);
        }
    }

    #[cfg(unix)]
    #[test]
    fn cleanup_does_not_follow_a_replaced_child_symlink() {
        let (root, env, catalog, request) = setup("child-link", false);
        let made = create_with(&env, &catalog, &request, &AtomicBool::new(false), &mut |_| {}, &mut fake).unwrap();
        std::fs::rename(root.join("Greeter/Greeter"), root.join("Outside")).unwrap();
        std::os::unix::fs::symlink(root.join("Outside"), root.join("Greeter/Greeter")).unwrap();
        assert!(made.discard().unwrap_err().contains("Cleanup left"));
        assert!(root.join("Outside/Greeter.csproj").exists());
        let _ = std::fs::remove_dir_all(root);
    }
}
