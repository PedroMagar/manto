// User configuration: desktop theme and remappable shortcuts.
//
// Loaded from `~/.manto/config.json` (USERPROFILE on Windows, HOME on Unix).
// Unknown fields are ignored; an unparsable value keeps the default binding,
// so a broken config file never breaks the desktop.

use std::path::PathBuf;

use crate::json::Json;
use crate::os::Key;

#[derive(Debug)]
pub struct Config {
    pub theme: u16,
    shortcuts: Vec<(Key, Action)>,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Action {
    NewTerminal,
    CloseWindow,
    ToggleMaximize,
    StartMenu,
    Help,
    SplitVertical,
    SplitHorizontal,
    Minimize,
    FocusNext,
    FocusPrev,
    ResizeActive,
    ToggleMouse,
    Quit,
}

impl Config {
    pub fn new(theme: u16) -> Self {
        Self {
            theme,
            shortcuts: default_shortcuts(),
        }
    }

    /// Load the configuration from disk, starting from the defaults and
    /// overlaying whatever the user configured.
    pub fn load() -> Self {
        Self::load_from(&config_path())
    }

    /// Load the configuration from `path`, starting from the defaults and
    /// overlaying whatever the user configured. A missing or broken file
    /// yields the defaults.
    pub fn load_from(path: &std::path::Path) -> Self {
        let mut config = Self::new(1);
        let Ok(source) = std::fs::read_to_string(path) else {
            return config;
        };
        let Ok(json) = crate::json::parse(&source) else {
            return config;
        };

        if let Some(theme) = json.field("theme").and_then(|v| v.as_f64()) {
            config.theme = (theme as i64).clamp(0, 2) as u16;
        }

        if let Some(Json::Obj(shortcuts)) = json.field("shortcuts") {
            for (name, value) in shortcuts {
                let Some(action) = action_for_name(name) else {
                    continue;
                };
                let Some(key) = value.str_value().and_then(parse_shortcut) else {
                    continue;
                };
                config.assign(action, key);
            }
        }
        config
    }

    /// Remap `action` to `key`, removing any earlier binding that collides.
    pub fn assign(&mut self, action: Action, key: Key) {
        self.shortcuts.retain(|(k, a)| *k != key || *a == action);
        self.shortcuts.retain(|(_, a)| *a != action);
        self.shortcuts.push((key, action));
    }

    /// The action bound to `key`, if any (most recent binding wins).
    pub fn resolve(&self, key: &Key) -> Option<Action> {
        self.shortcuts
            .iter()
            .rev()
            .find_map(|(k, a)| (k == key).then_some(*a))
    }
}

fn home_dir() -> Option<PathBuf> {
    #[cfg(unix)]
    {
        std::env::var_os("HOME").map(PathBuf::from)
    }
    #[cfg(windows)]
    {
        std::env::var_os("USERPROFILE").map(PathBuf::from)
    }
}

fn config_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".manto")
        .join("config.json")
}

fn default_shortcuts() -> Vec<(Key, Action)> {
    vec![
        (Key::CtrlT, Action::NewTerminal),
        (Key::CtrlW, Action::CloseWindow),
        (Key::CtrlF, Action::ToggleMaximize),
        (Key::CtrlD, Action::StartMenu),
        (Key::CtrlH, Action::Help),
        (Key::F1, Action::Help),
        (Key::AltV, Action::SplitVertical),
        (Key::AltH, Action::SplitHorizontal),
        (Key::CtrlX, Action::Minimize),
        (Key::CtrlN, Action::FocusNext),
        (Key::CtrlP, Action::FocusPrev),
        (Key::AltR, Action::ResizeActive),
        (Key::AltM, Action::ToggleMouse),
        (Key::CtrlDelete, Action::Quit),
    ]
}

fn action_for_name(name: &str) -> Option<Action> {
    match name.trim().to_ascii_lowercase().as_str() {
        "terminal" | "new_terminal" => Some(Action::NewTerminal),
        "close" | "close_window" => Some(Action::CloseWindow),
        "maximize" | "toggle_maximize" => Some(Action::ToggleMaximize),
        "start_menu" | "menu" => Some(Action::StartMenu),
        "help" => Some(Action::Help),
        "split_vertical" | "split_v" => Some(Action::SplitVertical),
        "split_horizontal" | "split_h" => Some(Action::SplitHorizontal),
        "minimize" => Some(Action::Minimize),
        "focus_next" | "next" => Some(Action::FocusNext),
        "focus_prev" | "prev" => Some(Action::FocusPrev),
        "resize" | "resize_active" => Some(Action::ResizeActive),
        "mouse" | "toggle_mouse" => Some(Action::ToggleMouse),
        "quit" => Some(Action::Quit),
        _ => None,
    }
}

