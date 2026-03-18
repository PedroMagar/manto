use std::io::Write;

use crate::terminal;
use crate::Application;

pub fn draw_desktop(out: &mut impl Write, theme: u16, w: u16, h: u16, title: &str) {
    match theme {
        1 => {
            terminal::move_to(out, 0, 0);
            write!(out, "└{:─^1$}┘", format!(" {} ", title), w as usize - 2).unwrap();

            terminal::move_to(out, 0, h - 2);
            write!(out, "│").unwrap();
            terminal::move_to(out, w - 1, h - 2);
            write!(out, "│").unwrap();
        }
        2 => {
            terminal::move_to(out, 0, 0);
            write!(out, "┌{:─^1$}┐", format!(" {} ", title), w as usize - 2).unwrap();

            for i in 1..(h - 1) {
                terminal::move_to(out, 0, i);
                write!(out, "│").unwrap();
                terminal::move_to(out, w - 1, i);
                write!(out, "│").unwrap();
            }
        }
        _ => {}
    }

    terminal::move_to(out, 0, h - 3);
    write!(out, "┌{:─^1$}┐", "", w as usize - 2).unwrap();

    terminal::move_to(out, 0, h - 2);
    write!(out, "│").unwrap();
    terminal::move_to(out, w - 1, h - 2);
    write!(out, "│").unwrap();

    terminal::move_to(out, 0, h - 1);
    write!(out, "└{:─^1$}┘", "", w as usize - 2).unwrap();
}

pub fn draw_window(out: &mut impl Write, app: &Application) {
    terminal::move_to(out, app.position_x, app.position_y);
    write!(out, "┌{:─^1$}┐", format!(" {} ", app.title), app.width as usize - 2).unwrap();

    for i in 1..(app.height - 1) {
        terminal::move_to(out, app.position_x + 1, app.position_y + i);
        write!(out, "{:1$}", "", (app.width - 2) as usize).unwrap();
    }

    for i in 1..(app.height - 1) {
        terminal::move_to(out, app.position_x, app.position_y + i);
        write!(out, "│").unwrap();
        terminal::move_to(out, app.position_x + app.width - 1, app.position_y + i);
        write!(out, "│").unwrap();
    }

    terminal::move_to(out, app.position_x, app.position_y + app.height - 1);
    write!(out, "└{:─^1$}┘", "", app.width as usize - 2).unwrap();
}

pub fn draw_button(out: &mut impl Write, x: u16, y: u16, label: &str, hovered: bool) {
    terminal::move_to(out, x, y);
    if hovered {
        terminal::print_styled(out, label, terminal::FG_BLACK, terminal::BG_WHITE);
    } else {
        terminal::print_styled(out, label, terminal::FG_WHITE, terminal::BG_DARK_GREY);
    }
}
