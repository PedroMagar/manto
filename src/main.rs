mod ansi;
mod application;
mod cmd;
mod gui;
mod os;
mod pointer;
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
use std::io::Write;
use std::time::Duration;

enum Mode {
    Normal,
    Moving          { app_idx: usize, offset_x: u16 },
    Resizing        { app_idx: usize },
    Typing,
    TerminalFocus   { app_idx: usize },
}

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
        | Mode::Resizing { app_idx }
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
    applications[idx].set_window_geometry(x, y, w.max(MIN_W), h.max(MIN_H));
    *mode = Mode::Normal;
    true
}

fn mode_targets_desktop(mode: &Mode, applications: &[Application], current_desktop: usize) -> bool {
    match mode {
        Mode::Moving { app_idx, .. }
        | Mode::Resizing { app_idx }
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
    let input_y = win.position_y + win.height.saturating_sub(2);
    let max_x = win.position_x + win.width.saturating_sub(2);

    pointer.x = input_x.min(max_x);
    pointer.y = input_y;
    pointer.clamp_to_bounds(screen_w, screen_h);
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
    typing_input: Option<&str>,
    commands: &[CommandEntry],
    panel_scroll: usize,
    current_desktop: usize,
    // Índice e input do terminal com foco para exibir cursor real.
    focused_terminal: Option<(usize, &str)>,
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
    if let Some(input) = typing_input {
        let max_len = (w - 2).saturating_sub(CMD_INPUT_X) as usize;
        let display = if input.len() > max_len { &input[input.len() - max_len..] } else { input };
        ansi::move_to(out, CMD_INPUT_X, h - 2);
        write!(out, "{:<width$}", display, width = max_len).unwrap();
        ansi::move_to(out, CMD_INPUT_X + display.len() as u16, h - 2);
        ansi::show_cursor(out);
    } else if let Some((term_idx, term_input)) = focused_terminal {
        // Cursor real dentro da janela de terminal com foco
        if let Some(win) = applications.get(term_idx)
            .filter(|a| a.on_desktop(current_desktop))
            .and_then(|a| a.window())
        {
            if win.height >= 5 {
                let prefix_len = TERMINAL_INPUT_PREFIX.chars().count();
                let inner_w    = (win.width - 2) as usize;
                let max_len    = inner_w.saturating_sub(prefix_len);
                let display    = if term_input.len() > max_len { &term_input[term_input.len() - max_len..] } else { term_input };
                let cursor_x   = win.position_x + 1 + prefix_len as u16;
                let cursor_y   = win.position_y + win.height - 2;
                ansi::move_to(out, cursor_x, cursor_y);
                write!(out, "{:<width$}", display, width = max_len).unwrap();
                ansi::move_to(out, cursor_x + display.len() as u16, cursor_y);
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
    let current_path         = String::from("./");
    let mut cmd_input        = String::new();
    let mut commands: Vec<CommandEntry> = Vec::new();
    let mut last_size     = os::size();
    let mut pointer       = Pointer::new(1 + STATUS_BAR_PREFIX.len() as u16, last_size.1 - 2);

    let mut applications = Vec::new();

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    let in_shell     = matches!(mode, Mode::Typing);
    let shell_path   = if in_shell { current_path.as_str() } else { "" };
    let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
        applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str()))
    } else { None };
    render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some(&cmd_input) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);

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

                _ => match &mut mode {
                    Mode::Normal => match key {
                        Key::CtrlQ => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::TopLeft) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlE => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::TopRight) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlZ => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::BottomLeft) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlV => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::BottomRight) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlH | Key::Backspace => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::Left) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlL => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::Right) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlK => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::Top) {
                                mode_changed = true;
                            }
                        }
                        Key::CtrlJ => {
                            if snap_active_window(&mut applications, &mut mode, current_desktop, last_size.0, last_size.1, SnapRegion::Bottom) {
                                mode_changed = true;
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
                                    tab_scroll = tab_scroll
                                        .min(max_tab_scroll(&applications, current_desktop, last_size.1));
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
                                    let scroll_handled = if let Some(win) =
                                        applications[top_idx].window_mut()
                                    {
                                        win.interact(pointer.x, pointer.y)
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
                                            win.height >= 5
                                                && pointer.y == win.position_y + win.height - 2
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
                                        mode = Mode::Resizing { app_idx: top_idx };
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
                            Key::Enter => {
                                let trimmed = cmd_input.trim().to_string();
                                if !trimmed.is_empty() {
                                    commands.push(CommandEntry::new(&trimmed));
                                    cmd_input.clear();
                                    panel_scroll = 0;
                                }
                                mode_changed = true;
                            }
                            Key::Backspace => {
                                cmd_input.pop();
                                mode_changed = true;
                            }
                            Key::Char(c) => {
                                cmd_input.push(c);
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

                    Mode::Resizing { app_idx } => match key {
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),
                        Key::Char(' ') | Key::Enter => {
                            let idx = *app_idx;
                            if let Some(win) = applications[idx].window_mut() {
                                win.width  = (pointer.x.saturating_sub(win.position_x) + 1).max(MIN_W);
                                win.height = (pointer.y.saturating_sub(win.position_y) + 1).max(MIN_H);
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
                            Key::Enter => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    let trimmed = t.cmd_input.trim().to_string();
                                    if !trimmed.is_empty() {
                                        t.commands.push(CommandEntry::new(&trimmed));
                                        t.cmd_input.clear();
                                        t.panel_scroll = 0;
                                    }
                                    mode_changed = true;
                                }
                            }
                            Key::Backspace => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    t.cmd_input.pop();
                                    mode_changed = true;
                                }
                            }
                            Key::Char(c) => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    t.cmd_input.push(c);
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
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                let in_shell     = matches!(mode, Mode::Typing);
                let shell_path   = if in_shell { current_path.as_str() } else { "" };
                let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
                    applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str()))
                } else { None };
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some(&cmd_input) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);
            }
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            scroll_offset = scroll_offset.wrapping_add(1);
            let cmds_changed = tick_all(&mut commands)
                || applications.iter_mut().any(|a| {
                    a.terminal.as_mut().map_or(false, |t| t.tick())
                });
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
            if size_changed || needs_scroll || cmds_changed {
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                let in_shell     = matches!(mode, Mode::Typing);
                let shell_path   = if in_shell { current_path.as_str() } else { "" };
                let focused_term = if let Mode::TerminalFocus { app_idx } = &mode {
                    applications.get(*app_idx).and_then(|a| a.terminal.as_ref()).map(|t| (*app_idx, t.cmd_input.as_str()))
                } else { None };
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, shell_path, if in_shell { Some(&cmd_input) } else { None }, if in_shell { &commands } else { &[] }, panel_scroll, current_desktop, focused_term);
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
        Mode::Resizing { app_idx } => {
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