/// Parse a shortcut descriptor like "ctrl+t", "alt+v", "enter" or "space".
/// Only keys the OS layer can actually produce are accepted.
fn parse_shortcut(spec: &str) -> Option<Key> {
    let spec = spec.trim().to_ascii_lowercase();
    if let Some(rest) = spec.strip_prefix("ctrl+") {
        return match rest {
            "1" => Some(Key::Ctrl1),
            "2" => Some(Key::Ctrl2),
            "3" => Some(Key::Ctrl3),
            "4" => Some(Key::Ctrl4),
            "c" => Some(Key::CtrlC),
            "d" => Some(Key::CtrlD),
            "e" => Some(Key::CtrlE),
            "f" => Some(Key::CtrlF),
            "h" => Some(Key::CtrlH),
            "j" => Some(Key::CtrlJ),
            "k" => Some(Key::CtrlK),
            "l" => Some(Key::CtrlL),
            "n" => Some(Key::CtrlN),
            "p" => Some(Key::CtrlP),
            "q" => Some(Key::CtrlQ),
            "w" => Some(Key::CtrlW),
            "v" => Some(Key::CtrlV),
            "x" => Some(Key::CtrlX),
            "z" => Some(Key::CtrlZ),
            "t" => Some(Key::CtrlT),
            "delete" => Some(Key::CtrlDelete),
            "enter" => Some(Key::CtrlEnter),
            _ => None,
        };
    }
    if let Some(rest) = spec.strip_prefix("alt+") {
        return match rest {
            "v" => Some(Key::AltV),
            "h" => Some(Key::AltH),
            "r" => Some(Key::AltR),
            "m" => Some(Key::AltM),
            "up" => Some(Key::AltUp),
            "down" => Some(Key::AltDown),
            "left" => Some(Key::AltLeft),
            "right" => Some(Key::AltRight),
            _ => None,
        };
    }
    match spec.as_str() {
        "enter" => Some(Key::Enter),
        "space" => Some(Key::Char(' ')),
        "escape" | "esc" => Some(Key::Escape),
        "f1" => Some(Key::F1),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_resolve_into_actions() {
        let config = Config::new(1);
        assert_eq!(config.resolve(&Key::CtrlT), Some(Action::NewTerminal));
        assert_eq!(config.resolve(&Key::CtrlF), Some(Action::ToggleMaximize));
        assert_eq!(config.resolve(&Key::AltM), Some(Action::ToggleMouse));
        assert_eq!(config.resolve(&Key::CtrlDelete), Some(Action::Quit));
        assert_eq!(config.resolve(&Key::CtrlH), Some(Action::Help));
        assert_eq!(config.resolve(&Key::F1), Some(Action::Help));
        assert_eq!(config.resolve(&Key::Enter), None);
    }

    #[test]
    fn assign_moves_a_shortcut() {
        let mut config = Config::new(1);
        // Reassigning an action moves it away from its old key.
        config.assign(Action::NewTerminal, Key::CtrlE);
        assert_eq!(config.resolve(&Key::CtrlE), Some(Action::NewTerminal));
        assert_eq!(config.resolve(&Key::CtrlT), None);
        // Two actions for one key: the latest wins.
        config.assign(Action::Minimize, Key::CtrlE);
        assert_eq!(config.resolve(&Key::CtrlE), Some(Action::Minimize));
        assert_eq!(config.resolve(&Key::CtrlT), None);
    }

    #[test]
    fn shortcut_spec_parsing() {
        assert_eq!(parse_shortcut("ctrl+t"), Some(Key::CtrlT));
        assert_eq!(parse_shortcut("ALT+V"), Some(Key::AltV));
        assert_eq!(parse_shortcut("ctrl+h"), Some(Key::CtrlH));
        assert_eq!(parse_shortcut("f1"), Some(Key::F1));
        assert_eq!(parse_shortcut("alt+m"), Some(Key::AltM));
        assert_eq!(parse_shortcut("ctrl+m"), None);
        assert_eq!(parse_shortcut("enter"), Some(Key::Enter));
        assert_eq!(parse_shortcut("space"), Some(Key::Char(' ')));
        assert_eq!(parse_shortcut("ctrl+a"), None);
        assert_eq!(parse_shortcut("garbage"), None);
    }

    #[test]
    fn action_names_parse() {
        assert_eq!(action_for_name("terminal"), Some(Action::NewTerminal));
        assert_eq!(action_for_name("start_menu"), Some(Action::StartMenu));
        assert_eq!(action_for_name("help"), Some(Action::Help));
        assert_eq!(action_for_name("split_v"), Some(Action::SplitVertical));
        assert_eq!(action_for_name("nope"), None);
    }

    #[test]
    fn config_parses_from_json() {
        let path = std::env::temp_dir().join(format!("manto-config-test-{}", std::process::id()));
        std::fs::write(
            &path,
            r#"{
                "theme": 2,
                "shortcuts": { "terminal": "ctrl+e", "quit": "ctrl+q" }
            }"#,
        )
        .unwrap();

        let config = Config::load_from(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(config.theme, 2);
        assert_eq!(config.resolve(&Key::CtrlE), Some(Action::NewTerminal));
        assert_eq!(config.resolve(&Key::CtrlQ), Some(Action::Quit));
        // Defaults for untouched actions survive.
        assert_eq!(config.resolve(&Key::CtrlF), Some(Action::ToggleMaximize));
    }

    #[test]
    fn broken_config_falls_back_to_defaults() {
        let path = std::env::temp_dir().join(format!("manto-config-broken-{}", std::process::id()));
        std::fs::write(&path, "{not json").unwrap();

        let config = Config::load_from(&path);
        let _ = std::fs::remove_file(&path);

        assert_eq!(config.theme, 1);
        assert_eq!(config.resolve(&Key::CtrlT), Some(Action::NewTerminal));
    }
}
