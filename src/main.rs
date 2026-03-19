mod application;
mod gui;
mod os;
mod pointer;
mod terminal;
mod window;

use gui::{draw_desktop, draw_tab, draw_scrollbar, tab_char_at, scrollbar_thumb, STATUS_BAR_PREFIX, STATUS_START, STATUS_START_X};
pub use application::Application;
use window::{Window, MIN_W, MIN_H};
use os::{Writer, Clock, Key};
use pointer::Pointer;
use std::io::Write;
use std::time::Duration;

enum Mode {
    Normal,
    Moving  { app_idx: usize, offset_x: u16 },
    Resizing { app_idx: usize },
}

/// Retorna o índice da janela visualmente no topo na posição (x, y).
fn topmost_window_at(applications: &[Application], x: u16, y: u16) -> Option<usize> {
    applications.iter().rposition(|app| {
        app.window().map_or(false, |win| {
            x >= win.position_x
                && x < win.position_x + win.width
                && y >= win.position_y
                && y < win.position_y + win.height
        })
    })
}

/// Calcula (app_idx, tab_y, tab_height) para cada app minimizado visível.
fn tab_layout(applications: &[Application], screen_h: u16, scroll: usize) -> Vec<(usize, u16, u16)> {
    let usable_h = screen_h.saturating_sub(4);
    let minimized: Vec<usize> = applications.iter().enumerate()
        .filter(|(_, a)| a.is_minimized())
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
fn max_tab_scroll(applications: &[Application], screen_h: u16) -> usize {
    let usable_h = screen_h.saturating_sub(4);
    let total = applications.iter().filter(|a| a.is_minimized()).count();
    if total == 0 || usable_h == 0 { return 0; }
    let tab_h: u16 = if (total as u16) * 8 <= usable_h { 8 } else { 6 };
    let max_visible = (usable_h / tab_h) as usize;
    total.saturating_sub(max_visible)
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
) {
    terminal::clear(out);
    draw_desktop(out, 1, w, h, "Manto", path);

    for app in applications {
        if let Some(win) = app.window() {
            win.draw(out, &app.title);
        }
    }

    // Abas e scrollbar
    let minimized_count = applications.iter().filter(|a| a.is_minimized()).count();
    let tab_x = w.saturating_sub(3);
    let sb_x  = w.saturating_sub(1);
    let sb_top = 1u16;
    let sb_bot = h.saturating_sub(4);
    let tabs = tab_layout(applications, h, tab_scroll);
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
        if let Some(win) = applications[idx].window() {
            win.draw_preview(out, pw, ph);
        }
    }

    // Hover no botão Start: inverte o bloco inteiro quando o cursor está sobre ele
    let start_end = STATUS_START_X + STATUS_START.len() as u16;
    if pointer.y == h - 2 && pointer.x >= STATUS_START_X && pointer.x < start_end {
        terminal::move_to(out, STATUS_START_X, h - 2);
        write!(out, "{}{}{}", terminal::REVERSE, STATUS_START, terminal::RESET).unwrap();
    }

    // Cursor unificado: ░ por padrão, negativo da célula clicável/borda em reverse video
    let effective_cursor = cursor_interaction.or_else(|| {
        let px = pointer.x;
        let py = pointer.y;

        // Scrollbar
        if minimized_count > tabs.len() && px == sb_x && py >= sb_top && py <= sb_bot {
            let track_len = (sb_bot - sb_top + 1) as usize;
            let (thumb_pos, thumb_len) = scrollbar_thumb(track_len, minimized_count, tabs.len(), tab_scroll);
            let row = (py - sb_top) as usize;
            return Some(if row >= thumb_pos && row < thumb_pos + thumb_len { '█' } else { '░' });
        }

        // Tab strip
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

        // Botão Start na barra de status
        let start_end = STATUS_START_X + STATUS_START.len() as u16;
        if py == h - 2 && px >= STATUS_START_X && px < start_end {
            return Some(STATUS_START.chars().nth((px - STATUS_START_X) as usize).unwrap_or(' '));
        }

        // Borda da janela no topo
        if let Some(top_idx) = topmost_window_at(applications, px, py) {
            if let Some(win) = applications[top_idx].window() {
                if let Some(ch) = win.char_at(px, py, &applications[top_idx].title) {
                    return Some(ch);
                }
            }
        }

        None
    });
    pointer.draw(out, effective_cursor);

    out.flush().unwrap();
}

