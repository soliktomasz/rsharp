//! Rider's Go to popups over the window: Go to File and Recent Files by name, Go to Type and Go to Symbol as Roslyn
//! finds them, Search Everywhere over all of those, Recent Locations, and Find in Files (with Replace in Files) in a
//! scope and a file mask, each place listed as the query is typed and Enter going there.

use crate::assets::{file_icon, icon};
use crate::field::{Changed, TextField};
use crate::find_action::{Choose, Dismiss, SelectNext, SelectPrevious};
use crate::language::ask;
use crate::workspace::{Hit, all_files, find_in_files, fits_mask};
use crate::{fuzzy, theme};
use gpui::{App, ClickEvent, Context, Entity, EventEmitter, FocusHandle, Focusable, Task, Window, div, prelude::*, px};
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
    /// Search Everywhere: types, files and symbols at once, in that order, as Rider's All tab lists them.
    // shortcut: actions are left to Find Action; add them once a person looks for one here.
    Everything,
    /// The places left and edited, the latest first.
    RecentLocations,
    FindInFiles,
    /// Find in Files with a replacement and Replace All.
    ReplaceInFiles,
    /// File Structure: the shown file's outline, given whole, filtered as typed with each match's containers kept.
    FileStructure,
}

impl Kind {
    fn placeholder(self) -> &'static str {
        match self {
            Kind::File => "Type a file name",
            Kind::RecentFiles => "Recent files",
            Kind::Type => "Type a type name",
            Kind::Symbol => "Type a symbol name",
            Kind::Everything => "Search everywhere",
            Kind::RecentLocations => "Recent locations",
            Kind::FindInFiles | Kind::ReplaceInFiles => "Find in files",
            Kind::FileStructure => "Filter by name",
        }
    }

    fn searches(self) -> bool {
        matches!(self, Kind::FindInFiles | Kind::ReplaceInFiles)
    }

    /// Whether it looks among the folder's files, walked as it opens.
    fn walks(self) -> bool {
        matches!(self, Kind::File | Kind::Everything | Kind::FindInFiles | Kind::ReplaceInFiles)
    }
}

/// Where Find in Files looks: the whole folder, or one directory (the shown file's, with its subdirectories).
#[derive(Clone, PartialEq, Debug)]
pub enum Scope {
    Solution,
    Directory(PathBuf),
}

/// A place listed: what it is called, what is beside it (its folder, its container, its line), and where it is (a
/// line and UTF-16 column, from 0, or none for a file).
#[derive(Clone, Debug, Default)]
pub struct Place {
    pub name: String,
    pub detail: String,
    pub path: PathBuf,
    pub at: Option<(usize, usize)>,
    /// File Structure's indent, a level per container.
    pub depth: usize,
    /// The icon in place of the file's: a symbol's kind.
    pub icon: Option<&'static str>,
}

pub enum Ended {
    Chosen(Place),
    /// Replace All: what to find, what to put in its place, and the files it was found in.
    ReplaceAll {
        query: String,
        replacement: String,
        files: Vec<PathBuf>,
    },
    Dismissed,
}

/// What the popup is opened with: the files it looks among (none walks the folder), the places it lists (Recent
/// Locations), the open files' text, the shown file's directory, and Roslyn.
#[derive(Default)]
pub struct Given {
    pub files: Vec<PathBuf>,
    pub places: Vec<Place>,
    pub open: HashMap<PathBuf, String>,
    pub here: Option<PathBuf>,
    pub server: Option<Arc<LanguageServer>>,
    /// The title above the field (File Structure's, naming the file).
    pub title: Option<String>,
    /// File Structure's outline of `here_file`, and the line the caret is on, whose innermost symbol it opens on.
    pub outline: Vec<crate::file_structure::OutlineRow>,
    pub here_file: Option<PathBuf>,
    pub caret_line: Option<usize>,
}

pub struct GoToPopup {
    kind: Kind,
    field: Entity<TextField>,
    /// Find in Files' file mask and Replace in Files' replacement.
    mask: Entity<TextField>,
    replacement: Entity<TextField>,
    scope: Scope,
    /// The directory the Directory scope offers: the shown file's.
    here: Option<PathBuf>,
    folder: PathBuf,
    /// The files the popup looks among (the folder's, or the recent ones), and the open files' text for Find in Files.
    files: Arc<Vec<PathBuf>>,
    open: Arc<HashMap<PathBuf, String>>,
    /// Recent Locations' places, the latest first.
    given: Vec<Place>,
    server: Option<Arc<LanguageServer>>,
    title: Option<String>,
    outline: Vec<crate::file_structure::OutlineRow>,
    here_file: PathBuf,
    places: Vec<Place>,
    selected: usize,
    asking: Option<Task<()>>,
    /// The folder's walk, apart from `asking` so a query typed meanwhile does not cancel it.
    walking: Option<Task<()>>,
}

