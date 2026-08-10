// Start menu manifest: declarative entries (label, kind, command, args,
// cwd, desktop) loaded from the user config path.
//
// JSON is parsed by a hand-written zero-dependency parser (`serde` stays
// commented in Cargo.toml), matching the portability policy of ARCHITECTURE.md.

use std::path::PathBuf;

use crate::json::Json;

// ── Model ─────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuKind {
    /// Interactive app: runs the program in a PTY/ConPTY session with the
    /// emulator rendering (editors, REPLs, `top`, ...).
    App,
    /// Plain terminal window with a persistent shell session in `cwd`.
    Terminal,
    /// Terminal window that runs `command args...` once through the shell
    /// session (builtins and external commands both work).
    Command,
}

impl MenuKind {
    fn parse(value: &str) -> Option<Self> {
        match value.trim().to_ascii_lowercase().as_str() {
            "app" => Some(MenuKind::App),
            "terminal" | "shell" => Some(MenuKind::Terminal),
            "command" | "cmd" | "run" => Some(MenuKind::Command),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct MenuItem {
    pub label:   String,
    pub kind:    MenuKind,
    pub command: String,
    pub args:    Vec<String>,
    pub cwd:     Option<String>,
    pub desktop: Option<usize>,
}

impl MenuItem {
    /// Shell command line built from `command` + `args`.
    pub fn command_line(&self) -> String {
        let mut line = self.command.trim().to_string();
        for arg in &self.args {
            if !line.is_empty() {
                line.push(' ');
            }
            line.push_str(arg);
        }
        line
    }

    /// Working directory, expanding a leading `~`, or `fallback`.
    pub fn resolve_cwd(&self, fallback: &str) -> String {
        match &self.cwd {
            Some(cwd) if !cwd.trim().is_empty() => expand_tilde(cwd.trim()),
            _ => fallback.to_string(),
        }
    }
}

/// Live state of the open start menu: entries plus keyboard selection.
pub struct MenuState {
    pub items:    Vec<MenuItem>,
    pub selected: usize,
    /// Rows scrolled off the top of the menu window.
    pub scroll:   usize,
}

impl MenuState {
    pub fn new(items: Vec<MenuItem>) -> Self {
        Self { items, selected: 0, scroll: 0 }
    }

    /// Keep the selected row inside the `visible` rows of the menu window.
    pub fn keep_selected_visible(&mut self, visible: usize) {
        let visible = visible.max(1);
        let max_scroll = self.items.len().saturating_sub(visible);
        if self.scroll > max_scroll {
            self.scroll = max_scroll;
        }
        if self.selected < self.scroll {
            self.scroll = self.selected;
        } else if self.selected >= self.scroll + visible {
            self.scroll = self.selected + 1 - visible;
        }
    }
}

// ── Config path ───────────────────────────────────────────────────────────────

/// `~/.manto/menu.json` (USERPROFILE on Windows, HOME elsewhere).
fn config_path() -> PathBuf {
    home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".manto")
        .join("menu.json")
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

fn expand_tilde(path: &str) -> String {
    if let Some(rest) = path.strip_prefix("~/") {
        if let Some(home) = home_dir() {
            return home.join(rest).to_string_lossy().to_string();
        }
    } else if path == "~"
        && let Some(home) = home_dir()
    {
        return home.to_string_lossy().to_string();
    }
    path.to_string()
}

/// Load the start menu manifest. A missing or unreadable file yields an
/// empty menu (the UI shows a hint instead of failing).
pub fn load() -> Vec<MenuItem> {
    match std::fs::read_to_string(config_path()) {
        Ok(source) => parse(&source).unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Parse a manifest source into menu items.
pub fn parse(source: &str) -> Result<Vec<MenuItem>, String> {
    let json = crate::json::parse(source)?;
    items_from_json(&json)
}

// ── Manifest mapping ──────────────────────────────────────────────────────────

fn items_from_json(json: &Json) -> Result<Vec<MenuItem>, String> {
    match json {
        Json::Arr(entries) => entries.iter().map(item_from_json).collect(),
        Json::Obj(fields) => {
            if let Some((_, Json::Arr(entries))) = fields.iter().find(|(key, _)| key == "items") {
                entries.iter().map(item_from_json).collect()
            } else {
                Ok(vec![item_from_json(json)?])
            }
        }
        _ => Err("manifest root must be an object or an array".to_string()),
    }
}

fn item_from_json(json: &Json) -> Result<MenuItem, String> {
    let Json::Obj(fields) = json else {
        return Err("menu item must be an object".to_string());
    };

    let field_str = |key: &str| -> Option<String> {
        fields.iter()
            .find(|(name, _)| name == key)
            .and_then(|(_, value)| match value {
                Json::Str(text) => Some(text.clone()),
                _ => None,
            })
    };

    let command = field_str("command").unwrap_or_default();
    let label = field_str("label")
        .filter(|label| !label.trim().is_empty())
        .unwrap_or_else(|| {
            if command.trim().is_empty() {
                "Start".to_string()
            } else {
                command.trim().to_string()
            }
        });

    let kind = field_str("kind")
        .as_deref()
        .and_then(MenuKind::parse)
        .unwrap_or(MenuKind::App);

    let mut args = Vec::new();
    if let Some((_, value)) = fields.iter().find(|(name, _)| name == "args") {
        match value {
            Json::Str(arg) => args.push(arg.clone()),
            Json::Arr(entries) => {
                for entry in entries {
                    match entry {
                        Json::Str(arg) => args.push(arg.clone()),
                        _ => return Err("args entries must be strings".to_string()),
                    }
                }
            }
            _ => return Err("args must be a string or an array of strings".to_string()),
        }
    }

    let cwd = field_str("cwd");
    let desktop = fields.iter()
        .find(|(name, _)| name == "desktop")
        .and_then(|(_, value)| match value {
            Json::Num(number) => Some(*number as i64),
            _ => None,
        })
        .map(|desktop| desktop.clamp(1, 4) as usize);

    Ok(MenuItem { label, kind, command, args, cwd, desktop })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_or_missing_source_parses_to_empty() {
        assert!(parse("").is_err());
        assert!(parse("  ").is_err());
        assert!(parse("[]").map(|items| items.is_empty()).unwrap_or(false));
    }

    #[test]
    fn manifest_array_parses_into_items() {
        let items = parse(r#"[
            { "label": "Vi", "kind": "app", "command": "vim", "args": ["-R"] },
            { "label": "Shell", "kind": "terminal", "cwd": "~" },
            { "label": "Testes", "kind": "command", "command": "cargo", "args": ["test", "--quiet"], "desktop": 3 }
        ]"#).unwrap();

        assert_eq!(items.len(), 3);
        assert_eq!(items[0].label, "Vi");
        assert_eq!(items[0].kind, MenuKind::App);
        assert_eq!(items[0].command, "vim");
        assert_eq!(items[0].args, vec!["-R"]);
        assert_eq!(items[0].cwd, None);
        assert_eq!(items[0].desktop, None);

        assert_eq!(items[1].kind, MenuKind::Terminal);
        assert_eq!(items[1].cwd.as_deref(), Some("~"));

        assert_eq!(items[2].kind, MenuKind::Command);
        assert_eq!(items[2].desktop, Some(3));
        assert_eq!(items[2].command_line(), "cargo test --quiet");
    }

    #[test]
    fn manifest_object_root_with_items_key() {
        let items = parse(r#"{
            "items": [
                { "label": "A", "command": "top" },
                { "label": "B", "kind": "command", "command": "dir" }
            ]
        }"#).unwrap();
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].kind, MenuKind::App);
        assert_eq!(items[1].kind, MenuKind::Command);
    }

    #[test]
    fn single_object_root_is_one_item() {
        let items = parse(r#"{ "label": "Só eu", "kind": "app", "command": "node" }"#).unwrap();
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].label, "Só eu");
    }

    #[test]
    fn kind_defaults_and_case_insensitive() {
        let items = parse(r#"[
            { "command": "x" },
            { "kind": "TERMINAL", "command": "" },
            { "kind": "run", "command": "y" }
        ]"#).unwrap();
        assert_eq!(items[0].kind, MenuKind::App);
        assert_eq!(items[1].kind, MenuKind::Terminal);
        assert_eq!(items[2].kind, MenuKind::Command);
        assert_eq!(items[1].command_line(), "");
    }

    #[test]
    fn label_falls_back_to_command() {
        let items = parse(r#"[{ "command": "htop" }]"#).unwrap();
        assert_eq!(items[0].label, "htop");
    }

    #[test]
    fn desktop_is_clamped() {
        let items = parse(r#"[
            { "desktop": 0, "command": "a" },
            { "desktop": 9, "command": "b" },
            { "desktop": 2.9, "command": "c" }
        ]"#).unwrap();
        assert_eq!(items[0].desktop, Some(1));
        assert_eq!(items[1].desktop, Some(4));
        assert_eq!(items[2].desktop, Some(2));
    }

    #[test]
    fn args_accept_single_string() {
        let items = parse(r#"[{ "kind": "command", "command": "echo", "args": "olá mundo" }]"#).unwrap();
        assert_eq!(items[0].args, vec!["olá mundo"]);
        assert_eq!(items[0].command_line(), "echo olá mundo");
    }

    #[test]
    fn command_line_joins_command_and_args() {
        let item = MenuItem {
            label: "x".to_string(),
            kind: MenuKind::App,
            command: "python".to_string(),
            args: vec!["-i".to_string(), "main.py".to_string()],
            cwd: None,
            desktop: None,
        };
        assert_eq!(item.command_line(), "python -i main.py");

        let item = MenuItem {
            command: "  ".to_string(),
            args: Vec::new(),
            ..item
        };
        assert_eq!(item.command_line(), "");
    }

    #[test]
    fn resolve_cwd_expands_tilde_and_falls_back() {
        let mut item = MenuItem {
            label: "x".to_string(),
            kind: MenuKind::Terminal,
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            desktop: None,
        };
        assert_eq!(item.resolve_cwd("/base"), "/base");
        assert!(!item.resolve_cwd("/base").is_empty());

        item.cwd = Some("   ".to_string());
        assert_eq!(item.resolve_cwd("/base"), "/base");

        item.cwd = Some("/tmp/manto".to_string());
        assert_eq!(item.resolve_cwd("/base"), "/tmp/manto");
    }

    #[test]
    fn example_menu_json_parses() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("example")
            .join("menu.json");
        let source = std::fs::read_to_string(path).expect("example/menu.json must exist");
        let items = parse(&source).expect("example/menu.json must be valid");
        assert!(!items.is_empty(), "example manifest has no items");
        assert!(items.iter().all(|item| !item.label.is_empty()));
        assert!(items.iter().any(|item| item.kind == MenuKind::App));
        assert!(items.iter().any(|item| item.kind == MenuKind::Terminal));
        assert!(items.iter().any(|item| item.kind == MenuKind::Command));
    }

    #[test]
    fn menu_state_selection_stays_visible() {
        let items: Vec<MenuItem> = (0..10).map(|i| MenuItem {
            label: format!("item {i}"),
            kind: MenuKind::App,
            command: String::new(),
            args: Vec::new(),
            cwd: None,
            desktop: None,
        }).collect();

        let mut state = MenuState::new(items);
        assert_eq!(state.scroll, 0);

        state.selected = 7;
        state.keep_selected_visible(4);
        assert_eq!(state.scroll, 4);

        state.selected = 2;
        state.keep_selected_visible(4);
        assert_eq!(state.scroll, 2);

        state.selected = 9;
        state.scroll = 999; // stale scroll clamps back
        state.keep_selected_visible(4);
        assert_eq!(state.scroll, 6);
    }
}