// Free screen selection support.
//
// `ScreenGrid` holds the plain characters of the last rendered frame so any
// visible cell (windows, terminals, chrome, status bar, tabs, panel) can be
// selected and copied, independent of what drew it.
//
// `StampWriter` wraps the real output writer, forwards every byte unchanged,
// and lexes the ANSI stream (CSI/OSC/SGR skipped) to stamp the printable text
// into the `ScreenGrid` at the correct cell. It relies on Manto rendering each
// frame with `ansi::clear` (ESC[2J) followed by a full redraw.

use std::io::{self, Write};

use crate::terminal_emulator::{Attributes, Cell, Color, Style};

/// An axis-aligned box over the screen (or a displayed content area).
/// Coordinates are 0-based (row, col).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoxSelect {
    pub anchor: (usize, usize),
    pub extent: (usize, usize),
}

impl BoxSelect {
    /// (top, bottom, left, right) bounds of the box.
    pub fn bounds(&self) -> (usize, usize, usize, usize) {
        let (ar, ac) = self.anchor;
        let (er, ec) = self.extent;
        (ar.min(er), ar.max(er), ac.min(ec), ac.max(ec))
    }
}

pub struct ScreenGrid {
    w: usize,
    h: usize,
    cells: Vec<Vec<Cell>>,
}

impl ScreenGrid {
    pub fn new(w: u16, h: u16) -> Self {
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        ScreenGrid {
            w,
            h,
            cells: vec![vec![Cell::default(); w]; h],
        }
    }

    pub fn resize(&mut self, w: u16, h: u16) {
        let (w, h) = (w.max(1) as usize, h.max(1) as usize);
        if w != self.w || h != self.h {
            self.w = w;
            self.h = h;
            self.cells = vec![vec![Cell::default(); w]; h];
        } else {
            self.clear();
        }
    }

    pub fn clear(&mut self) {
        for row in self.cells.iter_mut() {
            for c in row.iter_mut() {
                *c = Cell::default();
            }
        }
    }

    fn set(&mut self, x: usize, y: usize, cell: Cell) {
        if x < self.w && y < self.h {
            self.cells[y][x] = cell;
        }
    }

    /// Write a cell directly with a plain default style (used for overlays
    /// whose style is not captured through the writer).
    #[allow(dead_code)]
    pub fn set_cell(&mut self, x: u16, y: u16, ch: char) {
        self.set(
            x as usize,
            y as usize,
            Cell {
                ch,
                ..Cell::default()
            },
        );
    }

    /// Write a cell with an explicit style.
    #[allow(dead_code)]
    pub fn put(&mut self, x: u16, y: u16, cell: Cell) {
        self.set(x as usize, y as usize, cell);
    }

    pub fn width(&self) -> usize {
        self.w
    }

    pub fn height(&self) -> usize {
        self.h
    }

    pub fn char_at(&self, x: usize, y: usize) -> char {
        if x < self.w && y < self.h {
            self.cells[y][x].ch
        } else {
            ' '
        }
    }

    pub fn style_at(&self, x: usize, y: usize) -> Style {
        if x < self.w && y < self.h {
            self.cells[y][x].style
        } else {
            Style::default()
        }
    }

    /// Full cell (character + style) at (x, y).
    pub fn cell_at(&self, x: usize, y: usize) -> Cell {
        if x < self.w && y < self.h {
            self.cells[y][x]
        } else {
            Cell::default()
        }
    }

    /// Text of the box (cols left..=right, rows top..=bottom), trimming the
    /// trailing spaces of each row and joining rows with '\n'.
    pub fn box_text(&self, left: usize, top: usize, right: usize, bottom: usize) -> String {
        let rmax = self.w.saturating_sub(1);
        let mut out = String::new();
        for y in top..=bottom {
            let mut row: String = (left..=right.min(rmax))
                .map(|x| self.char_at(x, y))
                .collect();
            while row.ends_with(' ') {
                row.pop();
            }
            if y > top {
                out.push('\n');
            }
            out.push_str(&row);
        }
        out
    }
}

#[derive(PartialEq, Clone, Copy)]
enum Parse {
    Text,
    Esc,
    Csi,
    Osc,
}

/// Writer that forwards everything to `out` and stamps the printable text into
/// `grid`. Handles ESC[ H/f (cursor), ESC[ 2J (clear), CR/LF, and skips
/// SGR/DEC/OSC sequences.
pub struct StampWriter<'a, W: Write> {
    out: &'a mut W,
    grid: &'a mut ScreenGrid,
    pos: (u16, u16),
    parse: Parse,
    csi: Vec<u8>,
    utf: Vec<u8>,
    osc_esc: bool,
    xoff: u16,
    yoff: u16,
    style: Style,
}

