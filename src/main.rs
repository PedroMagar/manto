mod ansi;
mod application;
mod cmd;
mod gui;
mod os;
mod pointer;
mod terminal_backend;
mod window;

use gui::{draw_desktop, draw_status_bar, draw_tab, draw_scrollbar, draw_command_panel,
          draw_terminal_content, tab_char_at, scrollbar_thumb, desktop_at,
          STATUS_BAR_PREFIX, STATUS_START, STATUS_START_X, CMD_INPUT_X, DESKTOP_AREA_LEN,
          TERMINAL_INPUT_PREFIX};
use cmd::{CommandEntry, tick_all};
pub use application::{Application, TerminalState};
use window::{Window, MIN_W, MIN_H};
use os::{Writer, Clock, Key};
use pointer::Pointer;
use std::collections::HashSet;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::time::Duration;

enum Mode {
    Normal,
    Moving          { app_idx: usize, offset_x: u16 },
    Resizing        { app_idx: usize, edit: Option<ResizeEditState> },
    Typing,
    TerminalFocus   { app_idx: usize },
}

#[derive(Clone, Copy)]
enum ResizeAxis {
    Width,
    Height,
}

#[derive(Clone, Copy)]
enum ResizeOp {
    Add,
    Sub,
    Set,
}

struct ResizeEditState {
    axis: ResizeAxis,
    op: Option<ResizeOp>,
    value: String,
}

