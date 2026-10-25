//! An editor window: a tab dragged out of the main window, or Open Tab in New Window. Its tabs are editors of the
//! same buffers the main window's are, so a file shown in both is one text; the window closes with its last tab.

use crate::assets::{file_icon, icon};
use crate::editor::{self, CodeEditor};
use crate::editor_area::EditorArea;
use crate::theme;
use crate::window::CloseTab;
use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, Entity, Focusable, Pixels, Point, SharedString, TitlebarOptions,
    Window, WindowBounds, WindowHandle, WindowOptions, div, prelude::*, px, size,
};

// shortcut: an editor window has one group of tabs, no splits, find bar or tab menu, and is not kept with the
// folder; give it the main window's editor area once a person works in one for long.
pub struct EditorWindow {
    area: EditorArea,
}

/// Opens a window showing `editor`, its top left at `at` (where a dragged tab was let go).
pub fn open(editor: Entity<CodeEditor>, at: Point<Pixels>, cx: &mut App) -> Option<WindowHandle<EditorWindow>> {
    let title: SharedString =
        editor.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned().into();
    let bounds = Bounds::new(at, size(px(900.), px(640.)));
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions { title: Some(title), ..Default::default() }),
            window_min_size: Some(size(px(320.), px(200.))),
            ..Default::default()
        },
        |window, cx| {
            let focus = editor.focus_handle(cx);
            let view = cx.new(|cx| {
                cx.subscribe_in(&editor, window, |_, editor, _: &editor::ReadOnlyEdit, window, cx| {
                    editor::ask_clear_read_only(editor.clone(), editor::AfterClearing::Edit, window, cx)
                })
                .detach();
                let mut area = EditorArea::default();
                area.add(editor, false);
                area.groups[0].active = Some(0);
                EditorWindow { area }
            });
            window.focus(&focus);
            view
        },
    )
    .ok()
}

impl EditorWindow {
    pub(crate) fn buffers(&self, cx: &App) -> Vec<Entity<crate::buffer::Buffer>> {
        self.area.groups.iter().flat_map(|g| &g.tabs).map(|tab| tab.editor.read(cx).buffer.clone()).collect()
    }

    /// Each tab's file name, as a person reads the tab strip.
    pub fn tabs(&self, cx: &App) -> Vec<String> {
        self.area
            .group()
            .tabs
            .iter()
            .map(|t| t.editor.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned())
            .collect()
    }

    /// Each tab's text as it stands.
    pub fn texts(&self, cx: &App) -> Vec<String> {
        self.area.group().tabs.iter().map(|t| t.editor.read(cx).text().to_string()).collect()
    }

    fn show(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.area.groups[0].active = Some(index);
        window.focus(&self.area.group().tabs[index].editor.focus_handle(cx));
        cx.notify();
    }

    fn close(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        self.area.close(0, index, cx);
        match self.area.active_editor() {
            Some(editor) => window.focus(&editor.focus_handle(cx)),
            None => window.remove_window(),
        }
        cx.notify();
    }
}

impl Render for EditorWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let group = self.area.group();
        let tabs: Vec<_> = group
            .tabs
            .iter()
            .enumerate()
            .map(|(index, tab)| {
                let read = tab.editor.read(cx);
                let name = read.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                let selected = group.active == Some(index);
                let modified = read.is_modified(cx);
                div()
                    .id(("editor-window-tab", index))
                    .h(px(28.))
                    .flex()
                    .items_center()
                    .gap(px(6.))
                    .pl(px(8.))
                    .pr(px(4.))
                    .rounded(px(6.))
                    .border_1()
                    .when(selected, |d| d.bg(theme::tab_selected()).border_color(theme::tab_selected_border()))
                    .when(!selected, |d| d.border_color(gpui::transparent_black()).text_color(theme::text_muted()))
                    .child(icon(file_icon(&name), 16.))
                    .child(name)
                    .child(
                        div()
                            .id(("editor-window-close", index))
                            .size(px(16.))
                            .flex()
                            .items_center()
                            .justify_center()
                            .rounded(px(3.))
                            .hover(|d| d.bg(theme::button_hover()))
                            .child(if modified {
                                div().size(px(7.)).rounded_full().bg(theme::modified()).into_any_element()
                            } else {
                                icon("close", 16.).into_any_element()
                            })
                            .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| {
                                cx.stop_propagation();
                                w.close(index, window, cx)
                            })),
                    )
                    .on_click(cx.listener(move |w, _: &ClickEvent, window, cx| w.show(index, window, cx)))
            })
            .collect();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::island())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .on_action(cx.listener(|w, _: &CloseTab, window, cx| {
                if let Some(active) = w.area.group().active {
                    w.close(active, window, cx)
                }
            }))
            .on_action(cx.listener(|w, _: &editor::Save, window, cx| {
                if let Some(editor) = w.area.active_editor().cloned() {
                    if editor.read(cx).buffer.read(cx).read_only {
                        editor::ask_clear_read_only(editor, editor::AfterClearing::Save, window, cx);
                        return;
                    }
                    // shortcut: a save that fails here is logged, as the window has no status bar to say it in.
                    if let Err(error) = editor.update(cx, |e, cx| e.save(cx)) {
                        log::warn!("Could not save: {error}");
                    }
                }
            }))
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(2.))
                    .px(px(6.))
                    .border_b_1()
                    .border_color(theme::frame())
                    .children(tabs),
            )
            .children(self.area.active_editor().cloned().map(|e| div().flex_1().min_h_0().child(e)))
    }
}
