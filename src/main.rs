mod application;
mod gui;
mod os;
mod pointer;
mod terminal;
mod window;

use gui::{draw_desktop, draw_tab};
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

/// Calcula (app_idx, tab_y, tab_height) para cada app minimizado.
/// Altura padrão 8; compacta para 4 se não couber.
fn tab_layout(applications: &[Application], screen_h: u16) -> Vec<(usize, u16, u16)> {
    let usable_h = screen_h.saturating_sub(4);
    let minimized: Vec<usize> = applications.iter().enumerate()
        .filter(|(_, a)| a.is_minimized())
        .map(|(i, _)| i)
        .collect();

    if minimized.is_empty() || usable_h == 0 {
        return vec![];
    }

    let tab_h: u16 = if minimized.len() as u16 * 8 <= usable_h { 8 } else { 4 };
    let max_visible = (usable_h / tab_h) as usize;

    minimized.into_iter()
        .take(max_visible)
        .enumerate()
        .map(|(i, app_idx)| (app_idx, 1 + i as u16 * tab_h, tab_h))
        .collect()
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
) {
    terminal::clear(out);
    draw_desktop(out, 1, w, h, "Manto");

    for app in applications {
        if let Some(win) = app.window() {
            win.draw(out, &app.title);
        }
    }

    // Abas na direita para apps minimizados
    let tab_x = w.saturating_sub(3);
    for (app_idx, tab_y, tab_h) in tab_layout(applications, h) {
        draw_tab(out, tab_x, tab_y, tab_h, &applications[app_idx].title, scroll_offset);
    }

    if let Some((idx, pw, ph)) = resize_preview {
        if let Some(win) = applications[idx].window() {
            win.draw_preview(out, pw, ph);
        }
    }

    pointer.draw(out, cursor_interaction);

    // Hover nos cantos superiores da janela no topo: x (fechar) e - (minimizar)
    if let Some(top_idx) = topmost_window_at(applications, pointer.x, pointer.y) {
        if let Some(win) = applications[top_idx].window() {
            if pointer.y == win.position_y {
                if pointer.x == win.position_x + win.width - 1 {
                    terminal::move_to(out, pointer.x, pointer.y);
                    write!(out, "{}x{}", terminal::REVERSE, terminal::RESET).unwrap();
                } else if pointer.x == win.position_x {
                    terminal::move_to(out, pointer.x, pointer.y);
                    write!(out, "{}-{}", terminal::REVERSE, terminal::RESET).unwrap();
                }
            }
        }
    }

    out.flush().unwrap();
}

fn main() {
    let mut out = Writer::new();

    os::enable_raw_mode();
    terminal::enter_alt_screen(&mut out);
    terminal::hide_cursor(&mut out);
    out.flush().unwrap();

    let mut mode         = Mode::Normal;
    let mut scroll_offset: usize = 0;
    let mut last_size    = os::size();
    let mut pointer      = Pointer::new(3, last_size.1 - 2);

    let mut applications = vec![
        Application::windowed("Test",  Window::new(2,  1, 17, 8, 0)),
        Application::windowed("Test2", Window::new(10, 1, 17, 8, 0)),
    ];

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset);

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
                            let tab_x = last_size.0.saturating_sub(2);

                            // Aba no lado direito → restaura app minimizado
                            let on_tab = if pointer.x >= tab_x {
                                tab_layout(&applications, last_size.1)
                                    .into_iter()
                                    .find(|&(_, ty, th)| pointer.y >= ty && pointer.y < ty + th)
                                    .map(|(idx, _, _)| idx)
                            } else {
                                None
                            };

                            if let Some(app_idx) = on_tab {
                                applications[app_idx].restore();
                                mode_changed = true;
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

                                if is_minimize {
                                    applications[top_idx].minimize();
                                    mode_changed = true;
                                } else if is_close {
                                    applications.remove(top_idx);
                                    mode_changed = true;
                                } else if is_resize {
                                    mode = Mode::Resizing { app_idx: top_idx };
                                    mode_changed = true;
                                } else if is_title {
                                    let final_idx = if top_idx != applications.len() - 1 {
                                        let app = applications.remove(top_idx);
                                        applications.push(app);
                                        applications.len() - 1
                                    } else {
                                        top_idx
                                    };
                                    mode = Mode::Moving { app_idx: final_idx, offset_x };
                                    mode_changed = true;
                                } else if top_idx != applications.len() - 1 {
                                    let app = applications.remove(top_idx);
                                    applications.push(app);
                                    mode_changed = true;
                                }
                            }
                        }
                        _ => {}
                    },

                    Mode::Moving { .. } => match key {
                        Key::Up    => pointer.move_up(),
                        Key::Down  => pointer.move_down(last_size.1),
                        Key::Left  => pointer.move_left(),
                        Key::Right => pointer.move_right(last_size.0),
                        Key::Char(' ') => {
                            mode = Mode::Normal;
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
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset);
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
            }
            // Re-renderiza só se o tamanho mudou ou se há aba com título a rolar
            let needs_scroll = tab_layout(&applications, last_size.1).iter().any(|&(idx, _, tab_h)| {
                applications[idx].title.chars().count() > tab_h.saturating_sub(2) as usize
            });
            if size_changed || needs_scroll {
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer, scroll_offset);
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
