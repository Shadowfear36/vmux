#![allow(dead_code)]
use alacritty_terminal::term::{Term, Config as TermConfig};
use alacritty_terminal::event::{Event, EventListener};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::vte::ansi::{NamedColor, Color as VteColor, Processor};
use tokio::sync::mpsc;
use bitflags::bitflags;
use std::io::Write;
use std::sync::{Arc, Mutex};

/// Events emitted by the terminal state machine.
#[derive(Debug, Clone)]
pub enum TermEvent {
    TitleChanged(String),
    Bell,
    Exit,
}

/// Proxy that forwards alacritty terminal events to a tokio channel.
/// Also holds a PTY writer to respond synchronously to PtyWrite events
/// (e.g. ESC[6n cursor position responses that cmd.exe blocks on).
#[derive(Clone)]
pub struct EventProxy {
    tx: mpsc::UnboundedSender<TermEvent>,
    pty_writer: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl EventListener for EventProxy {
    fn send_event(&self, event: Event) {
        match event {
            Event::Title(t) => { let _ = self.tx.send(TermEvent::TitleChanged(t)); }
            Event::Bell => { let _ = self.tx.send(TermEvent::Bell); }
            Event::Exit => { let _ = self.tx.send(TermEvent::Exit); }
            Event::PtyWrite(data) => {
                if let Ok(mut w) = self.pty_writer.lock() {
                    let _ = w.write_all(data.as_bytes());
                }
            }
            _ => {}
        }
    }
}

pub struct TermSize {
    pub cols: usize,
    pub rows: usize,
}

impl Dimensions for TermSize {
    fn total_lines(&self) -> usize { self.rows }
    fn screen_lines(&self) -> usize { self.rows }
    fn columns(&self) -> usize { self.cols }
}

#[derive(Debug, Clone, Copy)]
pub enum CellColor {
    Named(NamedColor),
    Indexed(u8),
    Rgb(u8, u8, u8),
}

bitflags! {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct CellFlags: u8 {
        const BOLD      = 0b0001;
        const ITALIC    = 0b0010;
        const UNDERLINE = 0b0100;
        const DIM       = 0b1000;
    }
}

#[derive(Debug, Clone)]
pub struct RenderCell {
    pub col: usize,
    pub row: usize,
    pub ch: char,
    pub fg: CellColor,
    pub bg: CellColor,
    pub flags: CellFlags,
}

/// Immutable snapshot of the visible terminal grid for rendering.
pub struct GridSnapshot {
    pub cols: usize,
    pub rows: usize,
    pub cells: Vec<RenderCell>,
    pub cursor_col: usize,
    pub cursor_row: usize,
}

/// Wraps alacritty_terminal's Term + VTE Processor.
pub struct TermGrid {
    term: Term<EventProxy>,
    parser: Processor,
}

impl TermGrid {
    pub fn new(cols: u16, rows: u16, pty_writer: Arc<Mutex<Box<dyn Write + Send>>>) -> (Self, mpsc::UnboundedReceiver<TermEvent>) {
        let (tx, rx) = mpsc::unbounded_channel();
        let proxy = EventProxy { tx, pty_writer };
        let size = TermSize { cols: cols as usize, rows: rows as usize };
        let term = Term::new(TermConfig::default(), &size, proxy);
        (TermGrid { term, parser: Processor::new() }, rx)
    }

    /// Feed bytes from the PTY into the VT state machine.
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.term, bytes);
    }

    pub fn resize(&mut self, cols: u16, rows: u16) {
        let size = TermSize { cols: cols as usize, rows: rows as usize };
        self.term.resize(size);
    }

    /// Scroll the terminal view by `delta` lines (positive = scroll up / show history).
    pub fn scroll(&mut self, delta: i32) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Delta(delta));
    }

    /// Snap back to the bottom of the buffer (called on any keypress).
    pub fn scroll_to_bottom(&mut self) {
        use alacritty_terminal::grid::Scroll;
        self.term.scroll_display(Scroll::Bottom);
    }

    /// Extract all visible cells for rendering.
    pub fn snapshot(&self) -> GridSnapshot {
        use alacritty_terminal::term::cell::Flags;

        let grid = self.term.grid();
        let cols = grid.columns();
        let rows = grid.screen_lines();

        let mut cells = Vec::with_capacity(cols * rows);

        for line in 0..rows {
            for col in 0..cols {
                let point = Point::new(Line(line as i32), Column(col));
                let cell = &grid[point];

                let fg = vte_color_to_cell_color(cell.fg);
                let bg = vte_color_to_cell_color(cell.bg);

                let mut flags = CellFlags::empty();
                if cell.flags.contains(Flags::BOLD)      { flags |= CellFlags::BOLD; }
                if cell.flags.contains(Flags::ITALIC)    { flags |= CellFlags::ITALIC; }
                if cell.flags.contains(Flags::UNDERLINE) { flags |= CellFlags::UNDERLINE; }
                if cell.flags.contains(Flags::DIM)       { flags |= CellFlags::DIM; }

                cells.push(RenderCell { col, row: line, ch: cell.c, fg, bg, flags });
            }
        }

        let cursor = self.term.grid().cursor.point;

        GridSnapshot {
            cols,
            rows,
            cells,
            cursor_col: cursor.column.0,
            cursor_row: cursor.line.0.unsigned_abs() as usize,
        }
    }
}

fn vte_color_to_cell_color(color: VteColor) -> CellColor {
    match color {
        VteColor::Named(n) => CellColor::Named(n),
        VteColor::Indexed(i) => CellColor::Indexed(i),
        VteColor::Spec(rgb) => CellColor::Rgb(rgb.r, rgb.g, rgb.b),
    }
}
