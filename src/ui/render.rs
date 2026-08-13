// Full-frame composition: draws the desktop, windows, tabs, panels, status
// bar, input rows, and the pointer in a single pass.

use std::io::Write;

use super::ansi;
use super::pointer::Pointer;
use super::screen::{BoxSelect, ScreenGrid, StampWriter};
use super::window::{MIN_H, MIN_W};
use super::{
    CMD_INPUT_X, DESKTOP_AREA_LEN, STATUS_START, STATUS_START_X, TERMINAL_INPUT_PREFIX, desktop_at,
    draw_command_panel, draw_desktop, draw_emulator_content, draw_help_content, draw_menu_content,
    draw_scrollbar, draw_shell_content, draw_status_bar, draw_tab, draw_terminal_content,
    scrollbar_thumb, tab_char_at,
};
use crate::app::Application;
use crate::cmd::CommandEntry;
use crate::input;
use crate::wm::{Mode, tab_layout, topmost_window_at};

/// Everything `render` needs to compose one frame. Grouped so the function
/// signature stays honest as the frame inputs grow.
pub struct Frame<'a> {
    pub applications: &'a [Application],
    pub resize_preview: Option<(usize, u16, u16)>,
    pub cursor_interaction: Option<char>,
    pub w: u16,
    pub h: u16,
    pub pointer: &'a Pointer,
    pub scroll_offset: usize,
    pub tab_scroll: usize,
    pub path: &'a str,
    pub typing_input: Option<(&'a str, usize)>,
    pub commands: &'a [CommandEntry],
    pub panel_scroll: usize,
    pub current_desktop: usize,
    pub focused_terminal: Option<(usize, &'a str, usize)>,
    pub grid: &'a mut ScreenGrid,
    pub selection: Option<&'a BoxSelect>,
    pub theme: u16,
    pub full_redraw: bool,
    pub draw_pointer: bool,
}

pub fn render<W: std::io::Write>(out: &mut W, ctx: &mut Frame) {
    let applications = ctx.applications;
    let resize_preview = ctx.resize_preview;
    let cursor_interaction = ctx.cursor_interaction;
    let w = ctx.w;
    let h = ctx.h;
    let pointer = ctx.pointer;
    let scroll_offset = ctx.scroll_offset;
    let tab_scroll = ctx.tab_scroll;
    let path = ctx.path;
    let typing_input = ctx.typing_input;
    let commands = ctx.commands;
    let panel_scroll = ctx.panel_scroll;
    let current_desktop = ctx.current_desktop;
    let focused_terminal = ctx.focused_terminal;
    let grid = &mut ctx.grid;
    let selection = ctx.selection;
    let theme = ctx.theme;
    let full_redraw = ctx.full_redraw;
    let draw_pointer = ctx.draw_pointer;
    // Compose the new frame entirely in memory (a fresh grid plus a buffer we
    // throw away), so we can compare it to the previous frame and only emit
    // the rows that changed — no `clear` + full redraw per frame.
    let minimized_count = applications
        .iter()
        .filter(|a| a.on_desktop(current_desktop) && a.is_minimized())
        .count();
    let tab_x = w.saturating_sub(3);
    let sb_x = w.saturating_sub(1);
    let sb_top = 1u16;
    let sb_bot = h.saturating_sub(4);
    let tabs = tab_layout(applications, current_desktop, h, tab_scroll);

    let mut next = ScreenGrid::new(w, h);
    let mut buf = Vec::new();
    {
        let mut frame_out = StampWriter::new(&mut buf, &mut next);

        draw_desktop(&mut frame_out, theme, w, h, "Manto");

        // Draw windows back-to-front by real z-order (layer, then vector order).
        let mut draw_order: Vec<usize> = (0..applications.len()).collect();
        draw_order.sort_by_key(|&i| {
            let layer = applications[i].window().map_or(0, |win| win.layer);
            (layer, i)
        });
        for app_idx in draw_order {
            let app = &applications[app_idx];
            if app.on_desktop(current_desktop)
                && let Some(win) = app.window()
            {
                win.draw(&mut frame_out, &app.title);
                if let Some(term) = app.terminal.as_ref() {
                    if let Some(em) = term.emulator.as_ref() {
                        // Interactive terminal: full emulator grid + cursor.
                        let focused = focused_terminal.map(|(i, _, _)| i) == Some(app_idx);
                        draw_emulator_content(&mut frame_out, win, em, term.panel_scroll, focused);
                    } else if term.shell_session.is_some() {
                        draw_shell_content(
                            &mut frame_out,
                            win,
                            &term.shell_lines,
                            term.panel_scroll,
                            term.repl_prompt.as_deref(),
                        );
                    } else {
                        draw_terminal_content(
                            &mut frame_out,
                            win,
                            &term.path,
                            &term.commands,
                            term.panel_scroll,
                        );
                    }
                } else if let Some(menu) = app.menu.as_ref() {
                    // Start menu: manifest entries with the selection.
                    draw_menu_content(&mut frame_out, win, menu);
                } else if let Some(help) = app.help.as_ref() {
                    // Help window: the wrapped usage crib sheet.
                    draw_help_content(&mut frame_out, win, help);
                }
            }
        }

        if minimized_count > 0 {
            for &(app_idx, tab_y, tab_h) in &tabs {
                let is_hovered =
                    pointer.x >= tab_x && pointer.y >= tab_y && pointer.y < tab_y + tab_h;
                let offset = if is_hovered { scroll_offset } else { 0 };
                draw_tab(
                    &mut frame_out,
                    tab_x,
                    tab_y,
                    tab_h,
                    &applications[app_idx].title,
                    offset,
                );
            }
            draw_scrollbar(
                &mut frame_out,
                sb_x,
                sb_top,
                sb_bot,
                minimized_count,
                tabs.len(),
                tab_scroll,
            );
        }

        if let Some((idx, pw, ph)) = resize_preview
            && applications[idx].on_desktop(current_desktop)
            && let Some(win) = applications[idx].window()
        {
            win.draw_preview(&mut frame_out, pw, ph);
        }

        draw_command_panel(&mut frame_out, w, h, path, commands, panel_scroll);
        draw_status_bar(
            &mut frame_out,
            w,
            h,
            path,
            !commands.is_empty(),
            current_desktop,
        );

        if let Some((input, cursor_pos)) = typing_input {
            let max_len = (w - 2).saturating_sub(CMD_INPUT_X) as usize;
            let (display, _) = input::input_view(input, cursor_pos, max_len);
            ansi::move_to(&mut frame_out, CMD_INPUT_X, h - 2);
            write!(&mut frame_out, "{display:<max_len$}").unwrap();
        } else if let Some((term_idx, term_input, cursor_pos)) = focused_terminal {
            let interactive_mode = applications
                .get(term_idx)
                .and_then(|a| a.terminal.as_ref())
                .is_some_and(|t| t.interactive);
            if !interactive_mode
                && let Some(win) = applications
                    .get(term_idx)
                    .filter(|a| a.on_desktop(current_desktop))
                    .and_then(|a| a.window())
                && win.height >= 5
            {
                let repl = applications
                    .get(term_idx)
                    .and_then(|a| a.terminal.as_ref())
                    .and_then(|t| t.repl_prompt.clone());
                let prefix = repl.as_deref().unwrap_or(TERMINAL_INPUT_PREFIX);
                let prefix_len = prefix.chars().count();
                let inner_w = (win.width - 2) as usize;
                let max_len = inner_w.saturating_sub(prefix_len);
                let (display, _) = input::input_view(term_input, cursor_pos, max_len);
                let cursor_x = win.position_x + 1 + prefix_len as u16;
                let cursor_y = win.position_y + win.height - 2;
                ansi::move_to(&mut frame_out, cursor_x, cursor_y);
                write!(&mut frame_out, "{display:<max_len$}").unwrap();
            }
        }

        // Attribute-only highlights (hover over the start button, desktop
        // buttons, the free selection and the pointer over chrome) are drawn
        // through the stamp writer so their REVERSE style lands in the frame
        // and the style diff repaints the row.
        let start_end = STATUS_START_X + STATUS_START.len() as u16;
        if pointer.y == h - 2 && pointer.x >= STATUS_START_X && pointer.x < start_end {
            ansi::move_to(&mut frame_out, STATUS_START_X, h - 2);
            write!(
                &mut frame_out,
                "{}{}{}",
                ansi::REVERSE,
                STATUS_START,
                ansi::RESET
            )
            .unwrap();
        }

        if let Some(d) = desktop_at(pointer.x, pointer.y, w, h) {
            let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN);
            let sep_x = base_x + (d as u16 - 1) * 4;
            ansi::move_to(&mut frame_out, sep_x + 1, h - 2);
            write!(&mut frame_out, "{} {} {}", ansi::REVERSE, d, ansi::RESET).unwrap();
        }

        // Free screen selection: invert every cell of the box.
        if let Some(sel) = selection {
            let (top, bottom, left, right) = sel.bounds();
            for y in top..=bottom.min(h as usize - 1) {
                for x in left..=right.min(w as usize - 1) {
                    let ch = frame_out.grid().char_at(x, y);
                    ansi::move_to(&mut frame_out, x as u16, y as u16);
                    write!(&mut frame_out, "{}{}{}", ansi::REVERSE, ch, ansi::RESET).unwrap();
                }
            }
        }

        // Pointer: reversed when hovering interactive chrome, plain "░"
        // elsewhere. Hidden while typing in the dock or editing a line-mode
        // terminal (the caret marks the position there); inside an interactive
        // app it appears only while the mouse is in use so it does not stack
        // on the app's own cursor.
        if draw_pointer {
            let cursor_ctx = CursorContext {
                applications,
                current_desktop,
                pointer,
                w,
                h,
                tabs: &tabs,
                sb_x,
                sb_top,
                sb_bot,
                scroll_offset,
                tab_scroll,
                minimized_count,
                cursor_interaction,
            };
            pointer.draw(&mut frame_out, effective_cursor(&cursor_ctx));
        }
    }

    // Emit only the changed rows, then refresh the backing grid for selection
    // copy and the next frame's diff.
    emit_frame_diff(out, grid, &next, full_redraw);

    // Caret: show over the input field when editing, hide otherwise.
    match caret_position(
        &typing_input,
        &focused_terminal,
        applications,
        current_desktop,
        w,
        h,
    ) {
        Some((cx, cy)) => {
            ansi::move_to(out, cx, cy);
            ansi::show_cursor(out);
        }
        None => ansi::hide_cursor(out),
    }

    **grid = next;
    out.flush().unwrap();
}

