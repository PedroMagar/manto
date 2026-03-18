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
    write!(out, "└{:─^1$}", "", app.width as usize - 2).unwrap();
    write!(out, "┘").unwrap();
}

/// Desenha o DELTA do novo tamanho sobre o frame já renderizado, sem apagar o original.
///
/// Regras:
///  - Topo: extensão à direita só se new_w > app.width (não toca o título)
///  - Coluna direita: sempre, em nova posição, para todas as linhas internas do preview
///  - Fundo:
///      • Se new_h == app.height && new_w > app.width → estende após o ┘ original
///      • Caso contrário → desenha nova borda completa na nova posição
pub fn draw_window_preview(out: &mut impl Write, app: &Application, new_w: u16, new_h: u16) {
    if new_w == app.width && new_h == app.height {
        return;
    }

    let orig_right_x  = app.position_x + app.width - 1;
    let orig_bottom_y = app.position_y + app.height - 1;
    let new_right_x   = app.position_x + new_w - 1;
    let new_bottom_y  = app.position_y + new_h - 1;

    // Extensão da borda superior (só se a largura cresceu)
    if new_w > app.width {
        terminal::move_to(out, orig_right_x + 1, app.position_y);
        for _ in 0..(new_w - app.width - 1) {
            write!(out, "─").unwrap();
        }
        write!(out, "┐").unwrap();
    }

    // Coluna direita na nova posição — sempre, para todas as linhas internas do preview
    for i in 1..(new_h - 1) {
        terminal::move_to(out, new_right_x, app.position_y + i);
        write!(out, "│").unwrap();
    }

    // Borda inferior
    if new_h == app.height && new_w > app.width {
        // Mesma altura, mais larga: estende após o ┘ original (preserva o ┘)
        terminal::move_to(out, orig_right_x + 1, orig_bottom_y);
        for _ in 0..(new_w - app.width - 1) {
            write!(out, "─").unwrap();
        }
        write!(out, "┼").unwrap();
    } else {
        // Altura diferente ou mais estreita: nova borda completa na nova posição
        terminal::move_to(out, app.position_x, new_bottom_y);
        write!(out, "└{:─^1$}", "", new_w as usize - 2).unwrap();
        write!(out, "┼").unwrap();
    }
}

pub fn draw_button(out: &mut impl Write, x: u16, y: u16, label: &str, hovered: bool) {
    terminal::move_to(out, x, y);
    if hovered {
        terminal::print_styled(out, label, terminal::FG_BLACK, terminal::BG_WHITE);
    } else {
        terminal::print_styled(out, label, terminal::FG_WHITE, terminal::BG_DARK_GREY);
    }
}
