//! What every window test starts from: a folder of its own and a state store of its own, never the person's.

#![allow(dead_code)]

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::path::PathBuf;

pub struct Run {
    pub folder: PathBuf,
    pub state: PathBuf,
}

impl Run {
    pub fn new(name: &str, files: &[(&str, &str)]) -> Self {
        let base = std::env::temp_dir().join(format!("rsharp-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let folder = base.join("folder");
        std::fs::create_dir_all(&folder).unwrap();
        for (file, text) in files {
            let path = folder.join(file);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        Self { folder: rsharp_core::paths::canonical(&folder), state: base.join("state") }
    }

    /// The app started on this run's store, and a window with nothing open.
    pub fn window<'a>(&self, cx: &'a mut TestAppContext) -> (Entity<MainWindow>, &'a mut VisualTestContext) {
        let state = self.state.clone();
        if !cx.update(|cx| cx.has_global::<rsharp_app::state::Ide>()) {
            cx.update(|cx| {
                rsharp_app::init(state, cx);
                cx.global_mut::<rsharp_app::state::Ide>().servers = false;
                cx.global_mut::<rsharp_app::templates::Templates>().enabled = false;
            });
        }
        cx.add_window_view(MainWindow::new)
    }
}

impl Drop for Run {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(self.folder.parent().unwrap());
    }
}
