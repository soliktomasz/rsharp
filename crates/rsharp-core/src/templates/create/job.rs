//! Generation owns its output until the window accepts it, including a result arriving after Cancel.

use super::{arguments, plan};
use crate::templates::{CreateRequest, TemplateCatalog, TemplateEnvironment, command, developer_tools};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

#[derive(Clone, Debug)]
pub struct CreatedProjects {
    pub solution: Option<PathBuf>,
    pub projects: Vec<PathBuf>,
    pub warnings: Vec<String>,
    owned: Arc<OwnedOutput>,
}

impl CreatedProjects {
    /// Accepts completed output; a subsequent attachment failure keeps it, with its paths reported.
    pub fn keep(&self) {
        self.owned.kept.store(true, Ordering::Release);
    }
    pub fn discard(&self) -> Result<(), String> {
        if self.owned.kept.swap(true, Ordering::AcqRel) {
            return Ok(());
        }
        self.owned.cleanup()
    }
}

#[derive(Clone, Debug)]
pub enum CreationProgress {
    Message(String),
    Output(String),
}

pub fn create(
    environment: &TemplateEnvironment,
    catalog: &TemplateCatalog,
    request: &CreateRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(CreationProgress),
) -> Result<CreatedProjects, String> {
    create_with(environment, catalog, request, cancel, progress, &mut command::run)
}

pub(super) fn create_with(
    environment: &TemplateEnvironment,
    catalog: &TemplateCatalog,
    request: &CreateRequest,
    cancel: &AtomicBool,
    progress: &mut dyn FnMut(CreationProgress),
    run: &mut impl FnMut(
        std::process::Command,
        Duration,
        &AtomicBool,
        &mut dyn FnMut(&str),
    ) -> Result<command::CommandResult, String>,
) -> Result<CreatedProjects, String> {
    let plan = plan(request, catalog)?;
    if let Some(target) = &request.target {
        let solution = crate::solution::read(&target.solution)
            .ok_or_else(|| format!("{} is no longer a readable solution", target.solution.display()))?;
        if solution.projects().iter().any(|p| p.name.eq_ignore_ascii_case(request.project_name.trim())) {
            return Err(format!("Project {} already exists in the solution", request.project_name.trim()));
        }
        fn has_folder(entries: &[crate::solution::SolutionEntry], wanted: &str) -> bool {
            entries.iter().any(|e|matches!(e,crate::solution::SolutionEntry::Folder{path,children,..} if path==wanted || has_folder(children,wanted)))
        }
        if let Some(folder) = &target.folder
            && !has_folder(&solution.entries, folder)
        {
            return Err(format!("Solution folder {folder} no longer exists"));
        }
    }
    if cancel.load(Ordering::Acquire) {
        return Err("Cancelled".into());
    }
    let mut owned = OwnedOutput::new(&plan.root)?;
    let result = (|| {
        let mut invoke = |command, message: String| {
            progress(CreationProgress::Message(message));
            if cancel.load(Ordering::Acquire) {
                return Err("Cancelled".to_string());
            }
            run(command, Duration::from_secs(600), cancel, &mut |s| progress(CreationProgress::Output(s.into())))
        };
        let solution_args = || {
            let mut args: Vec<OsString> = vec![
                "sln".into(),
                "--name".into(),
                request.solution_name.trim().into(),
                "--output".into(),
                plan.root.as_os_str().into(),
            ];
            if catalog.solution_formats.len() > 1 {
                args.extend(["--format".into(), request.solution_format.clone().into()]);
            }
            args
        };
        if plan.solution.is_some() && !request.template.is_solution {
            let made = invoke(
                environment.new_command(&solution_args(), &plan.root),
                format!("Creating the solution {}…", request.solution_name.trim()),
            )?;
            if !made.status.success() {
                return Err(format!("The solution could not be created: {}", made.said()));
            }
            owned.capture()?;
        }
        let made = invoke(
            environment.new_command(&arguments(request, Some(&plan.project_folder))?, &plan.root),
            format!("Creating {} from {}…", request.name_given(), request.template.name),
        )?;
        if !made.status.success() {
            return Err(format!("{} could not be created: {}", request.name_given(), made.said()));
        }
        owned.capture()?;
        let mut projects = projects_in(&plan.project_folder, request.name_given())?;
        if projects.is_empty() {
            return Err(format!(
                "dotnet new {} made no project in {}",
                request.template.short_name(),
                plan.project_folder.display()
            ));
        }
        developer_tools::take_out(&plan.project_folder)?;
        owned.capture()?;
        let mut warnings = Vec::new();
        if request.variant().skip_restore_option.is_some() {
            for project in &projects {
                let args = vec!["restore".into(), project.as_os_str().into()];
                let restored = invoke(
                    environment.command(&args, &plan.root),
                    format!("Restoring {}…", project.file_stem().unwrap_or_default().to_string_lossy()),
                )?;
                if !restored.status.success() {
                    warnings.push(format!("{} could not be restored: {}", project.display(), restored.said()));
                }
            }
        }
        let solution = if request.template.is_solution {
            if let Some(solution) = solution_in(&plan.root, request.solution_name.trim())? {
                Some(solution)
            } else {
                let made = invoke(
                    environment.new_command(&solution_args(), &plan.root),
                    format!("Creating the solution {}…", request.solution_name.trim()),
                )?;
                if !made.status.success() {
                    return Err(format!("The solution could not be created: {}", made.said()));
                }
                owned.capture()?;
                plan.solution.clone()
            }
        } else {
            plan.solution.clone()
        };
        if cancel.load(Ordering::Acquire) {
            return Err("Cancelled".into());
        }
        owned.capture()?;
        for project in &mut projects {
            *project = crate::paths::canonical(project);
        }
        Ok((solution.map(|s| crate::paths::canonical(&s)), projects, warnings))
    })();
    match result {
        Ok((solution, projects, warnings)) => {
            Ok(CreatedProjects { solution, projects, warnings, owned: Arc::new(owned) })
        }
        Err(why) => {
            owned.kept.store(true, Ordering::Release);
            match owned.cleanup() {
                Ok(()) => Err(why),
                Err(cleanup) => Err(format!("{why}; {cleanup}")),
            }
        }
    }
}

