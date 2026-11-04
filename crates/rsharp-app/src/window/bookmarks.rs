//! The folder's bookmarks, as Passenger's MainWindow.Bookmarks keeps them: Toggle Bookmark on the caret's line, Next
//! and Previous across files, the Bookmarks window, each open file's gutter, kept with the folder.

use super::*;

impl MainWindow {
    /// The folder's bookmarks as kept, made absolute.
    pub(super) fn load_bookmarks(&mut self, folder: &Path, kept: &rsharp_core::bookmarks::Bookmarks) {
        let mut bookmarks = rsharp_core::bookmarks::Bookmarks::default();
        for mark in kept.all() {
            bookmarks.toggle(&folder.join(&mark.path), mark.line);
        }
        self.bookmarks = bookmarks;
        self.read_bookmark_texts(None);
    }

    /// The lines of closed files' bookmarks, read when the bookmarks or those files change, never while drawing.
    pub(super) fn read_bookmark_texts(&mut self, only: Option<&[PathBuf]>) {
        let mut files: Vec<PathBuf> = self.bookmarks.all().iter().map(|m| m.path.clone()).collect();
        files.dedup();
        if let Some(only) = only {
            files.retain(|f| only.contains(f));
        }
        for file in files {
            let lines =
                std::fs::read_to_string(&file).map(|t| t.lines().map(str::to_string).collect()).unwrap_or_default();
            self.bookmark_texts.insert(file, lines);
        }
        self.bookmark_texts.retain(|file, _| self.bookmarks.all().iter().any(|m| &m.path == file));
    }

    /// The bookmarks to keep with the folder, each path from the folder.
    pub(super) fn kept_bookmarks(&self, folder: &Path) -> rsharp_core::bookmarks::Bookmarks {
        let mut kept = rsharp_core::bookmarks::Bookmarks::default();
        for mark in self.bookmarks.all() {
            kept.toggle(mark.path.strip_prefix(folder).unwrap_or(&mark.path), mark.line);
        }
        kept
    }

    /// A file's bookmarks drawn in its gutter, when it is open.
    pub(super) fn show_bookmarks(&self, path: &Path, cx: &mut App) {
        let Some(buffer) = crate::buffer::find(&path.to_path_buf(), cx) else { return };
        let lines = self.bookmarks.lines_in(&rsharp_core::paths::canonical(path));
        buffer.update(cx, |b, cx| b.set_bookmarks(lines, cx));
    }

    /// A file's bookmarks carried through an edit to it, a line the edit removed taking its bookmark with it.
    pub(super) fn follow_bookmarks(&mut self, path: &Path, changes: &[rsharp_text::Change], cx: &mut Context<Self>) {
        let before = self.bookmarks.lines_in(path);
        if before.is_empty() {
            return;
        }
        self.bookmarks.follow(path, |line| rsharp_text::map_line_kept(changes, line));
        if self.bookmarks.lines_in(path) != before {
            self.read_bookmark_texts(Some(&[path.to_path_buf()]));
            self.show_bookmarks(path, cx);
            self.save_state(cx);
        }
    }

    fn toggle_bookmark(&mut self, cx: &mut Context<Self>) {
        let Some(editor) = self.active_editor() else { return };
        let (path, line) = {
            let editor = editor.read(cx);
            (rsharp_core::paths::canonical(&editor.path), editor.caret().line)
        };
        let added = self.bookmarks.toggle(&path, line);
        self.read_bookmark_texts(Some(&[path.clone()]));
        self.said = Some(if added { "Bookmark added" } else { "Bookmark removed" }.into());
        self.show_bookmarks(&path, cx);
        self.save_state(cx);
        cx.notify();
    }

