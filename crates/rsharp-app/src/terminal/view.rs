use crate::theme;
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler, Entity,
    EntityInputHandler, EventEmitter, FocusHandle, Focusable, GlobalElementId, InspectorElementId, IntoElement,
    LayoutId, MouseButton, MouseDownEvent, MouseMoveEvent, MouseUpEvent, Pixels, Point, ScrollWheelEvent, ShapedLine,
    Style, TextRun, UTF16Selection, Window, div, fill, font, point, prelude::*, px, relative, size,
};
use rsharp_terminal::{Flags, SelectionType, Session, Snapshot, TermMode, input};
use std::{collections::HashMap, ops::Range, path::PathBuf, sync::Arc};

pub struct FollowLink(pub rsharp_core::run::link::Link);
impl EventEmitter<FollowLink> for TerminalView {}

pub struct TerminalView {
    focus: FocusHandle,
    session: Option<Arc<Session>>,
    folder: PathBuf,
    bounds: Bounds<Pixels>,
    cell: gpui::Size<Pixels>,
    dims: (usize, usize),
    selecting: bool,
    problem: Option<String>,
    /// Shaped cells by their text, attributes, colour and font size: a size change makes new shapes.
    cache: HashMap<(String, u16, u32, u32), ShapedLine>,
    marked: String,
    program: bool,
    context_menu: bool,
    hovered: Option<(usize, usize)>,
    wheel: f32,
}
impl TerminalView {
    pub fn new(cx: &mut Context<Self>) -> Self {
        let weak = cx.weak_entity();
        cx.intercept_keystrokes(move |event, window, cx| {
            let _ = weak.update(cx, |v, cx| {
                if !v.focus.is_focused(window) || window.has_pending_keystrokes() {
                    return;
                }
                let key = &event.keystroke;
                let m = key.modifiers;
                let mac = cfg!(target_os = "macos");
                // A chord the IDE owns is the IDE's: on macOS Cmd is the platform key, and Shift together with it
                // is Find Action, not this terminal's Select All.
                let copy =
                    (mac && m.platform && !m.control && !m.alt && !m.shift) || (!mac && m.control && m.shift && !m.alt);
                if copy {
                    match key.key.as_str() {
                        "c" => v.copy(cx),
                        "v" => {
                            if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                                v.paste(&text, cx);
                            }
                        }
                        "a" => {
                            v.with_screen(|s| s.select_all());
                            cx.notify();
                        }
                        "k" if mac && !m.shift => {
                            v.with_screen(|s| s.clear());
                            cx.notify();
                        }
                        _ => return,
                    }
                    cx.stop_propagation();
                    return;
                }
                if mac && m.platform {
                    return;
                }
                if (m.shift && matches!(key.key.as_str(), "escape" | "f12"))
                    || (m.alt && matches!(key.key.as_str(), "f12" | "0" | "1" | "4" | "6" | "7" | "8"))
                {
                    return;
                }
                // Program tabs keep Run/Rerun/Stop; shell tabs use those keys as terminal function keys.
                if v.program && key.key == "f5" && (m.control || m.shift || m.alt) {
                    return;
                }
                if m.shift && !m.control && matches!(key.key.as_str(), "pageup" | "pagedown") {
                    let rows = v.dims.1.max(1) as i32;
                    v.with_screen(|s| s.scroll(if key.key == "pageup" { rows } else { -rows }));
                    cx.notify();
                    cx.stop_propagation();
                    return;
                }
                if key.key == "insert" && m.shift {
                    if let Some(text) = cx.read_from_clipboard().and_then(|c| c.text()) {
                        v.paste(&text, cx);
                    }
                    cx.stop_propagation();
                    return;
                }
                let app =
                    v.session.as_ref().is_some_and(|s| s.screen.lock().unwrap().mode().contains(TermMode::APP_CURSOR));
                if let Some(bytes) = input::key(&key.key, m.control, m.alt, m.shift, app) {
                    v.write(&bytes, cx);
                    cx.stop_propagation();
                }
            });
        })
        .detach();
        Self {
            focus: cx.focus_handle(),
            session: None,
            folder: PathBuf::new(),
            bounds: Bounds::default(),
            cell: size(px(8.), px(20.)),
            dims: (0, 0),
            selecting: false,
            problem: None,
            cache: HashMap::new(),
            marked: String::new(),
            program: false,
            context_menu: false,
            hovered: None,
            wheel: 0.,
        }
    }
    /// The cell the view lays its text out in, so a test can click the cell a character is in.
    pub fn cell(&self) -> gpui::Size<Pixels> {
        self.cell
    }

    pub fn set_session(
        &mut self,
        session: Option<Arc<Session>>,
        folder: PathBuf,
        program: bool,
        cx: &mut Context<Self>,
    ) {
        if self.session.as_ref().map(|s| s.pid) != session.as_ref().map(|s| s.pid) {
            self.dims = (0, 0);
            self.selecting = false;
            self.problem = None;
            self.marked.clear();
        }
        if let Some(session) = &session {
            let colors = (0..269)
                .map(|n| {
                    let color = if n < 256 {
                        rsharp_terminal::Color::Indexed(n as u8)
                    } else if n == 257 {
                        rsharp_terminal::Color::Named(rsharp_terminal::NamedColor::Background)
                    } else {
                        rsharp_terminal::Color::Named(rsharp_terminal::NamedColor::Foreground)
                    };
                    let c = theme::terminal_color(color, false);
                    ((c.r * 255.).round() as u32) << 16
                        | ((c.g * 255.).round() as u32) << 8
                        | (c.b * 255.).round() as u32
                })
                .collect();
            session.screen.lock().unwrap().set_colors(colors);
        }
        self.program = program;
        self.session = session;
        self.folder = folder;
        cx.notify();
    }
    fn with_screen(&self, f: impl FnOnce(&mut rsharp_terminal::Screen)) {
        if let Some(s) = &self.session {
            f(&mut s.screen.lock().unwrap());
        }
    }
    fn write(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(s) = &self.session {
            self.problem = s.write(text).err();
            self.with_screen(|s| s.reset_selection());
            cx.notify();
        }
    }
    fn paste(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(s) = &self.session {
            self.problem = s.paste(text).err();
            cx.notify();
        }
    }
    fn copy(&self, cx: &mut Context<Self>) {
        if let Some(text) = self.session.as_ref().and_then(|s| s.screen.lock().unwrap().selected_text()) {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }
    fn at(&self, position: Point<Pixels>) -> (usize, usize) {
        (
            ((f32::from(position.y - self.bounds.top()) / f32::from(self.cell.height)).max(0.) as usize)
                .min(self.dims.1.saturating_sub(1)),
            ((f32::from(position.x - self.bounds.left()) / f32::from(self.cell.width)).max(0.) as usize)
                .min(self.dims.0.saturating_sub(1)),
        )
    }
    fn mouse_report(
        &mut self,
        position: Point<Pixels>,
        button: u8,
        release: bool,
        modifiers: gpui::Modifiers,
        cx: &mut Context<Self>,
    ) -> bool {
        let mode = self.session.as_ref().map(|s| s.screen.lock().unwrap().mode()).unwrap_or_default();
        if modifiers.shift || !mode.intersects(TermMode::MOUSE_MODE) {
            return false;
        }
        let (row, col) = self.at(position);
        if let Some(bytes) = input::mouse(
            button,
            col,
            row,
            release,
            mode.contains(TermMode::SGR_MOUSE),
            modifiers.shift,
            modifiers.alt,
            modifiers.control,
        ) {
            self.write(&bytes, cx);
        }
        true
    }
    fn mouse_down(&mut self, e: &MouseDownEvent, w: &mut Window, cx: &mut Context<Self>) {
        w.focus(&self.focus);
        let button = match e.button {
            MouseButton::Left => 0,
            MouseButton::Middle => 1,
            _ => 2,
        };
        if self.mouse_report(e.position, button, false, e.modifiers, cx) {
            return;
        }
        self.context_menu = e.button == MouseButton::Right;
        cx.notify();
        let (row, col) = self.at(e.position);
        let modifier = if cfg!(target_os = "macos") { e.modifiers.platform } else { e.modifiers.control };
        if e.button == MouseButton::Left && self.follow(row, col, modifier, cx) {
            return;
        }
        if e.button == MouseButton::Left {
            self.selecting = true;
            let kind = if e.click_count >= 3 {
                SelectionType::Lines
            } else if e.click_count == 2 {
                SelectionType::Semantic
            } else if e.modifiers.alt {
                SelectionType::Block
            } else {
                SelectionType::Simple
            };
            self.with_screen(|s| s.select(row, col, e.modifiers.shift, kind));
            cx.notify();
        }
    }
    fn follow(&mut self, row: usize, col: usize, modifier: bool, cx: &mut Context<Self>) -> bool {
        let link = self.link_at(row, col, modifier);
        if let Some(link) = link {
            cx.emit(FollowLink(link));
            true
        } else {
            false
        }
    }
    fn link_at(&self, row: usize, col: usize, modifier: bool) -> Option<rsharp_core::run::link::Link> {
        let Some(session) = &self.session else {
            return None;
        };
        let (line, offset, hyperlink) = session.screen.lock().unwrap().logical_line(row, col);
        hyperlink
            .filter(|u| u.starts_with("https://") || u.starts_with("http://"))
            .map(|url| rsharp_core::run::link::Link::Url { at: offset..offset + 1, url })
            .or_else(|| rsharp_core::run::link::find(&line, offset).filter(|l| l.hits(offset)))
            .or_else(|| if modifier { file_link(&line, offset, &self.folder) } else { None })
    }
}

