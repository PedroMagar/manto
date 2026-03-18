mod application;
mod gui;
mod os;
mod pointer;
mod terminal;
mod window;

use gui::draw_desktop;
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

fn render(
    out: &mut Writer,
    applications: &[Application],
    resize_preview: Option<(usize, u16, u16)>,
    cursor_interaction: Option<char>,
    w: u16,
    h: u16,
    pointer: &Pointer,
) {
    terminal::clear(out);
    draw_desktop(out, 1, w, h, "Manto");

    for app in applications {
        if let Some(win) = app.window() {
            win.draw(out, &app.title);
        }
    }

    if let Some((idx, pw, ph)) = resize_preview {
        if let Some(win) = applications[idx].window() {
            win.draw_preview(out, pw, ph);
        }
    }

    pointer.draw(out, cursor_interaction);

    // Hover nos cantos superiores da janela no topo: x (fechar) e - (minimizar)
    // Só mostra se a janela no topo nessa posição tiver o canto ali
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

    let mut mode    = Mode::Normal;
    let mut last_size = os::size();
    let mut pointer   = Pointer::new(3, last_size.1 - 2);

    let mut applications = vec![
        Application::windowed("Test",  Window::new(2,  1, 17, 8, 0)),
        Application::windowed("Test2", Window::new(10, 1, 17, 8, 0)),
    ];

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer);

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
                        // Space: age apenas sobre a janela visualmente no topo
                        Key::Char(' ') => {
                            if let Some(top_idx) = topmost_window_at(&applications, pointer.x, pointer.y) {
                                let (is_resize, is_title, offset_x) = {
                                    let win = applications[top_idx].window().unwrap();
                                    let resize = pointer.x == win.position_x + win.width - 1
                                        && pointer.y == win.position_y + win.height - 1;
                                    let title = pointer.y == win.position_y
                                        && pointer.x > win.position_x
                                        && pointer.x < win.position_x + win.width - 1;
                                    let ox = pointer.x.saturating_sub(win.position_x);
                                    (resize, title, ox)
                                };

                                if is_resize {
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
                        // Space novamente → confirma o novo tamanho
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
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer);
            }
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            let new_size = os::size();
            if new_size != last_size {
                pointer.y = new_size.1 - (last_size.1 - pointer.y);
                last_size = new_size;
                pointer.clamp_to_bounds(last_size.0, last_size.1);
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                render(&mut out, &applications, preview, cursor, last_size.0, last_size.1, &pointer);
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
