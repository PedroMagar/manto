// Terminal emulator: interprets raw backend bytes into a VT-style cell grid
// (cursor, scroll regions, scrollback, colors). Host-independent (no #[cfg],
// no OS calls). Scope: C0 controls, CSI cursor/erase/scroll-region, SGR
// (8/16/256/truecolor), DEC private (?25, ?1049 alt screen, ?7 wrap), OSC
// ignored; bounded scrollback; wide chars occupy one cell (simplification).

use std::collections::VecDeque;

// ── Color & attributes ───────────────────────────────────────────────────────

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Color {
    Default,
    Indexed(u8),
    Rgb(u8, u8, u8),
}

impl Default for Color {
    fn default() -> Self { Color::Default }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Attributes(u16);

impl Attributes {
    pub const BOLD: u16      = 1 << 0;
    pub const DIM: u16       = 1 << 1;
    pub const ITALIC: u16    = 1 << 2;
    pub const UNDERLINE: u16 = 1 << 3;
    pub const BLINK: u16     = 1 << 4;
    pub const REVERSE: u16   = 1 << 5;
    pub const HIDDEN: u16    = 1 << 6;
    pub const STRIKE: u16    = 1 << 7;

    pub fn set(&mut self, flag: u16, on: bool) {
        if on { self.0 |= flag; } else { self.0 &= !flag; }
    }
    pub fn has(&self, flag: u16) -> bool { self.0 & flag != 0 }
    pub fn clear(&mut self) { self.0 = 0; }
    pub fn is_empty(&self) -> bool { self.0 == 0 }
}

/// A renderable style (foreground, background, attributes).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
pub struct Style {
    pub fg: Color,
    pub bg: Color,
    pub attrs: Attributes,
}

impl Style {
    pub fn reset(&mut self) {
        self.fg = Color::Default;
        self.bg = Color::Default;
        self.attrs.clear();
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Cell {
    pub ch: char,
    pub style: Style,
}

impl Default for Cell {
    fn default() -> Self {
        Cell { ch: ' ', style: Style::default() }
    }
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct Cursor {
    x: u16,
    y: u16,
    visible: bool,
}

// ── Terminal state ───────────────────────────────────────────────────────────

pub const DEFAULT_SCROLLBACK: usize = 4000;

pub struct Terminal {
    cols: u16,
    rows: u16,
    main: Screen,
    alt: Screen,
    scrollback: VecDeque<Vec<Cell>>,
    max_scrollback: usize,
    cursor: Cursor,
    saved_cursor: Cursor,
    style: Style,
    scroll_region_top: u16,
    scroll_region_bottom: u16,
    alt_active: bool,
    wrap: bool,
    pending_wrap: bool,
    parse: ParseState,
    csi: CsiBuf,
    osc_left: usize,
    utf8_pending: Vec<u8>,
}

struct Screen {
    cells: Vec<Cell>,
}

impl Screen {
    fn new(cols: u16, rows: u16) -> Self {
        let cols = (cols as usize).max(2);
        let rows = (rows as usize).max(2);
        Screen { cells: vec![Cell::default(); cols * rows] }
    }

    fn clear(&mut self) {
        for c in self.cells.iter_mut() { *c = Cell::default(); }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
enum ParseState {
    #[default]
    Ground,
    Escape,
    Csi,
    Osc,
}

#[derive(Debug, Default)]
struct CsiBuf {
    params: Vec<u32>,
    private: Option<u8>,
}

const PARAM_SEP: u32 = 0xFFFF;

struct Params {
    inner: Vec<u32>,
    idx: usize,
}

impl Params {
    fn new(v: Vec<u32>) -> Self { Params { inner: v, idx: 0 } }
    fn next(&mut self) -> Option<u32> {
        let v = self.inner.get(self.idx).copied();
        if v.is_some() { self.idx += 1; }
        v
    }
}

impl Terminal {
    pub fn new(cols: u16, rows: u16) -> Self {
        let cols = cols.max(2);
        let rows = rows.max(2);
        Terminal {
            cols,
            rows,
            main: Screen::new(cols, rows),
            alt: Screen::new(cols, rows),
            scrollback: VecDeque::new(),
            max_scrollback: DEFAULT_SCROLLBACK,
            cursor: Cursor { x: 0, y: 0, visible: true },
            saved_cursor: Cursor { x: 0, y: 0, visible: true },
            style: Style::default(),
            scroll_region_top: 0,
            scroll_region_bottom: rows - 1,
            alt_active: false,
            wrap: true,
            pending_wrap: false,
            parse: ParseState::Ground,
            csi: CsiBuf::default(),
            osc_left: 0,
            utf8_pending: Vec::new(),
        }
    }

    // ── Public accessors (used by the view) ────────────────────────────────

    pub fn cols(&self) -> u16 { self.cols }
    pub fn rows(&self) -> u16 { self.rows }
    /// Rows pushed out of the main screen into scrollback.
    pub fn scrollback_len(&self) -> usize { self.scrollback.len() }
    /// Total absolute rows = scrollback + visible screen.
    pub fn total_lines(&self) -> usize { self.scrollback.len() + self.rows as usize }
    /// Fetch an absolute row (0 = oldest). The first `scrollback_len()` rows
    /// live in scrollback; the rest map onto the live screen.
    pub fn line_at(&self, abs: usize) -> &[Cell] {
        let cols = self.cols as usize;
        if abs < self.scrollback.len() {
            &self.scrollback[abs]
        } else {
            let r = abs.saturating_sub(self.scrollback.len());
            let cells = if self.alt_active { &self.alt.cells } else { &self.main.cells };
            &cells[r * cols..(r + 1) * cols]
        }
    }
    /// Live screen cursor (column, row) in 0-based screen coordinates.
    pub fn cursor_pos(&self) -> (u16, u16) { (self.cursor.x, self.cursor.y) }
    pub fn cursor_visible(&self) -> bool { self.cursor.visible }
    #[allow(dead_code)]
    pub fn line_as_text(&self, abs: usize) -> String {
        self.line_at(abs).iter().map(|c| c.ch).collect()
    }

    // ── Feeding raw bytes ──────────────────────────────────────────────────

    /// Feed a chunk of raw output bytes into the emulator.
    pub fn process(&mut self, bytes: &[u8]) {
        for &b in bytes {
            match self.parse {
                ParseState::Ground => {
                    if b == 0x1b {
                        self.parse = ParseState::Escape;
                    } else if b < 0x20 || b == 0x7f {
                        self.control(b);
                    } else {
                        self.feed_utf8(b);
                    }
                }
                ParseState::Escape => self.escape(b),
                ParseState::Csi => self.csi_byte(b),
                ParseState::Osc => self.osc_byte(b),
            }
        }
    }

    /// Resize the grid. Content is preserved without rewrapping: the current
    /// screen is folded into the scrollback and the last `rows` lines are
    /// shown; long lines are truncated/padded to `cols`.
    pub fn resize(&mut self, cols: u16, rows: u16) {
        let cols = cols.max(2);
        let rows = rows.max(2);
        if cols == self.cols && rows == self.rows { return; }

        let mut all: Vec<Vec<Cell>> = Vec::new();
        for a in 0..self.total_lines() {
            all.push(self.line_at(a).to_vec());
        }
        let cursor_abs = self.scrollback.len() + self.cursor.y as usize;

        for line in all.iter_mut() {
            line.truncate(cols as usize);
            line.resize(cols as usize, Cell::default());
        }

        let keep = rows as usize;
        let mut screen_start = all.len().saturating_sub(keep);
        if all.len() > keep && cursor_abs + 1 < screen_start {
            // The cursor scrolled off the folded screen: bring it into view.
            screen_start = cursor_abs.min(all.len().saturating_sub(1));
        }

        self.cols = cols;
        self.rows = rows;
        self.main = Screen::new(cols, rows);
        self.alt = Screen::new(cols, rows);
        self.scrollback.clear();
        for line in all.drain(..screen_start) {
            self.push_scrollback(line);
        }
        for (i, line) in all.into_iter().enumerate() {
            if i < rows as usize {
                self.main.cells[i * cols as usize..(i + 1) * cols as usize]
                    .copy_from_slice(&line);
            }
        }

        let new_cursor_y = if cursor_abs >= screen_start {
            (cursor_abs - screen_start).min(rows as usize - 1) as u16
        } else {
            0
        };
        self.cursor.x = self.cursor.x.min(cols.saturating_sub(1));
        self.cursor.y = new_cursor_y;
        self.scroll_region_top = 0;
        self.scroll_region_bottom = rows - 1;
        self.pending_wrap = false;
        self.parse = ParseState::Ground;
        self.csi = CsiBuf::default();
        self.utf8_pending.clear();
    }

    // ── Screen routing ─────────────────────────────────────────────────────

    fn active_cells_mut(&mut self) -> &mut [Cell] {
        if self.alt_active { &mut self.alt.cells } else { &mut self.main.cells }
    }

    // ── Ground / C0 control handlers ───────────────────────────────────────

    fn control(&mut self, b: u8) {
        match b {
            0x07 => {}                           // BEL: ignored
            0x08 => self.cursor.x = self.cursor.x.saturating_sub(1), // BS
            0x09 => {
                // HT: next multiple of 8
                let next = ((self.cursor.x as usize / 8) + 1) * 8;
                self.cursor.x = (next as u16).min(self.cols - 1);
            }
            0x0a | 0x0b | 0x0c => self.line_feed(), // LF / VT / FF
            0x0d => {
                self.cursor.x = 0;                 // CR cancels pending wrap
                self.pending_wrap = false;
            }
            _ => {}
        }
    }

    fn feed_utf8(&mut self, b: u8) {
        self.utf8_pending.push(b);
        let n = match self.utf8_pending[0] {
            0x00..=0x7F => 1,
            0xC2..=0xDF => 2,
            0xE0..=0xEF => 3,
            0xF0..=0xF4 => 4,
            _ => { self.utf8_pending.clear(); return; }
        };
        if self.utf8_pending.len() < n { return; }

        if n == 1 {
            self.write_char(self.utf8_pending[0] as char);
        } else {
            let mut cp: u32 = match self.utf8_pending[0] {
                0xC2..=0xDF => (self.utf8_pending[0] & 0x1F) as u32,
                0xE0..=0xEF => (self.utf8_pending[0] & 0x0F) as u32,
                _           => (self.utf8_pending[0] & 0x07) as u32,
            };
            let mut ok = true;
            for &c in &self.utf8_pending[1..n] {
                if c & 0xC0 != 0x80 { ok = false; break; }
                cp = (cp << 6) | (c & 0x3F) as u32;
            }
            if ok {
                if let Some(ch) = char::from_u32(cp) {
                    self.write_char(ch);
                }
            }
        }
        self.utf8_pending.clear();
    }

    fn write_char(&mut self, ch: char) {
        if self.pending_wrap {
            self.pending_wrap = false;
            self.cursor.x = 0;
            self.line_feed();
        }
        let style = self.style;
        let idx = self.cursor.y as usize * self.cols as usize + self.cursor.x as usize;
        if let Some(cell) = self.active_cells_mut().get_mut(idx) {
            *cell = Cell { ch, style };
        }
        if self.cursor.x + 1 < self.cols {
            self.cursor.x += 1;
        } else if self.wrap {
            self.pending_wrap = true;
        }
    }

    /// Move the cursor down one line; scroll the region when at the bottom.
    fn line_feed(&mut self) {
        if self.cursor.y == self.scroll_region_bottom {
            self.scroll_up();
        } else if self.cursor.y < self.scroll_region_bottom {
            self.cursor.y += 1;
        }
    }

    /// Scroll the scroll region up by one line (the overflow goes to scrollback
    /// when on the main screen and the region covers the whole screen).
    fn scroll_up(&mut self) {
        let top = self.scroll_region_top as usize;
        let bottom = self.scroll_region_bottom as usize;
        let cols = self.cols as usize;

        if !self.alt_active && top == 0 {
            let line = self.main.cells[0..cols].to_vec();
            self.push_scrollback(line);
        }

        let cells = self.active_cells_mut();
        cells.copy_within((top + 1) * cols..(bottom + 1) * cols, top * cols);
        for cell in cells[bottom * cols..(bottom + 1) * cols].iter_mut() {
            *cell = Cell::default();
        }
    }

    fn push_scrollback(&mut self, line: Vec<Cell>) {
        self.scrollback.push_back(line);
        while self.scrollback.len() > self.max_scrollback {
            self.scrollback.pop_front();
        }
    }

    // ── Escape/OSC state ───────────────────────────────────────────────────

    fn escape(&mut self, b: u8) {
        match b {
            b'[' => {
                self.parse = ParseState::Csi;
                self.csi = CsiBuf::default();
            }
            b']' => {
                self.parse = ParseState::Osc;
                self.osc_left = 2;
            }
            // Other ESC sequences (charset selects, etc.): ignored.
            _ => self.parse = ParseState::Ground,
        }
    }

    fn osc_byte(&mut self, b: u8) {
        if b == 0x07 {
            self.parse = ParseState::Ground;
        } else if b == 0x1b {
            self.osc_left = 1;
        } else if self.osc_left == 1 && b == b'\\' {
            self.parse = ParseState::Ground;
        } else {
            self.osc_left = 0;
        }
    }

    // ── CSI state ──────────────────────────────────────────────────────────

    fn csi_byte(&mut self, b: u8) {
        if b >= 0x40 && b <= 0x7e {
            let cs = std::mem::take(&mut self.csi);
            self.dispatch_csi(cs, b);
            self.parse = ParseState::Ground;
        } else {
            match b {
                b'?' | b'>' | b'<' | b'=' if self.csi.private.is_none() => {
                    self.csi.private = Some(b);
                }
                b if b.is_ascii_digit() => {
                    self.csi.params.push((b as u32) - b'0' as u32);
                }
                b';' => self.csi.params.push(PARAM_SEP),
                b':' => {}
                _ => {}
            }
        }
    }

    fn dispatch_csi(&mut self, cs: CsiBuf, final_byte: u8) {
        // Translate placeholder tokens into a real parameter list.
        let mut params: Vec<u32> = Vec::new();
        let mut cur: Option<u32> = None;
        for t in cs.params {
            if t == PARAM_SEP {
                params.push(cur.take().unwrap_or(0));
            } else {
                cur = Some(cur.unwrap_or(0) * 10 + t);
            }
        }
        if let Some(v) = cur { params.push(v); }

        let private = cs.private;
        let mut p = Params::new(params.clone());
        macro_rules! param_or {
            ($default:expr) => {
                match p.next() {
                    Some(0) => $default,
                    Some(v) => v as u16,
                    None => $default,
                }
            };
        }

        match final_byte {
            b'm' => self.sgr(Params::new(params)),
            b'H' | b'f' => {
                let row = (param_or!(1)).saturating_sub(1);
                let col = (param_or!(1)).saturating_sub(1);
                self.move_to(row, col);
            }
            b'A' => self.move_up(param_or!(1)),
            b'B' => self.move_down(param_or!(1)),
            b'C' => self.move_right(param_or!(1)),
            b'D' => self.move_left(param_or!(1)),
            b'E' => {
                let n = (param_or!(1)).max(1);
                self.move_down(n);
                self.cursor.x = 0;
            }
            b'F' => {
                let n = (param_or!(1)).max(1);
                self.move_up(n);
                self.cursor.x = 0;
            }
            b'G' | b'`' => {
                let col = (param_or!(1)).saturating_sub(1);
                self.cursor.x = col.min(self.cols - 1);
            }
            b'd' => {
                let row = (param_or!(1)).saturating_sub(1);
                self.cursor.y = row.min(self.rows - 1);
            }
            b'J' => self.erase_display(param_or!(0)),
            b'K' => self.erase_line(param_or!(0)),
            b'X' => self.erase_chars(param_or!(1).max(1) as usize),
            b'P' => self.delete_chars(param_or!(1).max(1) as usize),
            b'@' => self.insert_chars(param_or!(1).max(1) as usize),
            b'r' => {
                let top = (param_or!(1)).saturating_sub(1);
                let bottom = (param_or!(self.rows)).saturating_sub(1);
                self.scroll_region_top = top.min(self.rows - 1);
                self.scroll_region_bottom = bottom.min(self.rows - 1).max(self.scroll_region_top);
                self.cursor.y = 0;
                self.cursor.x = 0;
            }
            b's' => self.saved_cursor = self.cursor,
            b'u' => self.cursor = self.saved_cursor,
            b'h' | b'l' if private == Some(b'?') => {
                let on = final_byte == b'h';
                for m in &params {
                    match *m {
                        25 => self.cursor.visible = on,
                        1049 => {
                            if on { self.enter_alt(); } else { self.leave_alt(); }
                        }
                        7 => self.wrap = on,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }

    fn move_to(&mut self, row: u16, col: u16) {
        self.cursor.y = row.min(self.rows - 1);
        self.cursor.x = col.min(self.cols - 1);
        self.pending_wrap = false;
    }

    fn move_up(&mut self, n: u16) {
        self.cursor.y = self.cursor.y.saturating_sub(n.max(1));
        self.pending_wrap = false;
    }

    fn move_down(&mut self, n: u16) {
        let n = n.max(1);
        self.cursor.y = (self.cursor.y.saturating_add(n)).min(self.rows - 1);
        self.pending_wrap = false;
    }

    fn move_right(&mut self, n: u16) {
        let n = n.max(1);
        self.cursor.x = (self.cursor.x.saturating_add(n)).min(self.cols - 1);
        self.pending_wrap = false;
    }

    fn move_left(&mut self, n: u16) {
        self.cursor.x = self.cursor.x.saturating_sub(n.max(1));
        self.pending_wrap = false;
    }

    // ── Erase ──────────────────────────────────────────────────────────────

    fn erase_line(&mut self, mode: u16) {
        let x = self.cursor.x as usize;
        let y = self.cursor.y as usize;
        let cols = self.cols as usize;
        let start = y * cols;
        let end = start + cols;
        match mode % 3 {
            0 => self.fill(start + x, end),
            1 => self.fill(start, start + x + 1),
            _ => self.fill(start, end),
        }
    }

    fn erase_display(&mut self, mode: u16) {
        let y = self.cursor.y as usize;
        let cols = self.cols as usize;
        let total = cols * self.rows as usize;
        match mode % 4 {
            0 => {
                self.erase_line(0);
                self.fill((y + 1) * cols, total);
            }
            1 => {
                self.erase_line(1);
                self.fill(0, y * cols);
            }
            2 => self.fill(0, total),
            _ => {
                self.erase_display(2);
                self.scrollback.clear();
            }
        }
    }

    /// ECH: erase `n` cells at the cursor (filled with blanks, cursor stays).
    fn erase_chars(&mut self, n: usize) {
        let idx = self.cursor.y as usize * self.cols as usize + self.cursor.x as usize;
        let end = (idx + n).min(self.cols as usize * self.rows as usize);
        self.fill(idx, end.max(idx));
    }

    /// DCH: delete `n` cells at the cursor, shifting the rest of the line
    /// left; cleared cells are blanked at the end of the line.
    fn delete_chars(&mut self, n: usize) {
        let row_start = self.cursor.y as usize * self.cols as usize;
        let start = self.cursor.x as usize;
        let cols = self.cols as usize;
        let end = row_start + cols;
        let n = n.min(cols - start);
        if n == 0 {
            return;
        }
        let cells = self.active_cells_mut();
        cells.copy_within(row_start + start + n..end, row_start + start);
        for cell in cells[end - n..end].iter_mut() {
            *cell = Cell::default();
        }
    }

    /// ICH: insert `n` blank cells at the cursor, shifting the rest of the
    /// line right; cells pushed past the end of the line are lost.
    fn insert_chars(&mut self, n: usize) {
        let row_start = self.cursor.y as usize * self.cols as usize;
        let start = self.cursor.x as usize;
        let cols = self.cols as usize;
        let end = row_start + cols;
        let n = n.min(cols - start);
        if n == 0 {
            return;
        }
        let cells = self.active_cells_mut();
        cells.copy_within(row_start + start..end - n, row_start + start + n);
        for cell in cells[row_start + start..row_start + start + n].iter_mut() {
            *cell = Cell::default();
        }
    }

    fn fill(&mut self, from: usize, to: usize) {
        let cells_len = if self.alt_active { self.alt.cells.len() } else { self.main.cells.len() };
        let to = to.max(from).min(cells_len);
        let style = self.style;
        let cells = self.active_cells_mut();
        for c in cells[from..to].iter_mut() {
            c.ch = ' ';
            c.style = style;
        }
    }

    // ── Alternate screen ───────────────────────────────────────────────────

    fn enter_alt(&mut self) {
        if self.alt_active { return; }
        self.alt_active = true;
        self.alt.clear();
        self.cursor = Cursor { x: 0, y: 0, visible: true };
    }

    fn leave_alt(&mut self) {
        if !self.alt_active { return; }
        self.alt_active = false;
        self.cursor = self.saved_cursor;
    }

    // ── SGR ────────────────────────────────────────────────────────────────

    fn sgr(&mut self, mut p: Params) {
        while let Some(param) = p.next() {
            match param {
                0 => self.style.reset(),
                1 => self.style.attrs.set(Attributes::BOLD, true),
                2 => self.style.attrs.set(Attributes::DIM, true),
                3 => self.style.attrs.set(Attributes::ITALIC, true),
                4 => self.style.attrs.set(Attributes::UNDERLINE, true),
                5 | 6 => self.style.attrs.set(Attributes::BLINK, true),
                7 => self.style.attrs.set(Attributes::REVERSE, true),
                8 => self.style.attrs.set(Attributes::HIDDEN, true),
                9 => self.style.attrs.set(Attributes::STRIKE, true),
                21 => self.style.attrs.set(Attributes::UNDERLINE, true),
                22 => {
                    self.style.attrs.set(Attributes::BOLD, false);
                    self.style.attrs.set(Attributes::DIM, false);
                }
                23 => self.style.attrs.set(Attributes::ITALIC, false),
                24 => self.style.attrs.set(Attributes::UNDERLINE, false),
                25 => self.style.attrs.set(Attributes::BLINK, false),
                27 => self.style.attrs.set(Attributes::REVERSE, false),
                28 => self.style.attrs.set(Attributes::HIDDEN, false),
                29 => self.style.attrs.set(Attributes::STRIKE, false),
                30..=37 => self.style.fg = Color::Indexed(param as u8 - 30),
                38 | 48 => {
                    let is_fg = param == 38;
                    match p.next() {
                        Some(5) => {
                            if let Some(n) = p.next() {
                                let c = Color::Indexed((n % 256) as u8);
                                if is_fg { self.style.fg = c; } else { self.style.bg = c; }
                            }
                        }
                        Some(2) => {
                            if let (Some(r), Some(g), Some(b)) = (p.next(), p.next(), p.next()) {
                                let c = Color::Rgb(r.min(255) as u8, g.min(255) as u8, b.min(255) as u8);
                                if is_fg { self.style.fg = c; } else { self.style.bg = c; }
                            }
                        }
                        _ => {}
                    }
                }
                39 => self.style.fg = Color::Default,
                40..=47 => self.style.bg = Color::Indexed(param as u8 - 40),
                49 => self.style.bg = Color::Default,
                90..=97 => self.style.fg = Color::Indexed(8 + (param as u8 - 90)),
                100..=107 => self.style.bg = Color::Indexed(8 + (param as u8 - 100)),
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term() -> Terminal {
        Terminal::new(10, 4)
    }

    fn first_line(t: &Terminal) -> String { row(t, 0) }

    fn row(t: &Terminal, abs: usize) -> String {
        t.line_as_text(abs).trim_end().to_string()
    }

    #[test]
    fn plain_text_with_crlf() {
        let mut t = term();
        t.process(b"ab\r\ncd\r\n");
        assert_eq!(first_line(&t), "ab");
        assert_eq!(row(&t, 1), "cd");
        assert_eq!(t.cursor_pos(), (0, 2));
    }

    #[test]
    fn scrollback_accumulates_on_overflow() {
        let mut t = term();
        for i in 0..6 {
            t.process(format!("line{i}\r\n").as_bytes());
        }
        // 4 rows screen: the first 3 lines scrolled off into scrollback.
        assert_eq!(t.scrollback_len(), 3);
        assert_eq!(t.total_lines(), 7);
        assert_eq!(row(&t, 0), "line0");
        assert_eq!(row(&t, 2), "line2");
        assert_eq!(row(&t, 3), "line3");
        assert_eq!(row(&t, 5), "line5");
    }

    #[test]
    fn sgr_8_16_256_and_truecolor() {
        let mut t = term();
        t.process(b"\x1b[31mred\x1b[0mdefault");
        let red: Color = Color::Indexed(1);
        let row = t.line_at(0);
        assert_eq!(row[0].ch, 'r');
        assert_eq!(row[0].style.fg, red);
        assert_eq!(row[3].ch, 'd'); // after reset -> default
        assert_eq!(row[3].style.fg, Color::Default);

        let mut t = term();
        t.process(b"\x1b[91mbright\x1b[0m");
        assert_eq!(t.line_at(0)[0].style.fg, Color::Indexed(9));

        let mut t = term();
        t.process(b"\x1b[38;5;201mx\x1b[0m");
        assert_eq!(t.line_at(0)[0].style.fg, Color::Indexed(201));

        let mut t = term();
        t.process(b"\x1b[48;2;10;20;30m \x1b[0m");
        assert_eq!(t.line_at(0)[0].style.bg, Color::Rgb(10, 20, 30));
    }

    #[test]
    fn bold_and_reverse_attributes() {
        let mut t = term();
        t.process(b"\x1b[1;7mX\x1b[0m");
        let c = t.line_at(0)[0];
        assert!(c.style.attrs.has(Attributes::BOLD));
        assert!(c.style.attrs.has(Attributes::REVERSE));
    }

    #[test]
    fn cursor_movement_and_erase() {
        let mut t = term();
        t.process(b"abcdef");
        t.process(b"\x1b[1;3HGH");
        let text: String = (0..6).map(|c| t.line_at(0)[c].ch).collect();
        assert_eq!(text, "abGHef");
        t.process(b"\x1b[1;1H\x1b[K");
        assert_eq!(first_line(&t), ""); // line erased
    }

    #[test]
    fn character_edit_controls_erase_and_shift() {
        let mut t = term();
        t.process(b"abcdef");
        assert_eq!(first_line(&t), "abcdef");
        // ECH (ESC[1X): erase the char under the cursor, cursor stays.
        t.process(b"\x1b[1;6H\x1b[1X");
        assert_eq!(first_line(&t), "abcde");
        assert_eq!(t.cursor_pos(), (5, 0));
        // DCH (ESC[1P): delete the char at the cursor, line shifts left.
        t.process(b"\x1b[1;1H\x1b[1P");
        assert_eq!(first_line(&t), "bcde");
        // ICH (ESC[1@): insert a blank at the cursor, line shifts right.
        t.process(b"\x1b[1;1H\x1b[1@");
        assert_eq!(first_line(&t), " bcde");
        // Multi-char variants (line is " bcde" before this step).
        t.process(b"\x1b[1;1H\x1b[2P");
        assert_eq!(first_line(&t), "cde");
    }

    #[test]
    fn erase_display_and_home() {
        let mut t = term();
        t.process(b"abc\r\ndef\r\n");
        t.process(b"\x1b[2J");
        assert_eq!(first_line(&t), ""); // screen cleared
        let mut t = term();
        t.process(b"abc\r\ndef");
        t.process(b"\x1b[2J\x1b[H");
        assert_eq!(t.cursor_pos(), (0, 0));
    }

    #[test]
    fn scroll_region_limits_scroll() {
        let mut t = term();
        t.process(b"\x1b[2;3r");
        for i in 0..6 {
            t.process(format!("L{i}\r\n").as_bytes());
        }
        assert_eq!(t.scrollback_len(), 0);
    }

    #[test]
    fn alternate_screen_isolates_content() {
        let mut t = term();
        t.process(b"main\r\n");
        t.process(b"\x1b[?1049h");
        t.process(b"alt-data");
        assert_eq!(row(&t, t.scrollback_len()), "alt-data");
        t.process(b"\x1b[?1049l");
        assert_eq!(first_line(&t), "main");
    }

    #[test]
    fn resize_keeps_content() {
        let mut t = term();
        t.process(b"aaa\r\nbbb\r\nccc\r\n");
        t.resize(10, 4);
        let text: Vec<String> = (0..4).map(|r| row(&t, t.total_lines() - 4 + r)).collect();
        assert!(text.contains(&"bbb".to_string()));
    }

    #[test]
    fn utf8_split_across_chunks() {
        let mut t = term();
        let mut bytes = "café".as_bytes().to_vec();
        let tail = bytes.split_off(4); // split inside 'é' (2-byte)
        t.process(&bytes);
        t.process(&tail);
        assert_eq!(first_line(&t), "café");
    }

    #[test]
    fn cr_not_lf_keeps_column() {
        let mut t = term();
        t.process(b"abcdef");
        t.process(b"\rXY");
        assert_eq!(first_line(&t), "XYcdef");
        assert_eq!(t.cursor_pos(), (2, 0));
    }
}
