use std::path::Path;
use crate::application::Application;
use crate::cmd::CommandEntry;
use crate::gui::TERMINAL_INPUT_PREFIX;
use crate::window::{Window, MIN_W, MIN_H};
use crate::pointer::Pointer;
use crate::os::Key;

#[derive(Debug)]
pub enum Mode {
    Normal,
    Moving          { app_idx: usize, offset_x: u16 },
    Resizing        { app_idx: usize, edit: Option<ResizeEditState> },
    Typing,
    TerminalFocus   { app_idx: usize },
}

#[derive(Clone, Copy)]
pub enum SnapRegion {
    Left,
    Right,
    Top,
    Bottom,
    TopLeft,
    TopRight,
    BottomLeft,
    BottomRight,
}

pub fn resolve_snap_region(key: &Key, held: crate::os::HeldArrowKeys) -> Option<SnapRegion> {
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
pub fn normalize_host_path(path: &Path) -> String {
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
pub fn normalize_host_path(path: &Path) -> String {
    path.display().to_string()
}

pub fn resolve_virtual_path(current_path: &str, target: &str) -> Result<String, String> {
    let target = target.trim();
    if target.is_empty() {
        return Ok(current_path.to_string());
    }

    let candidate = if Path::new(target).is_absolute() {
        std::path::PathBuf::from(target)
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

pub fn push_shell_command(commands: &mut Vec<CommandEntry>, current_path: &mut String, raw_command: &str) {
    let trimmed = raw_command.trim();
    if trimmed.is_empty() {
        return;
    }
    let command_cwd = current_path.clone();

    match trimmed.split_whitespace().next() {
        Some("cd") => {
            let rest = trimmed.strip_prefix("cd ").or_else(|| trimmed.strip_prefix("cd\t"));
            match rest {
                Some("") | None => {
                    commands.push(CommandEntry::completed(trimmed, &command_cwd, vec![command_cwd.clone()]));
                }
                Some(rest) => {
                    match resolve_virtual_path(current_path, rest) {
                        Ok(path) => {
                            commands.push(CommandEntry::completed(trimmed, &command_cwd, vec![path.clone()]));
                            *current_path = path;
                        }
                        Err(err) => commands.push(CommandEntry::completed(trimmed, &command_cwd, vec![err])),
                    }
                }
            }
        }
        Some("pwd" | "clear" | "help" | "exit") => {
            // With persistent shell sessions, builtins are handled by the shell itself.
            // These entries are only used in dock (Typing) mode, so keep local execution there.
            let output = CommandEntry::run_builtin(trimmed, current_path);
            commands.push(CommandEntry::completed(trimmed, &command_cwd, output));
        }
        _ => {
            commands.push(CommandEntry::spawn(trimmed, &command_cwd));
        }
    }
}

pub fn interact_terminal_horizontal_scroll(app: &mut Application, x: u16, y: u16) -> bool {
    let Some(term) = app.terminal.as_ref() else {
        return false;
    };
    if term.shell_session.is_some() {
        return false; // sessões não têm scroll horizontal
    }
    let Some(win) = app.window() else {
        return false;
    };

    let has_hscroll = win.content_w as usize > win.width.saturating_sub(2) as usize;
    let path_y = win.position_y + win.height.saturating_sub(if has_hscroll { 4 } else { 3 });
    if y != path_y || x <= win.position_x || x >= win.position_x + win.width - 1 {
        return false;
    }

    let inner_w = win.width.saturating_sub(2) as usize;
    let max_scroll = crate::gui::terminal_content_width(&term.path, &term.commands).saturating_sub(inner_w) as u16;
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

/// Interage com a scrollbar vertical de uma sessão de shell: mover o ponteiro
/// sobre a coluna da scrollbar posiciona o `panel_scroll` proporcionalmente.
pub fn interact_terminal_vertical_scroll(app: &mut Application, x: u16, y: u16) -> bool {
    let Some(term) = app.terminal.as_ref() else {
        return false;
    };
    if term.shell_session.is_none() {
        return false;
    }
    let Some(win) = app.window() else {
        return false;
    };
    if win.height < 5 {
        return false;
    }
    let content_h = win.height.saturating_sub(4) as usize;
    let sb_x = win.position_x.saturating_add(win.width).saturating_sub(2);
    if x != sb_x {
        return false;
    }
    let top = win.position_y + 1;
    if y < top || y >= top + content_h as u16 {
        return false;
    }

    let lines_len = term.shell_lines.len();
    if lines_len <= content_h {
        if let Some(t) = app.terminal.as_mut() {
            if t.panel_scroll != 0 {
                t.panel_scroll = 0;
                return true;
            }
        }
        return false;
    }
    let max_scroll = lines_len - content_h;
    let track = content_h;
    // Instagram na faixa: topo (idx 0) = início do conteúdo (panel_scroll=max),
    // fundo (idx track-1) = mais recente (panel_scroll=0). O `panel_scroll` é
    // "quanto subiu do fim", então invertemos o índice.
    let idx = (y - top) as usize;
    let down = ((idx + 1) * max_scroll / track).min(max_scroll);
    let target = max_scroll.saturating_sub(down);

    if let Some(t) = app.terminal.as_mut() {
        if t.panel_scroll != target {
            t.panel_scroll = target;
            return true;
        }
    }
    false
}

pub fn sync_terminal_window_metrics(applications: &mut [Application]) {
    for app in applications.iter_mut() {
        let Some(term) = app.terminal.as_ref() else {
            continue;
        };
        let is_session = term.shell_session.is_some();
        let content_w = crate::gui::terminal_content_width(&term.path, &term.commands);

        if let Some(win) = app.window_mut() {
            if is_session {
                // Sessões de shell não usam scroll horizontal; o scroll vertical
                // é intra-janela (draw_shell_content). content_w = 0 desativa o
                // scrollbar horizontal do chrome.
                win.content_w = 0;
                win.content_h = 0;
                win.scroll_x = 0;
            } else {
                let visible_w = win.width.saturating_sub(2) as usize;
                let max_scroll = content_w.saturating_sub(visible_w) as u16;
                win.content_w = content_w.min(u16::MAX as usize) as u16;
                win.scroll_x = win.scroll_x.min(max_scroll);
                win.content_h = 0;
            }
        }
    }
}

/// Retorna o índice da janela visualmente no topo na posição (x, y).
pub fn topmost_window_at(applications: &[Application], current_desktop: usize, x: u16, y: u16) -> Option<usize> {
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
pub fn tab_layout(applications: &[Application], current_desktop: usize, screen_h: u16, scroll: usize) -> Vec<(usize, u16, u16)> {
    let usable_h = screen_h.saturating_sub(4);
    let minimized: Vec<usize> = applications.iter().enumerate()
        .filter(|(_, a)| a.on_desktop(current_desktop) && a.is_minimized())
        .map(|(i, _)| i)
        .collect();

    if minimized.is_empty() || usable_h == 0 { return vec![]; }

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
pub fn max_tab_scroll(applications: &[Application], current_desktop: usize, screen_h: u16) -> usize {
    let usable_h = screen_h.saturating_sub(4);
    let total = applications.iter().filter(|a| a.on_desktop(current_desktop) && a.is_minimized()).count();
    if total == 0 || usable_h == 0 { return 0; }
    let tab_h: u16 = if (total as u16) * 8 <= usable_h { 8 } else { 6 };
    let max_visible = (usable_h / tab_h) as usize;
    total.saturating_sub(max_visible)
}

pub fn active_window_idx(applications: &[Application], mode: &Mode, current_desktop: usize) -> Option<usize> {
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

pub fn close_active_window(applications: &mut Vec<Application>, mode: &mut Mode, current_desktop: usize, screen_h: u16, tab_scroll: &mut usize) -> bool {
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

pub fn bring_window_to_front(applications: &mut Vec<Application>, idx: usize) -> usize {
    if idx >= applications.len() || idx == applications.len() - 1 {
        idx
    } else {
        let app = applications.remove(idx);
        applications.push(app);
        applications.len() - 1
    }
}

pub fn spawn_terminal_window(
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

pub fn spawn_terminal_window_at(
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
pub enum SplitDirection {
    Vertical,
    Horizontal,
}

pub fn split_active_terminal_window(
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

pub fn toggle_start_menu(applications: &mut Vec<Application>, current_desktop: usize, screen_h: u16, tab_scroll: &mut usize) -> bool {
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

pub fn toggle_active_maximize(applications: &mut [Application], mode: &Mode, current_desktop: usize, screen_w: u16, screen_h: u16) -> bool {
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

pub fn minimize_active_window(applications: &mut Vec<Application>, mode: &mut Mode, current_desktop: usize, screen_h: u16, tab_scroll: &mut usize) -> bool {
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

pub fn focus_relative_window(applications: &mut Vec<Application>, mode: &mut Mode, current_desktop: usize, backward: bool) -> bool {
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

pub fn move_active_window_to_desktop(
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

pub fn snap_rect(screen_w: u16, screen_h: u16, region: SnapRegion) -> (u16, u16, u16, u16) {
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

pub fn window_matches_geometry(win: &Window, x: u16, y: u16, w: u16, h: u16) -> bool {
    win.position_x == x && win.position_y == y && win.width == w && win.height == h
}

pub fn snap_active_window(
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

pub fn mode_targets_desktop(mode: &Mode, applications: &[Application], current_desktop: usize) -> bool {
    match mode {
        Mode::Moving { app_idx, .. }
        | Mode::Resizing { app_idx, .. }
        | Mode::TerminalFocus { app_idx } => applications
            .get(*app_idx)
            .map_or(false, |app| app.on_desktop(current_desktop)),
        Mode::Normal | Mode::Typing => true,
    }
}

pub fn place_pointer_on_terminal_input(pointer: &mut Pointer, applications: &[Application], app_idx: usize, screen_w: u16, screen_h: u16) {
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

pub fn enter_active_resize_mode(
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

pub fn resize_preview_size(win: &Window, pointer: &Pointer) -> (u16, u16) {
    (
        (pointer.x.saturating_sub(win.position_x) + 1).max(MIN_W),
        (pointer.y.saturating_sub(win.position_y) + 1).max(MIN_H),
    )
}

#[derive(Clone, Copy, Debug)]
pub enum ResizeAxis {
    Width,
    Height,
}

#[derive(Clone, Copy, Debug)]
pub enum ResizeOp {
    Add,
    Sub,
    Set,
}

#[derive(Debug)]
pub struct ResizeEditState {
    pub axis: ResizeAxis,
    pub op: Option<ResizeOp>,
    pub value: String,
}

pub fn apply_resize_edit(
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
