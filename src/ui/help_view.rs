// Help window content: draws the wrapped crib sheet inside the window,
// with a vertical scrollbar when the text overflows.

use std::io::Write;

use super::ansi;
use super::window::Window;
use crate::help::{HelpState, wrapped, wrapped_count};

pub fn draw_help_content(out: &mut impl Write, win: &Window, state: &HelpState) {
    let lx = win.position_x;
    let ty = win.position_y;
    let inner_w = (win.width as usize).saturating_sub(2);
    let inner_h = (win.height as usize).saturating_sub(2);
    if inner_w == 0 || inner_h == 0 {
        return;
    }

    let rows = wrapped(&state.lines, inner_w);
    let max_scroll = rows.len().saturating_sub(inner_h);
    let scroll = state.scroll.min(max_scroll);

    for row in 0..inner_h {
        let idx = scroll + row;
        let text = rows.get(idx).map(|s| s.as_str()).unwrap_or("");
        ansi::move_to(out, lx + 1, ty + 1 + row as u16);
        write!(out, "{text:<inner_w$}").unwrap();
    }

    if rows.len() > inner_h {
        super::draw_scrollbar(
            out,
            lx + inner_w as u16,
            ty + 1,
            ty + inner_h as u16,
            rows.len(),
            inner_h,
            scroll,
        );
    }
}

/// Larger scroll bound helper used by the input handlers (wheel, arrows):
/// the maximum `scroll` value that keeps content on screen.
pub fn help_max_scroll(lines: &[String], inner_w: usize, inner_h: usize) -> usize {
    wrapped_count(lines, inner_w).saturating_sub(inner_h)
}