/// Where the text caret should be drawn, or None to hide the cursor.
fn caret_position(
    typing_input: &Option<(&str, usize)>,
    focused_terminal: &Option<(usize, &str, usize)>,
    applications: &[Application],
    current_desktop: usize,
    w: u16,
    h: u16,
) -> Option<(u16, u16)> {
    if let Some((input, cursor_pos)) = typing_input {
        let max_len = (w - 2).saturating_sub(CMD_INPUT_X) as usize;
        let (_, cursor_col) = input::input_view(input, *cursor_pos, max_len);
        return Some((CMD_INPUT_X + cursor_col as u16, h - 2));
    }
    if let Some((term_idx, term_input, cursor_pos)) = focused_terminal {
        let interactive_mode = applications
            .get(*term_idx)
            .and_then(|a| a.terminal.as_ref())
            .is_some_and(|t| t.interactive);
        if interactive_mode {
            return None;
        }
        if let Some(win) = applications
            .get(*term_idx)
            .filter(|a| a.on_desktop(current_desktop))
            .and_then(|a| a.window())
            && win.height >= 5
        {
            let repl = applications
                .get(*term_idx)
                .and_then(|a| a.terminal.as_ref())
                .and_then(|t| t.repl_prompt.clone());
            let prefix = repl.as_deref().unwrap_or(TERMINAL_INPUT_PREFIX);
            let prefix_len = prefix.chars().count();
            let inner_w = (win.width - 2) as usize;
            let max_len = inner_w.saturating_sub(prefix_len);
            let (_, cursor_col) = input::input_view(term_input, *cursor_pos, max_len);
            let cursor_x = win.position_x + 1 + prefix_len as u16;
            let cursor_y = win.position_y + win.height - 2;
            return Some((cursor_x + cursor_col as u16, cursor_y));
        }
    }
    None
}