#[derive(Clone, Copy)]
enum SnapRegion {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

fn resolve_snap_region(key: &Key, held: os::HeldArrowKeys) -> Option<SnapRegion> {
    match key {
        Key::AltLeft => Some(if held.up {
            SnapRegion::TopLeft
        } else if held.down {
            SnapRegion::BottomLeft
        } else {
            SnapRegion::Left
        }),
        Key::AltRight => Some(if held.up {
            SnapRegion::TopRight
        } else if held.down {
            SnapRegion::BottomRight
        } else {
            SnapRegion::Right
        }),
        Key::AltUp => Some(if held.left {
            SnapRegion::TopLeft
        } else if held.right {
            SnapRegion::TopRight
        } else {
            SnapRegion::Top
        }),
        Key::AltDown => Some(if held.left {
            SnapRegion::BottomLeft
        } else if held.right {
            SnapRegion::BottomRight
        } else {
            SnapRegion::Bottom
        }),
        _ => None,
    }
}

#[cfg(windows)]
fn normalize_host_path(path: &Path) -> String {
    let raw = path.display().to_string();
    if let Some(rest) = raw.strip_prefix(r"\\?\UNC\") {
        format!(r"\\{}", rest)
    } else if let Some(rest) = raw.strip_prefix(r"\\?\") {
        rest.to_string()
    } else {
        raw
    }
}

#[cfg(not(windows))]
fn normalize_host_path(path: &Path) -> String {
    path.display().to_string()
}

fn resolve_virtual_path(current_path: &str, target: &str) -> Result<String, String> {
    let target = target.trim();
    if target.is_empty() {
        return Ok(current_path.to_string());
    }

    let candidate = if Path::new(target).is_absolute() {
        PathBuf::from(target)
    } else {
        Path::new(current_path).join(target)
    };

    let resolved = std::fs::canonicalize(&candidate)
        .map_err(|err| format!("cd: {target}: {err}"))?;
    if !resolved.is_dir() {
        return Err(format!("cd: {target}: not a directory"));
    }
    Ok(normalize_host_path(&resolved))
}

fn push_shell_command(commands: &mut Vec<CommandEntry>, current_path: &mut String, raw_command: &str) {
    let trimmed = raw_command.trim();
    if trimmed.is_empty() {
        return;
    }
    let command_cwd = current_path.clone();

    if trimmed == "cd" {
        commands.push(CommandEntry::completed(trimmed, &command_cwd, vec![command_cwd.clone()]));
        return;
    }

    if let Some(rest) = trimmed.strip_prefix("cd ")
        .or_else(|| trimmed.strip_prefix("cd\t")) {
        match resolve_virtual_path(current_path, rest) {
            Ok(path) => {
                commands.push(CommandEntry::completed(trimmed, &command_cwd, vec![path.clone()]));
                *current_path = path;
            }
            Err(err) => commands.push(CommandEntry::completed(trimmed, &command_cwd, vec![err])),
        }
        return;
    }

    commands.push(CommandEntry::spawn(trimmed, &command_cwd));
}

fn history_up(commands: &[CommandEntry], input: &mut String, index: &mut Option<usize>, draft: &mut Option<String>) -> bool {
    if commands.is_empty() {
        return false;
    }

    let next = match *index {
        Some(current) if current > 0 => current - 1,
        Some(_) => 0,
        None => {
            *draft = Some(input.clone());
            commands.len() - 1
        }
    };

    *index = Some(next);
    *input = commands[next].command.clone();
    true
}

fn history_down(commands: &[CommandEntry], input: &mut String, index: &mut Option<usize>, draft: &mut Option<String>) -> bool {
    let Some(current) = *index else {
        return false;
    };

    if current + 1 < commands.len() {
        let next = current + 1;
        *index = Some(next);
        *input = commands[next].command.clone();
    } else {
        *index = None;
        *input = draft.take().unwrap_or_default();
    }
    true
}

fn reset_history_navigation(index: &mut Option<usize>, draft: &mut Option<String>) {
    *index = None;
    *draft = None;
}

fn input_char_len(input: &str) -> usize {
    input.chars().count()
}

fn cursor_to_byte(input: &str, cursor: usize) -> usize {
    input.char_indices().nth(cursor).map(|(idx, _)| idx).unwrap_or(input.len())
}

fn move_input_cursor_left(cursor: &mut usize) -> bool {
    if *cursor == 0 {
        false
    } else {
        *cursor -= 1;
        true
    }
}

fn move_input_cursor_right(input: &str, cursor: &mut usize) -> bool {
    let len = input_char_len(input);
    if *cursor >= len {
        false
    } else {
        *cursor += 1;
        true
    }
}

fn insert_input_char(input: &mut String, cursor: &mut usize, ch: char) {
    let byte = cursor_to_byte(input, *cursor);
    input.insert(byte, ch);
    *cursor += 1;
}

fn backspace_input_char(input: &mut String, cursor: &mut usize) -> bool {
    if *cursor == 0 {
        return false;
    }

    let end = cursor_to_byte(input, *cursor);
    let start = cursor_to_byte(input, *cursor - 1);
    input.replace_range(start..end, "");
    *cursor -= 1;
    true
}

fn delete_input_char(input: &mut String, cursor: &mut usize) -> bool {
    let len = input_char_len(input);
    if *cursor >= len {
        return false;
    }

    let start = cursor_to_byte(input, *cursor);
    let end = cursor_to_byte(input, *cursor + 1);
    input.replace_range(start..end, "");
    true
}

fn input_view(input: &str, cursor: usize, max_len: usize) -> (String, usize) {
    if max_len == 0 {
        return (String::new(), 0);
    }

    let chars: Vec<char> = input.chars().collect();
    let len = chars.len();
    if len <= max_len {
        return (input.to_string(), cursor.min(len));
    }

    let mut start = cursor.saturating_sub(max_len.saturating_sub(1));
    if start + max_len > len {
        start = len.saturating_sub(max_len);
    }

    let end = (start + max_len).min(len);
    let display: String = chars[start..end].iter().collect();
    (display, cursor.saturating_sub(start).min(max_len))
}

fn token_bounds(input: &str, cursor: usize) -> (usize, usize) {
    let chars: Vec<char> = input.chars().collect();
    let cursor = cursor.min(chars.len());

    let mut start = cursor;
    while start > 0 && !chars[start - 1].is_whitespace() {
        start -= 1;
    }

    let mut end = cursor;
    while end < chars.len() && !chars[end].is_whitespace() {
        end += 1;
    }

    (start, end)
}

fn replace_token(input: &mut String, cursor: &mut usize, start: usize, end: usize, replacement: &str) {
    let start_byte = cursor_to_byte(input, start);
    let end_byte = cursor_to_byte(input, end);
    input.replace_range(start_byte..end_byte, replacement);
    *cursor = start + replacement.chars().count();
}

fn longest_common_prefix(values: &[String]) -> String {
    let Some(first) = values.first() else {
        return String::new();
    };

    let mut prefix: Vec<char> = first.chars().collect();
    for value in &values[1..] {
        let chars: Vec<char> = value.chars().collect();
        let common = prefix.iter().zip(chars.iter()).take_while(|(a, b)| a == b).count();
        prefix.truncate(common);
        if prefix.is_empty() {
            break;
        }
    }

    prefix.into_iter().collect()
}

fn path_token_parts(token: &str) -> (String, String) {
    match token.rfind(['\\', '/']) {
        Some(idx) => {
            let split = idx + 1;
            (token[..split].to_string(), token[split..].to_string())
        }
        None => (String::new(), token.to_string()),
    }
}

fn collect_path_candidates(current_path: &str, token: &str, dirs_only: bool) -> Vec<(String, bool)> {
    let (base_part, leaf) = path_token_parts(token);
    let base_path = if base_part.is_empty() {
        PathBuf::from(current_path)
    } else {
        let base = PathBuf::from(&base_part);
        if base.is_absolute() {
            base
        } else {
            Path::new(current_path).join(&base_part)
        }
    };

    let mut candidates = Vec::new();
    let Ok(entries) = std::fs::read_dir(&base_path) else {
        return candidates;
    };

    for entry in entries.flatten() {
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if dirs_only && !file_type.is_dir() {
            continue;
        }

        let name = entry.file_name().to_string_lossy().to_string();
        if !name.to_ascii_lowercase().starts_with(&leaf.to_ascii_lowercase()) {
            continue;
        }

        let mut display = format!("{}{}", base_part, name);
        if file_type.is_dir() {
            display.push(std::path::MAIN_SEPARATOR);
        }
        candidates.push((display, file_type.is_dir()));
    }

    candidates.sort_by(|a, b| a.0.cmp(&b.0));
    candidates
}

fn collect_command_candidates(current_path: &str, prefix: &str) -> Vec<String> {
    let mut seen = HashSet::new();
    let mut candidates = Vec::new();
    let prefix_lower = prefix.to_ascii_lowercase();

    let mut search_dirs = vec![PathBuf::from(current_path)];
    if let Some(path_var) = std::env::var_os("PATH") {
        search_dirs.extend(std::env::split_paths(&path_var));
    }

    #[cfg(windows)]
    let pathext: Vec<String> = std::env::var("PATHEXT")
        .unwrap_or_else(|_| ".COM;.EXE;.BAT;.CMD".to_string())
        .split(';')
        .map(|ext| ext.to_ascii_lowercase())
        .collect();

    for dir in search_dirs {
        let Ok(entries) = std::fs::read_dir(dir) else {
            continue;
        };

        for entry in entries.flatten() {
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_dir() {
                continue;
            }

            let file_name = entry.file_name().to_string_lossy().to_string();

            #[cfg(windows)]
            let candidate = {
                let path = entry.path();
                let ext = path.extension()
                    .map(|ext| format!(".{}", ext.to_string_lossy().to_ascii_lowercase()))
                    .unwrap_or_default();
                if !pathext.iter().any(|allowed| allowed == &ext) {
                    continue;
                }
                path.file_stem()
                    .map(|stem| stem.to_string_lossy().to_string())
                    .unwrap_or(file_name.clone())
            };

            #[cfg(unix)]
            let candidate = {
                use std::os::unix::fs::PermissionsExt;
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if meta.permissions().mode() & 0o111 == 0 {
                    continue;
                }
                file_name.clone()
            };

            if !candidate.to_ascii_lowercase().starts_with(&prefix_lower) {
                continue;
            }

            let key = candidate.to_ascii_lowercase();
            if seen.insert(key) {
                candidates.push(candidate);
            }
        }
    }

    candidates.sort();
    candidates
}

fn autocomplete_input(input: &mut String, cursor: &mut usize, current_path: &str) -> bool {
    let (start, end) = token_bounds(input, *cursor);
    let chars: Vec<char> = input.chars().collect();
    let token: String = chars[start..end].iter().collect();
    let first_token_end = chars.iter().position(|c| c.is_whitespace()).unwrap_or(chars.len());
    let first_token: String = chars[..first_token_end].iter().collect();

    let suggestions: Vec<String> = if start == 0 {
        collect_command_candidates(current_path, &token)
    } else if first_token == "cd" {
        collect_path_candidates(current_path, &token, true).into_iter().map(|(text, _)| text).collect()
    } else if token.contains(['\\', '/']) || token.starts_with('.') {
        collect_path_candidates(current_path, &token, false).into_iter().map(|(text, _)| text).collect()
    } else {
        Vec::new()
    };

    if suggestions.is_empty() {
        return false;
    }

    let replacement = if suggestions.len() == 1 {
        let mut value = suggestions[0].clone();
        if start == 0 && !value.ends_with(' ') {
            value.push(' ');
        }
        value
    } else {
        let lcp = longest_common_prefix(&suggestions);
        if lcp.chars().count() <= token.chars().count() {
            return false;
        }
        lcp
    };

    replace_token(input, cursor, start, end, &replacement);
    true
}

fn interact_terminal_horizontal_scroll(app: &mut Application, x: u16, y: u16) -> bool {
    let Some(term) = app.terminal.as_ref() else {
        return false;
    };
    let Some(win) = app.window() else {
        return false;
    };

    let has_hscroll = win.content_w as usize > win.width.saturating_sub(2) as usize;
    let path_y = win.position_y + win.height.saturating_sub(if has_hscroll { 4 } else { 3 });
    if y != path_y || x <= win.position_x || x >= win.position_x + win.width - 1 {
        return false;
    }

    let inner_w = win.width.saturating_sub(2) as usize;
    let max_scroll = gui::terminal_content_width(&term.path, &term.commands).saturating_sub(inner_w) as u16;
    if max_scroll == 0 {
        return false;
    }

    let mid = win.position_x + 1 + (inner_w as u16 / 2);
    if let Some(win) = app.window_mut() {
        if x < mid {
            win.scroll_x = win.scroll_x.saturating_sub(1);
        } else {
            win.scroll_x = (win.scroll_x + 1).min(max_scroll);
        }
        return true;
    }

    false
}

fn sync_terminal_window_metrics(applications: &mut [Application]) {
    for app in applications.iter_mut() {
        let Some(term) = app.terminal.as_ref() else {
            continue;
        };
        let content_w = gui::terminal_content_width(&term.path, &term.commands);

        if let Some(win) = app.window_mut() {
            let visible_w = win.width.saturating_sub(2) as usize;
            let max_scroll = content_w.saturating_sub(visible_w) as u16;
            win.content_w = content_w.min(u16::MAX as usize) as u16;
            win.scroll_x = win.scroll_x.min(max_scroll);
        }
    }
}

/// Retorna o índice da janela visualmente no topo na posição (x, y).
fn topmost_window_at(applications: &[Application], current_desktop: usize, x: u16, y: u16) -> Option<usize> {
    applications.iter().rposition(|app| {
        app.on_desktop(current_desktop) && app.window().map_or(false, |win| {
            x >= win.position_x
                && x < win.position_x + win.width
                && y >= win.position_y
                && y < win.position_y + win.height
        })
    })
}

/// Calcula (app_idx, tab_y, tab_height) para cada app minimizado visível.
fn tab_layout(applications: &[Application], current_desktop: usize, screen_h: u16, scroll: usize) -> Vec<(usize, u16, u16)> {
    let usable_h = screen_h.saturating_sub(4);
    let minimized: Vec<usize> = applications.iter().enumerate()
        .filter(|(_, a)| a.on_desktop(current_desktop) && a.is_minimized())
        .map(|(i, _)| i)
        .collect();

    if minimized.is_empty() || usable_h == 0 { return vec![]; }

    // tab_h 8 → 6 conteúdo + 2 bordas; compacto 6 → 4 conteúdo (mínimo garantido)
    let tab_h: u16 = if minimized.len() as u16 * 8 <= usable_h { 8 } else { 6 };
    let max_visible = (usable_h / tab_h) as usize;

    minimized.into_iter()
        .skip(scroll)
        .take(max_visible)
        .enumerate()
        .map(|(i, app_idx)| (app_idx, 1 + i as u16 * tab_h, tab_h))
        .collect()
}


/// Scroll máximo possível para as abas.
fn max_tab_scroll(applications: &[Application], current_desktop: usize, screen_h: u16) -> usize {
    let usable_h = screen_h.saturating_sub(4);
    let total = applications.iter().filter(|a| a.on_desktop(current_desktop) && a.is_minimized()).count();
    if total == 0 || usable_h == 0 { return 0; }
    let tab_h: u16 = if (total as u16) * 8 <= usable_h { 8 } else { 6 };
    let max_visible = (usable_h / tab_h) as usize;
    total.saturating_sub(max_visible)
}

fn active_window_idx(applications: &[Application], mode: &Mode, current_desktop: usize) -> Option<usize> {
    match mode {
        Mode::Moving { app_idx, .. }
        | Mode::Resizing { app_idx, .. }
        | Mode::TerminalFocus { app_idx } => applications
            .get(*app_idx)
            .filter(|app| app.on_desktop(current_desktop))
            .and_then(|app| app.window())
            .map(|_| *app_idx),
        Mode::Normal => applications.iter().enumerate()
            .rfind(|(_, app)| app.on_desktop(current_desktop) && app.window().is_some())
            .map(|(idx, _)| idx),
        Mode::Typing => None,
    }
}

fn close_active_window(applications: &mut Vec<Application>, mode: &mut Mode, current_desktop: usize, screen_h: u16, tab_scroll: &mut usize) -> bool {
    let Some(idx) = active_window_idx(applications, mode, current_desktop) else {
        return false;
    };

    let can_close = applications.get(idx)
        .and_then(|app| app.window())
        .map_or(false, |win| win.closable);

    if !can_close {
        return false;
    }

    applications.remove(idx);
    *tab_scroll = (*tab_scroll).min(max_tab_scroll(applications, current_desktop, screen_h));
    *mode = Mode::Normal;
    true
}

fn bring_window_to_front(applications: &mut Vec<Application>, idx: usize) -> usize {
    if idx >= applications.len() || idx == applications.len() - 1 {
        idx
    } else {
        let app = applications.remove(idx);
        applications.push(app);
        applications.len() - 1
    }
}

fn spawn_terminal_window(
    applications: &mut Vec<Application>,
    next_terminal_id: &mut usize,
    current_desktop: usize,
    screen_w: u16,
    screen_h: u16,
    path: &str,
    commands: Vec<CommandEntry>,
) -> usize {
    let id = *next_terminal_id;
    *next_terminal_id += 1;
    let title = format!("Terminal {}", id);
    let usable_h = screen_h.saturating_sub(4);
    let tw = (screen_w / 2).max(30).min(screen_w.saturating_sub(6));
    let th = (usable_h * 2 / 3).max(8).min(usable_h);
    let tx = (screen_w.saturating_sub(tw)) / 2;
    let ty = 1 + usable_h.saturating_sub(th) / 2;
    let win = Window::new(tx, ty, tw, th, 0);
    applications.push(Application::terminal_window(title, win, path.to_string(), commands).with_desktop(current_desktop));
    applications.len() - 1
}

fn spawn_terminal_window_at(
    applications: &mut Vec<Application>,
    next_terminal_id: &mut usize,
    current_desktop: usize,
    position_x: u16,
    position_y: u16,
    width: u16,
    height: u16,
    path: &str,
    commands: Vec<CommandEntry>,
) -> usize {
    let id = *next_terminal_id;
    *next_terminal_id += 1;
    let title = format!("Terminal {}", id);
    let win = Window::new(position_x, position_y, width, height, 0);
    applications.push(
        Application::terminal_window(title, win, path.to_string(), commands)
            .with_desktop(current_desktop),
    );
    applications.len() - 1
}

#[derive(Clone, Copy)]
enum SplitDirection {
    Vertical,
    Horizontal,
}

fn split_active_terminal_window(
    applications: &mut Vec<Application>,
    mode: &mut Mode,
    next_terminal_id: &mut usize,
    current_desktop: usize,
    direction: SplitDirection,
) -> Option<usize> {
    let idx = active_window_idx(applications, mode, current_desktop)?;
    if applications.get(idx)?.is_menu || applications.get(idx)?.terminal.is_none() {
        return None;
    }

    let (x, y, w, h, resizable, path) = {
        let app = applications.get(idx)?;
        let win = app.window()?;
        let path = app.terminal.as_ref()?.path.clone();
        (win.position_x, win.position_y, win.width, win.height, win.resizable, path)
    };

    if !resizable {
        return None;
    }

    let (current_geom, new_geom) = match direction {
        SplitDirection::Vertical => {
            if w < MIN_W.saturating_mul(2) {
                return None;
            }
            let left_w = (w / 2).max(MIN_W);
            let right_w = w.saturating_sub(left_w).max(MIN_W);
            (
                (x, y, left_w, h),
                (x + w.saturating_sub(right_w), y, right_w, h),
            )
        }
        SplitDirection::Horizontal => {
            if h < MIN_H.saturating_mul(2) {
                return None;
            }
            let top_h = (h / 2).max(MIN_H);
            let bottom_h = h.saturating_sub(top_h).max(MIN_H);
            (
                (x, y, w, top_h),
                (x, y + h.saturating_sub(bottom_h), w, bottom_h),
            )
        }
    };

    applications[idx].set_window_geometry(
        current_geom.0,
        current_geom.1,
        current_geom.2,
        current_geom.3,
    );

    Some(spawn_terminal_window_at(
        applications,
        next_terminal_id,
        current_desktop,
        new_geom.0,
        new_geom.1,
        new_geom.2,
        new_geom.3,
        &path,
        Vec::new(),
    ))
}

fn toggle_start_menu(applications: &mut Vec<Application>, current_desktop: usize, screen_h: u16, tab_scroll: &mut usize) -> bool {
    if let Some(idx) = applications.iter().position(|a| a.on_desktop(current_desktop) && a.is_menu) {
        applications.remove(idx);
    } else {
        let usable_h = screen_h.saturating_sub(4);
        let win_h = (usable_h * 3 / 4).max(MIN_H);
        let pos_y = screen_h.saturating_sub(3).saturating_sub(win_h);
        applications.push(Application::menu(
            "Start",
            Window::new(2, pos_y, 20, win_h, 0).without_chrome(),
        ).with_desktop(current_desktop));
    }
    *tab_scroll = (*tab_scroll).min(max_tab_scroll(applications, current_desktop, screen_h));
    true
}

fn toggle_active_maximize(applications: &mut [Application], mode: &Mode, current_desktop: usize, screen_w: u16, screen_h: u16) -> bool {
    let Some(idx) = active_window_idx(applications, mode, current_desktop) else {
        return false;
    };

    if applications[idx].is_maximized() {
        applications[idx].restore_maximize();
    } else {
        applications[idx].maximize(screen_w, screen_h);
    }
    true
}

fn minimize_active_window(applications: &mut Vec<Application>, mode: &mut Mode, current_desktop: usize, screen_h: u16, tab_scroll: &mut usize) -> bool {
    let Some(idx) = active_window_idx(applications, mode, current_desktop) else {
        return false;
    };

    let can_minimize = applications.get(idx)
        .and_then(|app| app.window())
        .map_or(false, |win| win.minimizable);
    if !can_minimize {
        return false;
    }

    if applications[idx].is_maximized() {
        applications[idx].restore_maximize();
    }
    applications[idx].minimize();
    *tab_scroll = (*tab_scroll).min(max_tab_scroll(applications, current_desktop, screen_h));
    *mode = Mode::Normal;
    true
}

fn focus_relative_window(applications: &mut Vec<Application>, mode: &mut Mode, current_desktop: usize, backward: bool) -> bool {
    let visible: Vec<usize> = applications.iter().enumerate()
        .filter(|(_, app)| app.on_desktop(current_desktop) && app.window().is_some())
        .map(|(idx, _)| idx)
        .collect();
    if visible.len() <= 1 {
        return false;
    }

    let active = active_window_idx(applications, mode, current_desktop).unwrap_or(*visible.last().unwrap());
    let current_pos = visible.iter().position(|&idx| idx == active).unwrap_or(visible.len() - 1);
    let target_pos = if backward {
        current_pos.checked_sub(1).unwrap_or(visible.len() - 1)
    } else {
        (current_pos + 1) % visible.len()
    };

    bring_window_to_front(applications, visible[target_pos]);
    *mode = Mode::Normal;
    true
}

fn move_active_window_to_desktop(
    applications: &mut Vec<Application>,
    mode: &mut Mode,
    current_desktop: &mut usize,
    target_desktop: usize,
    screen_h: u16,
    tab_scroll: &mut usize,
) -> bool {
    if target_desktop == *current_desktop {
        return false;
    }

    let Some(idx) = active_window_idx(applications, mode, *current_desktop) else {
        return false;
    };

    if applications[idx].is_menu {
        return false;
    }

    applications[idx].desktop = target_desktop;
    bring_window_to_front(applications, idx);
    *current_desktop = target_desktop;
    *tab_scroll = (*tab_scroll).min(max_tab_scroll(applications, *current_desktop, screen_h));
    if !mode_targets_desktop(mode, applications, *current_desktop) {
        *mode = Mode::Normal;
    }
    true
}

fn snap_rect(screen_w: u16, screen_h: u16, region: SnapRegion) -> (u16, u16, u16, u16) {
    let area_x = 2;
    let area_y = 1;
    let area_w = screen_w.saturating_sub(5).max(MIN_W);
    let area_h = screen_h.saturating_sub(4).max(MIN_H);

    let left_w = (area_w / 2).max(MIN_W);
    let right_w = area_w.saturating_sub(left_w).max(MIN_W);
    let top_h = (area_h / 2).max(MIN_H);
    let bottom_h = area_h.saturating_sub(top_h).max(MIN_H);

    match region {
        SnapRegion::Left => (area_x, area_y, left_w, area_h),
        SnapRegion::Right => {
            (area_x + area_w.saturating_sub(right_w), area_y, right_w, area_h)
        }
        SnapRegion::Top => (area_x, area_y, area_w, top_h),
        SnapRegion::Bottom => {
            (area_x, area_y + area_h.saturating_sub(bottom_h), area_w, bottom_h)
        }
        SnapRegion::TopLeft => (area_x, area_y, left_w, top_h),
        SnapRegion::TopRight => (area_x + area_w.saturating_sub(right_w), area_y, right_w, top_h),
        SnapRegion::BottomLeft => (area_x, area_y + area_h.saturating_sub(bottom_h), left_w, bottom_h),
        SnapRegion::BottomRight => (
            area_x + area_w.saturating_sub(right_w),
            area_y + area_h.saturating_sub(bottom_h),
            right_w,
            bottom_h,
        ),
    }
}

fn window_matches_geometry(win: &Window, x: u16, y: u16, w: u16, h: u16) -> bool {
    win.position_x == x && win.position_y == y && win.width == w && win.height == h
}

fn snap_active_window(
    applications: &mut [Application],
    mode: &mut Mode,
    current_desktop: usize,
    screen_w: u16,
    screen_h: u16,
    region: SnapRegion,
) -> bool {
    let Some(idx) = active_window_idx(applications, mode, current_desktop) else {
        return false;
    };

    let can_resize = applications.get(idx)
        .and_then(|app| app.window())
        .map_or(false, |win| win.resizable);
    if !can_resize {
        return false;
    }

    let (x, y, w, h) = snap_rect(screen_w, screen_h, region);
    if matches!(region, SnapRegion::Top) {
        if applications[idx].is_maximized() {
            if applications[idx]
                .saved_window()
                .map_or(false, |saved| window_matches_geometry(saved, x, y, w, h))
            {
                applications[idx].restore_maximize();
                *mode = Mode::Normal;
                return true;
            }
        } else if applications[idx]
            .window()
            .map_or(false, |win| window_matches_geometry(win, x, y, w, h))
        {
            applications[idx].maximize(screen_w, screen_h);
            *mode = Mode::Normal;
            return true;
        }
    }

    applications[idx].set_window_geometry(x, y, w.max(MIN_W), h.max(MIN_H));
    *mode = Mode::Normal;
    true
}

fn mode_targets_desktop(mode: &Mode, applications: &[Application], current_desktop: usize) -> bool {
    match mode {
        Mode::Moving { app_idx, .. }
        | Mode::Resizing { app_idx, .. }
        | Mode::TerminalFocus { app_idx } => applications
            .get(*app_idx)
            .map_or(false, |app| app.on_desktop(current_desktop)),
        Mode::Normal | Mode::Typing => true,
    }
}

fn place_pointer_on_terminal_input(pointer: &mut Pointer, applications: &[Application], app_idx: usize, screen_w: u16, screen_h: u16) {
    let Some(win) = applications.get(app_idx).and_then(|app| app.window()) else {
        return;
    };

    let prefix_len = TERMINAL_INPUT_PREFIX.chars().count() as u16;
    let input_x = win.position_x + 1 + prefix_len;
    let has_hscroll = win.content_w as usize > win.width.saturating_sub(2) as usize;
    let input_y = win.position_y + win.height.saturating_sub(if has_hscroll { 3 } else { 2 });
    let max_x = win.position_x + win.width.saturating_sub(2);

    pointer.x = input_x.min(max_x);
    pointer.y = input_y;
    pointer.clamp_to_bounds(screen_w, screen_h);
}

fn enter_active_resize_mode(
    applications: &[Application],
    mode: &mut Mode,
    current_desktop: usize,
    pointer: &mut Pointer,
    screen_w: u16,
    screen_h: u16,
) -> bool {
    let Some(idx) = active_window_idx(applications, mode, current_desktop) else {
        return false;
    };

    let Some(win) = applications.get(idx).and_then(|app| app.window()) else {
        return false;
    };

    if applications[idx].is_maximized() || !win.resizable {
        return false;
    }

    pointer.x = win.position_x + win.width.saturating_sub(1);
    pointer.y = win.position_y + win.height.saturating_sub(1);
    pointer.clamp_to_bounds(screen_w, screen_h);
    *mode = Mode::Resizing { app_idx: idx, edit: None };
    true
}

fn resize_preview_size(win: &Window, pointer: &Pointer) -> (u16, u16) {
    (
        (pointer.x.saturating_sub(win.position_x) + 1).max(MIN_W),
        (pointer.y.saturating_sub(win.position_y) + 1).max(MIN_H),
    )
}

fn apply_resize_edit(
    win: &Window,
    pointer: &mut Pointer,
    screen_w: u16,
    screen_h: u16,
    edit: &ResizeEditState,
) -> bool {
    let Ok(raw_value) = edit.value.parse::<u16>() else {
        return false;
    };

    let (width, height) = resize_preview_size(win, pointer);
    let target = match (edit.axis, edit.op, raw_value) {
        (_, None, _) => return false,
        (_, Some(_), 0) => 0,
        (ResizeAxis::Width, Some(ResizeOp::Add), value) => width.saturating_add(value),
        (ResizeAxis::Width, Some(ResizeOp::Sub), value) => width.saturating_sub(value),
        (ResizeAxis::Width, Some(ResizeOp::Set), value) => value,
        (ResizeAxis::Height, Some(ResizeOp::Add), value) => height.saturating_add(value),
        (ResizeAxis::Height, Some(ResizeOp::Sub), value) => height.saturating_sub(value),
        (ResizeAxis::Height, Some(ResizeOp::Set), value) => value,
    };

    match edit.axis {
        ResizeAxis::Width => {
            let width = target.max(MIN_W);
            pointer.x = win.position_x + width.saturating_sub(1);
        }
        ResizeAxis::Height => {
            let height = target.max(MIN_H);
            pointer.y = win.position_y + height.saturating_sub(1);
        }
    }

    pointer.clamp_to_bounds(screen_w, screen_h);
    true
}

fn render(
    out: &mut Writer,
    applications: &[Application],
    resize_preview: Option<(usize, u16, u16)>,
    cursor_interaction: Option<char>,
    w: u16,
    h: u16,
    pointer: &Pointer,
    scroll_offset: usize,
    tab_scroll: usize,
    path: &str,
    typing_input: Option<(&str, usize)>,
    commands: &[CommandEntry],
    panel_scroll: usize,
    current_desktop: usize,
    // Índice e input do terminal com foco para exibir cursor real.
    focused_terminal: Option<(usize, &str, usize)>,
) {
    ansi::clear(out);

    // 1. Fundo do desktop (sem barra de status)
    draw_desktop(out, 1, w, h, "Manto");

    // 2. Janelas em ordem de empilhamento: chrome e conteúdo na mesma passada.
    for app in applications {
        if app.on_desktop(current_desktop) {
            if let Some(win) = app.window() {
            win.draw(out, &app.title);
            if let Some(term) = app.terminal.as_ref() {
                draw_terminal_content(out, win, &term.path, &term.commands, term.panel_scroll);
            }
        }
        }
    }

    // 3. Abas e scrollbar lateral
    let minimized_count = applications.iter().filter(|a| a.on_desktop(current_desktop) && a.is_minimized()).count();
    let tab_x = w.saturating_sub(3);
    let sb_x  = w.saturating_sub(1);
    let sb_top = 1u16;
    let sb_bot = h.saturating_sub(4);
    let tabs = tab_layout(applications, current_desktop, h, tab_scroll);
    if minimized_count > 0 {
        for &(app_idx, tab_y, tab_h) in &tabs {
            let is_hovered = pointer.x >= tab_x
                && pointer.y >= tab_y
                && pointer.y < tab_y + tab_h;
            let offset = if is_hovered { scroll_offset } else { 0 };
            draw_tab(out, tab_x, tab_y, tab_h, &applications[app_idx].title, offset);
        }
        draw_scrollbar(out, sb_x, sb_top, sb_bot, minimized_count, tabs.len(), tab_scroll);
    }

    // 4. Preview de redimensionamento
    if let Some((idx, pw, ph)) = resize_preview {
        if applications[idx].on_desktop(current_desktop) {
            if let Some(win) = applications[idx].window() {
            win.draw_preview(out, pw, ph);
        }
        }
    }

    // 5. Painel de comandos — sobre janelas
    draw_command_panel(out, w, h, path, commands, panel_scroll);

    // 6. Barra de status — sempre por cima de tudo
    draw_status_bar(out, w, h, path, !commands.is_empty(), current_desktop);

    // 7. Hover no botão Start
    let start_end = STATUS_START_X + STATUS_START.len() as u16;
    if pointer.y == h - 2 && pointer.x >= STATUS_START_X && pointer.x < start_end {
        ansi::move_to(out, STATUS_START_X, h - 2);
        write!(out, "{}{}{}", ansi::REVERSE, STATUS_START, ansi::RESET).unwrap();
    }

    // 7.5. Hover nos botões de desktop
    if let Some(d) = desktop_at(pointer.x, pointer.y, w, h) {
        let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN);
        let sep_x  = base_x + (d as u16 - 1) * 4;
        ansi::move_to(out, sep_x + 1, h - 2);
        write!(out, "{} {} {}", ansi::REVERSE, d, ansi::RESET).unwrap();
    }

