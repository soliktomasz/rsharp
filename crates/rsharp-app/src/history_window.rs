//! Rider's Local History of a file, in a window of its own: the file's versions, newest first, and the chosen one's
//! differences from the text now; Revert puts the text back as it was then, as one step Undo takes back.

use crate::buffer::{Buffer, BufferEvent};
use crate::state::store;
use crate::theme;
use gpui::{
    App, AppContext, Bounds, ClickEvent, Context, Entity, SharedString, Task, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowOptions, div, prelude::*, px, size, uniform_list,
};
use rsharp_core::encodings::decode;
use rsharp_core::history::{LocalHistory, Revision};
use rsharp_text::{Document, LineChange, line_diff};
use std::path::PathBuf;

/// The process's Local History, under the state root.
pub fn local_history(cx: &App) -> LocalHistory {
    LocalHistory::new(store(cx).root().join("localHistory"))
}

/// Keeps a file's content as its newest version, off the UI thread.
pub fn record(path: PathBuf, bytes: Vec<u8>, label: Option<&'static str>, cx: &App) {
    let history = local_history(cx);
    cx.background_executor()
        .spawn(async move {
            if let Err(error) = history.record(&path, &bytes, label) {
                log::warn!("Local History could not keep {}: {error}", path.display());
            }
        })
        .detach();
}

pub struct HistoryWindow {
    buffer: Entity<Buffer>,
    revisions: Vec<Revision>,
    selected: Option<usize>,
    /// The chosen version's text, as the editor would hold it.
    version: Option<String>,
    lines: Vec<(LineChange, SharedString)>,
    _loading: Option<Task<()>>,
}

pub fn open(buffer: Entity<Buffer>, cx: &mut App) -> Option<WindowHandle<HistoryWindow>> {
    let name = buffer.read(cx).path.file_name().unwrap_or_default().to_string_lossy().into_owned();
    let bounds = Bounds::centered(None, size(px(1000.), px(680.)), cx);
    cx.open_window(
        WindowOptions {
            window_bounds: Some(WindowBounds::Windowed(bounds)),
            titlebar: Some(TitlebarOptions {
                title: Some(format!("Local History: {name}").into()),
                ..Default::default()
            }),
            ..Default::default()
        },
        |_, cx| {
            cx.new(|cx| {
                cx.subscribe(&buffer, |w: &mut HistoryWindow, _, event: &BufferEvent, cx| {
                    if matches!(event, BufferEvent::Edited { .. }) {
                        w.compare(cx);
                    }
                })
                .detach();
                let mut window = HistoryWindow {
                    buffer,
                    revisions: Vec::new(),
                    selected: None,
                    version: None,
                    lines: Vec::new(),
                    _loading: None,
                };
                window.reload(cx);
                window
            })
        },
    )
    .ok()
}

impl HistoryWindow {
    /// Reads the file's versions again, off the UI thread, and chooses the newest that differs from the text now
    /// (the newest is often the text as just saved, which has nothing to show).
    pub fn reload(&mut self, cx: &mut Context<Self>) {
        let (history, path) = (local_history(cx), self.buffer.read(cx).path.clone());
        let read = cx.background_executor().spawn(async move { history.revisions(&path) });
        self._loading = Some(cx.spawn(async move |this, cx| {
            let mut revisions = read.await;
            revisions.reverse();
            let _ = this.update(cx, |w, cx| {
                w.revisions = revisions;
                let first = (0..w.revisions.len())
                    .find(|i| {
                        w.choose(*i, cx);
                        w.lines.iter().any(|(change, _)| *change != LineChange::Same)
                    })
                    .unwrap_or(0);
                w.choose(first, cx);
            });
        }));
    }

    /// Each version's words, newest first, as the list shows them.
    pub fn listed(&self) -> Vec<String> {
        self.revisions.iter().map(describe).collect()
    }

    /// The lines of the chosen version's differences from the text now.
    pub fn lines(&self) -> Vec<(LineChange, String)> {
        self.lines.iter().map(|(c, l)| (*c, l.to_string())).collect()
    }

