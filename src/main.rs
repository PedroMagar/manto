mod application;
mod gui;
mod pointer;

use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    style::{self, Stylize},
    terminal::{self, ClearType},
};
use gui::{draw_desktop, draw_window, draw_button};
pub use application::Application;
use pointer::Pointer;
use std::io::{stdout, Write};
use std::time::{Duration, Instant};

fn render(stdout: &mut std::io::Stdout, applications: &Vec<Application>, hovered: bool, w: u16, h: u16, pointer: &Pointer) {
    execute!(stdout, terminal::Clear(ClearType::All)).unwrap();

    draw_desktop(stdout, 1, w, h, "Manto");

    for app in applications {
        draw_window(stdout, app);
    }
    let button_label = "[ Clique-me ]";
    let button_x = (w.saturating_sub(button_label.len() as u16)) / 2;
    let button_y = h / 2;
    draw_button(stdout, button_x, button_y, button_label, hovered);

    pointer.draw(stdout);
    stdout.flush().unwrap();
}

fn main() {
    let mut stdout = stdout();
    terminal::enable_raw_mode().unwrap();
    execute!(stdout, terminal::EnterAlternateScreen, cursor::Hide).unwrap();

    let mut hovered = false;
    let mut last_size = terminal::size().unwrap();
    let mut pointer = Pointer::new(3, last_size.1 - 2);
    let app = Application {
        title: String::from("Test"),
        width: 17,
        height: 8,
        position_x: 2,
        position_y: 1,
        layer: 0,
    };
    let app2 = Application {
        title: String::from("Test2"),
        width: 17,
        height: 8,
        position_x: 10,
        position_y: 1,
        layer: 0,
    };
    let applications = vec![app, app2];
    render(&mut stdout, &applications, hovered, last_size.0, last_size.1, &pointer);

    let mut last_check = Instant::now();

    loop {
        if event::poll(Duration::from_millis(50)).unwrap() {
            match event::read().unwrap() {
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    match (key_event.code, key_event.modifiers) {
                        (KeyCode::Char('q'), _) => break,
                        (KeyCode::Char('c'), KeyModifiers::CONTROL) => break,
                        (KeyCode::Up, _) => pointer.move_up(),
                        (KeyCode::Down, _) => pointer.move_down(last_size.1),
                        (KeyCode::Left, _) => pointer.move_left(),
                        (KeyCode::Right, _) => pointer.move_right(last_size.0),
                        (KeyCode::Enter, _) if hovered => {
                            execute!(
                                stdout,
                                cursor::MoveTo(2, last_size.1.saturating_sub(2)),
                                style::Print("Você clicou no botão!".green())
                            ).unwrap();
                            stdout.flush().unwrap();
                        }
                        _ => {}
                    }

                    let button_label = "[ Clique-me ]";
                    let button_x = (last_size.0.saturating_sub(button_label.len() as u16)) / 2;
                    let button_y = last_size.1 / 2;

                    hovered = pointer.y == button_y
                        && pointer.x >= button_x
                        && pointer.x < button_x + button_label.len() as u16;

                    render(&mut stdout, &applications, hovered, last_size.0, last_size.1, &pointer);
                }
                _ => {}
            }
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            let new_size = terminal::size().unwrap();
            if new_size != last_size {
                pointer.y = new_size.1 - (last_size.1 - pointer.y);
                last_size = new_size;
                pointer.clamp_to_bounds(last_size.0, last_size.1);
                render(&mut stdout, &applications, hovered, new_size.0, new_size.1, &pointer);
            }
            last_check = Instant::now();
        }
    }

    execute!(stdout, terminal::LeaveAlternateScreen, cursor::Show).unwrap();
    terminal::disable_raw_mode().unwrap();
}

