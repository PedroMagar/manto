mod application;
mod gui;
mod os;
mod pointer;
mod terminal;
mod window;

use gui::{draw_desktop, draw_tab, draw_scrollbar};
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

fn window_border_char(win: &Window, title: &str, x: u16, y: u16) -> char {
    let lx = win.position_x;
    let rx = win.position_x + win.width - 1;
    let ty = win.position_y;
    let by = win.position_y + win.height - 1;
    if y == ty {
        if x == lx { return '-'; }
        if x == rx { return 'x'; }
        let bar = format!("{:─^1$}", format!(" {} ", title), (win.width - 2) as usize);
        return bar.chars().nth((x - lx - 1) as usize).unwrap_or('─');
    }
    if y == by {
        if x == lx { return '└'; }
        if x == rx { return '┘'; }
        return '─';
    }
    '│'
}

fn tab_cell_char(tab_x: u16, tab_y: u16, tab_h: u16, title: &str, x: u16, y: u16, scroll_offset: usize) -> char {
    let content_rows = tab_h.saturating_sub(2) as usize;
    let padded = if title.chars().count() > content_rows {
        format!("{}  ", title)
    } else {
        title.to_string()
    };
    let chars: Vec<char> = padded.chars().collect();
    let len = chars.len();
    if y == tab_y || y == tab_y + tab_h - 1 {
        return if x == tab_x {
            if y == tab_y { '┌' } else { '└' }
        } else { '─' };
    }
    if x == tab_x { return '│'; }
    let i = (y - tab_y - 1) as usize;
    if len == 0 { ' ' } else if len <= content_rows { chars.get(i).copied().unwrap_or(' ') } else { chars[(scroll_offset + i) % len] }
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
) {
    terminal::clear(out);
    draw_desktop(out, 1, w, h, "Manto");

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

    // Cursor unificado: ░ por padrão, negativo da célula clicável/borda em reverse video
    let effective_cursor = cursor_interaction.or_else(|| {
        let px = pointer.x;
        let py = pointer.y;

        // Scrollbar
        if minimized_count > tabs.len() && px == sb_x && py >= sb_top && py <= sb_bot {
            let track_len = (sb_bot - sb_top + 1) as usize;
            let visible = tabs.len();
            let max_sc = minimized_count - visible;
            let thumb_len = (((visible as f32 / minimized_count as f32) * track_len as f32)
                .max(1.0) as usize)
                .min(track_len);
            let available = track_len - thumb_len;
            let thumb_pos = if max_sc > 0 { (tab_scroll * available / max_sc).min(available) } else { 0 };
            let row = (py - sb_top) as usize;
            return Some(if row >= thumb_pos && row < thumb_pos + thumb_len { '█' } else { '░' });
        }

        // Tab strip
        if px >= tab_x && px < sb_x {
            if let Some(&(app_idx, tab_y, tab_h)) = tabs.iter()
                .find(|&&(_, ty, th)| py >= ty && py < ty + th)
            {
                return Some(tab_cell_char(
                    tab_x, tab_y, tab_h,
                    &applications[app_idx].title,
                    px, py, scroll_offset,
                ));
            }
        }

        // Borda da janela no topo
        if let Some(top_idx) = topmost_window_at(applications, px, py) {
            if let Some(win) = applications[top_idx].window() {
                let lx = win.position_x;
                let rx = win.position_x + win.width - 1;
                let ty = win.position_y;
                let by = win.position_y + win.height - 1;
                let on_border = ((py == ty || py == by) && px >= lx && px <= rx)
                    || ((px == lx || px == rx) && py > ty && py < by);
                if on_border {
                    return Some(window_border_char(win, &applications[top_idx].title, px, py));
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
    let mut last_size     = os::size();
    let mut pointer       = Pointer::new(3, last_size.1 - 2);

    let mut applications = vec![
        Application::windowed("Test",    Window::new(2,  1, 17, 8, 0)),
        Application::windowed("Test2",   Window::new(22, 1, 17, 8, 0)),
        Application::windowed("Test3",   Window::new(2,  11, 17, 8, 0)),
        Application::windowed("Test4",   Window::new(22, 11, 17, 8, 0)),
    ];

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll);

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

                            // Scrollbar (coluna mais à direita): metade superior sobe, inferior desce
                            if pointer.x == sb_x {
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
                                let (is_minimize, is_close, is_resize, is_title, offset_x) = {
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
                                    )
                                };
                                let maximized = applications[top_idx].is_maximized();

                                if is_minimize {
                                    applications[top_idx].minimize();
                                    mode_changed = true;
                                } else if is_close {
                                    applications.remove(top_idx);
                                    tab_scroll = tab_scroll
                                        .min(max_tab_scroll(&applications, last_size.1));
                                    mode_changed = true;
                                } else if is_resize && !maximized {
                                    mode = Mode::Resizing { app_idx: top_idx };
                                    mode_changed = true;
                                } else if is_title {
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
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll);
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
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset, tab_scroll);
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
