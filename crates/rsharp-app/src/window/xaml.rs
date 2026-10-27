//! The window's side of the code help in XAML files: each open Avalonia or WPF XAML file given the XAML host and the
//! project it is read against, and the host told what changed on disk.

use super::*;

impl MainWindow {
    /// Gives a XAML file's buffer the XAML host, which starts with the first.
    pub(super) fn attach_xaml(&mut self, buffer: &Entity<Buffer>, cx: &mut Context<Self>) {
        let path = buffer.read(cx).path.clone();
        if !rsharp_lsp::xaml::is_xaml(&path) || !cx.global::<crate::state::Ide>().servers {
            return;
        }
        if self.xaml.is_none() {
            self.xaml = rsharp_lsp::xaml::XamlHost::carried();
        }
        let Some(host) = self.xaml.clone() else { return };
        let project = self.msbuild_project(&path);
        buffer.update(cx, |b, cx| b.attach_xaml(host, project, cx));
    }

    /// Files another program wrote: the projects that hold them read again, a project file's read afresh.
    // shortcut: the watcher passes over obj/, so a restore landing while a XAML file is open is not seen until the
    // project file changes; report project.assets.json apart, as Passenger's watcher does, once restores are run here.
    pub(super) fn xaml_files_changed(&self, paths: &[PathBuf], cx: &mut Context<Self>) {
        let Some(host) = self.xaml.clone() else { return };
        let paths = paths.to_vec();
        cx.background_executor()
            .spawn(async move {
                if let Err(failed) = host.changed(&paths) {
                    log::info!("XAML code help: {failed}");
                }
            })
            .detach();
    }
}
