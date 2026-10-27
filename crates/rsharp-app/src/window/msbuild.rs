//! The window's side of the code help in MSBuild files: each open project file, .props or .targets given the folder's
//! Host and the project it is evaluated through.

use super::*;

impl MainWindow {
    /// Gives an MSBuild file's buffer the Host, once there is one.
    pub(super) fn attach_msbuild(&self, buffer: &Entity<Buffer>, cx: &mut Context<Self>) {
        let Some(host) = self.host.clone() else { return };
        let path = buffer.read(cx).path.clone();
        if !rsharp_core::msbuild::is_msbuild(&path) {
            return;
        }
        // shortcut: the SDK's and packages' own MSBuild files (the insight's `installed` folders) open writable; make
        // them read-only, as Passenger's MSBuildFiles.IsInstalled does, once Go to Declaration into them is common.
        let solution = self.tree.as_ref().map(|t| t.solution().path.clone());
        let project = self.msbuild_project(&path);
        buffer.update(cx, |b, cx| b.attach_msbuild(host, project, solution, cx));
    }

    /// Every open MSBuild file given the Host that has just started.
    pub(super) fn attach_msbuild_files(&self, cx: &mut Context<Self>) {
        let buffers: Vec<_> = self.area.editors().map(|e| e.read(cx).buffer.clone()).collect();
        for buffer in buffers {
            self.attach_msbuild(&buffer, cx);
        }
    }

    /// The project an MSBuild file is evaluated through: itself, else the solution's project whose evaluation read
    /// it, else the nearest project above it (a Directory.Build.props's), else the file itself.
    pub(super) fn msbuild_project(&self, file: &Path) -> PathBuf {
        let extension = file.extension().and_then(|e| e.to_str()).unwrap_or_default().to_ascii_lowercase();
        if extension.ends_with("proj") {
            return file.to_path_buf();
        }
        if let Some(tree) = &self.tree {
            let me = rsharp_core::paths::canonical(file);
            for project in tree.solution().projects() {
                if tree
                    .evaluated(&project.path)
                    .is_some_and(|e| e.inputs.iter().any(|i| rsharp_core::paths::canonical(&i.path) == me))
                {
                    return project.path.clone();
                }
            }
        }
        for folder in file.ancestors().skip(1) {
            let mut found: Vec<PathBuf> = std::fs::read_dir(folder)
                .into_iter()
                .flatten()
                .flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.extension().and_then(|e| e.to_str()).is_some_and(|e| e.to_ascii_lowercase().ends_with("proj"))
                })
                .collect();
            found.sort();
            if let Some(project) = found.into_iter().next() {
                return project;
            }
        }
        file.to_path_buf()
    }
}
