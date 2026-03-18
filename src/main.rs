mod application;
mod gui;
mod os;
mod pointer;
mod terminal;
mod window;

use gui::{draw_desktop, draw_button};
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

fn render(
    out: &mut Writer,
    applications: &[Application],
    button_hovered: bool,
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

    let button_label = "[ Clique-me ]";
    let button_x = (w.saturating_sub(button_label.len() as u16)) / 2;
    let button_y = h / 2;
    draw_button(out, button_x, button_y, button_label, button_hovered);

    if let Some((idx, pw, ph)) = resize_preview {
        if let Some(win) = applications[idx].window() {
            win.draw_preview(out, pw, ph);
        }
    }

    pointer.draw(out, cursor_interaction);
    out.flush().unwrap();
}

fn main() {
    let mut out = Writer::new();

    os::enable_raw_mode();
    terminal::enter_alt_screen(&mut out);
    terminal::hide_cursor(&mut out);
    out.flush().unwrap();

    let mut mode    = Mode::Normal;
    let mut hovered = false;
    let mut last_size = os::size();
    let mut pointer   = Pointer::new(3, last_size.1 - 2);

    let mut applications = vec![
        Application::windowed("Test",  Window::new(2,  1, 17, 8, 0)),
        Application::windowed("Test2", Window::new(10, 1, 17, 8, 0)),
    ];

    let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
    render(&mut out, &applications, hovered, preview, cursor, last_size.0, last_size.1, &pointer);

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
                        Key::Enter if hovered => {
                            terminal::move_to(&mut out, 2, last_size.1.saturating_sub(2));
                            write!(out, "{}Você clicou no botão!{}", terminal::FG_GREEN, terminal::RESET).unwrap();
                            out.flush().unwrap();
                        }
                        // Space sobre uma janela: quina → resize, título → mover, resto → traz para frente
                        Key::Char(' ') => {
                            if let Some(idx) = applications.iter().position(|app| {
                                app.window().map_or(false, |win| {
                                    pointer.x == win.position_x + win.width - 1
                                        && pointer.y == win.position_y + win.height - 1
                                })
                            }) {
                                mode = Mode::Resizing { app_idx: idx };
                                mode_changed = true;
                            } else if let Some(idx) = applications.iter().rposition(|app| {
                                app.window().map_or(false, |win| {
                                    pointer.y == win.position_y
                                        && pointer.x > win.position_x
                                        && pointer.x < win.position_x + win.width - 1
                                })
                            }) {
                                let offset_x = pointer.x
                                    .saturating_sub(applications[idx].window().unwrap().position_x);
                                let final_idx = if idx != applications.len() - 1 {
                                    let app = applications.remove(idx);
                                    applications.push(app);
                                    applications.len() - 1
                                } else {
                                    idx
                                };
                                mode = Mode::Moving { app_idx: final_idx, offset_x };
                                mode_changed = true;
                            } else if let Some(idx) = applications.iter().rposition(|app| {
                                app.window().map_or(false, |win| {
                                    pointer.x >= win.position_x
                                        && pointer.x < win.position_x + win.width
                                        && pointer.y >= win.position_y
                                        && pointer.y < win.position_y + win.height
                                })
                            }) {
                                if idx != applications.len() - 1 {
                                    let app = applications.remove(idx);
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
                let button_label = "[ Clique-me ]";
                let button_x = (last_size.0.saturating_sub(button_label.len() as u16)) / 2;
                let button_y = last_size.1 / 2;
                hovered = pointer.y == button_y
                    && pointer.x >= button_x
                    && pointer.x < button_x + button_label.len() as u16;

                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                render(&mut out, &applications, hovered, preview, cursor, last_size.0, last_size.1, &pointer);
            }
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            let new_size = os::size();
            if new_size != last_size {
                pointer.y = new_size.1 - (last_size.1 - pointer.y);
                last_size = new_size;
                pointer.clamp_to_bounds(last_size.0, last_size.1);
                let (preview, cursor) = compute_render_state(&mode, &applications, &pointer);
                render(&mut out, &applications, hovered, preview, cursor, last_size.0, last_size.1, &pointer);
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
        Mode::Normal => (None, None),
    }
}
