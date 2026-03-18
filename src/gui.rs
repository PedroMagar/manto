use std::io::Write;

use crate::terminal;

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

pub fn draw_button(out: &mut impl Write, x: u16, y: u16, label: &str, hovered: bool) {
    terminal::move_to(out, x, y);
    if hovered {
        terminal::print_styled(out, label, terminal::FG_BLACK, terminal::BG_WHITE);
    } else {
        terminal::print_styled(out, label, terminal::FG_WHITE, terminal::BG_DARK_GREY);
    }
}
