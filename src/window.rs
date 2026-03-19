use std::io::Write;

use crate::terminal;

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
}

impl Window {
    pub fn new(position_x: u16, position_y: u16, width: u16, height: u16, layer: u16) -> Self {
        Self { position_x, position_y, width, height, layer, minimizable: true, closable: true, draggable: true, resizable: true }
    }

    /// Remove os botões de chrome (minimizar / fechar).
    pub fn without_chrome(mut self) -> Self {
        self.minimizable = false;
        self.closable = false;
        self.draggable = false;
        self.resizable = false;
        self
    }

    pub fn draw(&self, out: &mut impl Write, title: &str) {
        terminal::move_to(out, self.position_x, self.position_y);
        write!(out, "┌{:─^1$}┐", format!(" {} ", title), self.width as usize - 2).unwrap();

        for i in 1..(self.height - 1) {
            terminal::move_to(out, self.position_x + 1, self.position_y + i);
            write!(out, "{:1$}", "", (self.width - 2) as usize).unwrap();
        }

        for i in 1..(self.height - 1) {
            terminal::move_to(out, self.position_x, self.position_y + i);
            write!(out, "│").unwrap();
            terminal::move_to(out, self.position_x + self.width - 1, self.position_y + i);
            write!(out, "│").unwrap();
        }

        terminal::move_to(out, self.position_x, self.position_y + self.height - 1);
        write!(out, "└{:─^1$}", "", self.width as usize - 2).unwrap();
        write!(out, "┘").unwrap();
    }

    /// Retorna o caractere visível na borda em (x, y), ou None se for interior.
    /// Os cantos de ação respeitam as flags de chrome: '-'/'┌' e 'x'/'┐'.
    pub fn char_at(&self, x: u16, y: u16, title: &str) -> Option<char> {
        let lx = self.position_x;
        let rx = self.position_x + self.width - 1;
        let ty = self.position_y;
        let by = self.position_y + self.height - 1;
        if x < lx || x > rx || y < ty || y > by { return None; }
        if y == ty {
            if x == lx { return Some(if self.minimizable { '-' } else { '┌' }); }
            if x == rx { return Some(if self.closable   { 'x' } else { '┐' }); }
            let bar = format!("{:─^1$}", format!(" {} ", title), (self.width - 2) as usize);
            return Some(bar.chars().nth((x - lx - 1) as usize).unwrap_or('─'));
        }
        if y == by {
            if x == lx { return Some('└'); }
            if x == rx { return Some('┘'); }
            return Some('─');
        }
        if x == lx || x == rx { return Some('│'); }
        None
    }

    /// Desenha o DELTA do novo tamanho sobre o frame já renderizado, sem apagar o original.
    pub fn draw_preview(&self, out: &mut impl Write, new_w: u16, new_h: u16) {
        if new_w == self.width && new_h == self.height {
            return;
        }

        let orig_right_x  = self.position_x + self.width - 1;
        let orig_bottom_y = self.position_y + self.height - 1;
        let new_right_x   = self.position_x + new_w - 1;
        let new_bottom_y  = self.position_y + new_h - 1;

        if new_w > self.width {
            terminal::move_to(out, orig_right_x + 1, self.position_y);
            for _ in 0..(new_w - self.width - 1) {
                write!(out, "─").unwrap();
            }
            write!(out, "┐").unwrap();
        }

        for i in 1..(new_h - 1) {
            if i == self.height - 1 && new_right_x == orig_right_x {
                continue;
            }
            terminal::move_to(out, new_right_x, self.position_y + i);
            write!(out, "│").unwrap();
        }

        if new_h > self.height {
            for i in self.height..(new_h - 1) {
                terminal::move_to(out, self.position_x, self.position_y + i);
                write!(out, "│").unwrap();
            }
        }

        if new_h == self.height && new_w > self.width {
            terminal::move_to(out, orig_right_x + 1, orig_bottom_y);
            for _ in 0..(new_w - self.width - 1) {
                write!(out, "─").unwrap();
            }
            write!(out, "┼").unwrap();
        } else {
            terminal::move_to(out, self.position_x, new_bottom_y);
            write!(out, "└{:─^1$}", "", new_w as usize - 2).unwrap();
            write!(out, "┼").unwrap();
        }
    }
}
