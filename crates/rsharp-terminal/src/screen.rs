use crate::session::Outbox;
use alacritty_terminal::{
    Term,
    event::{Event, EventListener, WindowSize},
    grid::{Dimensions, Scroll},
    index::{Column, Line, Point, Side},
    selection::{Selection, SelectionType},
    term::{Config, TermMode, cell::Flags},
    vte::{
        ansi,
        ansi::{Color, CursorShape},
    },
};
use std::sync::{Arc, Mutex};

pub const HISTORY_LINES: usize = 10_000;

#[derive(Clone)]
pub(crate) struct Events {
    pub outbox: Arc<Outbox>,
    pub title: Arc<Mutex<String>>,
    size: Arc<Mutex<(usize, usize)>>,
    colors: Arc<Mutex<Vec<u32>>>,
}

impl EventListener for Events {
    fn send_event(&self, event: Event) {
        match event {
            Event::PtyWrite(text) => {
                let _ = self.outbox.push(text.into_bytes());
            }
            Event::Title(title) => *self.title.lock().unwrap() = title.chars().take(256).collect(),
            Event::ResetTitle => self.title.lock().unwrap().clear(),
            Event::TextAreaSizeRequest(format) => {
                let (cols, rows) = *self.size.lock().unwrap();
                let text =
                    format(WindowSize { num_cols: cols as u16, num_lines: rows as u16, cell_width: 0, cell_height: 0 });
                let _ = self.outbox.push(text.into_bytes());
            }
            Event::ColorRequest(index, format) => {
                let color = self.colors.lock().unwrap().get(index).copied().unwrap_or(0);
                let text = format(ansi::Rgb { r: (color >> 16) as u8, g: (color >> 8) as u8, b: color as u8 });
                let _ = self.outbox.push(text.into_bytes());
            }
            // Programs cannot read or overwrite the person's clipboard through escape sequences.
            _ => {}
        }
    }
}

pub struct Cell {
    pub row: usize,
    pub col: usize,
    pub text: String,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    pub selected: bool,
}

pub struct Snapshot {
    pub cells: Vec<Cell>,
    pub cols: usize,
    pub rows: usize,
    pub history: usize,
    pub offset: usize,
    pub cursor: Option<(usize, usize, CursorShape)>,
    pub mode: TermMode,
    pub title: String,
}

struct Size(usize, usize);
impl Dimensions for Size {
    fn total_lines(&self) -> usize {
        self.1
    }
    fn screen_lines(&self) -> usize {
        self.1
    }
    fn columns(&self) -> usize {
        self.0
    }
}

pub struct Screen {
    term: Term<Events>,
    parser: ansi::Processor,
    pub(crate) events: Events,
}

impl Screen {
    pub fn new(cols: usize, rows: usize) -> Self {
        let events = Events {
            outbox: Arc::new(Outbox::default()),
            title: Arc::new(Mutex::new(String::new())),
            size: Arc::new(Mutex::new((cols.max(2), rows.max(1)))),
            colors: Arc::new(Mutex::new(vec![0; 269])),
        };
        let config = Config { scrolling_history: HISTORY_LINES, ..Config::default() };
        Self {
            term: Term::new(config, &Size(cols.max(2), rows.max(1)), events.clone()),
            parser: ansi::Processor::new(),
            events,
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: usize, rows: usize) {
        *self.events.size.lock().unwrap() = (cols.max(2), rows.max(1));
        self.term.resize(Size(cols.max(2), rows.max(1)));
    }

    pub fn set_colors(&mut self, colors: Vec<u32>) {
        *self.events.colors.lock().unwrap() = colors;
    }
    pub fn visible_text(&self) -> String {
        let snapshot = self.snapshot();
        let mut text = String::new();
        let mut row = 0;
        for cell in snapshot.cells {
            if cell.row != row {
                text.push('\n');
                row = cell.row;
            }
            if !cell.flags.contains(Flags::WIDE_CHAR_SPACER) {
                text.push_str(&cell.text);
            }
        }
        text
    }

    pub fn mode(&self) -> TermMode {
        *self.term.mode()
    }

    pub fn snapshot(&self) -> Snapshot {
        let content = self.term.renderable_content();
        let offset = content.display_offset;
        let cursor_line = content.cursor.point.line.0 + offset as i32;
        let cursor = (content.mode.contains(TermMode::SHOW_CURSOR)
            && cursor_line >= 0
            && cursor_line < self.term.screen_lines() as i32)
            .then_some((cursor_line as usize, content.cursor.point.column.0, content.cursor.shape));
        let selection = content.selection;
        let cells = content
            .display_iter
            .map(|indexed| {
                let cell = indexed.cell;
                let mut text = cell.c.to_string();
                if let Some(extra) = cell.zerowidth() {
                    text.extend(extra);
                }
                Cell {
                    row: (indexed.point.line.0 + offset as i32) as usize,
                    col: indexed.point.column.0,
                    text,
                    fg: cell.fg,
                    bg: cell.bg,
                    flags: cell.flags,
                    selected: selection.as_ref().is_some_and(|s| s.contains(indexed.point)),
                }
            })
            .collect();
        Snapshot {
            cells,
            cols: self.term.columns(),
            rows: self.term.screen_lines(),
            history: self.term.history_size(),
            offset,
            cursor,
            mode: content.mode,
            title: self.events.title.lock().unwrap().clone(),
        }
    }

    pub fn text(&self) -> String {
        let mut out = String::new();
        for row in self.term.topmost_line().0..self.term.screen_lines() as i32 {
            for col in 0..self.term.columns() {
                let cell = &self.term.grid()[Point::new(Line(row), Column(col))];
                if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    out.push(cell.c);
                    if let Some(extra) = cell.zerowidth() {
                        out.extend(extra);
                    }
                }
            }
            out.push('\n');
        }
        out
    }

