//! RSharp: a Rider-inspired IDE for C# and .NET, written in Rust on gpui.

mod assets;
mod editor;
mod keymap;
mod launch;
mod theme;
mod window;
mod workspace;

use gpui::{App, Application, Bounds, TitlebarOptions, WindowBounds, WindowOptions, point, prelude::*, px, size};
use launch::LaunchOptions;
use window::MainWindow;

fn main() {
    let options = match LaunchOptions::parse(std::env::args().skip(1)) {
        Ok(options) => options,
        Err(why) => {
            eprintln!("rsharp: {why}");
            std::process::exit(2);
        }
    };
    let crashed = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        rsharp_core::process::end_all();
        crashed(info);
    }));

    Application::new().with_assets(assets::Assets).run(move |cx: &mut App| {
        assets::load_fonts(cx);
        keymap::bind(cx);
        cx.on_action(|_: &keymap::Quit, cx| cx.quit());
        cx.on_app_quit(|_| async { rsharp_core::process::end_all() }).detach();

        let bounds = Bounds::centered(None, size(px(1400.), px(880.)), cx);
        let window = cx
            .open_window(
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
                |_, cx| cx.new(MainWindow::new),
            )
            .expect("the main window opens");
        window
            .update(cx, |main, window, cx| {
                if let Some(path) = &options.path {
                    main.open_workspace(path, window, cx);
                    if let Some(file) = &options.file {
                        let file = if file.is_absolute() { file.clone() } else { path.join(file) };
                        let caret = options.caret.map(|(line, column)| (line - 1, column - 1));
                        main.open_file(&file, caret, window, cx);
                    }
                }
                cx.activate(true);
            })
            .expect("the main window is there");
    });
}
