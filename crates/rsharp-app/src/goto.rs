//! Rider's Go to popups over the window: Go to File and Recent Files by name, Go to Type and Go to Symbol as Roslyn
//! finds them, and Find in Files, each place listed as the query is typed and Enter going there.

use crate::assets::{file_icon, icon};
use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::language::ask;
use crate::workspace::{Hit, all_files, find_in_files};
use crate::{fuzzy, theme};
use gpui::{App, Context, Entity, EventEmitter, FocusHandle, Focusable, Task, Window, div, prelude::*, px};
use rsharp_lsp::LanguageServer;
use serde_json::json;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

const ROW_HEIGHT: f32 = 24.;
const SHOWN: usize = 14;
/// The most places a server's or a search's answer lists.
const MOST: usize = 500;
/// The most files Go to File and Find in Files look among.
// shortcut: the folder is walked each time the popup opens; keep the list once the folder watcher (phase 4) can
// keep it true.
const MOST_FILES: usize = 100_000;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    File,
    RecentFiles,
    Type,
    Symbol,
    FindInFiles,
}

impl Kind {
    fn placeholder(self) -> &'static str {
        match self {
            Kind::File => "Type a file name",
            Kind::RecentFiles => "Recent files",
            Kind::Type => "Type a type name",
            Kind::Symbol => "Type a symbol name",
            Kind::FindInFiles => "Find in files",
        }
    }
}

/// A place listed: what it is called, what is beside it (its folder, its container, its line), and where it is (a
/// line and UTF-16 column, from 0, or none for a file).
#[derive(Clone, Debug)]
pub struct Place {
    pub name: String,
    pub detail: String,
    pub path: PathBuf,
    pub at: Option<(usize, usize)>,
}

pub enum Ended {
    Chosen(Place),
    Dismissed,
}

pub struct GoToPopup {
    kind: Kind,
    field: Entity<TextField>,
    folder: PathBuf,
    /// The files the popup looks among (the folder's, or the recent ones), and the open files' text for Find in Files.
    files: Arc<Vec<PathBuf>>,
    open: Arc<HashMap<PathBuf, String>>,
    server: Option<Arc<LanguageServer>>,
    places: Vec<Place>,
    selected: usize,
    asking: Option<Task<()>>,
}

impl EventEmitter<Ended> for GoToPopup {}

/// Symbol kinds that are types, by the protocol's numbers: class, enum, interface, struct.
const TYPE_KINDS: &[u64] = &[5, 10, 11, 23];

impl GoToPopup {
    pub fn new(
        kind: Kind,
        folder: PathBuf,
        files: Vec<PathBuf>,
        open: HashMap<PathBuf, String>,
        server: Option<Arc<LanguageServer>>,
        cx: &mut Context<Self>,
    ) -> Self {
        let field = cx.new(|cx| TextField::new(kind.placeholder(), cx));
        cx.subscribe(&field, |popup, _, _: &Changed, cx| popup.query_changed(cx)).detach();
        let mut popup = Self {
            kind,
            field,
            folder,
            files: Arc::new(files),
            open: Arc::new(open),
            server,
            places: Vec::new(),
            selected: 0,
            asking: None,
        };
        if popup.files.is_empty() && matches!(kind, Kind::File | Kind::FindInFiles) {
            let folder = popup.folder.clone();
            popup.asking = Some(cx.spawn(async move |this, cx| {
                let files = cx.background_executor().spawn(async move { all_files(&folder, MOST_FILES) }).await;
                let _ = this.update(cx, |popup, cx| {
                    popup.files = Arc::new(files);
                    popup.query_changed(cx);
                });
            }));
        } else {
            popup.query_changed(cx);
        }
        popup
    }

    /// What the popup lists, as "name detail", as a person reads it.
    pub fn listed(&self) -> Vec<String> {
        self.places.iter().map(|p| format!("{} {}", p.name, p.detail).trim().to_string()).collect()
    }

    pub fn set_query(&mut self, query: &str, cx: &mut Context<Self>) {
        self.field.update(cx, |f, cx| f.set_text(query, cx));
    }

    fn relative(&self, path: &Path) -> String {
        let folder = path.parent().unwrap_or(path);
        folder.strip_prefix(&self.folder).unwrap_or(folder).to_string_lossy().into_owned()
    }

