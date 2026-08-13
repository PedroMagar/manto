use std::io::Write;

use super::ansi;

pub const MIN_W: u16 = 5;
pub const MIN_H: u16 = 3;

pub struct Window {
    pub position_x: u16,
    pub position_y: u16,
    pub width: u16,
    pub height: u16,
    pub layer: u16,
    pub minimizable: bool,
    pub closable: bool,
    pub draggable: bool,
    pub resizable: bool,
    /// Internal content resolution. 0 = same as the visible area (no scroll).
    pub content_w: u16,
    pub content_h: u16,
    pub scroll_x: u16,
    pub scroll_y: u16,
}

impl Window {
    pub fn new(position_x: u16, position_y: u16, width: u16, height: u16, layer: u16) -> Self {
        Self {
            position_x,
            position_y,
            width,
            height,
            layer,
            minimizable: true,
            closable: true,
            draggable: true,
            resizable: true,
            content_w: 0,
            content_h: 0,
            scroll_x: 0,
            scroll_y: 0,
        }
    }

    /// Set the internal content resolution, enabling scrollbars when needed.
    #[allow(dead_code)]
    pub fn with_content(mut self, content_w: u16, content_h: u16) -> Self {
        self.content_w = content_w;
        self.content_h = content_h;
        self
    }

    /// Remove the chrome buttons (minimize / close / drag / resize).
    pub fn without_chrome(mut self) -> Self {
        self.minimizable = false;
        self.closable = false;
        self.draggable = false;
        self.resizable = false;
        self
    }

    fn visible_w(&self) -> usize {
        self.width.saturating_sub(2) as usize
    }
    fn visible_h(&self) -> usize {
        self.height.saturating_sub(2) as usize
    }
    fn has_vscroll(&self) -> bool {
        self.content_h > 0 && self.content_h as usize > self.visible_h()
    }
    fn has_hscroll(&self) -> bool {
        self.content_w > 0 && self.content_w as usize > self.visible_w()
    }

    /// Compute (thumb_pos, thumb_len) for a window scrollbar.
    /// `track` = track size = visible size.
    fn scroll_thumb(track: usize, total: usize, scroll: usize) -> (usize, usize) {
        let thumb_len =
            (((track as f32 / total as f32) * track as f32).max(1.0) as usize).min(track);
        let available = track - thumb_len;
        let max_scroll = total - track;
        let thumb_pos = if max_scroll > 0 {
            (scroll * available / max_scroll).min(available)
        } else {
            0
        };
        (thumb_pos, thumb_len)
    }

    /// Return the scrollbar character (░ or █) for position `i` within the track.
    fn scroll_char(thumb_pos: usize, thumb_len: usize, i: usize) -> char {
        if i >= thumb_pos && i < thumb_pos + thumb_len {
            '█'
        } else {
            '░'
        }
    }

    pub fn draw(&self, out: &mut impl Write, title: &str) {
        let lx = self.position_x;
        let ty = self.position_y;
        let rx = lx + self.width - 1;
        let by = ty + self.height - 1;
        let vw = self.visible_w();
        let vh = self.visible_h();

        // Top border
        ansi::move_to(out, lx, ty);
        write!(out, "┌{:─^1$}┐", format!(" {} ", title), vw).unwrap();

        // Clear interior
        for i in 1..(self.height - 1) {
            ansi::move_to(out, lx + 1, ty + i);
            write!(out, "{:1$}", "", vw).unwrap();
        }

        // Left column
        for i in 1..(self.height - 1) {
            ansi::move_to(out, lx, ty + i);
            write!(out, "│").unwrap();
        }

        // Right column
        for i in 1..(self.height - 1) {
            ansi::move_to(out, rx, ty + i);
            write!(out, "│").unwrap();
        }

        // Bottom border
        ansi::move_to(out, lx, by);
        write!(out, "└{:─<1$}┘", "", vw).unwrap();

        // Interior horizontal scrollbar: second-to-last row, lx+1 .. rx-1
        if self.has_hscroll() {
            let htrack = vw.saturating_sub(if self.has_vscroll() { 1 } else { 0 });
            let (htp, htl) =
                Self::scroll_thumb(htrack, self.content_w as usize, self.scroll_x as usize);
            for i in 0..htrack {
                ansi::move_to(out, lx + 1 + i as u16, by - 1);
                write!(out, "{}", Self::scroll_char(htp, htl, i)).unwrap();
            }
        }

        // Interior vertical scrollbar: second-to-last column, ty+1 .. by-2 (or by-1 without hscroll)
        if self.has_vscroll() {
            let vtrack = vh.saturating_sub(if self.has_hscroll() { 1 } else { 0 });
            let (vtp, vtl) =
                Self::scroll_thumb(vtrack, self.content_h as usize, self.scroll_y as usize);
            for i in 0..vtrack {
                ansi::move_to(out, rx - 1, ty + 1 + i as u16);
                write!(out, "{}", Self::scroll_char(vtp, vtl, i)).unwrap();
            }
        }
    }

