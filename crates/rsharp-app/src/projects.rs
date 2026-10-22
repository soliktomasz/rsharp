//! The process's project windows, a folder or solution each, and the one way a start, Open, the recent list
//! and the command line open a project: in the window that already has it, else in a Welcome window, else in
//! a new window. Quit keeps every window's project for the next start.

use crate::state::{RecentSolutions, store};
use crate::window::MainWindow;
use crate::workspace::Workspace;
use gpui::{App, AppContext, Bounds, TitlebarOptions, WindowBounds, WindowHandle, WindowOptions, point, px, size};
use rsharp_core::state::StateDocument;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// What was open when RSharp last quit, each as it was opened.
#[derive(Serialize, Deserialize, Default, Clone, Debug, PartialEq)]
#[serde(default, rename_all = "camelCase")]
pub struct Session {
    pub open: Vec<PathBuf>,
}

impl StateDocument for Session {
    const FILE: &'static str = "session.json";
}

pub fn windows(cx: &App) -> Vec<WindowHandle<MainWindow>> {
    cx.windows().into_iter().filter_map(|w| w.downcast::<MainWindow>()).collect()
}

/// A new project window, showing Welcome.
pub fn new_window(cx: &mut App) -> WindowHandle<MainWindow> {
    let offset = px(28. * windows(cx).len() as f32);
    let mut bounds = Bounds::centered(None, size(px(1400.), px(880.)), cx);
    bounds.origin = bounds.origin + point(offset, offset);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some("RSharp".into()),
                appears_transparent: true,
                traffic_light_position: Some(point(px(14.), px(13.))),
            }),
            window_min_size: Some(size(px(640.), px(400.))),
            ..Default::default()
        },
        |window, cx| cx.new(|cx| MainWindow::new(window, cx)),
    )
    .expect("a project window opens")
}

/// Opens a project where Rider would: the window that has it comes forward, a Welcome window takes it, or a
/// new window opens for it. Returns the window it is in.
pub fn open_project(path: &Path, cx: &mut App) -> WindowHandle<MainWindow> {
    let path = rsharp_core::paths::canonical(path);
    let folder = Workspace::open(&path).folder;
    let all = windows(cx);
    if let Some(open) = all.iter().find(|w| w.read(cx).is_ok_and(|m| m.folder() == Some(folder.as_path()))) {
        let _ = open.update(cx, |_, window, _| window.activate_window());
        return *open;
    }
    let welcome = cx
        .active_window()
        .and_then(|w| w.downcast::<MainWindow>())
        .filter(|w| w.read(cx).is_ok_and(|m| !m.has_workspace()))
        .or_else(|| all.into_iter().find(|w| w.read(cx).is_ok_and(|m| !m.has_workspace())));
    let target = welcome.unwrap_or_else(|| new_window(cx));
    let _ = target.update(cx, |main, window, cx| {
        main.open_workspace(&path, window, cx);
        window.activate_window();
    });
    target
}

/// Keeps every window's project as RSharp quits, for the next start to open again.
pub fn keep_session(cx: &App) {
    let open =
        windows(cx).iter().filter_map(|w| w.read(cx).ok().and_then(|m| m.opened_as().map(Path::to_path_buf))).collect();
    let store = store(cx);
    store.write(&Session { open });
    store.flush();
}

/// What a start with nothing named opens: the projects open at the last quit, else the most recent one.
pub fn to_reopen(cx: &App) -> Vec<PathBuf> {
    let store = store(cx);
    let session: Vec<PathBuf> = store.read::<Session>().open.into_iter().filter(|p| p.exists()).collect();
    if !session.is_empty() {
        return session;
    }
    store.read::<RecentSolutions>().existing().next().map(|r| vec![r.path.clone()]).unwrap_or_default()
}