    pub fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        self.selected = (index < self.revisions.len()).then_some(index);
        let buffer = self.buffer.read(cx);
        self.version = self.selected.and_then(|i| {
            let bytes = local_history(cx).content(&self.revisions[i]).ok()?;
            // Read as the file is now read, its breaks held as `\n` as the editor holds them.
            let (text, _) = decode(&bytes, Some(buffer.charset));
            Some(Document::new(&text).text().to_string())
        });
        self.compare(cx);
    }

    fn compare(&mut self, cx: &mut Context<Self>) {
        let now = self.buffer.read(cx).document.text().to_string();
        self.lines = match &self.version {
            Some(version) => line_diff(version, &now).into_iter().map(|(c, l)| (c, l.to_string().into())).collect(),
            None => Vec::new(),
        };
        cx.notify();
    }

    /// Revert: the text back as the chosen version had it.
    pub fn revert(&mut self, cx: &mut Context<Self>) {
        if let Some(version) = self.version.clone() {
            self.buffer.update(cx, |b, cx| b.replace_all(version, cx));
        }
    }
}

/// A version's time, as near as says it, and the IDE's word for it if any.
fn describe(revision: &Revision) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let ago = now.saturating_sub(revision.time);
    // shortcut: times are said as how long ago; say the clock time once the app reads the system's time zone.
    let when = match ago {
        0..60 => "Just now".to_string(),
        60..3600 => format!("{} min ago", ago / 60),
        3600..86400 => format!("{} h ago", ago / 3600),
        _ => format!("{} days ago", ago / 86400),
    };
    match &revision.label {
        Some(label) => format!("{when}  {label}"),
        None => when,
    }
}

impl Render for HistoryWindow {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let rows: Vec<_> = self
            .revisions
            .iter()
            .enumerate()
            .map(|(i, revision)| {
                let selected = self.selected == Some(i);
                div()
                    .id(("revision", i))
                    .debug_selector(move || format!("revision {i}"))
                    .h(px(28.))
                    .px(px(10.))
                    .mx(px(6.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .truncate()
                    .when(selected, |d| d.bg(theme::selection()))
                    .when(!selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(describe(revision))
                    .on_click(cx.listener(move |w, _: &ClickEvent, _, cx| w.choose(i, cx)))
            })
            .collect();
        let lines = self.lines.clone();
        let count = lines.len();
        div()
            .size_full()
            .flex()
            .flex_col()
            .bg(theme::island())
            .text_color(theme::text())
            .font_family(theme::UI_FONT)
            .text_size(px(13.))
            .child(
                div()
                    .h(px(40.))
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .px(px(10.))
                    .border_b_1()
                    .border_color(theme::frame())
                    .child(
                        div()
                            .id("revert")
                            .debug_selector(|| "revert".into())
                            .h(px(26.))
                            .px(px(10.))
                            .flex()
                            .items_center()
                            .rounded(px(4.))
                            .border_1()
                            .border_color(theme::control_border())
                            .when(self.version.is_some(), |d| d.hover(|d| d.bg(theme::button_hover())))
                            .when(self.version.is_none(), |d| d.text_color(theme::text_secondary()))
                            .child("Revert")
                            .on_click(cx.listener(|w, _: &ClickEvent, _, cx| w.revert(cx))),
                    ),
            )
            .child(
                div()
                    .flex()
                    .flex_1()
                    .min_h_0()
                    .child(
                        div()
                            .id("revisions")
                            .w(px(260.))
                            .flex_none()
                            .py(px(6.))
                            .border_r_1()
                            .border_color(theme::frame())
                            .overflow_y_scroll()
                            .children(rows)
                            .when(self.revisions.is_empty(), |d| {
                                d.child(div().p(px(12.)).text_color(theme::text_secondary()).child("No history yet"))
                            }),
                    )
                    .child(
                        uniform_list("diff", count, move |range, _, _| {
                            range
                                .map(|i| {
                                    let (change, text) = &lines[i];
                                    let (mark, background) = match change {
                                        LineChange::Same => (" ", None),
                                        LineChange::Removed => ("-", Some(theme::editor::diff_deleted())),
                                        LineChange::Added => ("+", Some(theme::editor::diff_inserted())),
                                    };
                                    div()
                                        .w_full()
                                        .h(px(20.))
                                        .px(px(10.))
                                        .flex()
                                        .gap(px(10.))
                                        .whitespace_nowrap()
                                        .font_family(theme::CODE_FONT)
                                        .when_some(background, |d, bg| d.bg(bg))
                                        .child(div().w(px(10.)).text_color(theme::text_secondary()).child(mark))
                                        .child(text.clone())
                                })
                                .collect()
                        })
                        .flex_1()
                        .h_full(),
                    ),
            )
    }
}