    /// Return the visible border character at (x, y), or None for the interior.
    pub fn char_at(&self, x: u16, y: u16, title: &str) -> Option<char> {
        let lx = self.position_x;
        let rx = self.position_x + self.width - 1;
        let ty = self.position_y;
        let by = self.position_y + self.height - 1;
        if x < lx || x > rx || y < ty || y > by {
            return None;
        }

        if y == ty {
            if x == lx {
                return Some(if self.minimizable { '-' } else { '┌' });
            }
            if x == rx {
                return Some(if self.closable { 'x' } else { '┐' });
            }
            let bar = format!("{:─^1$}", format!(" {} ", title), (self.width - 2) as usize);
            return Some(bar.chars().nth((x - lx - 1) as usize).unwrap_or('─'));
        }

        if y == by {
            if x == lx {
                return Some('└');
            }
            if x == rx {
                return Some('┘');
            }
            return Some('─');
        }

        if x == rx {
            return Some('│');
        }
        if x == lx {
            return Some('│');
        }

        // Interior: vertical scrollbar (second-to-last column, excluding the junction)
        let vw = self.visible_w();
        let vh = self.visible_h();
        let both = self.has_vscroll() && self.has_hscroll();
        if self.has_vscroll() && x == rx - 1 && y > ty && y < by {
            if both && y == by - 1 {
                return Some(' ');
            }
            let vtrack = vh.saturating_sub(if both { 1 } else { 0 });
            let (vtp, vtl) =
                Self::scroll_thumb(vtrack, self.content_h as usize, self.scroll_y as usize);
            return Some(Self::scroll_char(vtp, vtl, (y - ty - 1) as usize));
        }

        // Interior: horizontal scrollbar (second-to-last row, excluding the junction)
        if self.has_hscroll() && y == by - 1 && x > lx && x < rx {
            let col = (x - lx - 1) as usize;
            let htrack = vw.saturating_sub(if both { 1 } else { 0 });
            if col < htrack {
                let (htp, htl) =
                    Self::scroll_thumb(htrack, self.content_w as usize, self.scroll_x as usize);
                return Some(Self::scroll_char(htp, htl, col));
            }
        }

        None
    }

    /// Handle an action (Space) at (x, y).
    /// Returns true if the window consumed it (scroll updated).
    pub fn interact(&mut self, x: u16, y: u16) -> bool {
        let lx = self.position_x;
        let rx = self.position_x + self.width - 1;
        let ty = self.position_y;
        let by = self.position_y + self.height - 1;
        let vh = self.visible_h();
        let vw = self.visible_w();

        let both = self.has_vscroll() && self.has_hscroll();

        // Interior vertical scrollbar: second-to-last column (excluding the junction)
        if self.has_vscroll() && x == rx - 1 && y > ty && y < by {
            if both && y == by - 1 {
                return false;
            }
            let vtrack = vh.saturating_sub(if both { 1 } else { 0 });
            let mid = ty + 1 + (vtrack / 2) as u16;
            if y < mid {
                self.scroll_y = self.scroll_y.saturating_sub(1);
            } else {
                self.scroll_y = (self.scroll_y + 1).min((self.content_h as usize - vtrack) as u16);
            }
            return true;
        }

        // Interior horizontal scrollbar: second-to-last row (excluding the junction)
        let htrack = vw.saturating_sub(if both { 1 } else { 0 });
        if self.has_hscroll() && y == by - 1 && x > lx && ((x - lx - 1) as usize) < htrack {
            let mid = lx + 1 + (htrack / 2) as u16;
            if x < mid {
                self.scroll_x = self.scroll_x.saturating_sub(1);
            } else {
                self.scroll_x = (self.scroll_x + 1).min((self.content_w as usize - htrack) as u16);
            }
            return true;
        }

        false
    }

    /// Draw the DELTA of the new size over the already rendered frame,
    /// without erasing the original.
    pub fn draw_preview(&self, out: &mut impl Write, new_w: u16, new_h: u16) {
        if new_w == self.width && new_h == self.height {
            return;
        }

        let orig_right_x = self.position_x + self.width - 1;
        let orig_bottom_y = self.position_y + self.height - 1;
        let new_right_x = self.position_x + new_w - 1;
        let new_bottom_y = self.position_y + new_h - 1;

        if new_w > self.width {
            ansi::move_to(out, orig_right_x + 1, self.position_y);
            for _ in 0..(new_w - self.width - 1) {
                write!(out, "─").unwrap();
            }
            write!(out, "┐").unwrap();
        }

        for i in 1..(new_h - 1) {
            if i == self.height - 1 && new_right_x == orig_right_x {
                continue;
            }
            ansi::move_to(out, new_right_x, self.position_y + i);
            write!(out, "│").unwrap();
        }

        if new_h > self.height {
            for i in self.height..(new_h - 1) {
                ansi::move_to(out, self.position_x, self.position_y + i);
                write!(out, "│").unwrap();
            }
        }

        if new_h == self.height && new_w > self.width {
            ansi::move_to(out, orig_right_x + 1, orig_bottom_y);
            for _ in 0..(new_w - self.width - 1) {
                write!(out, "─").unwrap();
            }
            write!(out, "┼").unwrap();
        } else {
            ansi::move_to(out, self.position_x, new_bottom_y);
            write!(out, "└{:─^1$}", "", new_w as usize - 2).unwrap();
            write!(out, "┼").unwrap();
        }
    }
}