impl<'a, W: Write> StampWriter<'a, W> {
    pub fn new(out: &'a mut W, grid: &'a mut ScreenGrid) -> Self {
        StampWriter {
            out,
            grid,
            pos: (0, 0),
            parse: Parse::Text,
            csi: Vec::new(),
            utf: Vec::new(),
            osc_esc: false,
            xoff: 0,
            yoff: 0,
            style: Style::default(),
        }
    }

    /// The captured screen grid (after rendering a frame).
    #[allow(dead_code)]
    pub fn grid(&self) -> &ScreenGrid {
        self.grid
    }

    fn place(&mut self, ch: char) {
        let (x, y) = self.pos;
        let gx = x.saturating_add(self.xoff) as usize;
        let gy = y.saturating_add(self.yoff) as usize;
        let cell = Cell {
            ch,
            style: self.style,
        };
        self.grid.set(gx, gy, cell);
        if x + 1 < self.grid.w as u16 {
            self.pos.0 = x + 1;
        }
    }

    fn feed_utf(&mut self, b: u8) {
        self.utf.push(b);
        let n = match self.utf[0] {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        };
        if self.utf.len() < n {
            return;
        }
        let s = String::from_utf8_lossy(&self.utf);
        if let Some(ch) = s.chars().next() {
            self.place(ch);
        }
        self.utf.clear();
    }

    fn handle_csi(&mut self, fin: u8) {
        let params: String = String::from_utf8_lossy(&self.csi).into_owned();
        match fin {
            b'H' | b'f' => {
                let mut it = params.split(';');
                let row: u16 = it
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(1u16)
                    .saturating_sub(1);
                let col: u16 = it
                    .next()
                    .and_then(|p| p.parse().ok())
                    .unwrap_or(1u16)
                    .saturating_sub(1);
                self.pos = (
                    col.min(self.grid.w as u16 - 1),
                    row.min(self.grid.h as u16 - 1),
                );
            }
            b'J' => {
                let mode: u16 = params.parse().unwrap_or(0);
                if mode == 2 {
                    self.grid.clear();
                    self.style.reset();
                    self.pos = (0, 0);
                }
            }
            b'm' => {
                apply_sgr(&mut self.style, &params);
            }
            _ => {}
        }
    }
}

impl<'a, W: Write> Write for StampWriter<'a, W> {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let n = self.out.write(buf)?;
        if n > 0 {
            self.stamp(&buf[..n]);
        }
        Ok(n)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.out.flush()
    }
}

impl<'a, W: Write> StampWriter<'a, W> {
    fn stamp(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.parse {
                Parse::Text => match b {
                    0x1b => self.parse = Parse::Esc,
                    0x0d => self.pos.0 = 0,
                    0x0a => self.pos.1 = self.pos.1.saturating_add(1),
                    b if b < 0x20 || b == 0x7f => {}
                    b => self.feed_utf(b),
                },
                Parse::Esc => match b {
                    b'[' => {
                        self.parse = Parse::Csi;
                        self.csi.clear();
                    }
                    b']' => {
                        self.parse = Parse::Osc;
                        self.osc_esc = false;
                    }
                    _ => self.parse = Parse::Text,
                },
                Parse::Csi => {
                    if (0x40..=0x7e).contains(&b) {
                        self.handle_csi(b);
                        self.parse = Parse::Text;
                    } else {
                        self.csi.push(b);
                    }
                }
                Parse::Osc => {
                    if b == 0x07 {
                        self.parse = Parse::Text;
                    } else if b == 0x1b {
                        self.osc_esc = true;
                    } else if self.osc_esc && b == b'\\' {
                        self.parse = Parse::Text;
                    } else {
                        self.osc_esc = false;
                    }
                }
            }
        }
    }
}

