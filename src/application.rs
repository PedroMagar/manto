use crate::window::Window;

pub struct Application {
    pub title: String,
    pub display: DisplayMode,
}

pub enum DisplayMode {
    Windowed(Window),
    // Fullscreen(Window),
    // Tab(Window),
    // Minimized,
}

impl Application {
    pub fn windowed(title: impl Into<String>, window: Window) -> Self {
        Self { title: title.into(), display: DisplayMode::Windowed(window) }
    }

    pub fn window(&self) -> Option<&Window> {
        match &self.display {
            DisplayMode::Windowed(w) => Some(w),
        }
    }

    pub fn window_mut(&mut self) -> Option<&mut Window> {
        match &mut self.display {
            DisplayMode::Windowed(w) => Some(w),
        }
    }
}
