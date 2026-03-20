use std::mem;

use crate::cmd::{CommandEntry, tick_all};
use crate::window::Window;

// ── Estado de janela terminal ─────────────────────────────────────────────────

pub struct TerminalState {
    pub commands:     Vec<CommandEntry>,
    pub cmd_input:    String,
    pub input_cursor: usize,
    pub panel_scroll: usize,
    pub path:         String,
    pub history_index: Option<usize>,
    pub history_draft: Option<String>,
}

impl TerminalState {
    pub fn new(path: String, commands: Vec<CommandEntry>) -> Self {
        Self {
            path,
            commands,
            cmd_input: String::new(),
            input_cursor: 0,
            panel_scroll: 0,
            history_index: None,
            history_draft: None,
        }
    }

    /// Avança um tick em todos os comandos. Retorna true se houve mudança.
    pub fn tick(&mut self) -> bool {
        tick_all(&mut self.commands)
    }
}

// ── Application ───────────────────────────────────────────────────────────────

pub struct Application {
    pub title:    String,
    pub display:  DisplayMode,
    pub desktop:  usize,
    /// Janelas de menu fecham ao perder o foco.
    pub is_menu:  bool,
    /// Presente em janelas de terminal; ausente em janelas comuns.
    pub terminal: Option<TerminalState>,
}

pub enum DisplayMode {
    Windowed(Window),
    Minimized(Window),
    Maximized { display: Window, saved: Window },
}

impl Application {
    pub fn windowed(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window), desktop: 1, is_menu: false, terminal: None }
    }

    pub fn menu(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window), desktop: 1, is_menu: true, terminal: None }
    }

    /// Cria uma janela de terminal com histórico de comandos pré-carregado.
    pub fn terminal_window(title: impl Into<String>, window: Window, path: String, commands: Vec<CommandEntry>) -> Self {
        Self {
            title:    title.into(),
            display:  DisplayMode::Windowed(window),
            desktop:  1,
            is_menu:  false,
            terminal: Some(TerminalState::new(path, commands)),
        }
    }

    pub fn with_desktop(mut self, desktop: usize) -> Self {
        self.desktop = desktop;
        self
    }

    pub fn on_desktop(&self, desktop: usize) -> bool {
        self.desktop == desktop
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

    pub fn set_window_geometry(&mut self, position_x: u16, position_y: u16, width: u16, height: u16) {
        let template = match &self.display {
            DisplayMode::Windowed(w) => Some(w),
            DisplayMode::Maximized { display, .. } => Some(display),
            DisplayMode::Minimized(_) => None,
        };

        let Some(template) = template else {
            return;
        };

        let mut win = Window::new(position_x, position_y, width, height, template.layer);
        win.minimizable = template.minimizable;
        win.closable = template.closable;
        win.draggable = template.draggable;
        win.resizable = template.resizable;
        win.content_w = template.content_w;
        win.content_h = template.content_h;
        win.scroll_x = template.scroll_x;
        win.scroll_y = template.scroll_y;
        self.display = DisplayMode::Windowed(win);
    }
}