impl EventEmitter<Ended> for GoToPopup {}

/// Symbol kinds that are types, by the protocol's numbers: class, enum, interface, struct.
const TYPE_KINDS: &[u64] = &[5, 10, 11, 23];

impl GoToPopup {
    pub fn new(kind: Kind, folder: PathBuf, given: Given, cx: &mut Context<Self>) -> Self {
        let field = cx.new(|cx| TextField::new(kind.placeholder(), cx));
        let mask = cx.new(|cx| TextField::new("File mask: *.cs, *.xaml", cx));
        let replacement = cx.new(|cx| TextField::new("Replace with", cx));
        for f in [&field, &mask] {
            cx.subscribe(f, |popup, _, _: &Changed, cx| popup.query_changed(cx)).detach();
        }
        let mut popup = Self {
            kind,
            field,
            mask,
            replacement,
            scope: Scope::Solution,
            here: given.here,
            folder,
            files: Arc::new(given.files),
            open: Arc::new(given.open),
            given: given.places,
            server: given.server,
            title: given.title,
            outline: given.outline,
            here_file: given.here_file.unwrap_or_default(),
            places: Vec::new(),
            selected: 0,
            asking: None,
            walking: None,
        };
        if popup.files.is_empty() && kind.walks() {
            let folder = popup.folder.clone();
            popup.walking = Some(cx.spawn(async move |this, cx| {
                let files = cx.background_executor().spawn(async move { all_files(&folder, MOST_FILES) }).await;
                let _ = this.update(cx, |popup, cx| {
                    popup.files = Arc::new(files);
                    popup.query_changed(cx);
                });
            }));
        } else {
            popup.query_changed(cx);
        }
        // Rider's File Structure opens on the innermost symbol the caret is in.
        if let Some(line) = given.caret_line.filter(|_| kind == Kind::FileStructure) {
            let inside =
                |row: &crate::file_structure::OutlineRow| row.range.start.line <= line && line <= row.range.end.line;
            popup.selected = popup.outline.iter().rposition(inside).unwrap_or(0);
        }
        popup
    }

    /// The title over the field, when the popup has one.
    pub fn title(&self) -> Option<String> {
        self.title.clone()
    }

    /// The name of the row Enter would go to.
    pub fn chosen_name(&self) -> Option<String> {
        self.places.get(self.selected).map(|place| place.name.clone())
    }

    /// What the popup lists, as "name detail", as a person reads it.
    pub fn listed(&self) -> Vec<String> {
        self.places.iter().map(|p| format!("{} {}", p.name, p.detail).trim().to_string()).collect()
    }

    /// Find in Files' scope chosen, as its tab is clicked.
    pub fn set_scope(&mut self, scope: Scope, cx: &mut Context<Self>) {
        self.scope = scope;
        self.query_changed(cx);
    }

    pub fn set_mask(&mut self, mask: &str, cx: &mut Context<Self>) {
        self.mask.update(cx, |f, cx| f.set_text(mask, cx));
    }

    pub fn set_replacement(&mut self, replacement: &str, cx: &mut Context<Self>) {
        self.replacement.update(cx, |f, cx| f.set_text(replacement, cx));
    }

    fn relative(&self, path: &Path) -> String {
        let folder = path.parent().unwrap_or(path);
        folder.strip_prefix(&self.folder).unwrap_or(folder).to_string_lossy().into_owned()
    }

    fn file_place(&self, path: &Path) -> Place {
        let name = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        Place { name, detail: self.relative(path), path: path.to_path_buf(), ..Default::default() }
    }

    /// The files whose names fit the query, best first; with `keep_order` the given order (the latest first) kept for
    /// an empty query.
    fn files_matching(&self, query: &str, keep_order: bool) -> Vec<Place> {
        let mut scored: Vec<(i32, usize)> = self
            .files
            .iter()
            .enumerate()
            .filter_map(|(i, p)| fuzzy::score(query, &p.file_name()?.to_string_lossy()).map(|s| (s, i)))
            .collect();
        if !keep_order || !query.is_empty() {
            scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
        }
        scored.into_iter().take(MOST).map(|(_, i)| self.file_place(&self.files[i])).collect()
    }

