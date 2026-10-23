//! RSharp: a Rider-inspired IDE for C# and .NET, written in Rust on gpui.

use gpui::{App, Application, Keystroke};
use rsharp_app::launch::{LaunchOptions, Step};
use rsharp_app::state::store;
use rsharp_app::{assets, keymap, projects};

fn main() {
    let started = std::time::Instant::now();
    let options = match LaunchOptions::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("rsharp: {why}");
            std::process::exit(2);
        }
    };
    let keystrokes: Vec<Keystroke> = match options
        .steps
        .iter()
        .flat_map(|step| match step {
            Step::Key(keys) => keys.split_whitespace().map(str::to_string).collect::<Vec<_>>(),
            Step::Type(text) => {
                text.chars().map(|c| if c == ' ' { "space".to_string() } else { c.to_string() }).collect()
            }
        })
        .map(|key| Keystroke::parse(&key).map_err(|e| format!("--key {key}: {e}")))
        .collect()
    {
        Ok(keystrokes) => keystrokes,
        Err(why) => {
            eprintln!("rsharp: {why}");
            std::process::exit(2);
        }
    };
    if let Some(folder) = &options.mcp {
        rsharp_core::logs::start(&rsharp_core::state::StateStore::default_root().join("logs"));
        let served = rsharp_mcp::serve(folder, std::io::stdin().lock(), std::io::stdout().lock());
        rsharp_core::process::end_all();
        if let Err(error) = served {
            eprintln!("rsharp: MCP: {error}");
            std::process::exit(1);
        }
        return;
    }
    if let Some(path) = options.path.as_ref().filter(|p| !p.exists()) {
        eprintln!("rsharp: {} does not exist", path.display());
        std::process::exit(2);
    }
    let crashed = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        log::error!("RSharp crashed: {info}");
        rsharp_core::process::end_all();
        crashed(info);
    }));

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        assets::load_fonts(cx);
        rsharp_app::init(rsharp_core::state::StateStore::default_root(), cx);
        cx.on_action(|_: &keymap::Quit, cx| cx.quit());
        cx.set_menus(rsharp_app::actions::menus());
        let quitting = store(cx);
        cx.on_app_quit(move |cx| {
            projects::keep_session(cx);
            quitting.flush();
            async { rsharp_core::process::end_all() }
        })
        .detach();
        // With nothing named, what was open at the last quit opens again, as Rider's Reopen projects on startup.
        let reopen = rsharp_app::settings::settings(cx).reopen_projects_on_startup;
        let paths = match &options.path {
            Some(path) => vec![path.clone()],
            None if reopen => projects::to_reopen(cx),
            None => Vec::new(),
        };
        let window = match paths.split_first() {
            None => projects::new_window(cx),
            Some((first, rest)) => {
                for path in rest {
                    projects::open_project(path, cx);
                }
                projects::open_project(first, cx)
            }
        };
        if let (Some(path), Some(file)) = (&options.path, &options.file) {
            let base = if path.is_file() { path.parent().unwrap_or(path) } else { path };
            let file = if file.is_absolute() { file.clone() } else { base.join(file) };
            let caret = options.caret.map(|(line, column)| (line - 1, column - 1));
            let _ = window.update(cx, |main, window, cx| main.open_file(&file, caret, window, cx));
        }
        cx.activate(true);
        if options.bench {
            cx.spawn(async move |cx| rsharp_app::bench::run(window, started, cx).await).detach();
        }
        if !keystrokes.is_empty() {
            let any = window.into();
            cx.spawn(async move |cx| {
                // Keys about the code wait for Roslyn to load the projects, at most a minute.
                for _ in 0..240 {
                    if !window.read_with(cx, |main, _| main.server_busy()).unwrap_or(false) {
                        break;
                    }
                    cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                }
                // Each step waits for what the one before opened (a dialog, a popup) to draw.
                for keystroke in keystrokes {
                    cx.background_executor().timer(std::time::Duration::from_millis(250)).await;
                    let target = cx.update(|cx| cx.active_window().unwrap_or(any)).unwrap_or(any);
                    let _ = target.update(cx, |_, window, cx| window.dispatch_keystroke(keystroke, cx));
                }
            })
            .detach();
        }
    });
}