fn main() {
    let mut out = Writer::new();

    os::enable_raw_mode();
    terminal::enter_alt_screen(&mut out);
    terminal::hide_cursor(&mut out);
    out.flush().unwrap();

    let mut mode             = Mode::Normal;
    let mut scroll_offset: usize = 0;
    let mut tab_scroll:    usize = 0;
    let mut last_space_time: Option<Clock> = None;
    let current_path         = String::new();
    let mut last_size     = os::size();
    let mut pointer       = Pointer::new(1 + STATUS_BAR_PREFIX.len() as u16, last_size.1 - 2);

    let mut applications = vec![
        // Conteúdo cabe na janela — sem scrollbars
        Application::windowed("Test",  Window::new(2,  1,  17, 8, 0)),
        // Conteúdo mais alto — scrollbar vertical
        Application::windowed("Test2", Window::new(22, 1,  17, 8, 0).with_content(0,  20)),
        // Conteúdo mais largo — scrollbar horizontal
        Application::windowed("Test3", Window::new(2,  11, 17, 8, 0).with_content(40, 0)),
        // Conteúdo maior nas duas direções — ambas as scrollbars
        Application::windowed("Test4", Window::new(22, 11, 17, 8, 0).with_content(40, 20)),
    ];

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, &current_path);

    let mut last_check = Clock::now();

    loop {
        if os::poll(50) {
            let key  = os::read_key();
            let prev = (pointer.x, pointer.y);
            let mut mode_changed = false;

            match key {
                Key::Char('q') | Key::CtrlC => break,

                _ => match &mut mode {
                    Mode::Normal => match key {
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),

                        Key::Char(' ') => {
                            let sb_x   = last_size.0.saturating_sub(1);
                            let sb_top = 1u16;
                            let sb_bot = last_size.1.saturating_sub(4);
                            let tab_x  = last_size.0.saturating_sub(3);

                            // Botão Start: toggle do menu
                            let start_end = STATUS_START_X + STATUS_START.len() as u16;
                            if pointer.y == last_size.1 - 2
                                && pointer.x >= STATUS_START_X
                                && pointer.x < start_end
                            {
                                if let Some(idx) = applications.iter().position(|a| a.is_menu) {
                                    applications.remove(idx);
                                } else {
                                    let usable_h = last_size.1.saturating_sub(4);
                                    let win_h = (usable_h * 3 / 4).max(MIN_H);
                                    let pos_y  = last_size.1.saturating_sub(3).saturating_sub(win_h);
                                    applications.push(Application::menu(
                                        "Start",
                                        Window::new(2, pos_y, 20, win_h, 0).without_chrome(),
                                    ));
                                }
                                tab_scroll = tab_scroll.min(max_tab_scroll(&applications, last_size.1));
                                mode_changed = true;
                            // Scrollbar (coluna mais à direita): metade superior sobe, inferior desce
                            } else if pointer.x == sb_x {
                                last_space_time = None;
                                let mid = (sb_top + sb_bot) / 2;
                                if pointer.y <= mid {
                                    tab_scroll = tab_scroll.saturating_sub(1);
                                } else {
                                    tab_scroll = (tab_scroll + 1)
                                        .min(max_tab_scroll(&applications, last_size.1));
                                }
                                mode_changed = true;
                            // Aba → restaura app minimizado
                            } else if pointer.x >= tab_x {
                                last_space_time = None;
                                let on_tab = tab_layout(&applications, last_size.1, tab_scroll)
                                    .into_iter()
                                    .find(|&(_, ty, th)| pointer.y >= ty && pointer.y < ty + th)
                                    .map(|(idx, _, _)| idx);

                                if let Some(app_idx) = on_tab {
                                    applications[app_idx].restore();
                                    tab_scroll = tab_scroll
                                        .min(max_tab_scroll(&applications, last_size.1));
                                    mode_changed = true;
                                }
                            // Janela
                            } else if let Some(top_idx) =
                                topmost_window_at(&applications, pointer.x, pointer.y)
                            {
                                // Fecha menu se a ação foi fora dele
                                let mut skip = false;
                                if let Some(menu_idx) = applications.iter().position(|a| a.is_menu) {
                                    if top_idx != menu_idx {
                                        applications.remove(menu_idx);
                                        tab_scroll = tab_scroll
                                            .min(max_tab_scroll(&applications, last_size.1));
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

                                    if !scroll_handled {
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
                                            .min(max_tab_scroll(&applications, last_size.1));
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
                                    } // if !scroll_handled
                                } // if !skip
                            }
                        }
                        _ => {}
                    },

                    Mode::Moving { app_idx, .. } => match key {
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),
                        Key::Char(' ') => {
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
                        Key::Char(' ') => {
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
                },
            }

            // Em modo Normal: coluna do scrollbar acessível só quando há scroll;
            // quando acessível, o ponteiro fica limitado à faixa vertical da scrollbar.
            if matches!(&mode, Mode::Normal) {
                let sb_x = last_size.0.saturating_sub(1);
                if pointer.x == sb_x {
                    let minimized_count = applications.iter().filter(|a| a.is_minimized()).count();
                    let tab_count = tab_layout(&applications, last_size.1, tab_scroll).len();
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
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, &current_path);
            }
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            scroll_offset = scroll_offset.wrapping_add(1);
            let new_size = os::size();
            let size_changed = new_size != last_size;
            if size_changed {
                pointer.y = new_size.1 - (last_size.1 - pointer.y);
                last_size = new_size;
                pointer.clamp_to_bounds(last_size.0, last_size.1);
                tab_scroll = tab_scroll.min(max_tab_scroll(&applications, last_size.1));
            }
            // Só anima o título da aba sob o cursor
            let tab_x = last_size.0.saturating_sub(3);
            let needs_scroll = tab_layout(&applications, last_size.1, tab_scroll)
                .iter()
                .any(|&(idx, tab_y, tab_h)| {
                    let is_hovered = pointer.x >= tab_x
                        && pointer.y >= tab_y
                        && pointer.y < tab_y + tab_h;
                    is_hovered
                        && applications[idx].title.chars().count() > tab_h.saturating_sub(2) as usize
                });
            if size_changed || needs_scroll {
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll, &current_path);
            }
            last_check = Clock::now();
        }
    }

    terminal::leave_alt_screen(&mut out);
    terminal::show_cursor(&mut out);
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
        Mode::Moving { .. } => (None, None),
        Mode::Normal      => (None, None),
    }
}