pub(crate) fn walk(folder: &Path, include_build: bool, files: &mut Vec<PathBuf>) -> Result<(), String> {
    for entry in std::fs::read_dir(folder).map_err(|e| format!("{}: {e}", folder.display()))? {
        let entry = entry.map_err(|e| e.to_string())?;
        let kind = entry.file_type().map_err(|e| e.to_string())?;
        if kind.is_symlink() {
            continue;
        }
        if kind.is_dir() {
            if include_build || !matches!(entry.file_name().to_str(), Some("bin" | "obj")) {
                walk(&entry.path(), include_build, files)?;
            }
        } else if kind.is_file() {
            files.push(entry.path());
        }
    }
    Ok(())
}

fn projects_in(folder: &Path, name: &str) -> Result<Vec<PathBuf>, String> {
    let mut files = Vec::new();
    walk(folder, false, &mut files)?;
    files.retain(|p| {
        p.extension().is_some_and(|e| ["csproj", "fsproj", "vbproj"].iter().any(|s| e.eq_ignore_ascii_case(s)))
    });
    files.sort_by(|a, b| {
        let named = |p: &Path| p.file_stem().is_some_and(|n| n.eq_ignore_ascii_case(name));
        named(b).cmp(&named(a)).then(a.cmp(b))
    });
    Ok(files)
}

