use std::io::Write;

use crate::terminal;

/// Conteúdo fixo da barra de status (antes da área de input).
pub const STATUS_BAR_PREFIX: &str = " Start | .> ";
/// Texto e posição x do botão Start dentro da linha da barra (coluna 0 = │).
/// Inclui os espaços de padding para a área de hover/click.
pub const STATUS_START: &str = " Start ";
pub const STATUS_START_X: u16 = 1; // logo após │

pub fn draw_desktop(out: &mut impl Write, theme: u16, w: u16, h: u16, title: &str, path: &str) {
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

    // Barra de status inferior
    let inner = (w - 2) as usize;
    terminal::move_to(out, 0, h - 3);
    if path.is_empty() {
        write!(out, "┌{:─<1$}┐", "", inner).unwrap();
    } else {
        let label = format!("── {} ", path);
        let fill = inner.saturating_sub(label.len());
        write!(out, "┌{}{:─<2$}┐", label, "", fill).unwrap();
    }

    terminal::move_to(out, 0, h - 2);
    write!(out, "│{:<1$}│", STATUS_BAR_PREFIX, inner).unwrap();

    terminal::move_to(out, 0, h - 1);
    write!(out, "└{:─<1$}┘", "", inner).unwrap();
}

/// Retorna o caractere do conteúdo de uma aba na linha `row` (0-indexed dentro dos content rows).
fn tab_content_char(title: &str, content_rows: usize, row: usize, scroll_offset: usize) -> char {
    let padded = if title.chars().count() > content_rows {
        format!("{}  ", title)
    } else {
        title.to_string()
    };
    let chars: Vec<char> = padded.chars().collect();
    let len = chars.len();
    if len == 0 { ' ' }
    else if len <= content_rows { chars.get(row).copied().unwrap_or(' ') }
    else { chars[(scroll_offset + row) % len] }
}

/// Desenha uma aba vertical de largura 2.
/// O título rola 1 char/segundo quando é maior que as linhas disponíveis.
pub fn draw_tab(out: &mut impl Write, x: u16, y: u16, height: u16, title: &str, scroll_offset: usize) {
    let content_rows = height.saturating_sub(2) as usize;

    terminal::move_to(out, x, y);
    write!(out, "┌─").unwrap();

    for i in 0..content_rows {
        let ch = tab_content_char(title, content_rows, i, scroll_offset);
        terminal::move_to(out, x, y + 1 + i as u16);
        write!(out, "│{}", ch).unwrap();
    }

    terminal::move_to(out, x, y + height - 1);
    write!(out, "└─").unwrap();
}

/// Retorna o caractere visível na posição (x, y) de uma aba.
pub fn tab_char_at(tab_x: u16, tab_y: u16, tab_h: u16, title: &str, x: u16, y: u16, scroll_offset: usize) -> char {
    let content_rows = tab_h.saturating_sub(2) as usize;
    if y == tab_y || y == tab_y + tab_h - 1 {
        return if x == tab_x { if y == tab_y { '┌' } else { '└' } } else { '─' };
    }
    if x == tab_x { return '│'; }
    tab_content_char(title, content_rows, (y - tab_y - 1) as usize, scroll_offset)
}

/// Calcula (thumb_pos, thumb_len) para uma scrollbar.
pub fn scrollbar_thumb(track_len: usize, total: usize, visible: usize, scroll: usize) -> (usize, usize) {
    let thumb_len = (((visible as f32 / total as f32) * track_len as f32).max(1.0) as usize)
        .min(track_len);
    let available = track_len - thumb_len;
    let max_scroll = total - visible;
    let thumb_pos = if max_scroll > 0 { (scroll * available / max_scroll).min(available) } else { 0 };
    (thumb_pos, thumb_len)
}

/// Desenha a scrollbar vertical em (x, top..=bot).
/// `total`   = número total de itens
/// `visible` = número de itens visíveis
/// `scroll`  = posição atual do scroll
/// Sem setas — apenas trilha (░) e thumb (█).
pub fn draw_scrollbar(
    out: &mut impl Write,
    x: u16, top: u16, bot: u16,
    total: usize, visible: usize, scroll: usize,
) {
    if total <= visible || bot < top { return; }

    let track_len = (bot - top + 1) as usize;
    let (thumb_pos, thumb_len) = scrollbar_thumb(track_len, total, visible, scroll);

    for row in top..=bot {
        terminal::move_to(out, x, row);
        write!(out, "░").unwrap();
    }
    for i in 0..thumb_len {
        terminal::move_to(out, x, top + thumb_pos as u16 + i as u16);
        write!(out, "█").unwrap();
    }
}