    fn file_place(&self, path: &Path) -> Place {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Place { name, detail: self.relative(path), path: path.to_path_buf(), at: None }
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.field.read(cx).text();
        self.selected = 0;
        match self.kind {
            Kind::File | Kind::RecentFiles => {
                let mut scored: Vec<(i32, usize)> = self
                    .files
                    .iter()
                    .enumerate()
                    .filter_map(|(i, p)| fuzzy::score(&query, &p.file_name()?.to_string_lossy()).map(|s| (s, i)))
                    .collect();
                // Recent Files keeps its order (the latest first) among equals; Go to File sorts by score.
                if self.kind == Kind::File || !query.is_empty() {
                    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
                }
                self.places = scored.into_iter().take(MOST).map(|(_, i)| self.file_place(&self.files[i])).collect();
            }
            Kind::Type | Kind::Symbol => {
                let Some(server) = self.server.clone().filter(|_| !query.is_empty()) else {
                    self.places.clear();
                    self.asking = None;
                    cx.notify();
                    return;
                };
                let types_only = self.kind == Kind::Type;
                self.asking = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let Ok(answer) =
                        this.update(cx, |_, cx| ask(&server, "workspace/symbol", json!({ "query": query }), cx))
                    else {
                        return;
                    };
                    let found = answer.await.map(|a| rsharp_lsp::symbols(&a)).unwrap_or_default();
                    let _ = this.update(cx, |popup, cx| {
                        let mut scored: Vec<(i32, Place)> = found
                            .into_iter()
                            .filter(|s| !types_only || TYPE_KINDS.contains(&s.kind))
                            .filter_map(|s| {
                                let score = fuzzy::score(&query, &s.name)?;
                                let file = s.location.path.file_name()?.to_string_lossy().into_owned();
                                let detail =
                                    if s.container.is_empty() { file } else { format!("{} ({file})", s.container) };
                                let at = (s.location.span.start.line, s.location.span.start.character);
                                Some((score, Place { name: s.name, detail, path: s.location.path, at: Some(at) }))
                            })
                            .collect();
                        scored.sort_by(|a, b| b.0.cmp(&a.0));
                        popup.places = scored.into_iter().take(MOST).map(|(_, p)| p).collect();
                        popup.selected = 0;
                        cx.notify();
                    });
                }));
            }
            Kind::FindInFiles => {
                let (files, open) = (self.files.clone(), self.open.clone());
                self.asking = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(150)).await;
                    let hits: Vec<Hit> =
                        cx.background_executor().spawn(async move { find_in_files(&files, &open, &query, MOST) }).await;
                    let _ = this.update(cx, |popup, cx| {
                        popup.places = hits
                            .into_iter()
                            .map(|h| {
                                let file = h.path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                                let detail = format!("{file}:{}", h.line + 1);
                                Place { name: h.text, detail, path: h.path, at: Some((h.line, h.column)) }
                            })
                            .collect();
                        popup.selected = 0;
                        cx.notify();
                    });
                }));
            }
        }
        cx.notify();
    }

    fn choose(&mut self, index: usize, cx: &mut Context<Self>) {
        if let Some(place) = self.places.get(index) {
            cx.emit(Ended::Chosen(place.clone()));
        }
    }
}

impl Focusable for GoToPopup {
    fn focus_handle(&self, cx: &App) -> FocusHandle {
        self.field.focus_handle(cx)
    }
}

impl Render for GoToPopup {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let first = self.selected.saturating_sub(SHOWN - 1);
        let code = self.kind == Kind::FindInFiles;
        let rows: Vec<_> = self
            .places
            .iter()
            .enumerate()
            .skip(first)
            .take(SHOWN)
            .map(|(row, place)| {
                div()
                    .id(("place", row))
                    .debug_selector(move || format!("go to: {row}"))
                    .h(px(ROW_HEIGHT))
                    .mx(px(6.))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(row == self.selected, |d| d.bg(theme::selection()))
                    .when(row != self.selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(icon(file_icon(&place.path.to_string_lossy()), 16.))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .when(code, |d| d.font_family(theme::CODE_FONT))
                            .child(place.name.clone()),
                    )
                    .child(
                        div()
                            .flex_none()
                            .max_w(px(280.))
                            .truncate()
                            .text_color(theme::text_secondary())
                            .child(place.detail.clone()),
                    )
                    .on_click(cx.listener(move |p, _, _, cx| p.choose(row, cx)))
            })
            .collect();
        let count = self.places.len();
        div()
            .key_context("FindAction")
            .w(px(680.))
            .flex()
            .flex_col()
            .pb(px(6.))
            .rounded(px(10.))
            .bg(theme::popup())
            .border_1()
            .border_color(theme::popup_border())
            .shadow_lg()
            .on_action(cx.listener(|p, _: &SelectPrevious, _, cx| {
                p.selected = p.selected.saturating_sub(1);
                cx.notify()
            }))
            .on_action(cx.listener(|p, _: &SelectNext, _, cx| {
                p.selected = (p.selected + 1).min(p.places.len().saturating_sub(1));
                cx.notify()
            }))
            .on_action(cx.listener(|p, _: &Choose, _, cx| {
                let s = p.selected;
                p.choose(s, cx)
            }))
            .on_action(cx.listener(|_, _: &Dismiss, _, cx| cx.emit(Ended::Dismissed)))
            .child(
                div()
                    .h(px(40.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::popup_border())
                    .mb(px(6.))
                    .child(icon("search", 16.))
                    .child(div().flex_1().child(self.field.clone()))
                    .when(self.kind == Kind::FindInFiles && count > 0, |d| {
                        d.child(div().text_color(theme::text_secondary()).child(match count {
                            MOST => format!("{MOST}+ matches"),
                            1 => "1 match".to_string(),
                            n => format!("{n} matches"),
                        }))
                    }),
            )
            .children(rows)
            .when(count == 0, |d| {
                d.child(
                    div()
                        .h(px(ROW_HEIGHT * 2.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .text_color(theme::text_secondary())
                        .child("Nothing found"),
                )
            })
    }
}