    /// The files Find in Files looks in: its scope's, those fitting the mask.
    fn searched_files(&self, cx: &App) -> Vec<PathBuf> {
        let mask = self.mask.read(cx).text();
        self.files
            .iter()
            .filter(|p| match &self.scope {
                Scope::Solution => true,
                Scope::Directory(folder) => p.starts_with(folder),
            })
            .filter(|p| fits_mask(&p.file_name().unwrap_or_default().to_string_lossy(), &mask))
            .cloned()
            .collect()
    }

    fn query_changed(&mut self, cx: &mut Context<Self>) {
        let query = self.field.read(cx).text();
        self.selected = 0;
        match self.kind {
            Kind::FileStructure => {
                let kept = crate::file_structure::filter(&self.outline, &query);
                self.places = kept
                    .iter()
                    .map(|&i| {
                        let row = &self.outline[i];
                        let (name, kind) = row.label();
                        Place {
                            name: name.to_string(),
                            detail: kind.to_string(),
                            path: self.here_file.clone(),
                            at: Some((row.selection.start.line, row.selection.start.character)),
                            depth: row.depth,
                            icon: Some(crate::file_structure::kind_icon(row.kind)),
                            ..Default::default()
                        }
                    })
                    .collect();
                // The first row that matches itself, not one kept as a match's container.
                let lowered = query.trim().to_lowercase();
                self.selected = self
                    .places
                    .iter()
                    .position(|p| lowered.is_empty() || p.name.to_lowercase().contains(&lowered))
                    .or_else(|| self.places.iter().position(|p| fuzzy::score(query.trim(), &p.name).is_some()))
                    .unwrap_or(0);
            }
            Kind::File => self.places = self.files_matching(&query, false),
            Kind::RecentFiles => self.places = self.files_matching(&query, true),
            Kind::RecentLocations => {
                self.places = self
                    .given
                    .iter()
                    .filter(|p| fuzzy::score(&query, &format!("{} {}", p.detail, p.name)).is_some())
                    .cloned()
                    .collect();
            }
            Kind::Type | Kind::Symbol | Kind::Everything => {
                // Search Everywhere lists the files at once; the server's types go before them and its other symbols
                // after them as they come.
                let files = if self.kind == Kind::Everything { self.files_matching(&query, false) } else { Vec::new() };
                self.places = files.clone();
                let Some(server) = self.server.clone().filter(|_| !query.is_empty()) else {
                    self.asking = None;
                    cx.notify();
                    return;
                };
                let kind = self.kind;
                self.asking = Some(cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(Duration::from_millis(100)).await;
                    let Ok(answer) =
                        this.update(cx, |_, cx| ask(&server, "workspace/symbol", json!({ "query": query }), cx))
                    else {
                        return;
                    };
                    let found = answer.await.map(|a| rsharp_lsp::symbols(&a)).unwrap_or_default();
                    let _ = this.update(cx, |popup, cx| {
                        let mut scored: Vec<(i32, bool, Place)> = found
                            .into_iter()
                            .filter(|s| kind != Kind::Type || TYPE_KINDS.contains(&s.kind))
                            .filter_map(|s| {
                                let score = fuzzy::score(&query, &s.name)?;
                                let file = s.location.path.file_name()?.to_string_lossy().into_owned();
                                let detail =
                                    if s.container.is_empty() { file } else { format!("{} ({file})", s.container) };
                                let at = (s.location.span.start.line, s.location.span.start.character);
                                let place = Place {
                                    name: s.name,
                                    detail,
                                    path: s.location.path,
                                    at: Some(at),
                                    ..Default::default()
                                };
                                Some((score, TYPE_KINDS.contains(&s.kind), place))
                            })
                            .collect();
                        scored.sort_by(|a, b| b.0.cmp(&a.0));
                        let (types, members): (Vec<_>, Vec<_>) =
                            scored.into_iter().partition(|(_, is_type, _)| *is_type);
                        popup.places = types
                            .into_iter()
                            .map(|(_, _, p)| p)
                            .chain(files)
                            .chain(members.into_iter().map(|(_, _, p)| p))
                            .take(MOST)
                            .collect();
                        popup.selected = 0;
                        cx.notify();
                    });
                }));
            }
            Kind::FindInFiles | Kind::ReplaceInFiles => {
                let (files, open) = (self.searched_files(cx), self.open.clone());
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
                                Place {
                                    name: h.text,
                                    detail,
                                    path: h.path,
                                    at: Some((h.line, h.column)),
                                    ..Default::default()
                                }
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

    /// Replace All: every match in the files searched, found again for the query as it stands (the list is capped and
    /// may lag the typing), given to the window to replace.
    pub fn replace_all(&mut self, cx: &mut Context<Self>) {
        let query = self.field.read(cx).text();
        if query.is_empty() {
            return;
        }
        let replacement = self.replacement.read(cx).text();
        let (searched, open) = (self.searched_files(cx), self.open.clone());
        self.asking = Some(cx.spawn(async move |this, cx| {
            let found = query.clone();
            let hits = cx
                .background_executor()
                .spawn(async move { find_in_files(&searched, &open, &found, usize::MAX) })
                .await;
            let mut files: Vec<PathBuf> = hits.into_iter().map(|h| h.path).collect();
            files.dedup();
            let _ = this.update(cx, |_, cx| cx.emit(Ended::ReplaceAll { query, replacement, files }));
        }));
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
        let code = self.kind.searches() || self.kind == Kind::RecentLocations;
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
                    .pl(px(8. + place.depth as f32 * 18.))
                    .pr(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .when(row == self.selected, |d| d.bg(theme::selection()))
                    .when(row != self.selected, |d| d.hover(|d| d.bg(theme::row_hover())))
                    .child(icon(place.icon.unwrap_or_else(|| file_icon(&place.path.to_string_lossy())), 16.))
                    .child(
                        div()
                            .flex_1()
                            .truncate()
                            .when(code, |d| d.font_family(theme::code_font()))
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
        let chip = |id: &'static str, title: String, chosen: bool| {
            div()
                .id(id)
                .debug_selector(move || format!("scope: {id}"))
                .h(px(24.))
                .px(px(8.))
                .rounded(px(4.))
                .flex()
                .items_center()
                .when(chosen, |d| d.bg(theme::tab_selected()))
                .when(!chosen, |d| d.text_color(theme::text_muted()).hover(|d| d.bg(theme::button_hover())))
                .child(title)
        };
        let scopes = self.kind.searches().then(|| {
            let directory = self.here.clone();
            div()
                .h(px(32.))
                .px(px(10.))
                .flex()
                .items_center()
                .gap(px(4.))
                .child(
                    chip("solution", "Solution".into(), self.scope == Scope::Solution)
                        .on_click(cx.listener(|p, _: &ClickEvent, _, cx| p.set_scope(Scope::Solution, cx))),
                )
                .children(directory.map(|dir| {
                    let name = dir.file_name().unwrap_or_default().to_string_lossy().into_owned();
                    let chosen = self.scope == Scope::Directory(dir.clone());
                    chip("directory", format!("Directory: {name}"), chosen).on_click(
                        cx.listener(move |p, _: &ClickEvent, _, cx| p.set_scope(Scope::Directory(dir.clone()), cx)),
                    )
                }))
                .child(div().flex_1())
                .child(
                    div()
                        .w(px(200.))
                        .h(px(24.))
                        .px(px(6.))
                        .rounded(px(4.))
                        .border_1()
                        .border_color(theme::control_border())
                        .child(self.mask.clone()),
                )
        });
        let replacing = (self.kind == Kind::ReplaceInFiles).then(|| {
            div()
                .h(px(36.))
                .px(px(14.))
                .flex()
                .items_center()
                .gap(px(8.))
                .border_b_1()
                .border_color(theme::popup_border())
                .child(div().flex_1().child(self.replacement.clone()))
                .child(
                    div()
                        .id("replace-all")
                        .debug_selector(|| "replace all".into())
                        .h(px(24.))
                        .px(px(10.))
                        .rounded(px(4.))
                        .flex()
                        .items_center()
                        .bg(theme::accent())
                        .text_color(theme::on_accent())
                        .child("Replace All")
                        .on_click(cx.listener(|p, _: &ClickEvent, _, cx| p.replace_all(cx))),
                )
        });
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
            .children(self.title.clone().map(|title| {
                div().h(px(32.)).px(px(14.)).flex().items_center().font_weight(gpui::FontWeight::SEMIBOLD).child(title)
            }))
            .child(
                div()
                    .h(px(40.))
                    .px(px(14.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .border_b_1()
                    .border_color(theme::popup_border())
                    .child(icon("search", 16.))
                    .child(div().flex_1().child(self.field.clone()))
                    .when(self.kind.searches() && count > 0, |d| {
                        d.child(div().text_color(theme::text_secondary()).child(match count {
                            MOST => format!("{MOST}+ matches"),
                            1 => "1 match".to_string(),
                            n => format!("{n} matches"),
                        }))
                    }),
            )
            .children(replacing)
            .children(scopes)
            .child(div().h(px(6.)))
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