fn file_link(line: &str, offset: usize, folder: &std::path::Path) -> Option<rsharp_core::run::link::Link> {
    let is_delim = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '(' | ')' | '[' | ']');
    let start = line[..offset.min(line.len())].rfind(is_delim).map(|p| p + 1).unwrap_or(0);
    let end = line[offset.min(line.len())..].find(is_delim).map(|p| offset + p).unwrap_or(line.len());
    let raw = line.get(start..end)?.trim_end_matches([',', ';']);
    let (name, number) =
        raw.rsplit_once(':').filter(|(_, n)| n.chars().all(|c| c.is_ascii_digit())).unwrap_or((raw, "1"));
    let path = if name.starts_with("~/") { rsharp_core::programs::home().join(&name[2..]) } else { folder.join(name) };
    path.is_file().then(|| rsharp_core::run::link::Link::File {
        at: start..end,
        path,
        line: number.parse::<usize>().unwrap_or(1).saturating_sub(1),
    })
}
impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}
impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("terminal-view")
            .debug_selector(|| "terminal screen".into())
            .key_context("Terminal")
            .track_focus(&self.focus)
            .flex_1()
            .min_h_0()
            .min_w_0()
            .overflow_hidden()
            .cursor(if self.hovered.is_some() { CursorStyle::PointingHand } else { CursorStyle::IBeam })
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(|v, e: &MouseMoveEvent, _, cx| {
                let at = v.at(e.position);
                let modifier = if cfg!(target_os = "macos") { e.modifiers.platform } else { e.modifiers.control };
                let hover = v.link_at(at.0, at.1, modifier).map(|_| at);
                if hover != v.hovered {
                    v.hovered = hover;
                    cx.notify();
                }
                if v.selecting && e.pressed_button == Some(MouseButton::Left) {
                    let (r, c) = v.at(e.position);
                    v.with_screen(|s| s.select(r, c, true, SelectionType::Simple));
                    cx.notify();
                } else {
                    let mode = v.session.as_ref().map(|s| s.screen.lock().unwrap().mode()).unwrap_or_default();
                    if mode.contains(TermMode::MOUSE_MOTION)
                        || (mode.contains(TermMode::MOUSE_DRAG) && e.pressed_button.is_some())
                    {
                        v.mouse_report(
                            e.position,
                            32 + if e.pressed_button.is_some() { 0 } else { 3 },
                            false,
                            e.modifiers,
                            cx,
                        );
                    }
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|v, e: &MouseUpEvent, _, cx| {
                    v.selecting = false;
                    v.mouse_report(e.position, 0, true, e.modifiers, cx);
                }),
            )
            .on_scroll_wheel(cx.listener(|v, e: &ScrollWheelEvent, _, cx| {
                let delta = e.delta.pixel_delta(v.cell.height).y;
                v.wheel += f32::from(delta) / f32::from(v.cell.height);
                let lines = v.wheel.trunc() as i32;
                v.wheel -= lines as f32;
                if lines != 0 {
                    if !v.mouse_report(e.position, if lines > 0 { 64 } else { 65 }, false, e.modifiers, cx) {
                        v.with_screen(|s| s.scroll(lines));
                    }
                    cx.notify();
                }
                cx.stop_propagation();
            }))
            .child(GridElement(cx.entity()))
            .when(self.context_menu, |d| {
                d.child(
                    div()
                        .absolute()
                        .top_0()
                        .right_0()
                        .w(px(170.))
                        .p(px(6.))
                        .rounded(px(8.))
                        .bg(theme::popup())
                        .border_1()
                        .border_color(theme::popup_border())
                        // A click on the menu is the menu's, not the grid's: pressing Copy must not first
                        // replace what was selected with the cell under the pointer.
                        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                        .on_mouse_down(MouseButton::Right, |_, _, cx| cx.stop_propagation())
                        .children(["Copy", "Paste", "Select All", "Clear Buffer"].into_iter().map(|title| {
                            div()
                                .id(title)
                                .debug_selector(move || format!("terminal menu: {title}"))
                                .h(px(24.))
                                .px(px(8.))
                                .flex()
                                .items_center()
                                .rounded(px(4.))
                                .hover(|d| d.bg(theme::row_hover()))
                                .child(title)
                                .on_click(cx.listener(move |v, _, _, cx| {
                                    v.context_menu = false;
                                    match title {
                                        "Copy" => v.copy(cx),
                                        "Paste" => {
                                            if let Some(text) = cx.read_from_clipboard().and_then(|i| i.text()) {
                                                v.paste(&text, cx);
                                            }
                                        }
                                        "Select All" => v.with_screen(|s| s.select_all()),
                                        _ => v.with_screen(|s| s.clear()),
                                    }
                                    cx.notify();
                                }))
                        })),
                )
            })
            .when_some(self.problem.clone(), |d, p| {
                d.child(div().absolute().bottom_0().left_0().bg(theme::island()).text_color(theme::error()).child(p))
            })
    }
}
impl EntityInputHandler for TerminalView {
    fn text_for_range(
        &mut self,
        _: Range<usize>,
        _: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        Some(self.marked.clone())
    }
    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.marked.encode_utf16().count()..self.marked.encode_utf16().count(),
            reversed: false,
        })
    }
    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        (!self.marked.is_empty()).then_some(0..self.marked.encode_utf16().count())
    }
    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        cx.notify();
    }
    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked.clear();
        self.write(text, cx);
    }
    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = text.to_owned();
        cx.notify();
    }
    fn bounds_for_range(
        &mut self,
        _: Range<usize>,
        _: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (r, c, _) = self.session.as_ref()?.snapshot().cursor?;
        Some(Bounds::new(
            self.bounds.origin + point(self.cell.width * c as f32, self.cell.height * r as f32),
            self.cell,
        ))
    }
    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        Some(0)
    }
}
struct GridElement(Entity<TerminalView>);
impl IntoElement for GridElement {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}
impl Element for GridElement {
    type RequestLayoutState = ();
    type PrepaintState = Option<Snapshot>;
    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }
    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        w: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let _ = cx;
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = relative(1.).into();
        (w.request_layout(style, [], cx), ())
    }
    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        w: &mut Window,
        cx: &mut App,
    ) -> Option<Snapshot> {
        // The terminal's cells are the editor's: its font size, its line height, the same code font.
        let settings = crate::settings::settings(cx).clone();
        let line_height = px(settings.line_height());
        let f = gpui::Font { features: gpui::FontFeatures::disable_ligatures(), ..font(theme::CODE_FONT) };
        let run = TextRun {
            len: 1,
            font: f,
            color: theme::editor::text().into(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let width = w.text_system().shape_line("0".into(), px(settings.editor_font_size), &[run], None).width;
        self.0.update(cx, |v, _| {
            v.bounds = bounds;
            v.cell = size(width, line_height);
            let dims = (
                (f32::from(bounds.size.width) / f32::from(width)).floor().max(2.) as usize,
                (f32::from(bounds.size.height) / f32::from(line_height)).floor().max(1.) as usize,
            );
            if v.dims != dims {
                v.dims = dims;
                if let Some(s) = &v.session {
                    s.resize(dims.0, dims.1);
                }
            }
            v.session.as_ref().map(|s| s.snapshot())
        })
    }
    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        snapshot: &mut Option<Snapshot>,
        w: &mut Window,
        cx: &mut App,
    ) {
        let focus = self.0.read(cx).focus.clone();
        let font_size = px(crate::settings::settings(cx).editor_font_size);
        w.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        let Some(snapshot) = snapshot else {
            return;
        };
        self.0.update(cx, |v, cx| {
            if v.cache.len() > 4096 {
                v.cache.clear();
            }
            for cell in &snapshot.cells {
                if cell.row >= snapshot.rows
                    || cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER)
                {
                    continue;
                }
                let bold = cell.flags.contains(Flags::BOLD);
                let mut fg = theme::terminal_color(cell.fg, bold);
                let mut bg = theme::terminal_color(cell.bg, false);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                if cell.selected {
                    bg = theme::editor::selection();
                }
                if cell.flags.contains(Flags::DIM) {
                    fg.a *= 0.66;
                }
                let origin = bounds.origin + point(v.cell.width * cell.col as f32, v.cell.height * cell.row as f32);
                let width = v.cell.width * if cell.flags.contains(Flags::WIDE_CHAR) { 2. } else { 1. };
                w.paint_quad(fill(Bounds::new(origin, size(width, v.cell.height)), bg));
                if cell.text != " " && !cell.flags.contains(Flags::HIDDEN) {
                    let color = ((fg.r * 255.) as u32) << 24
                        | ((fg.g * 255.) as u32) << 16
                        | ((fg.b * 255.) as u32) << 8
                        | (fg.a * 255.) as u32;
                    let key = (cell.text.clone(), cell.flags.bits(), color, f32::from(font_size).to_bits());
                    let shaped = v.cache.entry(key).or_insert_with(|| {
                        let mut font =
                            gpui::Font { features: gpui::FontFeatures::disable_ligatures(), ..font(theme::CODE_FONT) };
                        if bold {
                            font.weight = gpui::FontWeight::BOLD;
                        }
                        if cell.flags.contains(Flags::ITALIC) {
                            font.style = gpui::FontStyle::Italic;
                        }
                        w.text_system().shape_line(
                            cell.text.clone().into(),
                            font_size,
                            &[TextRun {
                                len: cell.text.len(),
                                font,
                                color: fg.into(),
                                background_color: None,
                                underline: None,
                                strikethrough: None,
                            }],
                            None,
                        )
                    });
                    let _ = shaped.paint(origin, v.cell.height, w, cx);
                }
                if v.hovered == Some((cell.row, cell.col))
                    || cell.flags.intersects(Flags::ALL_UNDERLINES | Flags::STRIKEOUT)
                {
                    let y = v.cell.height * if cell.flags.contains(Flags::STRIKEOUT) { 0.5 } else { 0.9 };
                    w.paint_quad(fill(Bounds::new(origin + point(px(0.), y), size(width, px(1.))), fg));
                }
            }
            if let Some((row, col, shape)) = snapshot.cursor.filter(|_| focus.is_focused(w)) {
                use rsharp_terminal::CursorShape;
                let origin = bounds.origin + point(v.cell.width * col as f32, v.cell.height * row as f32);
                let rect = match shape {
                    CursorShape::Hidden => None,
                    CursorShape::Underline => {
                        Some(Bounds::new(origin + point(px(0.), v.cell.height - px(2.)), size(v.cell.width, px(2.))))
                    }
                    CursorShape::Beam => Some(Bounds::new(origin, size(px(2.), v.cell.height))),
                    _ => Some(Bounds::new(origin, v.cell)),
                };
                if let Some(rect) = rect {
                    w.paint_quad(fill(rect, {
                        let mut color = theme::editor::caret();
                        color.a = if shape == CursorShape::Block { 0.45 } else { 1. };
                        color
                    }));
                }
            }
        });
    }
}
