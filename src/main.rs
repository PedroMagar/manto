mod ansi;
mod application;
mod cmd;
mod gui;
mod history;
mod input;
mod os;
mod pointer;
mod terminal_backend;
mod window;
mod wm;

use gui::{draw_desktop, draw_status_bar, draw_tab, draw_scrollbar, draw_command_panel,
          draw_terminal_content, tab_char_at, scrollbar_thumb, desktop_at,
          STATUS_BAR_PREFIX, STATUS_START, STATUS_START_X, CMD_INPUT_X, DESKTOP_AREA_LEN,
          TERMINAL_INPUT_PREFIX};
use cmd::{CommandEntry, tick_all};
use history::History;
pub use application::{Application, TerminalState};
use window::{Window, MIN_W, MIN_H};
use os::{Writer, Clock, Key};
use pointer::Pointer;
use std::io::Write;
use std::time::Duration;

use crate::wm::{SnapRegion, resolve_snap_region, normalize_host_path, push_shell_command,
                sync_terminal_window_metrics, topmost_window_at, tab_layout, max_tab_scroll,
                close_active_window, bring_window_to_front, spawn_terminal_window,
                split_active_terminal_window, toggle_start_menu, toggle_active_maximize,
                minimize_active_window, focus_relative_window, move_active_window_to_desktop,
                snap_rect, snap_active_window, place_pointer_on_terminal_input,
                enter_active_resize_mode, apply_resize_edit, ResizeEditState, Mode};

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
    focused_terminal: Option<(usize, &str, usize)>,
) {
    ansi::clear(out);

    draw_desktop(out, 1, w, h, "Manto");

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

    if let Some((idx, pw, ph)) = resize_preview {
        if applications[idx].on_desktop(current_desktop) {
            if let Some(win) = applications[idx].window() {
                win.draw_preview(out, pw, ph);
            }
        }
    }

    draw_command_panel(out, w, h, path, commands, panel_scroll);
    draw_status_bar(out, w, h, path, !commands.is_empty(), current_desktop);

    let start_end = STATUS_START_X + STATUS_START.len() as u16;
    if pointer.y == h - 2 && pointer.x >= STATUS_START_X && pointer.x < start_end {
        ansi::move_to(out, STATUS_START_X, h - 2);
        write!(out, "{}{}{}", ansi::REVERSE, STATUS_START, ansi::RESET).unwrap();
    }

    if let Some(d) = desktop_at(pointer.x, pointer.y, w, h) {
        let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN);
        let sep_x  = base_x + (d as u16 - 1) * 4;
        ansi::move_to(out, sep_x + 1, h - 2);
        write!(out, "{} {} {}", ansi::REVERSE, d, ansi::RESET).unwrap();
    }

    if let Some((input, cursor_pos)) = typing_input {
        let max_len = (w - 2).saturating_sub(CMD_INPUT_X) as usize;
        let (display, cursor_col) = input::input_view(input, cursor_pos, max_len);
        ansi::move_to(out, CMD_INPUT_X, h - 2);
        write!(out, "{:<width$}", display, width = max_len).unwrap();
        ansi::move_to(out, CMD_INPUT_X + cursor_col as u16, h - 2);
        ansi::show_cursor(out);
    } else if let Some((term_idx, term_input, cursor_pos)) = focused_terminal {
        if let Some(win) = applications.get(term_idx)
            .filter(|a| a.on_desktop(current_desktop))
            .and_then(|a| a.window())
        {
            if win.height >= 5 {
                let prefix_len = TERMINAL_INPUT_PREFIX.chars().count();
                let inner_w    = (win.width - 2) as usize;
                let max_len    = inner_w.saturating_sub(prefix_len);
                let (display, cursor_col) = input::input_view(term_input, cursor_pos, max_len);
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
    let mut last_size     = os::size();
    let mut pointer       = Pointer::new(1 + STATUS_BAR_PREFIX.len() as u16, last_size.1 - 2);

    let mut applications = Vec::new();
    sync_terminal_window_metrics(&mut applications);

    let history = History::new();
    let loaded_history = history.load(1000);
    let mut commands: Vec<CommandEntry> = if loaded_history.is_empty() {
        Vec::new()
    } else {
        let cwd = current_path.clone();
        loaded_history.iter().map(|line| {
            CommandEntry::completed(line, &cwd, vec![line.clone()])
        }).collect()
    };

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
                        wm::SplitDirection::Vertical,
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
                        wm::SplitDirection::Horizontal,
                    ) {
                        place_pointer_on_terminal_input(&mut pointer, &applications, app_idx, last_size.0, last_size.1);
                        mode = Mode::TerminalFocus { app_idx };
                        mode_changed = true;
                    }
                }

                Key::CtrlC => {
                    match &mode {
                        Mode::TerminalFocus { app_idx } => {
                            if let Some(t) = applications[*app_idx].terminal.as_mut() {
                                for cmd in t.commands.iter_mut().rev() {
                                    if cmd.is_running_external() {
                                        cmd.kill();
                                        t.commands.push(CommandEntry::completed(
                                            "^C", &t.path, vec!["".to_string()],
                                        ));
                                        mode_changed = true;
                                        break;
                                    }
                                }
                            }
                        }
                        Mode::Typing => {}
                        _ => {}
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
                            if !wm::mode_targets_desktop(&mode, &applications, current_desktop) {
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

                            if let Some(d) = desktop_at(pointer.x, pointer.y, last_size.0, last_size.1) {
                                current_desktop = d;
                                tab_scroll = tab_scroll.min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                if !wm::mode_targets_desktop(&mode, &applications, current_desktop) {
                                    mode = Mode::Normal;
                                }
                                mode_changed = true;
                            } else if pointer.y == last_size.1 - 2
                                && pointer.x >= CMD_INPUT_X.saturating_sub(TERMINAL_INPUT_PREFIX.len() as u16)
                            {
                                mode = Mode::Typing;
                                panel_scroll = 0;
                                mode_changed = true;
                            } else {
                                let start_end = STATUS_START_X + STATUS_START.len() as u16;
                                if pointer.y == last_size.1 - 2
                                    && pointer.x >= STATUS_START_X
                                    && pointer.x < start_end
                                {
                                    toggle_start_menu(&mut applications, current_desktop, last_size.1, &mut tab_scroll);
                                    mode_changed = true;
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
                                } else if let Some(top_idx) =
                                    topmost_window_at(&applications, current_desktop, pointer.x, pointer.y)
                                {
                                    let mut skip = false;
                                    if let Some(menu_idx) = applications.iter().position(|a| a.on_desktop(current_desktop) && a.is_menu) {
                                        if top_idx != menu_idx {
                                            applications.remove(menu_idx);
                                            tab_scroll = tab_scroll
                                                .min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                            mode_changed = true;
                                            skip = true;
                                        }
                                    }
                                    if !skip {
                                        let scroll_handled = if let Some(app) = applications.get_mut(top_idx) {
                                            let handled = if let Some(win) = app.window_mut() {
                                                win.interact(pointer.x, pointer.y)
                                            } else {
                                                false
                                            };
                                            handled || wm::interact_terminal_horizontal_scroll(app, pointer.x, pointer.y)
                                        } else {
                                            false
                                        };
                                        if scroll_handled {
                                            mode_changed = true;
                                        }

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
                                        }
                                    }
                                }
                            }
                        }
                        _ => {}
                    },

                    Mode::Typing => {
                        match key {
                            Key::Escape | Key::End => {
                                mode = Mode::Normal;
                                mode_changed = true;
                            }
                            Key::CtrlEnter => {
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
                                if input::history_up(&commands, &mut cmd_input, &mut history_index, &mut history_draft) {
                                    cmd_cursor = input::input_char_len(&cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Down => {
                                if input::history_down(&commands, &mut cmd_input, &mut history_index, &mut history_draft) {
                                    cmd_cursor = input::input_char_len(&cmd_input);
                                    mode_changed = true;
                                }
                            }
                            Key::Left => {
                                if input::move_input_cursor_left(&mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Right => {
                                if input::move_input_cursor_right(&cmd_input, &mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Tab => {
                                input::reset_history_navigation(&mut history_index, &mut history_draft);
                                if input::autocomplete_input(&mut cmd_input, &mut cmd_cursor, &current_path) {
                                    mode_changed = true;
                                }
                            }
                            Key::Enter => {
                                let trimmed = cmd_input.trim().to_string();
                                if !trimmed.is_empty() {
                                    push_shell_command(&mut commands, &mut current_path, &trimmed);
                                    history.append(&trimmed);
                                    cmd_input.clear();
                                    cmd_cursor = 0;
                                    input::reset_history_navigation(&mut history_index, &mut history_draft);
                                    panel_scroll = 0;
                                }
                                mode_changed = true;
                            }
                            Key::Delete => {
                                input::reset_history_navigation(&mut history_index, &mut history_draft);
                                if input::delete_input_char(&mut cmd_input, &mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Backspace => {
                                input::reset_history_navigation(&mut history_index, &mut history_draft);
                                if input::backspace_input_char(&mut cmd_input, &mut cmd_cursor) {
                                    mode_changed = true;
                                }
                            }
                            Key::Char(c) => {
                                input::reset_history_navigation(&mut history_index, &mut history_draft);
                                input::insert_input_char(&mut cmd_input, &mut cmd_cursor, c);
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
                            *edit = Some(ResizeEditState { axis: wm::ResizeAxis::Width, op: None, value: String::new() });
                            mode_changed = true;
                        }
                        Key::Char('y') | Key::Char('v') => {
                            *edit = Some(ResizeEditState { axis: wm::ResizeAxis::Height, op: None, value: String::new() });
                            mode_changed = true;
                        }
                        _ if edit.is_some() => {
                            let mut clear_edit = false;
                            let mut changed_pointer = false;

                            if let Some(state) = edit.as_mut() {
                                match key {
                                    Key::Char(' ') => {}
                                    Key::Char('+') if state.op.is_none() => {
                                        state.op = Some(wm::ResizeOp::Add);
                                        mode_changed = true;
                                    }
                                    Key::Char('-') if state.op.is_none() => {
                                        state.op = Some(wm::ResizeOp::Sub);
                                        mode_changed = true;
                                    }
                                    Key::Char('=') if state.op.is_none() => {
                                        state.op = Some(wm::ResizeOp::Set);
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
                                let (width, height) = wm::resize_preview_size(win, &pointer);
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
                                    if input::history_up(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                        t.input_cursor = input::input_char_len(&t.cmd_input);
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Down => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if input::history_down(&t.commands, &mut t.cmd_input, &mut t.history_index, &mut t.history_draft) {
                                        t.input_cursor = input::input_char_len(&t.cmd_input);
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Left => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if input::move_input_cursor_left(&mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Right => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    if input::move_input_cursor_right(&t.cmd_input, &mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Tab => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    if input::autocomplete_input(&mut t.cmd_input, &mut t.input_cursor, &t.path) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Enter => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    let trimmed = t.cmd_input.trim().to_string();
                                    if !trimmed.is_empty() {
                                        push_shell_command(&mut t.commands, &mut t.path, &trimmed);
                                        history.append(&trimmed);
                                        if let Some(last) = t.commands.last() {
                                            if last.is_exit() {
                                                applications.remove(idx);
                                                tab_scroll = tab_scroll.min(max_tab_scroll(&applications, current_desktop, last_size.1));
                                                mode = Mode::Normal;
                                                mode_changed = true;
                                                return;
                                            }
                                        }
                                        t.cmd_input.clear();
                                        t.input_cursor = 0;
                                        input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                        t.panel_scroll = 0;
                                    }
                                    mode_changed = true;
                                }
                            }
                            Key::Delete => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    if input::delete_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Backspace => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    if input::backspace_input_char(&mut t.cmd_input, &mut t.input_cursor) {
                                        mode_changed = true;
                                    }
                                }
                            }
                            Key::Char(c) => {
                                if let Some(t) = applications[idx].terminal.as_mut() {
                                    input::reset_history_navigation(&mut t.history_index, &mut t.history_draft);
                                    input::insert_input_char(&mut t.cmd_input, &mut t.input_cursor, c);
                                    mode_changed = true;
                                }
                            }
                            _ => {}
                        }
                    },
                },
            }

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

        assert!(input::history_up(&commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo c");
        assert_eq!(index, Some(2));

        assert!(input::history_up(&commands, &mut input, &mut index, &mut draft));
        assert_eq!(input, "echo b");
        assert_eq!(index, Some(1));
    }

    #[test]
    fn history_down_restores_draft_after_latest() {
        let commands = fixture_commands();
        let mut input = String::from("ec");
        let mut index = None;
        let mut draft = None;

        input::history_up(&commands, &mut input, &mut index, &mut draft);
        input::history_down(&commands, &mut input, &mut index, &mut draft);

        assert_eq!(input, "ec");
        assert_eq!(index, None);
    }

    #[test]
    fn token_bounds_find_current_word() {
        let input = "cd targ";
        assert_eq!(input::token_bounds(input, 7), (3, 7));
        assert_eq!(input::token_bounds(input, 2), (0, 2));
    }

    #[test]
    fn autocomplete_cd_completes_directory() {
        let base = std::env::temp_dir().join(format!("manto-test-{}", std::process::id()));
        let target = base.join("target-dir");
        std::fs::create_dir_all(&target).unwrap();

        let mut input = String::from("cd tar");
        let mut cursor = input::input_char_len(&input);
        let base_str = base.display().to_string();

        let changed = input::autocomplete_input(&mut input, &mut cursor, &base_str);

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
            axis: wm::ResizeAxis::Width,
            op: Some(wm::ResizeOp::Add),
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
            axis: wm::ResizeAxis::Height,
            op: Some(wm::ResizeOp::Set),
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
        assert!(wm::window_matches_geometry(win, top.0, top.1, top.2, top.3));
        assert!(!applications[0].is_maximized());

        assert!(snap_active_window(&mut applications, &mut mode, 1, 120, 40, SnapRegion::Top));
        assert!(applications[0].is_maximized());

        assert!(snap_active_window(&mut applications, &mut mode, 1, 120, 40, SnapRegion::Top));
        let win = applications[0].window().unwrap();
        assert!(wm::window_matches_geometry(win, top.0, top.1, top.2, top.3));
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
            wm::SplitDirection::Vertical,
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
            wm::SplitDirection::Horizontal,
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