/// Apply a CSI SGR parameter list (e.g. "38;5;196;1" or "0") to a running
/// style, reconstructing the absolute cell style from Manto's minimal SGR
/// transitions (RESET + only the differences that changed).
fn apply_sgr(style: &mut Style, params: &str) {
    let mut it = params.split(';');
    while let Some(p) = it.next() {
        let param = p.parse::<u16>().unwrap_or(0);
        match param {
            0 => style.reset(),
            1 => style.attrs.set(Attributes::BOLD, true),
            2 => style.attrs.set(Attributes::DIM, true),
            3 => style.attrs.set(Attributes::ITALIC, true),
            4 => style.attrs.set(Attributes::UNDERLINE, true),
            5 | 6 => style.attrs.set(Attributes::BLINK, true),
            7 => style.attrs.set(Attributes::REVERSE, true),
            8 => style.attrs.set(Attributes::HIDDEN, true),
            9 => style.attrs.set(Attributes::STRIKE, true),
            21 => style.attrs.set(Attributes::UNDERLINE, true),
            22 => {
                style.attrs.set(Attributes::BOLD, false);
                style.attrs.set(Attributes::DIM, false);
            }
            23 => style.attrs.set(Attributes::ITALIC, false),
            24 => style.attrs.set(Attributes::UNDERLINE, false),
            25 => style.attrs.set(Attributes::BLINK, false),
            27 => style.attrs.set(Attributes::REVERSE, false),
            28 => style.attrs.set(Attributes::HIDDEN, false),
            29 => style.attrs.set(Attributes::STRIKE, false),
            30..=37 => style.fg = Color::Indexed(param as u8 - 30),
            38 | 48 => {
                let is_fg = param == 38;
                match it.next().and_then(|s| s.parse::<u16>().ok()) {
                    Some(5) => {
                        if let Some(n) = it.next().and_then(|s| s.parse::<u16>().ok()) {
                            let c = Color::Indexed((n % 256) as u8);
                            if is_fg {
                                style.fg = c;
                            } else {
                                style.bg = c;
                            }
                        }
                    }
                    Some(2) => {
                        if let (Some(r), Some(g), Some(b)) = (
                            it.next().and_then(|s| s.parse::<u16>().ok()),
                            it.next().and_then(|s| s.parse::<u16>().ok()),
                            it.next().and_then(|s| s.parse::<u16>().ok()),
                        ) {
                            let c =
                                Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8);
                            if is_fg {
                                style.fg = c;
                            } else {
                                style.bg = c;
                            }
                        }
                    }
                    _ => {}
                }
            }
            39 => style.fg = Color::Default,
            40..=47 => style.bg = Color::Indexed(param as u8 - 40),
            49 => style.bg = Color::Default,
            90..=97 => style.fg = Color::Indexed(8 + (param as u8 - 90)),
            100..=107 => style.bg = Color::Indexed(8 + (param as u8 - 100)),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn box_select_bounds() {
        let s = BoxSelect {
            anchor: (2, 1),
            extent: (5, 3),
        };
        assert_eq!(s.bounds(), (2, 5, 1, 3));
        let s = BoxSelect {
            anchor: (5, 3),
            extent: (2, 1),
        };
        assert_eq!(s.bounds(), (2, 5, 1, 3));
    }

    #[test]
    fn stamp_writer_builds_grid() {
        let mut grid = ScreenGrid::new(12, 4);
        {
            let mut sink = Vec::new();
            let mut w = StampWriter::new(&mut sink, &mut grid);
            // clear frame + home
            write!(w, "\x1b[2J\x1b[H").unwrap();
            write!(w, "hello").unwrap();
            write!(w, "\r\nworld").unwrap();
            write!(w, "\x1b[0m").unwrap(); // SGR ignored
            write!(w, "\x1b[1;2HX").unwrap(); // cursor move
        }
        assert_eq!(grid.char_at(0, 0), 'h');
        assert_eq!(grid.char_at(4, 0), 'o');
        assert_eq!(grid.char_at(0, 1), 'w');
        assert_eq!(grid.char_at(1, 0), 'X'); // moved to row 0 col 1
        assert_eq!(grid.box_text(0, 0, 4, 0), "hXllo");
        assert_eq!(grid.box_text(0, 0, 4, 1), "hXllo\nworld");
    }

    #[test]
    fn stamp_writer_handles_unicode() {
        let mut grid = ScreenGrid::new(12, 2);
        let mut sink = Vec::new();
        {
            let mut w = StampWriter::new(&mut sink, &mut grid);
            write!(w, "café").unwrap();
        }
        assert_eq!(grid.box_text(0, 0, 5, 0), "café");
    }

    #[test]
    fn box_text_right_trims() {
        let mut grid = ScreenGrid::new(10, 1);
        let mut sink = Vec::new();
        {
            let mut w = StampWriter::new(&mut sink, &mut grid);
            write!(w, "ab  ").unwrap();
        }
        assert_eq!(grid.box_text(0, 0, 9, 0), "ab");
    }
}
