//! Settings as a person changes them: found by searching, clicked, kept, and in force in every editor.

mod support;

use gpui::{Modifiers, TestAppContext, VisualTestContext};
use rsharp_app::keymap::OpenSettings;
use rsharp_app::settings::{AppSettings, settings};
use rsharp_app::settings_dialog::{self, SettingsDialog};
use support::Run;

/// Clicks the middle of the element a dialog tags with `selector`.
fn click(cx: &mut VisualTestContext, selector: &'static str) {
    let bounds = cx.debug_bounds(selector).unwrap_or_else(|| panic!("nothing on screen is {selector}"));
    cx.simulate_click(bounds.center(), Modifiers::default());
    cx.run_until_parked();
}

fn open_settings(main_cx: &mut VisualTestContext) -> VisualTestContext {
    main_cx.dispatch_action(OpenSettings);
    main_cx.run_until_parked();
    let handle = main_cx.update(|_, cx| settings_dialog::window(cx)).expect("Settings did not open");
    let cx = VisualTestContext::from_window(handle, main_cx);
    cx.run_until_parked();
    cx
}

fn dialog<R>(cx: &mut VisualTestContext, read: impl FnOnce(&SettingsDialog, &gpui::App) -> R) -> R {
    let handle =
        cx.update(|_, cx| settings_dialog::window(cx)).expect("Settings is open").downcast::<SettingsDialog>().unwrap();
    handle.read_with(&*cx, |dialog, cx| read(dialog, cx)).unwrap()
}

#[gpui::test]
fn a_setting_found_by_search_and_ticked_off_is_in_force_and_kept(cx: &mut TestAppContext) {
    let run = Run::new("settings-check", &[]);
    let (_, main_cx) = run.window(cx);
    let mut dialog_cx = open_settings(main_cx);

    dialog_cx.simulate_input("line num");
    assert_eq!(dialog(&mut dialog_cx, |d, cx| d.page_names(cx)), ["General"], "searching 'line num' left other pages");
    click(&mut dialog_cx, "check: Show line numbers");
    assert!(dialog_cx.update(|_, cx| settings(cx).show_line_numbers), "a ticked box was in force before OK");
    click(&mut dialog_cx, "button: OK");

    assert!(!main_cx.update(|_, cx| settings(cx).show_line_numbers), "OK did not put the setting in force");
    assert!(main_cx.update(|_, cx| settings_dialog::window(cx)).is_none(), "OK did not close Settings");
    main_cx.update(|_, cx| rsharp_app::state::store(cx).flush());
    let kept: serde_json::Value =
        serde_json::from_slice(&std::fs::read(run.state.join("settings.json")).unwrap()).unwrap();
    assert_eq!(kept["showLineNumbers"], false, "the setting was not written to settings.json");
}

#[gpui::test]
fn a_number_out_of_range_blocks_ok_and_says_why(cx: &mut TestAppContext) {
    let run = Run::new("settings-number", &[]);
    let (_, main_cx) = run.window(cx);
    let mut dialog_cx = open_settings(main_cx);

    dialog_cx.simulate_input("font");
    click(&mut dialog_cx, "field: Size");
    dialog_cx.simulate_keystrokes("secondary-a");
    dialog_cx.simulate_input("200");
    dialog_cx.run_until_parked();
    assert_eq!(
        dialog(&mut dialog_cx, |d, _| d.problem().map(str::to_string)).as_deref(),
        Some("Size must be a number from 6 to 72")
    );
    click(&mut dialog_cx, "button: OK");
    assert!(
        main_cx.update(|_, cx| settings_dialog::window(cx)).is_some(),
        "OK closed Settings over a size out of range"
    );

    click(&mut dialog_cx, "field: Size");
    dialog_cx.simulate_keystrokes("secondary-a");
    dialog_cx.simulate_input("16");
    click(&mut dialog_cx, "button: Apply");
    let in_force = main_cx.update(|_, cx| settings(cx).clone());
    assert_eq!(in_force.editor_font_size, 16., "Apply did not put the size in force");
    assert_eq!(in_force.line_height(), AppSettings { editor_font_size: 16., ..in_force.clone() }.line_height());
    assert!(main_cx.update(|_, cx| settings_dialog::window(cx)).is_some(), "Apply closed Settings");
}
