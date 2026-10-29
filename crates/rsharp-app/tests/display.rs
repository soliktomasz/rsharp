//! What the editor shows of the text, driven by Rider's keys: folded regions, soft wrap, the bracket pair, the
//! font size and whitespace.

mod support;

use gpui::{TestAppContext, VisualTestContext};
use rsharp_app::editor;
use support::Run;

const MAC: bool = cfg!(target_os = "macos");

fn open<'a>(
    name: &str,
    text: &str,
    caret: (usize, usize),
    cx: &'a mut TestAppContext,
) -> (Run, &'a mut VisualTestContext) {
    open_named(name, "Program.cs", text, caret, cx)
}

fn open_named<'a>(
    name: &str,
    file: &str,
    text: &str,
    caret: (usize, usize),
    cx: &'a mut TestAppContext,
) -> (Run, &'a mut VisualTestContext) {
    let run = Run::new(name, &[(file, text)]);
    let (main, cx) = run.window(cx);
    main.update_in(cx, |main, window, cx| {
        main.open_workspace(&run.folder, window, cx);
        main.open_file(&run.folder.join(file), Some(caret), window, cx);
    });
    cx.run_until_parked();
    (run, cx)
}

/// Types at the caret and saves, and gives back the file.
fn type_and_save(run: &Run, cx: &mut VisualTestContext, typed: &str) -> String {
    type_and_save_in(run, "Program.cs", cx, typed)
}

fn type_and_save_in(run: &Run, file: &str, cx: &mut VisualTestContext, typed: &str) -> String {
    cx.simulate_input(typed);
    cx.simulate_keystrokes("secondary-s");
    cx.run_until_parked();
    std::fs::read_to_string(run.folder.join(file)).unwrap()
}

const CLASS: &str = "class C\n{\n    void M()\n    {\n        int x;\n    }\n}\n// end\n";

#[gpui::test]
fn collapse_all_hides_the_bodies_and_down_steps_over_them(cx: &mut TestAppContext) {
    let (run, cx) = open("fold", CLASS, (0, 0), cx);
    cx.simulate_keystrokes(if MAC { "cmd-shift--" } else { "ctrl-m ctrl-a" });
    cx.simulate_keystrokes("down down");
    assert_eq!(
        type_and_save(&run, cx, "X"),
        "class C\n{\n    void M()\n    {\n        int x;\n    }\n}\nX// end\n",
        "Down went into folded lines instead of over them"
    );

    cx.simulate_keystrokes(if MAC { "cmd-shift-=" } else { "ctrl-m ctrl-x" });
    cx.simulate_keystrokes("up up");
    assert_eq!(
        type_and_save(&run, cx, "Y"),
        "class C\n{\n    void M()\n    {\n        int x;\n Y   }\n}\nX// end\n",
        "Expand All left lines folded"
    );
}

#[gpui::test]
fn an_msbuild_file_folds_by_its_elements_as_its_own_grammar_reads_them(cx: &mut TestAppContext) {
    let project = "<Project>\n  <PropertyGroup>\n    <A>1</A>\n  </PropertyGroup>\n</Project>\n";
    let (run, cx) = open_named("fold-xml", "App.csproj", project, (0, 0), cx);
    cx.simulate_keystrokes(if MAC { "cmd-shift--" } else { "ctrl-m ctrl-a" });
    cx.simulate_keystrokes("down");
    assert_eq!(
        type_and_save_in(&run, "App.csproj", cx, "X"),
        "<Project>\n  <PropertyGroup>\n    <A>1</A>\n  </PropertyGroup>\n</Project>\nX",
        "Collapse All did not fold the project file's root element, so Down went into it"
    );
}

#[gpui::test]
fn collapse_folds_the_region_at_the_caret_and_a_caret_landing_inside_unfolds_it(cx: &mut TestAppContext) {
    let (run, cx) = open("fold-one", CLASS, (3, 4), cx);
    cx.simulate_keystrokes(if MAC { "cmd--" } else { "ctrl-m ctrl-s" });
    cx.simulate_keystrokes("down");
    assert_eq!(
        type_and_save(&run, cx, "X"),
        "class C\n{\n    void M()\n    {\n        int x;\n    }\n}X\n// end\n",
        "Collapse did not fold the method's body, or folded more than it"
    );
    // Find selects what the fold hides, which opens it.
    cx.simulate_keystrokes("secondary-f");
    cx.simulate_input("x;");
    cx.simulate_keystrokes("escape");
    cx.run_until_parked();
    cx.simulate_keystrokes("escape up");
    assert_eq!(
        type_and_save(&run, cx, "Y"),
        "class C\n{\n    void M()\n    {Y\n        int x;\n    }\n}X\n// end\n",
        "the folded body did not open for a caret landing in it"
    );
}

#[gpui::test]
fn with_soft_wrap_on_down_moves_within_a_long_line(cx: &mut TestAppContext) {
    let long = "word ".repeat(200);
    let (run, cx) = open("wrap", &format!("{long}\nend\n"), (0, 0), cx);
    cx.dispatch_action(editor::ToggleSoftWrap);
    cx.run_until_parked();
    cx.simulate_keystrokes("down");
    let saved = type_and_save(&run, cx, "X");
    assert!(saved.ends_with("\nend\n") && saved.contains("X"), "Down left the wrapped line for the next one");
    assert!(!saved.starts_with('X'), "Down did not move at all");
}

#[gpui::test]
fn match_brace_moves_the_caret_to_the_pair(cx: &mut TestAppContext) {
    let (run, cx) = open("brace", "class C { void M(int a, int b) { } }\n", (0, 16), cx);
    cx.simulate_keystrokes(if MAC { "ctrl-m" } else { "ctrl-]" });
    assert_eq!(
        type_and_save(&run, cx, "X"),
        "class C { void M(int a, int bX) { } }\n",
        "the caret did not go to the bracket's pair"
    );
}

#[gpui::test]
fn the_font_size_and_whitespace_change_in_every_editor(cx: &mut TestAppContext) {
    let (_run, cx) = open("zoom", "int x;\n", (0, 0), cx);
    let size = |cx: &mut VisualTestContext| cx.update(|_, cx| rsharp_app::settings::settings(cx).editor_font_size);
    let before = size(cx);
    cx.dispatch_action(editor::IncreaseFontSize);
    cx.dispatch_action(editor::IncreaseFontSize);
    cx.run_until_parked();
    assert_eq!(size(cx), before + 2., "Increase Font Size did not make the editor font larger");
    cx.dispatch_action(editor::ResetFontSize);
    cx.run_until_parked();
    assert_eq!(size(cx), before, "Reset Font Size did not go back to the default");

    cx.dispatch_action(editor::ToggleWhitespace);
    cx.run_until_parked();
    assert!(cx.update(|_, cx| rsharp_app::settings::settings(cx).show_whitespace), "Show Whitespaces stayed off");
}