/// Inputs for `effective_cursor`: everything needed to decide the transient
/// pointer glyph (hover over tabs, start button, desktop buttons, window
/// chrome, scrollbars).
struct CursorContext<'a> {
    applications: &'a [Application],
    current_desktop: usize,
    pointer: &'a Pointer,
    w: u16,
    h: u16,
    tabs: &'a [(usize, u16, u16)],
    sb_x: u16,
    sb_top: u16,
    sb_bot: u16,
    scroll_offset: usize,
    tab_scroll: usize,
    minimized_count: usize,
    cursor_interaction: Option<char>,
}

/// The transient character under the pointer (hover over tabs, the start
/// button, desktop buttons, window chrome, scrollbars).
fn effective_cursor(ctx: &CursorContext) -> Option<char> {
    let applications = ctx.applications;
    let current_desktop = ctx.current_desktop;
    let pointer = ctx.pointer;
    let w = ctx.w;
    let h = ctx.h;
    let tabs = ctx.tabs;
    let sb_x = ctx.sb_x;
    let sb_top = ctx.sb_top;
    let sb_bot = ctx.sb_bot;
    let scroll_offset = ctx.scroll_offset;
    let tab_scroll = ctx.tab_scroll;
    let minimized_count = ctx.minimized_count;
    let cursor_interaction = ctx.cursor_interaction;
    let px = pointer.x;
    let py = pointer.y;

    if minimized_count > tabs.len() && px == sb_x && py >= sb_top && py <= sb_bot {
        let track_len = (sb_bot - sb_top + 1) as usize;
        let (thumb_pos, thumb_len) =
            scrollbar_thumb(track_len, minimized_count, tabs.len(), tab_scroll);
        let row = (py - sb_top) as usize;
        return Some(if row >= thumb_pos && row < thumb_pos + thumb_len {
            '█'
        } else {
            '░'
        });
    }

    let tab_x = w.saturating_sub(3);
    if px >= tab_x
        && px < sb_x
        && let Some(&(app_idx, tab_y, tab_h)) =
            tabs.iter().find(|&&(_, ty, th)| py >= ty && py < ty + th)
    {
        return Some(tab_char_at(
            tab_x,
            tab_y,
            tab_h,
            &applications[app_idx].title,
            px,
            py,
            scroll_offset,
        ));
    }

    let start_end = STATUS_START_X + STATUS_START.len() as u16;
    if py == h - 2 && px >= STATUS_START_X && px < start_end {
        return Some(
            STATUS_START
                .chars()
                .nth((px - STATUS_START_X) as usize)
                .unwrap_or(' '),
        );
    }

    if let Some(d) = desktop_at(px, py, w, h) {
        let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN);
        let sep_x = base_x + (d as u16 - 1) * 4;
        let offset = px - (sep_x + 1);
        return Some(if offset == 1 {
            char::from_digit(d as u32, 10).unwrap_or(' ')
        } else {
            ' '
        });
    }

    if let Some(top_idx) = topmost_window_at(applications, current_desktop, px, py)
        && let Some(win) = applications[top_idx].window()
        && let Some(ch) = win.char_at(px, py, &applications[top_idx].title)
    {
        return Some(ch);
    }

    cursor_interaction
}

