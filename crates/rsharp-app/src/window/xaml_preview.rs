//! The XAML Preview (Passenger's XamlPreview and AvaloniaPreviewHost): an Avalonia file drawn by the project's own
//! Avalonia.Designer.HostApp on its last build, the file's text sent as it is typed, started again after each build.
//! The XAML host speaks Avalonia's previewer protocol; RSharp starts the HostApp it names, under the Lifeline. The
//! file's elements come with their boxes (XamlPreview's PreviewElements): a click in the frame selects the start tag of
//! the element under it, and the element at the caret is outlined over the frame.

use super::*;
use rsharp_lsp::xaml::{PreviewEvent, PreviewFrame};
use std::cell::Cell;
use std::ops::Range;
use std::rc::Rc;

/// How wide the outline around the element at the caret is, outside its box.
const OUTLINE_WIDTH: f32 = 2.;

/// An element of the previewed file: its start tag and its whole markup as bytes of the text, and its box in the frame
/// at 96 DPI (x, y, width, height).
#[derive(Clone, Debug)]
struct Drawn {
    tag: Range<usize>,
    whole: Range<usize>,
    bounds: [f32; 4],
}

/// The file being previewed and what the previewer last said of it.
pub(super) struct Preview {
    path: PathBuf,
    session: Option<String>,
    /// The HostApp, which ends as the preview does.
    _host_app: Option<rsharp_core::process::Child>,
    frame: Option<(Arc<gpui::RenderImage>, PreviewFrame)>,
    error: Option<String>,
    status: String,
    _events: Task<()>,
    updating: Option<Task<()>>,
    _edits: Option<gpui::Subscription>,
    /// Parents before children, as the frame last drew them; none while typing has yet to reach the previewer.
    elements: Vec<Drawn>,
    typing: bool,
    /// Where the frame was last painted on the window and how much it was made smaller to fit, for a click.
    placed: Rc<Cell<Option<(gpui::Point<Pixels>, f32)>>>,
}

impl MainWindow {
    /// View > XAML Preview: the open tool window previewing the active Avalonia file.
    pub(super) fn open_preview(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.dock.is_open(Tool::XamlPreview) {
            self.toggle_tool(Tool::XamlPreview, window, cx);
        }
        match self.active_editor().map(|e| e.read(cx).path.clone()) {
            Some(path) if rsharp_lsp::xaml::is_xaml(&path) => self.start_preview(path, cx),
            _ => {
                self.said = Some("XAML Preview draws an Avalonia XAML file: open one".into());
                cx.notify();
            }
        }
    }

    /// A XAML file opened while the preview is up: the file shown is the one previewed.
    pub(super) fn follow_preview(&mut self, path: &Path, cx: &mut Context<Self>) {
        if self.preview.as_ref().is_some_and(|p| p.path != path) && rsharp_lsp::xaml::is_xaml(path) {
            self.start_preview(path.to_path_buf(), cx);
        }
    }

    /// The preview started again on the project's newest build, as a build that just ended made one.
    pub(super) fn restart_preview(&mut self, cx: &mut Context<Self>) {
        if let Some(path) = self.preview.as_ref().map(|p| p.path.clone()) {
            self.start_preview(path, cx);
        }
    }

    fn start_preview(&mut self, path: PathBuf, cx: &mut Context<Self>) {
        self.stop_preview(cx);
        if self.xaml.is_none() && cx.global::<crate::state::Ide>().servers {
            self.xaml = rsharp_lsp::xaml::XamlHost::carried();
        }
        let (Some(host), Some(buffer)) = (self.xaml.clone(), crate::buffer::find(&path, cx)) else {
            self.said = Some("XAML Preview needs the XAML host, which RSharp's build makes".into());
            cx.notify();
            return;
        };
        let (project, text) = (self.msbuild_project(&path), buffer.read(cx).document.text().to_string());
        let (sender, mut events) = futures::channel::mpsc::unbounded();
        let (asked, at) = (host.clone(), path.clone());
        // The session lasts until it is stopped: it waits on a thread of its own, not one of the executor's.
        std::thread::spawn(move || {
            let said = sender.clone();
            let ended = asked.preview(&project, &at, &text, 1., move |event| drop(said.unbounded_send(Ok(event))));
            let _ = sender.unbounded_send(Err(ended));
        });
        let events = cx.spawn(async move |this, cx| {
            while let Some(event) = events.next().await {
                if this.update(cx, |w, cx| w.preview_event(event, cx)).is_err() {
                    return;
                }
            }
        });
        let edits = cx.subscribe(&buffer, |w, buffer, event: &crate::buffer::BufferEvent, cx| {
            if matches!(event, crate::buffer::BufferEvent::Edited { .. }) {
                // The elements name places in the text before this edit, until the previewer draws this one.
                if let Some(preview) = &mut w.preview {
                    preview.elements.clear();
                    preview.typing = true;
                }
                let text = buffer.read(cx).document.text().to_string();
                w.update_preview(text, cx);
            }
        });
        self.preview = Some(Preview {
            path: path.clone(),
            session: None,
            _host_app: None,
            frame: None,
            error: None,
            status: format!("Starting the previewer on {}…", path.file_name().unwrap_or_default().to_string_lossy()),
            _events: events,
            updating: None,
            _edits: Some(edits),
            elements: Vec::new(),
            typing: false,
            placed: Rc::default(),
        });
        cx.notify();
    }

