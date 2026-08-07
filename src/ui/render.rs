// Full-frame composition: draws the desktop, windows, tabs, panels, status
// bar, input rows, and the pointer in a single pass.

use super::ansi;
use super::pointer::Pointer;
use super::window::{MIN_W, MIN_H};
use super::{desktop_at, draw_command_panel, draw_desktop, draw_scrollbar, draw_status_bar,
            draw_tab, draw_terminal_content, draw_shell_content, scrollbar_thumb, tab_char_at,
            CMD_INPUT_X, DESKTOP_AREA_LEN, STATUS_START, STATUS_START_X, TERMINAL_INPUT_PREFIX};
use crate::app::Application;
use crate::cmd::CommandEntry;
use crate::input;
use crate::wm::{tab_layout, topmost_window_at, Mode};

pub fn render<W: std::io::Write>(
    out: &mut W,
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
                    if term.shell_session.is_some() {
                        draw_shell_content(out, win, &term.shell_lines, term.panel_scroll, term.repl_prompt.as_deref());
                    } else {
                        draw_terminal_content(out, win, &term.path, &term.commands, term.panel_scroll);
                    }
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

    let input_active = typing_input.is_some() || focused_terminal.is_some();

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
                // Prefix: the " .> " bar or the active REPL prompt (e.g. ">>>").
                let repl = applications.get(term_idx)
                    .and_then(|a| a.terminal.as_ref())
                    .and_then(|t| t.repl_prompt.clone());
                let prefix = repl.as_deref().unwrap_or(TERMINAL_INPUT_PREFIX);
                let prefix_len = prefix.chars().count();
                let inner_w    = (win.width - 2) as usize;
                let max_len    = inner_w.saturating_sub(prefix_len);
                let (display, cursor_col) = input::input_view(term_input, cursor_pos, max_len);
                let cursor_x   = win.position_x + 1 + prefix_len as u16;
                let cursor_y   = win.position_y + win.height - 2;
                ansi::move_to(out, cursor_x, cursor_y);
                write!(out, "{:<width$}", display, width = max_len).unwrap();
                ansi::move_to(out, cursor_x + cursor_col as u16, cursor_y);
                ansi::show_cursor(out);
            }
        }
    } else {
        ansi::hide_cursor(out);
    }
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
    if !input_active {
        pointer.draw(out, effective_cursor);
    }

    out.flush().unwrap();
}

pub fn compute_render_state(
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
    use crate::ui::window::Window;

    fn strip_sgr(seg: &str) -> String {
        let mut out = String::new();
        let bytes = seg.as_bytes();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                // Skip to the final letter (m, H, etc.)
                if let Some(rel) = bytes[i + 2..].iter().position(|&b| b.is_ascii_alphabetic()) {
                    i += 2 + rel + 1;
                    continue;
                }
            }
            out.push(bytes[i] as char);
            i += 1;
        }
        out
    }

    fn out_of_bounds_moves(buf: &[u8], w: u16, h: u16) -> Vec<(u16, u16)> {
        let bytes = buf;
        let mut bad = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == 0x1b && i + 1 < bytes.len() && bytes[i + 1] == b'[' {
                if let Some(rel) = bytes[i + 2..].iter().position(|&b| b == b'H') {
                    let seg = String::from_utf8_lossy(&bytes[i + 2..i + 2 + rel]).into_owned();
                    let clean = strip_sgr(&seg);
                    if let Some((r, c)) = clean.split_once(';') {
                        if let (Ok(row), Ok(col)) = (r.trim().parse::<u16>(), c.trim().parse::<u16>()) {
                            // move_to emits (y+1, x+1); validate within the screen.
                            if row == 0 || row > h || col == 0 || col > w {
                                bad.push((row, col));
                            }
                        }
                    }
                    i += 2 + rel + 1;
                    continue;
                }
            }
            i += 1;
        }
        bad
    }

    #[test]
    fn render_with_new_maximized_terminal_stays_in_bounds() {
        let w: u16 = 100;
        let h: u16 = 30;
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();

        // Maximized terminal (covers the usable area) + a normal terminal.
        let mut applications = vec![
            Application::terminal_window("Terminal 1", Window::new(2, 1, w - 5, h - 4, 0), cwd.clone(), Vec::new()),
            Application::terminal_window("Terminal 2", Window::new(10, 4, 50, 18, 0), cwd.clone(), Vec::new()),
        ];

        // Fill the session with many lines to exercise scrollbar/scroll.
        assert!(applications[0].terminal.as_ref().unwrap().has_session());
        for i in 0..500 {
            applications[0].terminal.as_mut().unwrap()
                .push_shell_line(format!("linha de saída do shell número {i} com conteúdo acentuado çã"));
        }
        applications[0].terminal.as_mut().unwrap().panel_scroll = 120;
        applications[1].terminal.as_mut().unwrap()
            .push_shell_line("poucas linhas".to_string());

        let pointer = Pointer::new(20, 10);
        let focused_term = Some((0, "", 0));
        let mut buf = Vec::new();
        render(
            &mut buf,
            &applications,
            None, None, w, h,
            &pointer, 0, 0, "", None, &[], 0, 1, focused_term,
        );

        let bad = out_of_bounds_moves(&buf, w, h);
        assert!(bad.is_empty(), "render wrote out of bounds: {bad:?}");
    }

    #[test]
    fn render_with_terminal_near_bottom_stays_in_bounds() {
        let w: u16 = 80;
        let h: u16 = 24;
        let cwd = std::env::current_dir().unwrap().to_string_lossy().to_string();
        // Terminal window flush with the bottom (status bar at h-4..h-1).
        let applications = vec![
            Application::terminal_window(
                "Terminal bottom",
                Window::new(2, h - 8, 40, 6, 0),
                cwd.clone(),
                Vec::new(),
            ),
        ];
        assert!(applications[0].terminal.as_ref().unwrap().has_session());
        let pointer = Pointer::new(20, 10);
        let mut buf = Vec::new();
        render(
            &mut buf,
            &applications,
            None, None, w, h,
            &pointer, 0, 0, "", None, &[], 0, 1, None,
        );
        let bad = out_of_bounds_moves(&buf, w, h);
        assert!(bad.is_empty(), "render wrote out of bounds: {bad:?}");
    }
}
