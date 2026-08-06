// Presentation layer: ANSI primitives, window chrome, pointer, and all
// screen drawing (desktop, status bar, tabs, panels, terminal content).

pub mod ansi;
pub mod pointer;
pub mod window;

mod panel;
mod render;
mod terminal_view;

pub use panel::draw_command_panel;
pub use render::{compute_render_state, render};
pub use terminal_view::{draw_shell_content, draw_terminal_content, terminal_content_width};

use std::io::Write;

/// Fixed status bar content (before the input area).
pub const STATUS_BAR_PREFIX: &str = " Start | .> ";
/// Start button text and x position within the bar row (column 0 = │).
pub const STATUS_START: &str = " Start ";
pub const STATUS_START_X: u16 = 1;
/// X column where the command input area starts (after the full prefix).
pub const CMD_INPUT_X: u16 = 1 + STATUS_BAR_PREFIX.len() as u16;
/// Number of virtual desktops shown in the status bar.
pub const DESKTOP_COUNT: usize = 4;
/// Visual width of the desktop area in the bar: "| N " × DESKTOP_COUNT = 16 columns.
pub const DESKTOP_AREA_LEN: u16 = DESKTOP_COUNT as u16 * 4;
/// Input prompt prefix in terminal windows.
pub const TERMINAL_INPUT_PREFIX: &str = " .> ";

pub fn draw_desktop(out: &mut impl Write, theme: u16, w: u16, h: u16, title: &str) {
    match theme {
        1 => {
            ansi::move_to(out, 0, 0);
            write!(out, "└{:─^1$}┘", format!(" {} ", title), w as usize - 2).unwrap();
        }
        2 => {
            ansi::move_to(out, 0, 0);
            write!(out, "┌{:─^1$}┐", format!(" {} ", title), w as usize - 2).unwrap();
            for i in 1..(h - 1) {
                ansi::move_to(out, 0, i);
                write!(out, "│").unwrap();
                ansi::move_to(out, w - 1, i);
                write!(out, "│").unwrap();
            }
        }
        _ => {}
    }
}

/// Draw the status bar (bottom 3 rows).
pub fn draw_status_bar(out: &mut impl Write, w: u16, h: u16, path: &str, panel_open: bool, current_desktop: usize) {
    let inner = (w - 2) as usize;
    let (cl, cr) = if panel_open { ('├', '┤') } else { ('┌', '┐') };
    ansi::move_to(out, 0, h - 3);
    if path.is_empty() {
        write!(out, "{}{:─<width$}{}", cl, "", cr, width = inner).unwrap();
    } else {
        let label = format!("── {} ", path);
        let fill = inner.saturating_sub(label.chars().count());
        write!(out, "{}{}{:─<width$}{}", cl, label, "", cr, width = fill).unwrap();
    }
    ansi::move_to(out, 0, h - 2);
    let prefix_len  = STATUS_BAR_PREFIX.chars().count();
    let desktop_len = DESKTOP_COUNT * 4; // "| N " × 4 = 16 visual columns
    let pad = inner.saturating_sub(prefix_len + desktop_len);
    write!(out, "│{}{:<pad$}", STATUS_BAR_PREFIX, "", pad = pad).unwrap();
    for d in 1..=DESKTOP_COUNT {
        write!(out, "|").unwrap();
        if d == current_desktop {
            write!(out, "{} {} {}", ansi::REVERSE, d, ansi::RESET).unwrap();
        } else {
            write!(out, " {} ", d).unwrap();
        }
    }
    write!(out, "│").unwrap();
    ansi::move_to(out, 0, h - 1);
    write!(out, "└{:─<1$}┘", "", inner).unwrap();
}

/// Return the 1-based index of the desktop button at (x, y), or None.
/// Each button occupies 3 visual columns ` N ` separated by `|`.
/// Layout (left to right): `| 1 | 2 | 3 | 4 │`
pub fn desktop_at(x: u16, y: u16, w: u16, h: u16) -> Option<usize> {
    if y != h - 2 { return None; }
    let base_x = w.saturating_sub(1 + DESKTOP_AREA_LEN); // column of the first '|'
    for d in 1..=DESKTOP_COUNT {
        let sep_x = base_x + (d as u16 - 1) * 4;
        let btn_start = sep_x + 1;
        let btn_end   = sep_x + 3;
        if x >= btn_start && x <= btn_end {
            return Some(d);
        }
    }
    None
}

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

/// Draw a vertical tab of width 2. The title scrolls when longer than the
/// available rows.
pub fn draw_tab(out: &mut impl Write, x: u16, y: u16, height: u16, title: &str, scroll_offset: usize) {
    let content_rows = height.saturating_sub(2) as usize;
    ansi::move_to(out, x, y);
    write!(out, "┌─").unwrap();
    for i in 0..content_rows {
        let ch = tab_content_char(title, content_rows, i, scroll_offset);
        ansi::move_to(out, x, y + 1 + i as u16);
        write!(out, "│{}", ch).unwrap();
    }
    ansi::move_to(out, x, y + height - 1);
    write!(out, "└─").unwrap();
}

/// Return the visible character at (x, y) of a tab.
pub fn tab_char_at(tab_x: u16, tab_y: u16, tab_h: u16, title: &str, x: u16, y: u16, scroll_offset: usize) -> char {
    let content_rows = tab_h.saturating_sub(2) as usize;
    if y == tab_y || y == tab_y + tab_h - 1 {
        return if x == tab_x { if y == tab_y { '┌' } else { '└' } } else { '─' };
    }
    if x == tab_x { return '│'; }
    tab_content_char(title, content_rows, (y - tab_y - 1) as usize, scroll_offset)
}

/// Compute (thumb_pos, thumb_len) for a scrollbar.
pub fn scrollbar_thumb(track_len: usize, total: usize, visible: usize, scroll: usize) -> (usize, usize) {
    let thumb_len = (((visible as f32 / total as f32) * track_len as f32).max(1.0) as usize)
        .min(track_len);
    let available = track_len - thumb_len;
    let max_scroll = total - visible;
    let thumb_pos = if max_scroll > 0 { (scroll * available / max_scroll).min(available) } else { 0 };
    (thumb_pos, thumb_len)
}

/// Draw the vertical scrollbar at (x, top..=bot). Track (░) and thumb (█) only.
pub fn draw_scrollbar(
    out: &mut impl Write,
    x: u16, top: u16, bot: u16,
    total: usize, visible: usize, scroll: usize,
) {
    if total <= visible || bot < top { return; }
    let track_len = (bot - top + 1) as usize;
    let (thumb_pos, thumb_len) = scrollbar_thumb(track_len, total, visible, scroll);
    for row in top..=bot {
        ansi::move_to(out, x, row);
        write!(out, "░").unwrap();
    }
    for i in 0..thumb_len {
        ansi::move_to(out, x, top + thumb_pos as u16 + i as u16);
        write!(out, "█").unwrap();
    }
}