    // 8. Cursor: real (typing / terminal focus) ou ponteiro (░ / reverse)
    if let Some((input, cursor_pos)) = typing_input {
        let max_len = (w - 2).saturating_sub(CMD_INPUT_X) as usize;
        let (display, cursor_col) = input_view(input, cursor_pos, max_len);
        ansi::move_to(out, CMD_INPUT_X, h - 2);
        write!(out, "{:<width$}", display, width = max_len).unwrap();
        ansi::move_to(out, CMD_INPUT_X + cursor_col as u16, h - 2);
        ansi::show_cursor(out);
    } else if let Some((term_idx, term_input, cursor_pos)) = focused_terminal {
        // Cursor real dentro da janela de terminal com foco
        if let Some(win) = applications.get(term_idx)
            .filter(|a| a.on_desktop(current_desktop))
            .and_then(|a| a.window())
        {
            if win.height >= 5 {
                let prefix_len = TERMINAL_INPUT_PREFIX.chars().count();
                let inner_w    = (win.width - 2) as usize;
                let max_len    = inner_w.saturating_sub(prefix_len);
                let (display, cursor_col) = input_view(term_input, cursor_pos, max_len);
                let cursor_x   = win.position_x + 1 + prefix_len as u16;
                let has_hscroll = win.content_w as usize > win.width.saturating_sub(2) as usize;
                let cursor_y   = win.position_y + win.height - if has_hscroll { 3 } else { 2 };
                ansi::move_to(out, cursor_x, cursor_y);
                write!(out, "{:<width$}", display, width = max_len).unwrap();
                ansi::move_to(out, cursor_x + cursor_col as u16, cursor_y);
                ansi::show_cursor(out);
            }
        }
    } else {
        ansi::hide_cursor(out);
        let effective_cursor = cursor_interaction.or_else(|| {
            let px = pointer.x;
            let py = pointer.y;

            if minimized_count > tabs.len() && px == sb_x && py >= sb_top && py <= sb_bot {
                let track_len = (sb_bot - sb_top + 1) as usize;
                let (thumb_pos, thumb_len) = scrollbar_thumb(track_len, minimized_count, tabs.len(), tab_scroll);
                let row = (py - sb_top) as usize;
                return Some(if row >= thumb_pos && row < thumb_pos + thumb_len { '█' } else { '░' });
            }

            if px >= tab_x && px < sb_x {
                if let Some(&(app_idx, tab_y, tab_h)) = tabs.iter()
                    .find(|&&(_, ty, th)| py >= ty && py < ty + th)
                {
                    return Some(tab_char_at(
                        tab_x, tab_y, tab_h,
                        &applications[app_idx].title,
                        px, py, scroll_offset,
                    ));
                }
            }

            let start_end = STATUS_START_X + STATUS_START.len() as u16;
            if py == h - 2 && px >= STATUS_START_X && px < start_end {
                return Some(STATUS_START.chars().nth((px - STATUS_START_X) as usize).unwrap_or(' '));
            }

            if let Some(d) = desktop_at(px, py, w, h) {
                let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN);
                let sep_x  = base_x + (d as u16 - 1) * 4;
                let offset = px - (sep_x + 1);
                return Some(if offset == 1 { char::from_digit(d as u32, 10).unwrap_or(' ') } else { ' ' });
            }

            if let Some(top_idx) = topmost_window_at(applications, current_desktop, px, py) {
                if let Some(win) = applications[top_idx].window() {
                    if let Some(ch) = win.char_at(px, py, &applications[top_idx].title) {
                        return Some(ch);
                    }
                }
            }

            None
        });
        pointer.draw(out, effective_cursor);
    }

    out.flush().unwrap();
}