    fn preview_event(
        &mut self,
        event: Result<PreviewEvent, Result<Option<String>, rsharp_lsp::ServerFailed>>,
        cx: &mut Context<Self>,
    ) {
        let Some(preview) = &mut self.preview else { return };
        match event {
            Ok(PreviewEvent::Started { session, arguments, folder }) => {
                preview.session = Some(session);
                let Some(dotnet) = rsharp_core::programs::dotnet() else { return };
                let mut command = rsharp_core::dotnet::command(&dotnet, &arguments, &folder);
                command
                    .stdin(std::process::Stdio::null())
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null());
                match rsharp_core::process::spawn(command) {
                    Ok(child) => preview._host_app = Some(child),
                    Err(error) => preview.status = format!("The previewer could not start: {error}"),
                }
            }
            Ok(PreviewEvent::Frame(frame)) => {
                let image = image::RgbaImage::from_raw(frame.width, frame.height, frame.bgra.clone());
                if let Some(image) = image {
                    let render = gpui::RenderImage::new(vec![image::Frame::new(image)]);
                    preview.frame = Some((Arc::new(render), frame));
                    preview.status = "Live".into();
                }
            }
            Ok(PreviewEvent::Error(error)) => {
                preview.status = if error.is_some() { "The XAML did not load".into() } else { "Live".into() };
                preview.error = error;
            }
            Ok(PreviewEvent::Empty(true)) => {
                preview.status = "Nothing to draw: the root is 0 × 0 (hidden, or sized to content with none and no \
                                  d:DesignWidth and d:DesignHeight)"
                    .into();
            }
            Ok(PreviewEvent::Empty(false)) => {}
            Ok(PreviewEvent::Elements(_)) if preview.typing => {}
            Ok(PreviewEvent::Elements(elements)) => {
                let Some(buffer) = crate::buffer::find(&preview.path, cx) else { return };
                let document = &buffer.read(cx).document;
                let at = |p: rsharp_lsp::Point| document.offset_of_utf16(p.line, p.character);
                let range = |s: rsharp_lsp::Span| at(s.start)..at(s.end);
                preview.elements = elements
                    .into_iter()
                    .map(|e| Drawn { tag: range(e.tag), whole: range(e.whole), bounds: e.bounds })
                    .collect();
            }
            Err(Ok(Some(why))) => preview.status = why,
            Err(Ok(None)) => {}
            Err(Err(failed)) => preview.status = format!("The previewer stopped: {failed}"),
        }
        cx.notify();
    }

    /// The file as typed, sent once typing pauses.
    fn update_preview(&mut self, text: String, cx: &mut Context<Self>) {
        let (Some(preview), Some(host)) = (&mut self.preview, self.xaml.clone()) else { return };
        let Some(session) = preview.session.clone() else { return };
        preview.updating = Some(cx.spawn(async move |this, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
            // From here the elements that come are of this text's frame.
            let _ = this.update(cx, |w, _| w.preview.iter_mut().for_each(|p| p.typing = false));
            let sent = cx.background_executor().spawn(async move { host.preview_update(&session, &text) }).await;
            if let Err(failed) = sent {
                log::info!("XAML Preview: {failed}");
            }
        }));
    }

    pub(super) fn stop_preview(&mut self, cx: &mut Context<Self>) {
        let Some(preview) = self.preview.take() else { return };
        if let (Some(session), Some(host)) = (preview.session, self.xaml.clone()) {
            cx.background_executor()
                .spawn(async move {
                    let _ = host.preview_stop(&session);
                })
                .detach();
        }
    }

    pub(super) fn preview_subtitle(&self) -> Option<String> {
        self.preview.as_ref().map(|p| p.path.file_name().unwrap_or_default().to_string_lossy().into_owned())
    }

    pub(super) fn preview_content(&self, cx: &mut Context<Self>) -> impl IntoElement {
        let Some(preview) = &self.preview else {
            return empty_text("XAML Preview draws the Avalonia file open in the editor.")
                .track_focus(&self.preview_focus)
                .into_any_element();
        };
        let status = div().px(px(8.)).py(px(4.)).text_color(theme::text_secondary()).child(preview.status.clone());
        let error = preview.error.clone().map(|e| div().px(px(8.)).py(px(4.)).text_color(theme::error()).child(e));
        // The side it is docked on, which a frame wider than it is made smaller to fit.
        let room = match self.dock.place_of(Tool::XamlPreview) {
            crate::dock::Place::RightTop | crate::dock::Place::RightBottom => Some(self.dock.right_width),
            crate::dock::Place::LeftTop | crate::dock::Place::LeftBottom => Some(self.dock.left_width),
            _ => None,
        };
        let outlined = self.outlined(cx).map(|e| e.bounds);
        let frame = preview.frame.as_ref().map(|(image, frame)| {
            // At the size it has at 96 DPI, as Avalonia's designer lays it out.
            let (width, height) =
                (frame.width as f32 * 96. / frame.dpi as f32, frame.height as f32 * 96. / frame.dpi as f32);
            let fit = room.map_or(1., |room| ((room - 32.) / width).clamp(0.1, 1.));
            let placed = preview.placed.clone();
            div()
                .relative()
                .w(px(width * fit))
                .h(px(height * fit))
                .debug_selector(|| "xaml preview frame".into())
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|w, e: &MouseDownEvent, window, cx| w.choose_in_preview(e.position, window, cx)),
                )
                .child(gpui::img(gpui::ImageSource::Render(image.clone())).size_full())
                .child(
                    gpui::canvas(
                        |_, _, _| (),
                        move |bounds, _, window, _| {
                            placed.set(Some((bounds.origin, fit)));
                            let Some([x, y, w, h]) = outlined else { return };
                            let (grow, at) = (OUTLINE_WIDTH, |v: f32| px(v * fit));
                            let outline = gpui::Bounds::new(
                                bounds.origin + gpui::point(at(x) - px(grow), at(y) - px(grow)),
                                gpui::size(at(w) + px(2. * grow), at(h) + px(2. * grow)),
                            );
                            window.paint_quad(
                                gpui::outline(outline, theme::accent(), gpui::BorderStyle::Solid)
                                    .border_widths(px(grow)),
                            );
                        },
                    )
                    .absolute()
                    .top_0()
                    .left_0()
                    .size_full(),
                )
        });
        div()
            .id("xaml-preview")
            .track_focus(&self.preview_focus)
            .flex_1()
            .min_h_0()
            .overflow_scroll()
            .flex()
            .flex_col()
            .child(status)
            .children(error)
            .children(frame.map(|f| div().p(px(8.)).child(f)))
            .into_any_element()
    }

    /// The editor showing the previewed file, when it is the one the keys go to.
    fn previewed_editor(&self, cx: &App) -> Option<Entity<CodeEditor>> {
        let path = &self.preview.as_ref()?.path;
        self.active_editor().filter(|e| &e.read(cx).path == path).cloned()
    }

    /// The innermost drawn element whose markup holds the caret, which the frame outlines.
    fn outlined(&self, cx: &App) -> Option<&Drawn> {
        let caret = self.previewed_editor(cx)?.read(cx).primary().head;
        let elements = &self.preview.as_ref()?.elements;
        elements.iter().filter(|e| e.whole.contains(&caret)).min_by_key(|e| e.whole.len())
    }

    /// A click in the frame: the deepest element drawn there has its start tag selected, the caret at its start.
    fn choose_in_preview(&mut self, at: gpui::Point<Pixels>, window: &mut Window, cx: &mut Context<Self>) {
        let Some(preview) = &self.preview else { return };
        let Some((origin, fit)) = preview.placed.get() else { return };
        let (x, y) = (f32::from(at.x - origin.x) / fit, f32::from(at.y - origin.y) / fit);
        let inside = |e: &&Drawn| {
            let [left, top, w, h] = e.bounds;
            x >= left && x < left + w && y >= top && y < top + h
        };
        let Some(tag) = preview.elements.iter().rfind(inside).map(|e| e.tag.clone()) else { return };
        let Some(editor) = self.previewed_editor(cx) else { return };
        editor.update(cx, |e, cx| e.select(rsharp_text::Selection { anchor: tag.end, head: tag.start }, cx));
        window.focus(&editor.focus_handle(cx));
        cx.notify();
    }

    /// Test access: the box the outline is drawn around, in the frame's units, and each drawn element's start tag and
    /// box.
    pub fn preview_outlined(&self, cx: &App) -> Option<[f32; 4]> {
        self.outlined(cx).map(|e| e.bounds)
    }

    pub fn preview_elements(&self, cx: &App) -> Vec<(String, [f32; 4])> {
        let Some(preview) = &self.preview else { return Vec::new() };
        let Some(buffer) = crate::buffer::find(&preview.path, cx) else { return Vec::new() };
        let text = buffer.read(cx).document.text();
        preview.elements.iter().map(|e| (text.byte_slice(e.tag.clone()).to_string(), e.bounds)).collect()
    }

    /// Test access: the frame the preview shows, by its size in pixels, and the BGRA of a pixel in it.
    pub fn preview_frame(&self) -> Option<(u32, u32)> {
        self.preview.as_ref()?.frame.as_ref().map(|(_, f)| (f.width, f.height))
    }

    pub fn preview_pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        let (_, frame) = self.preview.as_ref()?.frame.as_ref()?;
        let at = (y * frame.width + x) as usize * 4;
        frame.bgra.get(at..at + 4).map(|p| [p[0], p[1], p[2], p[3]])
    }

    /// Test access: why the XAML did not load, and what the preview says.
    pub fn preview_error(&self) -> Option<String> {
        self.preview.as_ref()?.error.clone()
    }

    pub fn preview_status(&self) -> Option<String> {
        self.preview.as_ref().map(|p| p.status.clone())
    }
}
