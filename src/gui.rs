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

/// Desenha uma aba vertical de largura 3.
/// O título rola 1 char/segundo quando é maior que as linhas disponíveis.
pub fn draw_tab(out: &mut impl Write, x: u16, y: u16, height: u16, title: &str, scroll_offset: usize) {
    let content_rows = height.saturating_sub(2) as usize;
    // Adiciona 2 espaços de separação quando o título precisa rolar
    let padded = if title.chars().count() > content_rows {
        format!("{}  ", title)
    } else {
        title.to_string()
    };
    let chars: Vec<char> = padded.chars().collect();
    let len = chars.len();

    terminal::move_to(out, x, y);
    write!(out, "┌─").unwrap();

    for i in 0..content_rows {
        let ch = if len == 0 {
            ' '
        } else if len <= content_rows {
            chars.get(i).copied().unwrap_or(' ')
        } else {
            chars[(scroll_offset + i) % len]
        };
        terminal::move_to(out, x, y + 1 + i as u16);
        write!(out, "│{}", ch).unwrap();
    }

    terminal::move_to(out, x, y + height - 1);
    write!(out, "└─").unwrap();
}

/// Desenha uma scrollbar vertical em (x, top..=bot).
/// `total`   = número total de itens
/// `visible` = número de itens visíveis
/// `scroll`  = posição atual do scroll
/// Desenha a scrollbar apenas quando há mais itens do que os visíveis.
/// Sem setas — apenas trilha (░) e thumb (█).
pub fn draw_scrollbar(
    out: &mut impl Write,
    x: u16, top: u16, bot: u16,
    total: usize, visible: usize, scroll: usize,
) {
    if total <= visible || bot < top { return; }

    let track_len = (bot - top + 1) as usize;
    let max_scroll = total - visible;
    let thumb_len = (((visible as f32 / total as f32) * track_len as f32).max(1.0) as usize)
        .min(track_len);
    let available = track_len - thumb_len;
    let thumb_pos = if max_scroll > 0 { (scroll * available / max_scroll).min(available) } else { 0 };

    for row in top..=bot {
        terminal::move_to(out, x, row);
        write!(out, "░").unwrap();
    }
    for i in 0..thumb_len {
        terminal::move_to(out, x, top + thumb_pos as u16 + i as u16);
        write!(out, "█").unwrap();
    }
}

#[allow(dead_code)]
pub fn draw_button(out: &mut impl Write, x: u16, y: u16, label: &str, hovered: bool) {
    terminal::move_to(out, x, y);
    if hovered {
        terminal::print_styled(out, label, terminal::FG_BLACK, terminal::BG_WHITE);
    } else {
        terminal::print_styled(out, label, terminal::FG_WHITE, terminal::BG_DARK_GREY);
    }
}