    fn step_bookmark(&mut self, forward: bool, window: &mut Window, cx: &mut Context<Self>) {
        let here = self.active_editor().map(|e| {
            let e = e.read(cx);
            (rsharp_core::paths::canonical(&e.path), e.caret().line)
        });
        let (path, line) = here.unwrap_or_default();
        let found = if forward { self.bookmarks.next(&path, line) } else { self.bookmarks.previous(&path, line) };
        match found {
            Some(mark) => self.open_file(&mark.path, Some((mark.line, 0)), window, cx),
            None => {
                self.said = Some("No bookmarks".into());
                cx.notify();
            }
        }
    }

    pub(super) fn bookmark_actions<T: InteractiveElement + FluentBuilder>(&self, d: T, cx: &mut Context<Self>) -> T {
        d.when(self.active_editor().is_some(), |d| {
            d.on_action(cx.listener(|w, _: &ToggleBookmark, _, cx| w.toggle_bookmark(cx)))
        })
        .when(!self.bookmarks.all().is_empty(), |d| {
            d.on_action(cx.listener(|w, _: &NextBookmark, window, cx| w.step_bookmark(true, window, cx)))
                .on_action(cx.listener(|w, _: &PreviousBookmark, window, cx| w.step_bookmark(false, window, cx)))
        })
        .on_action(cx.listener(|w, _: &ShowBookmarks, window, cx| {
            if !w.dock.is_open(Tool::Bookmarks) {
                w.toggle_tool(Tool::Bookmarks, window, cx);
            }
        }))
    }

    /// A bookmark's line as it reads now: the open text's, else the line kept from the file.
    fn bookmark_text(&self, mark: &rsharp_core::bookmarks::Bookmark, cx: &App) -> String {
        match crate::buffer::find(&mark.path, cx) {
            Some(buffer) => {
                buffer.read(cx).document.text().get_line(mark.line).map(|l| l.to_string()).unwrap_or_default()
            }
            None => {
                self.bookmark_texts.get(&mark.path).and_then(|lines| lines.get(mark.line)).cloned().unwrap_or_default()
            }
        }
        .trim()
        .to_string()
    }

    /// The Bookmarks window's rows as a person reads them: the file and line, then the line's text.
    pub fn bookmark_rows(&self, cx: &App) -> Vec<String> {
        self.bookmarks
            .all()
            .iter()
            .map(|mark| {
                let file = mark.path.file_name().unwrap_or_default().to_string_lossy();
                format!("{file}:{}  {}", mark.line + 1, self.bookmark_text(mark, cx))
            })
            .collect()
    }

    pub(super) fn bookmarks_content(&self, cx: &mut Context<Self>) -> gpui::AnyElement {
        if self.bookmarks.all().is_empty() {
            return empty_text(if cfg!(target_os = "macos") {
                "No bookmarks. F3 bookmarks the caret's line."
            } else {
                "No bookmarks. Ctrl+K, K bookmarks the caret's line."
            })
            .track_focus(&self.bookmarks_focus)
            .into_any_element();
        }
        div()
            .id("bookmarks")
            .track_focus(&self.bookmarks_focus)
            .flex_1()
            .min_h_0()
            .overflow_y_scroll()
            .px(px(6.))
            .children(self.bookmarks.all().iter().enumerate().map(|(i, mark)| {
                let (path, line) = (mark.path.clone(), mark.line);
                let file = path.file_name().unwrap_or_default().to_string_lossy().into_owned();
                div()
                    .id(("bookmark", i))
                    .debug_selector(move || format!("bookmark: {i}"))
                    .h(px(ROW_HEIGHT))
                    .px(px(8.))
                    .rounded(px(4.))
                    .flex()
                    .items_center()
                    .gap(px(8.))
                    .hover(|d| d.bg(theme::row_hover()))
                    .child(icon("gutter-bookmark", 14.))
                    .child(div().flex_none().text_color(theme::text_secondary()).child(format!("{file}:{}", line + 1)))
                    .child(div().truncate().font_family(theme::code_font()).child(self.bookmark_text(mark, cx)))
                    .on_click(
                        cx.listener(move |w, _: &ClickEvent, window, cx| {
                            w.open_file(&path, Some((line, 0)), window, cx)
                        }),
                    )
            }))
            .into_any_element()
    }
}
