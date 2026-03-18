mod application;
mod gui;
mod os;
mod pointer;
mod terminal;

use gui::{draw_desktop, draw_window, draw_button};
pub use application::Application;
use os::{Writer, Clock, Key};
use pointer::Pointer;
use std::io::Write;
use std::time::Duration;

fn render(out: &mut Writer, applications: &Vec<Application>, hovered: bool, w: u16, h: u16, pointer: &Pointer) {
    terminal::clear(out);

    draw_desktop(out, 1, w, h, "Manto");

    for app in applications {
        draw_window(out, app);
    }

    let button_label = "[ Clique-me ]";
    let button_x = (w.saturating_sub(button_label.len() as u16)) / 2;
    let button_y = h / 2;
    draw_button(out, button_x, button_y, button_label, hovered);

    pointer.draw(out);
    out.flush().unwrap();
}

fn main() {
    let mut out = Writer::new();

    os::enable_raw_mode();
    terminal::enter_alt_screen(&mut out);
    terminal::hide_cursor(&mut out);
    out.flush().unwrap();

    let mut hovered   = false;
    let mut last_size = os::size();
    let mut pointer   = Pointer::new(3, last_size.1 - 2);

    let app = Application {
        title:      String::from("Test"),
        width:      17,
        height:     8,
        position_x: 2,
        position_y: 1,
        layer:      0,
    };
    let app2 = Application {
        title:      String::from("Test2"),
        width:      17,
        height:     8,
        position_x: 10,
        position_y: 1,
        layer:      0,
    };
    let applications = vec![app, app2];

    render(&mut out, &applications, hovered, last_size.0, last_size.1, &pointer);

    let mut last_check = Clock::now();

    loop {
        if os::poll(50) {
            match os::read_key() {
                Key::Char('q') | Key::CtrlC => break,
                Key::Up    => pointer.move_up(),
                Key::Down  => pointer.move_down(last_size.1),
                Key::Left  => pointer.move_left(),
                Key::Right => pointer.move_right(last_size.0),
                Key::Enter if hovered => {
                    terminal::move_to(&mut out, 2, last_size.1.saturating_sub(2));
                    write!(out, "{}Você clicou no botão!{}", terminal::FG_GREEN, terminal::RESET).unwrap();
                    out.flush().unwrap();
                }
                _ => {}
            }

            let button_label = "[ Clique-me ]";
            let button_x = (last_size.0.saturating_sub(button_label.len() as u16)) / 2;
            let button_y = last_size.1 / 2;

            hovered = pointer.y == button_y
                && pointer.x >= button_x
                && pointer.x < button_x + button_label.len() as u16;

            render(&mut out, &applications, hovered, last_size.0, last_size.1, &pointer);
        }

        if last_check.elapsed() >= Duration::from_secs(1) {
            let new_size = os::size();
            if new_size != last_size {
                pointer.y = new_size.1 - (last_size.1 - pointer.y);
                last_size = new_size;
                pointer.clamp_to_bounds(last_size.0, last_size.1);
                render(&mut out, &applications, hovered, new_size.0, new_size.1, &pointer);
            }
            last_check = Clock::now();
        }
    }

    terminal::leave_alt_screen(&mut out);
    terminal::show_cursor(&mut out);
    out.flush().unwrap();
    os::disable_raw_mode();
}
