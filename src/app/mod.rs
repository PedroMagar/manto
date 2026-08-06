// Application domain state: the logical app, its display mode, the terminal
// window state, and the desktop session that ties everything together.

pub mod desktop;
pub mod terminal;

pub use desktop::Desktop;
pub use terminal::TerminalState;

use std::mem;

use crate::cmd::CommandEntry;
use crate::ui::window::Window;

pub struct Application {
    pub title:    String,
    pub display:  DisplayMode,
    pub desktop:  usize,
    /// Menu windows close when they lose focus.
    pub is_menu:  bool,
    /// Present in terminal windows; absent in plain windows.
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

    /// Create a terminal window with preloaded command history and a
    /// persistent shell session.
    pub fn terminal_window(title: impl Into<String>, window: Window, path: String, commands: Vec<CommandEntry>) -> Self {
        match TerminalState::with_shell(path.clone(), commands.clone()) {
            Ok(ts) => Self {
                title:    title.into(),
                display:  DisplayMode::Windowed(window),
                desktop:  1,
                is_menu:  false,
                terminal: Some(ts),
            },
            Err(_) => {
                // Fallback to non-session mode if shell spawn fails
                Self {
                    title:    title.into(),
                    display:  DisplayMode::Windowed(window),
                    desktop:  1,
                    is_menu:  false,
                    terminal: Some(TerminalState::new(path, commands)),
                }
            }
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

    pub fn saved_window(&self) -> Option<&Window> {
        match &self.display {
            DisplayMode::Maximized { saved, .. } => Some(saved),
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

    /// Maximize the window to fill the usable screen area, preserving the
    /// original geometry for restore.
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