fn solution_in(root: &Path, name: &str) -> Result<Option<PathBuf>, String> {
    let mut solutions = Vec::new();
    for entry in std::fs::read_dir(root).map_err(|e| e.to_string())? {
        let path = entry.map_err(|e| e.to_string())?.path();
        if path.is_file()
            && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("sln") || e.eq_ignore_ascii_case("slnx"))
        {
            solutions.push(path);
        }
    }
    solutions.sort_by(|a, b| {
        let named = |p: &Path| p.file_stem().is_some_and(|n| n.eq_ignore_ascii_case(name));
        named(b).cmp(&named(a)).then(a.cmp(b))
    });
    Ok(solutions.into_iter().next())
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    #[cfg(unix)]
    dev: u64,
    #[cfg(unix)]
    ino: u64,
    #[cfg(not(unix))]
    created: Option<std::time::SystemTime>,
}
fn identity(meta: &std::fs::Metadata) -> Identity {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Identity { dev: meta.dev(), ino: meta.ino() }
    }
    #[cfg(not(unix))]
    {
        Identity { created: meta.created().ok() }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileMark {
    identity: Identity,
    len: u64,
    modified: Option<std::time::SystemTime>,
    hash: u64,
}
fn mark(path: &Path) -> Result<FileMark, String> {
    use std::hash::{Hash, Hasher};
    let meta = std::fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    let mut hash = std::hash::DefaultHasher::new();
    if meta.file_type().is_symlink() {
        std::fs::read_link(path).map_err(|e| e.to_string())?.hash(&mut hash);
    } else {
        std::fs::read(path).map_err(|e| e.to_string())?.hash(&mut hash);
    }
    Ok(FileMark { identity: identity(&meta), len: meta.len(), modified: meta.modified().ok(), hash: hash.finish() })
}

#[derive(Debug)]
struct OwnedOutput {
    root: PathBuf,
    existed: bool,
    root_identity: Identity,
    files: BTreeMap<PathBuf, FileMark>,
    dirs: BTreeMap<PathBuf, Identity>,
    kept: AtomicBool,
}
impl OwnedOutput {
    fn new(root: &Path) -> Result<Self, String> {
        let existed = root.exists();
        if !existed {
            std::fs::create_dir_all(root.parent().ok_or("The destination has no parent")?)
                .map_err(|e| e.to_string())?;
            std::fs::create_dir(root).map_err(|e| format!("{}: {e}", root.display()))?;
        }
        let meta = std::fs::symlink_metadata(root).map_err(|e| e.to_string())?;
        if !meta.is_dir()
            || meta.file_type().is_symlink()
            || std::fs::read_dir(root).map_err(|e| e.to_string())?.next().is_some()
        {
            return Err(format!("{} is no longer an empty directory", root.display()));
        }
        let root_identity = identity(&meta);
        Ok(Self {
            root: root.into(),
            existed,
            dirs: BTreeMap::from([(root.into(), root_identity.clone())]),
            root_identity,
            files: Default::default(),
            kept: AtomicBool::new(false),
        })
    }
    fn same_root(&self) -> bool {
        std::fs::symlink_metadata(&self.root)
            .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && identity(&m) == self.root_identity)
    }
    fn capture(&mut self) -> Result<(), String> {
        if !self.same_root() {
            return Err(format!("The destination {} changed during creation", self.root.display()));
        }
        fn visit(
            folder: &Path,
            files: &mut BTreeMap<PathBuf, FileMark>,
            dirs: &mut BTreeMap<PathBuf, Identity>,
        ) -> Result<(), String> {
            for e in std::fs::read_dir(folder).map_err(|e| e.to_string())? {
                let e = e.map_err(|e| e.to_string())?;
                let path = e.path();
                let meta = std::fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
                if meta.is_dir() && !meta.file_type().is_symlink() {
                    dirs.insert(path.clone(), identity(&meta));
                    visit(&path, files, dirs)?;
                } else {
                    files.insert(path.clone(), mark(&path)?);
                }
            }
            Ok(())
        }
        visit(&self.root, &mut self.files, &mut self.dirs)
    }
    fn cleanup(&self) -> Result<(), String> {
        if !self.root.exists() {
            return Ok(());
        }
        let left = || format!("Cleanup left {}: it contains changed or unowned output", self.root.display());
        if !self.same_root() {
            return Err(left());
        }
        for (path, before) in &self.files {
            let parents_match =
                path.parent().into_iter().flat_map(Path::ancestors).take_while(|p| p.starts_with(&self.root)).all(
                    |p| {
                        self.dirs.get(p).is_some_and(|id| {
                            std::fs::symlink_metadata(p)
                                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && identity(&m) == *id)
                        })
                    },
                );
            if parents_match && mark(path).is_ok_and(|now| now == *before) {
                let _ = std::fs::remove_file(path);
            }
        }
        let mut dirs: Vec<_> = self.dirs.iter().filter(|(p, _)| **p != self.root).collect();
        dirs.sort_by_key(|(p, _)| std::cmp::Reverse(p.components().count()));
        for (path, id) in dirs {
            if std::fs::symlink_metadata(path)
                .is_ok_and(|m| m.is_dir() && !m.file_type().is_symlink() && identity(&m) == *id)
            {
                let _ = std::fs::remove_dir(path);
            }
        }
        if !self.existed {
            std::fs::remove_dir(&self.root).map_err(|_| left())?;
        } else if std::fs::read_dir(&self.root).map_err(|_| left())?.next().is_some() {
            return Err(left());
        }
        Ok(())
    }
}
impl Drop for OwnedOutput {
    fn drop(&mut self) {
        if !self.kept.load(Ordering::Acquire)
            && let Err(why) = self.cleanup()
        {
            log::warn!("{why}");
        }
    }
}
