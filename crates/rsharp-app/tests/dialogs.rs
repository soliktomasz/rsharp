//! The refactorings that take options, through their dialogs, against the real Roslyn with Passenger's extension:
//! Change Signature, Extract Interface, Move, Pull Members Up and Push Members Down. Skips, saying why, when Roslyn's server is not there.

mod support;

use gpui::{Entity, TestAppContext, VisualTestContext};
use rsharp_app::window::MainWindow;
use std::path::Path;
use std::time::{Duration, Instant};
use support::Run;

const PROJECT: &str = r#"<Project Sdk="Microsoft.NET.Sdk"><PropertyGroup><TargetFramework>net10.0</TargetFramework></PropertyGroup></Project>"#;
const SHAPES: &str = "namespace Demo;\n\npublic class Shape\n{\n}\n\npublic class Circle : Shape\n{\n    public double Radius;\n    public double Area() => 3 * Radius * Radius;\n}\n\npublic class Calculator\n{\n    public int Add(int a, int b) => a + b;\n}\n";

fn wait_for(cx: &mut VisualTestContext, until: impl Fn(&mut VisualTestContext) -> bool) -> bool {
    let deadline = Instant::now() + Duration::from_secs(180);
    while Instant::now() < deadline {
        cx.executor().advance_clock(Duration::from_millis(50));
        cx.run_until_parked();
        if until(cx) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    false
}

fn file(folder: &Path, name: &str) -> String {
    std::fs::read_to_string(folder.join(name)).unwrap_or_default()
}

/// The dialog of `action` opened with the caret at a line and column of a file, asked again while Roslyn, just loaded,
/// cannot describe what is there yet.
fn open(
    main: &Entity<MainWindow>,
    cx: &mut VisualTestContext,
    path: &Path,
    at: (usize, usize),
    action: fn() -> Box<dyn gpui::Action>,
) -> Entity<rsharp_app::refactor_dialog::RefactorDialog> {
    main.update_in(cx, |m, window, cx| m.open_file(path, Some(at), window, cx));
    cx.update(|window, cx| window.dispatch_action(action(), cx));
    let opened = wait_for(cx, |cx| {
        if main.read_with(cx, |m, _| m.refactoring().is_none() && m.said().is_some_and(|s| s.contains("not available")))
        {
            std::thread::sleep(Duration::from_millis(500));
            cx.update(|window, cx| window.dispatch_action(action(), cx));
        }
        main.read_with(cx, |m, _| m.refactoring().is_some())
    });
    assert!(
        opened,
        "the dialog never opened; the status bar says {:?}",
        main.read_with(cx, |m, _| m.said().map(str::to_string))
    );
    main.read_with(cx, |m, _| m.refactoring()).unwrap()
}

/// OK pressed, and the dialog waited out; its problem when it stayed open.
fn accept(
    main: &Entity<MainWindow>,
    dialog: &Entity<rsharp_app::refactor_dialog::RefactorDialog>,
    cx: &mut VisualTestContext,
) {
    dialog.update(cx, |d, cx| d.accept(cx));
    let closed = wait_for(cx, |cx| main.read_with(cx, |m, _| m.refactoring().is_none()));
    assert!(closed, "the refactoring was refused: {:?}", dialog.read_with(cx, |d, _| d.problem().map(str::to_string)));
}

#[gpui::test]
fn the_refactorings_with_options_change_the_code_through_their_dialogs(cx: &mut TestAppContext) {
    if rsharp_lsp::roslyn_server().is_none() {
        eprintln!("skipped: Roslyn's language server is not installed");
        return;
    }
    use rsharp_app::editor::{ChangeSignature, ExtractInterface, MoveType, PullMembersUp, PushMembersDown};
    let run = Run::new("dialogs", &[("Demo.csproj", PROJECT), ("Shapes.cs", SHAPES)]);
    let (main, cx) = run.window(cx);
    cx.update(|_, cx| cx.global_mut::<rsharp_app::state::Ide>().servers = true);
    main.update_in(cx, |m, window, cx| m.open_workspace(&run.folder, window, cx));
    assert!(wait_for(cx, |cx| main.read_with(cx, |m, _| m.server_ready())), "Roslyn never loaded the project");
    let shapes = run.folder.join("Shapes.cs");
    let save = |main: &Entity<MainWindow>, cx: &mut VisualTestContext| {
        cx.simulate_keystrokes("secondary-s");
        let _ = main;
        cx.run_until_parked();
    };

    // Change Signature: the second parameter renamed, its use following.
    let dialog = open(&main, cx, &shapes, (14, 16), || Box::new(ChangeSignature));
    dialog.update(cx, |d, cx| d.set("parameter 1", "second", cx));
    accept(&main, &dialog, cx);
    save(&main, cx);
    assert!(
        file(&run.folder, "Shapes.cs").contains("Add(int a, int second) => a + second"),
        "Change Signature left:\n{}",
        file(&run.folder, "Shapes.cs")
    );

    // Extract Interface: ICalculator in a file of its own, the class implementing it.
    let dialog = open(&main, cx, &shapes, (12, 14), || Box::new(ExtractInterface));
    dialog.update(cx, |d, cx| {
        d.set("name", "ICalculator", cx);
        d.set("file", "ICalculator.cs", cx);
    });
    accept(&main, &dialog, cx);
    save(&main, cx);
    assert!(
        file(&run.folder, "ICalculator.cs").contains("interface ICalculator"),
        "Extract Interface made no interface file"
    );
    assert!(
        file(&run.folder, "Shapes.cs").contains("class Calculator : ICalculator"),
        "the class does not implement the interface"
    );

    // Move: Circle into a file of its own.
    let dialog = open(&main, cx, &shapes, (6, 14), || Box::new(MoveType));
    dialog.update(cx, |d, cx| d.set("file", "Circle.cs", cx));
    accept(&main, &dialog, cx);
    save(&main, cx);
    assert!(file(&run.folder, "Circle.cs").contains("class Circle"), "Move made no Circle.cs");
    assert!(!file(&run.folder, "Shapes.cs").contains("class Circle"), "Move left Circle in Shapes.cs");

    // Pull Members Up: Area into Shape.
    let circle = run.folder.join("Circle.cs");
    let line = file(&run.folder, "Circle.cs").lines().position(|l| l.contains("class Circle")).unwrap();
    let dialog = open(&main, cx, &circle, (line, 14), || Box::new(PullMembersUp));
    dialog.update(cx, |d, cx| d.choose("Area", cx));
    accept(&main, &dialog, cx);
    main.update_in(cx, |m, window, cx| m.open_file(&shapes, None, window, cx));
    save(&main, cx);
    let shape = file(&run.folder, "Shapes.cs");
    assert!(shape.contains("Area()"), "Pull Members Up did not put Area in Shape:\n{shape}");

    // Push Members Down: Area back from Shape into Circle.
    let line = shape.lines().position(|l| l.contains("class Shape")).unwrap();
    let dialog = open(&main, cx, &shapes, (line, 14), || Box::new(PushMembersDown));
    dialog.update(cx, |d, cx| d.choose("Area", cx));
    accept(&main, &dialog, cx);
    for path in [&shapes, &circle] {
        main.update_in(cx, |m, window, cx| m.open_file(path, None, window, cx));
        save(&main, cx);
    }
    assert!(file(&run.folder, "Circle.cs").contains("Area()"), "Push Members Down did not put Area back in Circle");
    assert!(!file(&run.folder, "Shapes.cs").contains("Area()"), "Push Members Down left Area in Shape");
}
