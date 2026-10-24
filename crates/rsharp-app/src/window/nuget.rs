//! The window's NuGet: its tool window over the solution's projects and the folder's nuget.config files, the
//! projects followed after a change and the sources read again after a nuget.config changes on disk.

use super::*;
use crate::nuget_view::{NuGetView, ProjectPackages};

impl MainWindow {
    pub fn nuget(&self) -> &Entity<NuGetView> {
        &self.nuget
    }

    /// Tells the NuGet window the folder its sources are read from and each project's packages as evaluated.
    pub(super) fn sync_nuget(&mut self, cx: &mut Context<Self>) {
        let root = self.workspace.as_ref().map(|w| {
            w.solution.as_ref().and_then(|s| s.parent().map(PathBuf::from)).unwrap_or_else(|| w.folder.clone())
        });
        let projects = self
            .tree
            .as_ref()
            .map(|tree| {
                tree.solution()
                    .projects()
                    .into_iter()
                    .filter(|p| p.exists)
                    .map(|p| {
                        let mut packages: Vec<(String, Option<String>)> = Vec::new();
                        let evaluated = tree.evaluated(&p.path);
                        for package in evaluated.iter().flat_map(|e| &e.dependencies).flat_map(|d| &d.packages) {
                            if !package.implicit
                                && !packages.iter().any(|(id, _)| id.eq_ignore_ascii_case(&package.name))
                            {
                                packages.push((package.name.clone(), package.version.clone()));
                            }
                        }
                        ProjectPackages { path: p.path.clone(), name: p.name.clone(), packages }
                    })
                    .collect()
            })
            .unwrap_or_default();
        self.nuget.update(cx, |v, cx| {
            v.set_root(root, cx);
            v.set_projects(projects, cx);
        });
    }

    /// A nuget.config written, made or deleted on disk: the NuGet window reads its sources again.
    pub(super) fn package_sources_changed(&mut self, paths: &[PathBuf], cx: &mut Context<Self>) {
        if paths.iter().any(|p| p.file_name().is_some_and(|n| n.eq_ignore_ascii_case("nuget.config"))) {
            self.nuget.update(cx, |v, cx| v.reload_sources(cx));
        }
    }
}
