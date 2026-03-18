use std::io::Write;

use crate::terminal;

pub struct Pointer {
    pub x: u16,
    pub y: u16,
}

impl Pointer {
    pub fn new(x: u16, y: u16) -> Self {
        Self { x, y }
    }

    pub fn clamp_to_bounds(&mut self, width: u16, height: u16) {
        self.x = self.x.max(1).min(width.saturating_sub(2));
        self.y = self.y.max(1).min(height.saturating_sub(2));
    }

    pub fn move_up(&mut self) {
        self.y = self.y.saturating_sub(1).max(1);
    }

    pub fn move_down(&mut self, max: u16) {
        self.y = (self.y + 1).min(max.saturating_sub(2));
    }

    pub fn move_left(&mut self) {
        self.x = self.x.saturating_sub(1).max(1);
    }

    pub fn move_right(&mut self, max: u16) {
        self.x = (self.x + 1).min(max.saturating_sub(2));
    }

    /// Desenha o cursor.
    /// `interaction`: None = cursor normal (░)
    ///                Some(c) = cursor interativo — mostra `c` em reverse video
    pub fn draw(&self, out: &mut impl Write, interaction: Option<char>) {
        terminal::move_to(out, self.x, self.y);
        match interaction {
            None    => write!(out, "░").unwrap(),
            Some(c) => write!(out, "{}{}{}", terminal::REVERSE, c, terminal::RESET).unwrap(),
        }
    }
}