/// Damage-based frame emission: for every row whose characters or styles
/// changed, rewrite the whole row at its screen position, carrying the
/// per-cell style (colors, attributes, reverse) through minimal SGR
/// transitions. On a size change the screen is cleared first.
fn emit_frame_diff<W: std::io::Write>(
    out: &mut W,
    prev: &ScreenGrid,
    next: &ScreenGrid,
    full_redraw: bool,
) {
    use crate::terminal_emulator::Style;
    let (w, h) = (next.width(), next.height());
    if full_redraw || prev.width() != w || prev.height() != h {
        ansi::clear(out);
    }
    for y in 0..h {
        let row_changed = (0..w).any(|x| {
            prev.char_at(x, y) != next.char_at(x, y) || prev.style_at(x, y) != next.style_at(x, y)
        });
        if !row_changed {
            continue;
        }
        ansi::move_to(out, 0, y as u16);
        let mut prev_style: Option<Style> = None;
        for x in 0..w {
            let cell = next.cell_at(x, y);
            ansi::sgr(out, prev_style.as_ref(), &cell.style);
            write!(out, "{}", cell.ch).unwrap();
            prev_style = Some(cell.style);
        }
        // Reset the trailing style so the next row starts clean.
        ansi::sgr(out, prev_style.as_ref(), &Style::default());
    }
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
        Mode::Moving { .. } => (None, None),
        Mode::Typing => (None, None),
        Mode::TerminalFocus { .. } => (None, None),
        Mode::Normal => (None, None),
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
            if bytes[i] == 0x1b
                && i + 1 < bytes.len()
                && bytes[i + 1] == b'['
                && let Some(rel) = bytes[i + 2..].iter().position(|&b| b == b'H')
            {
                let seg = String::from_utf8_lossy(&bytes[i + 2..i + 2 + rel]).into_owned();
                let clean = strip_sgr(&seg);
                if let Some((r, c)) = clean.split_once(';')
                    && let (Ok(row), Ok(col)) = (r.trim().parse::<u16>(), c.trim().parse::<u16>())
                {
                    // move_to emits (y+1, x+1); validate within the screen.
                    if row == 0 || row > h || col == 0 || col > w {
                        bad.push((row, col));
                    }
                }
                i += 2 + rel + 1;
                continue;
            }
            i += 1;
        }
        bad
    }

    #[test]
    fn render_with_new_maximized_terminal_stays_in_bounds() {
        let w: u16 = 100;
        let h: u16 = 30;
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Maximized terminal (covers the usable area) + a normal terminal.
        let mut applications = vec![
            Application::terminal_window(
                "Terminal 1",
                Window::new(2, 1, w - 5, h - 4, 0),
                cwd.clone(),
                Vec::new(),
            ),
            Application::terminal_window(
                "Terminal 2",
                Window::new(10, 4, 50, 18, 0),
                cwd.clone(),
                Vec::new(),
            ),
        ];

        // Fill the session with many lines to exercise scrollbar/scroll.
        assert!(applications[0].terminal.as_ref().unwrap().has_session());
        for i in 0..500 {
            applications[0]
                .terminal
                .as_mut()
                .unwrap()
                .push_shell_line(format!(
                    "linha de saída do shell número {i} com conteúdo acentuado çã"
                ));
        }
        applications[0].terminal.as_mut().unwrap().panel_scroll = 120;
        applications[1]
            .terminal
            .as_mut()
            .unwrap()
            .push_shell_line("poucas linhas".to_string());

        let pointer = Pointer::new(20, 10);
        let focused_term = Some((0, "", 0));
        let mut buf = Vec::new();
        let mut grid = crate::ui::screen::ScreenGrid::new(w, h);
        render(
            &mut buf,
            &mut Frame {
                applications: &applications,
                resize_preview: None,
                cursor_interaction: None,
                w,
                h,
                pointer: &pointer,
                scroll_offset: 0,
                tab_scroll: 0,
                path: "",
                typing_input: None,
                commands: &[],
                panel_scroll: 0,
                current_desktop: 1,
                focused_terminal: focused_term,
                grid: &mut grid,
                selection: None,
                theme: 1,
                full_redraw: false,
                draw_pointer: true,
            },
        );

        let bad = out_of_bounds_moves(&buf, w, h);
        assert!(bad.is_empty(), "render wrote out of bounds: {bad:?}");
    }

    #[test]
    fn render_with_terminal_near_bottom_stays_in_bounds() {
        let w: u16 = 80;
        let h: u16 = 24;
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();
        // Terminal window flush with the bottom (status bar at h-4..h-1).
        let applications = vec![Application::terminal_window(
            "Terminal bottom",
            Window::new(2, h - 8, 40, 6, 0),
            cwd.clone(),
            Vec::new(),
        )];
        assert!(applications[0].terminal.as_ref().unwrap().has_session());
        let pointer = Pointer::new(20, 10);
        let mut buf = Vec::new();
        let mut grid = crate::ui::screen::ScreenGrid::new(w, h);
        render(
            &mut buf,
            &mut Frame {
                applications: &applications,
                resize_preview: None,
                cursor_interaction: None,
                w,
                h,
                pointer: &pointer,
                scroll_offset: 0,
                tab_scroll: 0,
                path: "",
                typing_input: None,
                commands: &[],
                panel_scroll: 0,
                current_desktop: 1,
                focused_terminal: None,
                grid: &mut grid,
                selection: None,
                theme: 1,
                full_redraw: false,
                draw_pointer: true,
            },
        );
        let bad = out_of_bounds_moves(&buf, w, h);
        assert!(bad.is_empty(), "render wrote out of bounds: {bad:?}");
    }

    #[test]
    fn render_interactive_emulator_stays_in_bounds() {
        use crate::app::DisplayMode;
        use crate::app::terminal::TerminalState;
        use crate::terminal_emulator::Terminal;

        let w: u16 = 100;
        let h: u16 = 30;
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // Build an interactive terminal without spawning a real session.
        let mut ts = TerminalState::new(cwd, Vec::new());
        ts.interactive = true;
        let mut em = Terminal::new(80, 24);
        for i in 0..300 {
            em.process(format!("linha de saída {i} com conteúdo acentuado çãẽ\r\n").as_bytes());
        }
        ts.emulator = Some(em);
        ts.panel_scroll = 12;

        let applications = vec![Application {
            title: "App".to_string(),
            display: DisplayMode::Windowed(Window::new(2, 2, 80, 24, 0)),
            desktop: 1,
            is_menu: false,
            terminal: Some(ts),
            menu: None,
            help: None,
        }];

        let pointer = Pointer::new(20, 10);
        let mut buf = Vec::new();
        let mut grid = crate::ui::screen::ScreenGrid::new(w, h);
        render(
            &mut buf,
            &mut Frame {
                applications: &applications,
                resize_preview: None,
                cursor_interaction: None,
                w,
                h,
                pointer: &pointer,
                scroll_offset: 0,
                tab_scroll: 0,
                path: "",
                typing_input: None,
                commands: &[],
                panel_scroll: 0,
                current_desktop: 1,
                focused_terminal: Some((0, "", 0)),
                grid: &mut grid,
                selection: None,
                theme: 1,
                full_redraw: false,
                draw_pointer: true,
            },
        );
        let bad = out_of_bounds_moves(&buf, w, h);
        assert!(bad.is_empty(), "render wrote out of bounds: {bad:?}");
    }

    #[test]
    fn render_help_window_stays_in_bounds() {
        let w: u16 = 100;
        let h: u16 = 30;
        let cwd = std::env::current_dir()
            .unwrap()
            .to_string_lossy()
            .to_string();

        // A help window next to terminals: its wrapped crib sheet must stay
        // inside the window and the screen.
        let applications = vec![
            Application::help_window("Help", Window::new(12, 4, 80, 22, 2)),
            Application::terminal_window("T", Window::new(2, 2, 30, 8, 0), cwd.clone(), Vec::new()),
        ];

        let pointer = Pointer::new(20, 10);
        let mut buf = Vec::new();
        let mut grid = crate::ui::screen::ScreenGrid::new(w, h);
        render(
            &mut buf,
            &mut Frame {
                applications: &applications,
                resize_preview: None,
                cursor_interaction: None,
                w,
                h,
                pointer: &pointer,
                scroll_offset: 0,
                tab_scroll: 0,
                path: "",
                typing_input: None,
                commands: &[],
                panel_scroll: 0,
                current_desktop: 1,
                focused_terminal: None,
                grid: &mut grid,
                selection: None,
                theme: 1,
                full_redraw: false,
                draw_pointer: true,
            },
        );
        let bad = out_of_bounds_moves(&buf, w, h);
        assert!(bad.is_empty(), "render wrote out of bounds: {bad:?}");
    }

    #[test]
    fn emit_diff_rewrites_only_changed_rows() {
        use super::emit_frame_diff;
        let mut prev = crate::ui::screen::ScreenGrid::new(5, 3);
        let mut next = crate::ui::screen::ScreenGrid::new(5, 3);
        for y in 0..3 {
            for x in 0..5 {
                let ch = if (x, y) == (2, 1) { 'Z' } else { 'a' };
                prev.set_cell(x, y, 'a');
                next.set_cell(x, y, ch);
            }
        }

        let mut buf = Vec::new();
        emit_frame_diff(&mut buf, &prev, &next, false);
        let s = String::from_utf8_lossy(&buf);

        assert!(
            s.contains("\u{1b}[2;1H"),
            "changed row must be repositioned"
        );
        assert!(s.contains('Z'), "changed cell must be emitted");
        assert!(
            !s.contains("\u{1b}[2J"),
            "no full-screen clear on a normal frame"
        );
        // Only the changed row is written: 4 identical cells + the 'Z'.
        assert_eq!(s.matches('a').count(), 4, "unchanged rows must be skipped");
    }

    #[test]
    fn emit_diff_clears_on_full_redraw_request() {
        use super::emit_frame_diff;
        let prev = crate::ui::screen::ScreenGrid::new(5, 3);
        let next = crate::ui::screen::ScreenGrid::new(5, 3);
        let mut buf = Vec::new();
        emit_frame_diff(&mut buf, &prev, &next, true);
        assert!(String::from_utf8_lossy(&buf).contains("\u{1b}[2J"));
    }

    #[test]
    fn emit_diff_repaints_style_change_on_same_char() {
        // Same character in both frames, but one cell gains a REVERSE style:
        // the row must still be re-emitted (hovers are attribute-only).
        use super::emit_frame_diff;
        use crate::terminal_emulator::{Attributes, Cell};

        let mut prev = crate::ui::screen::ScreenGrid::new(3, 2);
        let mut next = crate::ui::screen::ScreenGrid::new(3, 2);
        for y in 0..2 {
            for x in 0..3 {
                prev.set_cell(x, y, 'x');
                next.set_cell(x, y, 'x');
            }
        }
        let mut highlighted = Cell {
            ch: 'x',
            ..Cell::default()
        };
        highlighted.style.attrs.set(Attributes::REVERSE, true);
        next.put(1, 0, highlighted); // (x=1, row 0) gains reverse

        let mut buf = Vec::new();
        emit_frame_diff(&mut buf, &prev, &next, false);
        let s = String::from_utf8_lossy(&buf);
        assert!(
            s.contains("\u{1b}[1;1H"),
            "row with a style toggle is emitted"
        );
        assert!(s.contains("\u{1b}[7m"), "flagged cell is rendered inverted");
        assert!(
            !s.contains("\u{1b}[2J"),
            "no full clear for an attribute-only change"
        );
    }
}