fn main() {
    let mut out = Writer::new();

    os::enable_raw_mode();
    ansi::enter_alt_screen(&mut out);
    ansi::hide_cursor(&mut out);
    out.flush().unwrap();

    let mut mode             = Mode::Normal;
    let mut scroll_offset:    usize = 0;
    let mut tab_scroll:       usize = 0;
    let mut panel_scroll:     usize = 0;
    let mut current_desktop:  usize = 1;
    let mut next_terminal_id: usize = 1;
    let mut last_space_time: Option<Clock> = None;
    let mut current_path     = std::env::current_dir()
        .map(|path| normalize_host_path(&path))
        .unwrap_or_else(|_| ".".to_string());
    let mut cmd_input        = String::new();
    let mut cmd_cursor       = 0usize;
    let mut history_index: Option<usize> = None;
    let mut history_draft: Option<String> = None;
    let mut commands: Vec<CommandEntry> = Vec::new();
    let mut last_size     = os::size();
    let mut pointer       = Pointer::new(1 + STATUS_BAR_PREFIX.len() as u16, last_size.1 - 2);

    let mut applications = Vec::new();
    sync_terminal_window_metrics(&mut applications);

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    let in_shell     = matches!(mode, Mode::Typing);
    let shell_path   = if in_shell { current_path.as_str() } else { "" };
    let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
        applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str(), t.input_cursor))
    } else { None };
    render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some((&cmd_input, cmd_cursor)) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);

    let mut last_check = Clock::now();

    loop {
        if os::poll(50) {
            let key  = os::read_key();
            let prev = (pointer.x, pointer.y);
            let mut mode_changed = false;

            match key {
                Key::Ctrl1 => {
                    if move_active_window_to_desktop(&mut applications, &mut mode, &mut current_desktop, 1, last_size.1, &mut tab_scroll) {
                        mode_changed = true;
                    }
                }
                Key::Ctrl2 => {
                    if move_active_window_to_desktop(&mut applications, &mut mode, &mut current_desktop, 2, last_size.1, &mut tab_scroll) {
                        mode_changed = true;
                    }
                }
                Key::Ctrl3 => {
                    if move_active_window_to_desktop(&mut applications, &mut mode, &mut current_desktop, 3, last_size.1, &mut tab_scroll) {
                        mode_changed = true;
                    }
                }
                Key::Ctrl4 => {
                    if move_active_window_to_desktop(&mut applications, &mut mode, &mut current_desktop, 4, last_size.1, &mut tab_scroll) {
                        mode_changed = true;
                    }
                }
                Key::CtrlDelete => break,
                Key::CtrlF => {
                    if toggle_active_maximize(&mut applications, &mode, current_desktop, last_size.0, last_size.1) {
                        mode_changed = true;
                    }
                }
                Key::CtrlN => {
                    if focus_relative_window(&mut applications, &mut mode, current_desktop, false) {
                        mode_changed = true;
                    }
                }
                Key::CtrlP => {
                    if focus_relative_window(&mut applications, &mut mode, current_desktop, true) {
                        mode_changed = true;
                    }
                }
                Key::CtrlW => {
                    if close_active_window(&mut applications, &mut mode, current_desktop, last_size.1, &mut tab_scroll) {
                        mode_changed = true;
                    }
                }

                // Ctrl+T: novo terminal vazio, de qualquer modo.
                Key::CtrlT => {
                    let app_idx = spawn_terminal_window(
                        &mut applications,
                        &mut next_terminal_id,
                        current_desktop,
                        last_size.0,
                        last_size.1,
                        &current_path,
                        Vec::new(),
                    );
                    place_pointer_on_terminal_input(&mut pointer, &applications, app_idx, last_size.0, last_size.1);
                    mode = Mode::TerminalFocus { app_idx };
                    mode_changed = true;
                }
                Key::AltR => {
                    if enter_active_resize_mode(
                        &applications,
                        &mut mode,
                        current_desktop,
                        &mut pointer,
                        last_size.0,
                        last_size.1,
                    ) {
                        mode_changed = true;
                    }
                }
                Key::AltV => {
                    if let Some(app_idx) = split_active_terminal_window(
                        &mut applications,
                        &mut mode,
                        &mut next_terminal_id,
                        current_desktop,
                        SplitDirection::Vertical,
                    ) {
                        place_pointer_on_terminal_input(&mut pointer, &applications, app_idx, last_size.0, last_size.1);
                        mode = Mode::TerminalFocus { app_idx };
                        mode_changed = true;
                    }
                }
                Key::AltH => {
                    if let Some(app_idx) = split_active_terminal_window(
                        &mut applications,
                        &mut mode,
                        &mut next_terminal_id,
                        current_desktop,
                        SplitDirection::Horizontal,
                    ) {
                        place_pointer_on_terminal_input(&mut pointer, &applications, app_idx, last_size.0, last_size.1);
                        mode = Mode::TerminalFocus { app_idx };
                        mode_changed = true;
                    }
                }

                _ => match &mut mode {
                    Mode::Normal => match key {
                        Key::AltUp | Key::AltDown | Key::AltLeft | Key::AltRight => {
                            if let Some(region) = resolve_snap_region(&key, os::held_arrow_keys()) {
                                if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, region) {
                                    mode_changed = true;
                                }
                            }
                        }
                        Key::Char(digit @ '1'..='4') => {
                            current_desktop = digit.to_digit(10).unwrap_or(1) as usize;
                            tab_scroll = tab_scroll.min(max_tab_scroll(&applications, current_desktop, last_size.1));
                            if !mode_targets_desktop(&mode, &applications, current_desktop) {
                                mode = Mode::Normal;
                            }
                            mode_changed = true;
                        }
                        Key::CtrlD => {
                            if toggle_start_menu(&mut applications, current_desktop, last_size.1, &mut tab_scroll) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlX => {
                            if minimize_active_window(&mut applications, &mut mode, current_desktop, last_size.1, &mut tab_scroll) {
                                mode_changed = true;
                            }
                        }
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),

                        Key::Home => {
                            pointer.x = CMD_INPUT_X;
                            pointer.y = last_size.1 - 2;
                        }

                        Key::Char(' ') | Key::Enter => {
                            let sb_x   = last_size.0.saturating_sub(1);
                            let sb_top = 1u16;
                            let sb_bot = last_size.1.saturating_sub(4);
                            let tab_x  = last_size.0.saturating_sub(3);

                            // Botões de desktop (prioridade sobre área de comando)
                            if let Some(d) = desktop_at(pointer.x, pointer.y, last_size.0, last_size.1) {
                                current_desktop = d;
                                tab_scroll = tab_scroll.min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                if !mode_targets_desktop(&mode, &applications, current_desktop) {
                                    mode = Mode::Normal;
                                }
                                mode_changed = true;
                            // Área de comando na barra de status
                            } else if pointer.y == last_size.1 - 2
                                && pointer.x >= CMD_INPUT_X.saturating_sub(TERMINAL_INPUT_PREFIX.len() as u16)
                            {
                                mode = Mode::Typing;
                                panel_scroll = 0;
                                mode_changed = true;
                            // Botão Start: toggle do menu
                            } else {
                            let start_end = STATUS_START_X + STATUS_START.len() as u16;
                            if pointer.y == last_size.1 - 2
                                && pointer.x >= STATUS_START_X
                                && pointer.x < start_end
                            {
                                toggle_start_menu(&mut applications, current_desktop, last_size.1, &mut tab_scroll);
                                mode_changed = true;
                            // Scrollbar (coluna mais à direita): metade superior sobe, inferior desce
                            } else if pointer.x == sb_x {
                                last_space_time = None;
                                let mid = (sb_top + sb_bot) / 2;
                                if pointer.y <= mid {
                                    tab_scroll = tab_scroll.saturating_sub(1);
                                } else {
                                    tab_scroll = (tab_scroll + 1)
                                        .min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                }
                                mode_changed = true;
                            // Aba → restaura app minimizado
                            } else if pointer.x >= tab_x {
                                last_space_time = None;
                                let on_tab = tab_layout(&applications, current_desktop, last_size.1, tab_scroll)
                                    .into_iter()
                                    .find(|&(_, ty, th)| pointer.y >= ty && pointer.y < ty + th)
                                    .map(|(idx, _, _)| idx);

                                if let Some(app_idx) = on_tab {
                                    applications[app_idx].restore();
                                    let restored_idx = bring_window_to_front(&mut applications, app_idx);
                                    tab_scroll = tab_scroll
                                        .min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                    if applications[restored_idx].terminal.is_some() {
                                        place_pointer_on_terminal_input(&mut pointer, &applications, restored_idx, last_size.0, last_size.1);
                                        mode = Mode::TerminalFocus { app_idx: restored_idx };
                                    }
                                    mode_changed = true;
                                }
                            // Janela
                            } else if let Some(top_idx) =
                                topmost_window_at(&applications, current_desktop, pointer.x, pointer.y)
                            {
                                // Fecha menu se a ação foi fora dele
                                let mut skip = false;
                                if let Some(menu_idx) = applications.iter().position(|a| a.on_desktop(current_desktop) && a.is_menu) {
                                    if top_idx != menu_idx {
                                        applications.remove(menu_idx);
                                        tab_scroll = tab_scroll
                                            .min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                        mode_changed = true;
                                        skip = true; // top_idx inválido após remove
                                    }
                                }
                                if !skip {
                                    // Scroll interno da janela tem prioridade
                                    let scroll_handled = if let Some(app) = applications.get_mut(top_idx) {
                                        let handled = if let Some(win) = app.window_mut() {
                                            win.interact(pointer.x, pointer.y)
                                        } else {
                                            false
                                        };
                                        handled || interact_terminal_horizontal_scroll(app, pointer.x, pointer.y)
                                    } else {
                                        false
                                    };
                                    if scroll_handled {
                                        mode_changed = true;
                                    }

                                    // Clique na linha de input de janela terminal
                                    let is_terminal_input = {
                                        let app = &applications[top_idx];
                                        app.terminal.is_some() && app.window().map_or(false, |win| {
                                            let has_hscroll = win.content_w as usize > win.width.saturating_sub(2) as usize;
                                            win.height >= 5
                                                && pointer.y == win.position_y + win.height.saturating_sub(if has_hscroll { 3 } else { 2 })
                                                && pointer.x > win.position_x
                                                && pointer.x < win.position_x + win.width - 1
                                        })
                                    };
                                    if is_terminal_input && !scroll_handled {
                                        if top_idx != applications.len() - 1 {
                                            let app = applications.remove(top_idx);
                                            applications.push(app);
                                        }
                                        place_pointer_on_terminal_input(&mut pointer, &applications, applications.len() - 1, last_size.0, last_size.1);
                                        mode = Mode::TerminalFocus { app_idx: applications.len() - 1 };
                                        mode_changed = true;
                                    }

                                    if !scroll_handled && !is_terminal_input {
                                    let (is_minimize, is_close, is_resize, is_title, offset_x,
                                         win_minimizable, win_closable, win_draggable, win_resizable) = {
                                        let win = applications[top_idx].window().unwrap();
                                        let lx = win.position_x;
                                        let rx = win.position_x + win.width - 1;
                                        let ty = win.position_y;
                                        let by = win.position_y + win.height - 1;
                                        (
                                            pointer.x == lx && pointer.y == ty,
                                            pointer.x == rx && pointer.y == ty,
                                            pointer.x == rx && pointer.y == by,
                                            pointer.y == ty && pointer.x > lx && pointer.x < rx,
                                            pointer.x.saturating_sub(lx),
                                            win.minimizable,
                                            win.closable,
                                            win.draggable,
                                            win.resizable,
                                        )
                                    };
                                    let maximized = applications[top_idx].is_maximized();

                                    if is_minimize && win_minimizable {
                                        applications[top_idx].minimize();
                                        mode_changed = true;
                                    } else if is_close && win_closable {
                                        applications.remove(top_idx);
                                        tab_scroll = tab_scroll
                                            .min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                        mode_changed = true;
                                    } else if is_resize && !maximized && win_resizable {
                                        mode = Mode::Resizing { app_idx: top_idx, edit: None };
                                        mode_changed = true;
                                    } else if is_title && win_draggable {
                                        // Duplo toque na barra de título → maximizar / restaurar
                                        let now = Clock::now();
                                        let is_double = last_space_time
                                            .as_ref()
                                            .map(|t| t.elapsed() < Duration::from_millis(300))
                                            .unwrap_or(false);
                                        last_space_time = if is_double { None } else { Some(now) };

                                        if is_double {
                                            if maximized {
                                                applications[top_idx].restore_maximize();
                                            } else {
                                                applications[top_idx].maximize(last_size.0, last_size.1);
                                            }
                                            mode_changed = true;
                                        } else if !maximized {
                                            let final_idx = if top_idx != applications.len() - 1 {
                                                let app = applications.remove(top_idx);
                                                applications.push(app);
                                                applications.len() - 1
                                            } else {
                                                top_idx
                                            };
                                            mode = Mode::Moving { app_idx: final_idx, offset_x };
                                            mode_changed = true;
                                        }
                                    } else {
                                        last_space_time = None;
                                        if top_idx != applications.len() - 1 {
                                            let app = applications.remove(top_idx);
                                            applications.push(app);
                                            mode_changed = true;
                                        }
                                    }
                                    } // if !scroll_handled && !is_terminal_input
                                } // if !skip
                            }
                            } // else (não é área de comando)
                        }
                        _ => {}
                    },

                    Mode::Typing => {
                        match key {
                            Key::Escape | Key::End => {
                                mode = Mode::Normal;
                                mode_changed = true;
                            }
                            // Ctrl+Enter: destaca o terminal para uma janela flutuante.
                            Key::CtrlEnter => {
                                // Área utilizável: linhas 1..=h-4 (dock ocupa h-3..h-1).
                                let _usable_h = last_size.1.saturating_sub(4); // = h-4 = último row válido
                                let cmds = std::mem::take(&mut commands);
                                cmd_input.clear();
                                cmd_cursor = 0;
                                panel_scroll = 0;
                                let app_idx = spawn_terminal_window(
                                    &mut applications,
                                    &mut next_terminal_id,
                                    current_desktop,
                                    last_size.0,
                                    last_size.1,
                                    &current_path,
                                    cmds,
                                );
                                place_pointer_on_terminal_input(&mut pointer, &applications, app_idx, last_size.0, last_size.1);
                                // Foca imediatamente a nova janela terminal.
                                mode = Mode::TerminalFocus { app_idx };
                                mode_changed = true;
                            }
                            Key::PageUp => {
                                panel_scroll = panel_scroll.saturating_add(1);
                                mode_changed = true;
                            }
                            Key::PageDown => {
                                panel_scroll = panel_scroll.saturating_sub(1);
                                mode_changed = true;
                            }
                            Key::Up => {
                                if history_up(&commands, &mut cmd_input, &mut history_index, &mut history_draft) {
                                    cmd_cursor = input_char_len(&cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Down => {
                                if history_down(&commands, &mut cmd_input, &mut history_index, &mut history_draft) {
                                    cmd_cursor = input_char_len(&cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Left => {
                                if move_input_cursor_left(&mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Right => {
                                if move_input_cursor_right(&cmd_input, &mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Tab => {
                                reset_history_navigation(&mut history_index, &mut history_draft);
                                if autocomplete_input(&mut cmd_input, &mut cmd_cursor, &current_path) {
                                    mode_changed = true;
                                }
                            }
                            Key::Enter => {
                                let trimmed = cmd_input.trim().to_string();
                                if !trimmed.is_empty() {
                                    push_shell_command(&mut commands, &mut current_path, &trimmed);
                                    cmd_input.clear();
                                    cmd_cursor = 0;
                                    reset_history_navigation(&mut history_index, &mut history_draft);
                                    panel_scroll = 0;
                                }
                                mode_changed = true;
                            }
                            Key::Delete => {
                                reset_history_navigation(&mut history_index, &mut history_draft);
                                if delete_input_char(&mut cmd_input, &mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Backspace => {
                                reset_history_navigation(&mut history_index, &mut history_draft);
                                if backspace_input_char(&mut cmd_input, &mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Char(c) => {
                                reset_history_navigation(&mut history_index, &mut history_draft);
                                insert_input_char(&mut cmd_input, &mut cmd_cursor, c);
                                mode_changed = true;
                            }
                            _ => {}
                        }
                    }

                    Mode::Moving { app_idx, .. } => match key {
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),
                        Key::Char(' ') | Key::Enter => {
                            let idx = *app_idx;
                            let is_double = last_space_time
                                .as_ref()
                                .map(|t| t.elapsed() < Duration::from_millis(300))
                                .unwrap_or(false);
                            last_space_time = None;
                            mode = Mode::Normal;
                            if is_double {
                                applications[idx].maximize(last_size.0, last_size.1);
                            }
                            mode_changed = true;
                        }
                        _ => {}
                    },

                    Mode::Resizing { app_idx, edit } => match key {
                        Key::Escape => {
                            if edit.is_some() {
                                *edit = None;
                            } else {
                                mode = Mode::Normal;
                            }
                            mode_changed = true;
                        }
                        Key::Char('x') | Key::Char('h') => {
                            *edit = Some(ResizeEditState { axis: ResizeAxis::Width, op: None, value: String::new() });
                            mode_changed = true;
                        }
                        Key::Char('y') | Key::Char('v') => {
                            *edit = Some(ResizeEditState { axis: ResizeAxis::Height, op: None, value: String::new() });
                            mode_changed = true;
                        }
                        _ if edit.is_some() => {
                            let mut clear_edit = false;
                            let mut changed_pointer = false;

                            if let Some(state) = edit.as_mut() {
                                match key {
                                    Key::Char(' ') => {}
                                    Key::Char('+') if state.op.is_none() => {
                                        state.op = Some(ResizeOp::Add);
                                        mode_changed = true;
                                    }
                                    Key::Char('-') if state.op.is_none() => {
                                        state.op = Some(ResizeOp::Sub);
                                        mode_changed = true;
                                    }
                                    Key::Char('=') if state.op.is_none() => {
                                        state.op = Some(ResizeOp::Set);
                                        mode_changed = true;
                                    }
                                    Key::Char(c) if state.op.is_some() && c.is_ascii_digit() => {
                                        state.value.push(c);
                                        mode_changed = true;
                                    }
                                    Key::Backspace if state.op.is_some() && !state.value.is_empty() => {
                                        state.value.pop();
                                        mode_changed = true;
                                    }
                                    Key::Enter => {
                                        let idx = *app_idx;
                                        if let Some(win) = applications[idx].window() {
                                            if !state.value.is_empty() {
                                                changed_pointer = apply_resize_edit(win, &mut pointer, last_size.0, last_size.1, state);
                                            }
                                        }
                                        clear_edit = true;
                                        mode_changed = true;
                                    }
                                    _ => {
                                        clear_edit = true;
                                        mode_changed = true;
                                    }
                                }
                            }

                            if clear_edit {
                                *edit = None;
                            }
                            if changed_pointer {
                                pointer.clamp_to_bounds(last_size.0, last_size.1);
                            }
                        }
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),
                        Key::Char(' ') | Key::Enter => {
                            let idx = *app_idx;
                            if let Some(win) = applications[idx].window_mut() {
                                let (width, height) = resize_preview_size(win, &pointer);
                                win.width = width;
                                win.height = height;
                            }
                            mode = Mode::Normal;
                            mode_changed = true;
                        }
                        _ => {}
                    },

                    Mode::TerminalFocus { app_idx } => {
                        let idx = *app_idx;
                        match key {
                            Key::Escape | Key::End => {
                                mode = Mode::Normal;
                                mode_changed = true;
                            }
                            Key::PageUp => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    t.panel_scroll = t.panel_scroll.saturating_add(1);
                                    mode_changed = true;
                                }
                            }
                            Key::PageDown => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    t.panel_scroll = t.panel_scroll.saturating_sub(1);
                                    mode_changed = true;
                                }
                            }
                            Key::Up => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if history_up(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                        t.input_cursor = input_char_len(&t.cmd_input);
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Down => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if history_down(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                        t.input_cursor = input_char_len(&t.cmd_input);
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Left => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if move_input_cursor_left(&mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Right => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if move_input_cursor_right(&t.cmd_input, &mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Tab => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    if autocomplete_input(&mut t.cmd_input, &mut t.input_cursor, &t.path) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Enter => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    let trimmed = t.cmd_input.trim().to_string();
                                    if !trimmed.is_empty() {
                                        push_shell_command(&mut t.commands, &mut t.path, &trimmed);
                                        t.cmd_input.clear();
                                        t.input_cursor = 0;
                                        reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                        t.panel_scroll = 0;
                                    }
                                    mode_changed = true;
                                }
                            }
                            Key::Delete => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    if delete_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Backspace => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    if backspace_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Char(c) => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    insert_input_char(&mut t.cmd_input, &mut t.input_cursor, c);
                                    mode_changed = true;
                                }
                            }
                            _ => {}
                        }
                    },
                },
            }

            // Em modo Normal: coluna do scrollbar acessível só quando há scroll;
            // quando acessível, o ponteiro fica limitado à faixa vertical da scrollbar.
            if matches!(&mode, Mode::Normal) {
                let sb_x = last_size.0.saturating_sub(1);
                if pointer.x == sb_x {
                    let minimized_count = applications.iter()
                        .filter(|a| a.on_desktop(current_desktop) && a.is_minimized())
                        .count();
                    let tab_count = tab_layout(&applications, current_desktop, last_size.1, tab_scroll).len();
                    if minimized_count <= tab_count {
                        pointer.x = sb_x.saturating_sub(1);
                    } else {
                        let sb_top = 1u16;
                        let sb_bot = last_size.1.saturating_sub(4);
                        pointer.y = pointer.y.max(sb_top).min(sb_bot);
                    }
                }
            }

            // Atualiza posição da janela em tempo real durante o movimento
            if let Mode::Moving { app_idx, offset_x } = &mode {
                if let Some(win) = applications[*app_idx].window_mut() {
                    win.position_x = pointer.x.saturating_sub(*offset_x);
                    win.position_y = pointer.y;
                }
            }

            let moved = (pointer.x, pointer.y) != prev;
            if moved || mode_changed {
                sync_terminal_window_metrics(&mut applications);
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                let in_shell     = matches!(mode, Mode::Typing);
                let shell_path   = if in_shell { current_path.as_str() } else { "" };
                let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
                    applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str(), t.input_cursor))
                } else { None };
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some((&cmd_input, cmd_cursor)) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);
            }
        }

        let cmds_changed = tick_all(&mut commands)
            || applications.iter_mut().any(|a| {
                a.terminal.as_mut().map_or(false, |t| t.tick())
            });
        if cmds_changed {
            sync_terminal_window_metrics(&mut applications);
            let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
            let in_shell     = matches!(mode, Mode::Typing);
            let shell_path   = if in_shell { current_path.as_str() } else { "" };
            let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
                applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str(), t.input_cursor))
            } else { None };
            render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some((&cmd_input, cmd_cursor)) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            scroll_offset = scroll_offset.wrapping_add(1);
            let new_size = os::size();
            let size_changed = new_size != last_size;
            if size_changed {
                pointer.y = new_size.1 - (last_size.1 - pointer.y);
                last_size = new_size;
                pointer.clamp_to_bounds(last_size.0, last_size.1);
                tab_scroll = tab_scroll.min(max_tab_scroll(&applications, current_desktop, last_size.1));
            }
            // Só anima o título da aba sob o cursor
            let tab_x = last_size.0.saturating_sub(3);
            let needs_scroll = tab_layout(&applications, current_desktop, last_size.1, tab_scroll)
                .iter()
                .any(|&(idx, tab_y, tab_h)| {
                    let is_hovered = pointer.x >= tab_x
                        && pointer.y >= tab_y
                        && pointer.y < tab_y + tab_h;
                    is_hovered
                        && applications[idx].title.chars().count() > tab_h.saturating_sub(2) as usize
                });
            if size_changed || needs_scroll {
                sync_terminal_window_metrics(&mut applications);
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                let in_shell     = matches!(mode, Mode::Typing);
                let shell_path   = if in_shell { current_path.as_str() } else { "" };
                let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
                    applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str(), t.input_cursor))
                } else { None };
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some((&cmd_input, cmd_cursor)) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);
            }
            last_check = Clock::now();
        }
    }

    ansi::leave_alt_screen(&mut out);
    ansi::show_cursor(&mut out);
    out.flush().unwrap();
    os::disable_raw_mode();
}

