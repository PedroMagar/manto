use std::mem;

use crate::window::Window;

pub struct Application {
    pub title: String,
    pub display: DisplayMode,
}

pub enum DisplayMode {
    Windowed(Window),
    Minimized(Window),
    Maximized { display: Window, saved: Window },
}

impl Application {
    pub fn windowed(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window) }
    }

    pub fn window(&self) -> Option<&Window> {
        match &self.display {
            DisplayMode::Windowed(w)                  => Some(w),
            DisplayMode::Maximized { display: w, .. } => Some(w),
            _ => None,
        }
    }

    pub fn window_mut(&mut self) -> Option<&mut Window> {
        match &mut self.display {
            DisplayMode::Windowed(w)                  => Some(w),
            DisplayMode::Maximized { display: w, .. } => Some(w),
            _ => None,
        }
    }

    pub fn is_minimized(&self) -> bool {
        matches!(self.display, DisplayMode::Minimized(_))
    }

    pub fn is_maximized(&self) -> bool {
        matches!(self.display, DisplayMode::Maximized { .. })
    }

    pub fn minimize(&mut self) {
        let old = mem::replace(&mut self.display, DisplayMode::Minimized(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Windowed(w) => DisplayMode::Minimized(w),
            other => other,
        };
    }

    pub fn restore(&mut self) {
        let old = mem::replace(&mut self.display, DisplayMode::Windowed(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Minimized(w) => DisplayMode::Windowed(w),
            other => other,
        };
    }

    /// Maximiza a janela para ocupar o espaço útil da tela,
    /// preservando a geometria original para restauração.
    /// Deixa 1 coluna à esquerda (futura sidebar) e 3 à direita (abas + scrollbar),
    /// 1 linha no topo e 3 no fundo (painel do desktop).
    pub fn maximize(&mut self, screen_w: u16, screen_h: u16) {
        let old = mem::replace(&mut self.display, DisplayMode::Minimized(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Windowed(w) => DisplayMode::Maximized {
                display: Window::new(
                    2, 1,
                    screen_w.saturating_sub(5),
                    screen_h.saturating_sub(4),
                    w.layer,
                ),
                saved: w,
            },
            other => other,
        };
    }

    pub fn restore_maximize(&mut self) {
        let old = mem::replace(&mut self.display, DisplayMode::Windowed(Window::new(0, 0, 1, 1, 0)));
        self.display = match old {
            DisplayMode::Maximized { saved, .. } => DisplayMode::Windowed(saved),
            other => other,
        };
    }
}