    pub fn scroll(&mut self, delta: i32) {
        self.term.scroll_display(Scroll::Delta(delta));
    }
    pub fn bottom(&mut self) {
        self.term.scroll_display(Scroll::Bottom);
    }
    pub fn clear(&mut self) {
        self.term.grid_mut().clear_viewport::<Color>();
        self.term.grid_mut().clear_history();
        self.term.selection = None;
    }
    fn point(&self, row: usize, col: usize) -> Point {
        Point::new(
            Line(row.min(self.term.screen_lines() - 1) as i32 - self.term.grid().display_offset() as i32),
            Column(col.min(self.term.columns() - 1)),
        )
    }
    pub fn select(&mut self, row: usize, col: usize, extend: bool, kind: SelectionType) {
        let point = self.point(row, col);
        if extend {
            if let Some(selection) = &mut self.term.selection {
                selection.update(point, Side::Right);
            }
        } else {
            self.term.selection = Some(Selection::new(kind, point, Side::Left));
        }
    }
    pub fn select_all(&mut self) {
        let mut selection =
            Selection::new(SelectionType::Simple, Point::new(self.term.topmost_line(), Column(0)), Side::Left);
        selection.update(Point::new(self.term.bottommost_line(), self.term.last_column()), Side::Right);
        self.term.selection = Some(selection);
    }
    pub fn selected_text(&self) -> Option<String> {
        self.term.selection_to_string()
    }
    pub fn reset_selection(&mut self) {
        self.term.selection = None;
    }

    /// A wrapped line's text and the byte clicked, bounded even for output containing no line break.
    pub fn logical_line(&self, row: usize, col: usize) -> (String, usize, Option<String>) {
        let point = self.point(row, col);
        let grid = self.term.grid();
        let mut start = point.line.0;
        while start > self.term.topmost_line().0
            && point.line.0 - start < 128
            && grid[Point::new(Line(start - 1), self.term.last_column())].flags.contains(Flags::WRAPLINE)
        {
            start -= 1;
        }
        let mut out = String::new();
        let mut clicked = 0;
        let link = grid[point].hyperlink().map(|h| h.uri().to_string());
        for line in start..=(start + 256).min(self.term.bottommost_line().0) {
            for column in 0..self.term.columns() {
                if line == point.line.0 && column == point.column.0 {
                    clicked = out.len();
                }
                let cell = &grid[Point::new(Line(line), Column(column))];
                if !cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    out.push(cell.c);
                    if let Some(extra) = cell.zerowidth() {
                        out.extend(extra);
                    }
                }
            }
            if !grid[Point::new(Line(line), self.term.last_column())].flags.contains(Flags::WRAPLINE) {
                break;
            }
        }
        (out, clicked, link)
    }
}