fn compute_render_state(
    mode: &Mode,
    applications: &[Application],
    pointer: &Pointer,
) -> (Option<(usize, u16, u16)>, Option<char>) {
    match mode {
        Mode::Resizing { app_idx, .. } => {
            let idx = *app_idx;
            if let Some(win) = applications[idx].window() {
                let pw = (pointer.x.saturating_sub(win.position_x) + 1).max(MIN_W);
                let ph = (pointer.y.saturating_sub(win.position_y) + 1).max(MIN_H);
                (Some((idx, pw, ph)), Some('┼'))
            } else {
                (None, None)
            }
        }
        Mode::Moving { .. }          => (None, None),
        Mode::Typing                 => (None, None),
        Mode::TerminalFocus { .. }   => (None, None),
        Mode::Normal                 => (None, None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cmd::{CommandEntry, CommandStatus};

    fn fixture_commands() -> Vec<CommandEntry> {
        vec![
            CommandEntry::fixture("echo a", &["a"], CommandStatus::Complete),
            CommandEntry::fixture("echo b", &["b"], CommandStatus::Complete),
            CommandEntry::fixture("echo c", &["c"], CommandStatus::Complete),
        ]
    }

    #[test]
    fn history_up_walks_back_from_latest() {
        let commands = fixture_commands();
        let mut input = String::new();
        let mut index = None;
        let mut draft = None;

        assert!(history_up(&commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo c");
        assert_eq!(index, Some(2));

        assert!(history_up(&commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo b");
        assert_eq!(index, Some(1));
    }

    #[test]
    fn history_down_restores_draft_after_latest() {
        let commands = fixture_commands();
        let mut input = String::from("ec");
        let mut index = None;
        let mut draft = None;

        history_up(&commands, &mut input, &mut index, &mut draft);
        history_down(&commands, &mut input, &mut index, &mut draft);

        assert_eq!(input, "ec");
        assert_eq!(index, None);
    }

    #[test]
    fn token_bounds_find_current_word() {
        let input = "cd targ";
        assert_eq!(token_bounds(input, 7), (3, 7));
        assert_eq!(token_bounds(input, 2), (0, 2));
    }

    #[test]
    fn autocomplete_cd_completes_directory() {
        let base = std::env::temp_dir().join(format!("manto-test-{}", std::process::id()));
        let target = base.join("target-dir");
        std::fs::create_dir_all(&target).unwrap();

        let mut input = String::from("cd tar");
        let mut cursor = input_char_len(&input);
        let base_str = base.display().to_string();

        let changed = autocomplete_input(&mut input, &mut cursor, &base_str);

        assert!(changed);
        assert!(input.starts_with("cd target-dir"));
        assert!(input.ends_with(std::path::MAIN_SEPARATOR));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn ctrl_arrow_combines_into_quadrant() {
        assert!(matches!(
            resolve_snap_region(&Key::AltLeft, os::HeldArrowKeys { up: true, ..Default::default() }),
            Some(SnapRegion::TopLeft)
        ));
        assert!(matches!(
            resolve_snap_region(&Key::AltUp, os::HeldArrowKeys { left: true, ..Default::default() }),
            Some(SnapRegion::TopLeft)
        ));
    }

    #[test]
    fn ctrl_arrow_same_axis_stays_half_snap() {
        assert!(matches!(
            resolve_snap_region(&Key::AltUp, os::HeldArrowKeys::default()),
            Some(SnapRegion::Top)
        ));
        assert!(matches!(
            resolve_snap_region(&Key::AltDown, os::HeldArrowKeys::default()),
            Some(SnapRegion::Bottom)
        ));
    }

    #[test]
    fn alt_r_enters_resize_mode_on_active_window() {
        let applications = vec![
            Application::windowed("Test", Window::new(10, 5, 20, 8, 0)),
        ];
        let mut mode = Mode::Normal;
        let mut pointer = Pointer::new(1, 1);

        assert!(enter_active_resize_mode(&applications, &mut mode, 1, &mut pointer, 120, 40));
        assert!(matches!(mode, Mode::Resizing { app_idx: 0, .. }));
        assert_eq!(pointer.x, 29);
        assert_eq!(pointer.y, 12);
    }

    #[test]
    fn apply_resize_edit_updates_width_preview() {
        let win = Window::new(10, 5, 20, 8, 0);
        let mut pointer = Pointer::new(29, 12);
        let edit = ResizeEditState {
            axis: ResizeAxis::Width,
            op: Some(ResizeOp::Add),
            value: "5".to_string(),
        };

        assert!(apply_resize_edit(&win, &mut pointer, 120, 40, &edit));
        assert_eq!(pointer.x, 34);
        assert_eq!(pointer.y, 12);
    }

    #[test]
    fn apply_resize_edit_sets_height_preview() {
        let win = Window::new(10, 5, 20, 8, 0);
        let mut pointer = Pointer::new(29, 12);
        let edit = ResizeEditState {
            axis: ResizeAxis::Height,
            op: Some(ResizeOp::Set),
            value: "4".to_string(),
        };

        assert!(apply_resize_edit(&win, &mut pointer, 120, 40, &edit));
        assert_eq!(pointer.x, 29);
        assert_eq!(pointer.y, 8);
    }

    #[test]
    fn top_snap_toggles_with_maximize_on_repeat() {
        let mut applications = vec![
            Application::windowed("Test", Window::new(10, 5, 20, 8, 0)),
        ];
        let mut mode = Mode::Normal;
        let top = snap_rect(120, 40, SnapRegion::Top);

        assert!(snap_active_window(&mut applications, &mut mode, 1, 120, 40, SnapRegion::Top));
        let win = applications[0].window().unwrap();
        assert!(window_matches_geometry(win, top.0, top.1, top.2, top.3));
        assert!(!applications[0].is_maximized());

        assert!(snap_active_window(&mut applications, &mut mode, 1, 120, 40, SnapRegion::Top));
        assert!(applications[0].is_maximized());

        assert!(snap_active_window(&mut applications, &mut mode, 1, 120, 40, SnapRegion::Top));
        let win = applications[0].window().unwrap();
        assert!(window_matches_geometry(win, top.0, top.1, top.2, top.3));
        assert!(!applications[0].is_maximized());
    }

    #[test]
    fn split_vertical_creates_new_terminal_on_right() {
        let mut applications = vec![
            Application::terminal_window("Terminal 1", Window::new(10, 5, 20, 8, 0), "D:\\tmp".to_string(), Vec::new()),
        ];
        let mut mode = Mode::TerminalFocus { app_idx: 0 };
        let mut next_terminal_id = 2;

        let new_idx = split_active_terminal_window(
            &mut applications,
            &mut mode,
            &mut next_terminal_id,
            1,
            SplitDirection::Vertical,
        ).unwrap();

        assert_eq!(applications.len(), 2);
        assert_eq!(new_idx, 1);
        let left = applications[0].window().unwrap();
        let right = applications[1].window().unwrap();
        assert_eq!((left.position_x, left.position_y, left.width, left.height), (10, 5, 10, 8));
        assert_eq!((right.position_x, right.position_y, right.width, right.height), (20, 5, 10, 8));
        assert_eq!(applications[1].terminal.as_ref().unwrap().path, "D:\\tmp");
    }

    #[test]
    fn split_horizontal_creates_new_terminal_below() {
        let mut applications = vec![
            Application::terminal_window("Terminal 1", Window::new(10, 5, 20, 8, 0), "D:\\tmp".to_string(), Vec::new()),
        ];
        let mut mode = Mode::TerminalFocus { app_idx: 0 };
        let mut next_terminal_id = 2;

        let new_idx = split_active_terminal_window(
            &mut applications,
            &mut mode,
            &mut next_terminal_id,
            1,
            SplitDirection::Horizontal,
        ).unwrap();

        assert_eq!(applications.len(), 2);
        assert_eq!(new_idx, 1);
        let top = applications[0].window().unwrap();
        let bottom = applications[1].window().unwrap();
        assert_eq!((top.position_x, top.position_y, top.width, top.height), (10, 5, 20, 4));
        assert_eq!((bottom.position_x, bottom.position_y, bottom.width, bottom.height), (10, 9, 20, 4));
        assert_eq!(applications[1].terminal.as_ref().unwrap().path, "D:\\tmp");
    }
}
