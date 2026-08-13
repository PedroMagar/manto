// Help window: a static crib sheet of Manto usage, mirroring the
// README "How To Use" and shortcut sections. The content is compiled in so
// the window works on any host without reading files.

/// Live state of the open help window.
pub struct HelpState {
    /// Source lines (no wrapping applied).
    pub lines: Vec<String>,
    /// Rows scrolled off the top of the window.
    pub scroll: usize,
}

impl HelpState {
    pub fn new() -> Self {
        Self {
            lines: content(),
            scroll: 0,
        }
    }
}

/// Number of display rows the source `lines` occupy once wrapped to `width`.
pub fn wrapped_count(lines: &[String], width: usize) -> usize {
    let width = width.max(1);
    lines
        .iter()
        .map(|line| {
            let len = line.chars().count();
            if len == 0 { 1 } else { len.div_ceil(width) }
        })
        .sum()
}

/// The source `lines` wrapped to `width` characters each.
pub fn wrapped(lines: &[String], width: usize) -> Vec<String> {
    let width = width.max(1);
    lines
        .iter()
        .flat_map(|line| {
            let chars: Vec<char> = line.chars().collect();
            if chars.is_empty() {
                vec![String::new()]
            } else {
                chars
                    .chunks(width)
                    .map(|chunk| chunk.iter().collect())
                    .collect()
            }
        })
        .collect()
}

/// The help crib sheet, line by line.
pub fn content() -> Vec<String> {
    CRIBSHEET.iter().map(|line| line.to_string()).collect()
}

const CRIBSHEET: &[&str] = &[
    "MANTO  —  HOW TO USE",
    "─────────────────────",
    "",
    "CONTEXTS",
    "  Normal         move the pointer, interact with windows",
    "  Typing         type into the dock shell (.> )",
    "  TerminalFocus  type inside a detached terminal",
    "  Moving         reposition the active window",
    "  Resizing       change the active window size",
    "",
    "MOUSE  (toggle with Alt+M)",
    "  Left click         activate what is under the pointer",
    "  Click + drag       select a text box",
    "                     (Enter / Ctrl+C copies, Esc clears)",
    "  Drag title bar     move the window",
    "  Drag bottom-right  resize the window",
    "  Double-click title maximize / restore",
    "  Right click        raise the window under the pointer",
    "  Wheel              scroll terminals, the panel or the rail",
    "",
    "GLOBAL SHORTCUTS",
    "  Ctrl+T       open a new terminal window",
    "  Ctrl+W       close the active window",
    "  Ctrl+F       maximize / restore the active window",
    "  Ctrl+N/P     focus the next / previous window",
    "  Ctrl+X       minimize the active window",
    "  Ctrl+D       open / close the Start menu",
    "  Alt+M        toggle mouse input",
    "  Ctrl+H / F1  toggle this help window",
    "  Ctrl+1..4    move the active window to desktop 1-4",
    "  1..4         switch to desktop 1-4",
    "  Ctrl+Delete  quit Manto (saves the session)",
    "",
    "SNAP AND SPLIT",
    "  Alt+Arrow           snap the window to a half",
    "  Alt+Arrow+other     snap to a quarter",
    "  Alt+V / Alt+H       split the terminal vertically / horizontally",
    "  Alt+R               enter resize mode for the active window",
    "",
    "NORMAL MODE",
    "  Arrows        move the pointer",
    "  Home          move the pointer to the dock input",
    "  :             start typing in the dock",
    "  Space/Enter   activate what is under the pointer",
    "  Shift+Arrows  free screen text selection",
    "  Esc           clear the selection",
    "",
    "DOCK SHELL (.> )",
    "  Esc / End    leave typing mode",
    "  Ctrl+Enter   detach the dock into a terminal window",
    "  Up / Down    browse command history",
    "  Left / Right move the text cursor",
    "  Tab          autocomplete commands and paths",
    "  Enter        run the command",
    "  PageUp/Down  scroll the command panel",
    "",
    "TERMINAL WINDOW",
    "  Esc / End    return to the desktop",
    "  Up / Down    command history",
    "  Left / Right move the text cursor",
    "  Tab          autocomplete",
    "  Enter        run the command",
    "  PageUp/Down  scroll the terminal output",
    "  #i app       run an interactive app (vim, python, shell...)",
    "  inside #i:   Ctrl+C interrupt, Ctrl+V paste, mouse forwarded",
    "",
    "MOVING MODE",
    "  Arrows        move the window preview",
    "  Space / Enter confirm the new position",
    "",
    "RESIZE MODE",
    "  Arrows        change the size preview",
    "  Space / Enter apply and exit",
    "  Esc           cancel",
    "  X/H width, Y/V height, then +/-/=/digits, Enter applies",
    "",
    "CONFIGURATION",
    "  ~/.manto/config.json  theme (0-2) + remappable shortcuts",
    "  ~/.manto/menu.json    Start menu entries",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_count_matches_wrapped_len() {
        let lines = vec!["hello world".to_string(), String::new(), "ab".to_string()];
        for width in [1, 2, 5, 80] {
            let wrapped = wrapped(&lines, width);
            assert_eq!(wrapped_count(&lines, width), wrapped.len());
        }
    }

    #[test]
    fn wrapping_breaks_long_lines() {
        let lines = vec!["abcdef".to_string()];
        assert_eq!(
            wrapped(&lines, 2),
            vec!["ab".to_string(), "cd".to_string(), "ef".to_string()]
        );
        // Empty source lines stay as a single row.
        assert_eq!(wrapped(&[String::new()], 5), vec![String::new()]);
    }
}
