//! RSharp's shell as a library, so the tests drive the same window the executable opens.

pub mod actions;
pub mod assets;
pub mod bench;
pub mod buffer;
pub mod dock;
pub mod editor;
pub mod editor_area;
pub mod editor_window;
pub mod field;
pub mod find_action;
pub mod find_bar;
pub mod fuzzy;
pub mod goto;
pub mod history_window;
pub mod keymap;
pub mod language;
pub mod launch;
pub mod projects;
pub mod refactor_dialog;
pub mod settings;
pub mod settings_dialog;
pub mod solution_view;
pub mod state;
pub mod theme;
pub mod window;
pub mod workspace;

/// What the app needs before its first window: Rider's keys and the state store at `state_root`.
pub fn init(state_root: std::path::PathBuf, cx: &mut gpui::App) {
    rsharp_core::logs::start(&state_root.join("logs"));
    keymap::bind(cx);
    let store = rsharp_core::state::StateStore::open(state_root);
    cx.set_global(settings::Settings(store.read::<settings::AppSettings>().clamped()));
    cx.set_global(state::Ide { store, servers: true });
    cx.on_action(|_: &keymap::OpenSettings, cx| settings_dialog::open(cx));
    let history = history_window::local_history(cx);
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = history.prune() {
                log::warn!("Local History could not prune: {error}");
            }
        })
        .detach();
}
