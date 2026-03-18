use std::mem;

use crate::window::Window;

pub struct Application {
    pub title: String,
    pub display: DisplayMode,
}

pub enum DisplayMode {
    Windowed(Window),
    Minimized(Window),
    // Fullscreen(Window),
    // Tab(Window),
}

impl Application {
    pub fn windowed(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window) }
    }

    pub fn window(&self) -> Option<&Window> {
        match &self.display {
            DisplayMode::Windowed(w) => Some(w),
            _ => None,
        }
    }

    pub fn window_mut(&mut self) -> Option<&mut Window> {
        match &mut self.display {
            DisplayMode::Windowed(w) => Some(w),
            _ => None,
        }
    }

    pub fn is_minimized(&self) -> bool {
        matches!(self.display, DisplayMode::Minimized(_))
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
}
