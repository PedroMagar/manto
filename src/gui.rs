use crossterm::{cursor, execute, style::{self, Stylize}};

use crate::Application;

pub fn draw_desktop(stdout: &mut std::io::Stdout, theme: u16, w: u16, h: u16, title: &str) {
    match theme {
        1 => {
                execute!(
                    stdout,
                    cursor::MoveTo(0, 0),
                    style::Print(format!("└{:─^1$}┘", format!(" {} ", title), w as usize - 2))
                ).unwrap();
                
                execute!(
                    stdout,
                    cursor::MoveTo(0, h-2),
                    style::Print("│"),
                    cursor::MoveTo(w - 1, h-2),
                    style::Print("│")
                ).unwrap();
            }
        2 => {
                execute!(
                    stdout,
                    cursor::MoveTo(0, 0),
                    style::Print(format!("┌{:─^1$}┐", format!(" {} ", title), w as usize - 2))
                ).unwrap();

                for i in 1..(h - 1) {
                    execute!(
                        stdout,
                        cursor::MoveTo(0, i),
                        style::Print("│"),
                        cursor::MoveTo(w - 1, i),
                        style::Print("│")
                    ).unwrap();
                }
            }
        _ => {}
    }

    execute!(
        stdout,
        cursor::MoveTo(0, h-3),
        style::Print(format!("┌{:─^1$}┐", "", w as usize - 2))
    ).unwrap();

    execute!(
        stdout,
        cursor::MoveTo(0, h-2),
        style::Print("│"),
        cursor::MoveTo(w - 1, h-2),
        style::Print("│")
    ).unwrap();

    execute!(
        stdout,
        cursor::MoveTo(0, h - 1),
        style::Print(format!("└{:─^1$}┘", "", w as usize - 2))
    ).unwrap();
}

pub fn draw_window(stdout: &mut std::io::Stdout, app: &Application) {
    execute!(
        stdout,
        cursor::MoveTo(app.position_x, app.position_y),
        style::Print(format!("┌{:─^1$}┐", format!(" {} ", app.title), app.width as usize - 2))
    ).unwrap();
    
    for i in 1..(app.height - 1) {
        for j in 1..(app.width - 1) {
            execute!(
                stdout,
                cursor::MoveTo(app.position_x + j, app.position_y + i),
                style::Print(" ")
            ).unwrap();
        }
    }

    for i in 1..(app.height - 1) {
        execute!(
            stdout,
            cursor::MoveTo(app.position_x, app.position_y + i),
            style::Print("│"),
            cursor::MoveTo(app.position_x+(app.width - 1), app.position_y + i),
            style::Print("│")
        ).unwrap();
    }

    execute!(
        stdout,
        cursor::MoveTo(app.position_x, app.position_y + (app.height - 1)),
        style::Print(format!("└{:─^1$}┘", "", app.width as usize - 2))
    ).unwrap();
}

pub fn draw_button(stdout: &mut std::io::Stdout, x: u16, y: u16, label: &str, hovered: bool) {
    let styled = if hovered {
        label.black().on_white()
    } else {
        label.white().on_dark_grey()
    };
    execute!(stdout, cursor::MoveTo(x, y), style::PrintStyledContent(styled)).unwrap();
}
