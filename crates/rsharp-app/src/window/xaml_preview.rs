//! The XAML Preview (Passenger's XamlPreview and AvaloniaPreviewHost): an Avalonia file drawn by the project's own
//! Avalonia.Designer.HostApp on its last build, the file's text sent as it is typed, started again after each build.
//! The XAML host speaks Avalonia's previewer protocol; RSharp starts the HostApp it names, under the Lifeline.

use super::*;
use rsharp_lsp::xaml::{PreviewEvent, PreviewFrame};

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
        preview.updating = Some(cx.spawn(async move |_, cx| {
            cx.background_executor().timer(std::time::Duration::from_millis(300)).await;
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

    pub(super) fn preview_content(&self, _cx: &mut Context<Self>) -> impl IntoElement {
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
        let frame = preview.frame.as_ref().map(|(image, frame)| {
            // At the size it has at 96 DPI, as Avalonia's designer lays it out.
            let (width, height) =
                (frame.width as f32 * 96. / frame.dpi as f32, frame.height as f32 * 96. / frame.dpi as f32);
            let fit = room.map_or(1., |room| ((room - 32.) / width).clamp(0.1, 1.));
            gpui::img(gpui::ImageSource::Render(image.clone())).w(px(width * fit)).h(px(height * fit))
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
